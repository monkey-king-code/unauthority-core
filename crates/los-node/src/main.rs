// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// UNAUTHORITY (LOS) - VALIDATOR NODE
//
// Main entry point for the los-node binary.
// Runs the full validator: REST API, gRPC, P2P gossip, Tor hidden service,
// consensus engine, reward distribution, and state persistence.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#![recursion_limit = "512"]

use base64::Engine as _;
use los_consensus::abft::ABFTConsensus; // aBFT engine for consensus stats & safety validation
use los_consensus::checkpoint::{
    CheckpointManager, CheckpointSignature, FinalityCheckpoint, PendingCheckpoint,
    CHECKPOINT_INTERVAL,
}; // Finality checkpoints
use los_consensus::slashing::SlashingManager; // Slashing enforcement
use los_consensus::voting::calculate_voting_power; // Linear voting: Power = Stake
use los_core::pow_mint::{verify_mining_hash, MiningState}; // PoW Mint distribution engine
use los_core::validator_rewards::ValidatorRewardPool;
use los_core::{
    AccountState, Block, BlockType, Ledger, CIL_PER_LOS, MIN_VALIDATOR_REGISTER_CIL,
    MIN_VALIDATOR_STAKE_CIL,
};
use los_network::{LosNode, NetworkEvent};
use los_vm::{dex_registry, token_registry, ContractCall, WasmEngine};
use rate_limiter::{filters::rate_limit, RateLimiter};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::io::{self, AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;
use zeroize::Zeroizing;

/// Safe mutex lock that recovers from poisoned state instead of panicking.
/// When a thread panics while holding a lock, the Mutex becomes "poisoned".
/// Instead of cascading panics, we recover the inner data.
fn safe_lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            eprintln!("⚠️ WARNING: Mutex was poisoned, recovering...");
            poisoned.into_inner()
        }
    }
}

use std::fs;
use std::time::{Duration, Instant};

// Named constants for consensus thresholds (no more magic numbers)
/// Linear voting power threshold for send confirmation (production)
const SEND_CONSENSUS_THRESHOLD: u128 = 20_000;
/// Minimum DISTINCT voters required for consensus on production.
/// Prevents single-validator self-consensus even if one validator has enough
/// voting power to exceed the threshold alone.
///
/// Dynamic minimum based on active validator count.
/// Uses standard BFT quorum: 2f+1 where f = (n-1)/3.
/// With 4 validators: f=1, quorum=3 (75% participation).
/// With 7 validators: f=2, quorum=5 (71% participation).
/// With 10 validators: f=3, quorum=7 (70% participation).
/// Floor of 2 ensures at least two independent validators on bootstrap.
///
/// IMPORTANT: Sender does NOT self-vote in CONFIRM_RES/VOTE_RES flow.
/// So max possible voters = n-1. With n=4, max voters=3, quorum=3 → works.
/// Old formula ceil(n*2/3)+1 produced 4 for n=4 → impossible (sender excluded).
fn min_distinct_voters(active_validator_count: usize) -> usize {
    if active_validator_count <= 1 {
        return 1; // Single-validator network (bootstrap only)
    }
    // Standard BFT: f = (n-1)/3, quorum = 2f+1
    let f = (active_validator_count - 1) / 3;
    let bft_quorum = 2 * f + 1;
    bft_quorum.max(2)
}
/// Minimum threshold for testnet functional mode (bypasses real consensus)
/// MAINNET: This constant exists but is never reachable — testnet_config forces Production level.
const TESTNET_FUNCTIONAL_THRESHOLD: u128 = 1;
/// Initial testnet balance for functional testing (1000 LOS)
/// MAINNET: Never used — functional testnet path is unreachable on mainnet builds.
const TESTNET_INITIAL_BALANCE: u128 = 1000 * CIL_PER_LOS;
/// Total supply: 21,936,236 LOS (protocol constant, validated against genesis on mainnet)
const TOTAL_SUPPLY_LOS: u128 = 21_936_236;
const TOTAL_SUPPLY_CIL: u128 = TOTAL_SUPPLY_LOS * CIL_PER_LOS;
/// Testnet faucet payout per request (5,000 LOS).
/// MAINNET: Faucet endpoint is disabled on mainnet builds — this value is never used.
const FAUCET_AMOUNT_CIL: u128 = 5_000 * CIL_PER_LOS;

mod db; // Sled database persistence
mod genesis;
mod grpc_server;
mod mempool; // Transaction mempool
mod metrics; // Prometheus metrics
mod rate_limiter; // Anti-spam rate limiter
mod testnet_config;
mod tor_service; // Automatic Tor Hidden Service generation
mod validator_api; // Validator key management (generate, import)
mod validator_rewards;
use db::LosDatabase;
use metrics::LosMetrics;
use warp::Filter;

const LEDGER_FILE: &str = "ledger_state.json";

// Race condition protection: Atomic flags for save state
static SAVE_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static SAVE_DIRTY: AtomicBool = AtomicBool::new(false);

/// Create a JSON API reply with automatic HTTP status code based on body content.
///
/// Rules:
/// - If body has `"code": N` → uses N as HTTP status code
/// - If body has `"status": "error"` without code → HTTP 400
/// - If body has `"error"` key → HTTP 400
/// - Otherwise → HTTP 200
///
/// This replaces bare `warp::reply::json()` calls that always return HTTP 200,
/// ensuring error responses get proper HTTP 4xx/5xx status codes.
fn api_json(body: serde_json::Value) -> warp::reply::WithStatus<warp::reply::Json> {
    let code = body
        .get("code")
        .and_then(|c| c.as_u64())
        .map(|c| c as u16)
        .unwrap_or_else(|| {
            if body.get("status").and_then(|s| s.as_str()) == Some("error")
                || body.get("error").is_some()
            {
                400
            } else {
                200
            }
        });
    let status = warp::http::StatusCode::from_u16(code)
        .unwrap_or(warp::http::StatusCode::INTERNAL_SERVER_ERROR);
    warp::reply::with_status(warp::reply::json(&body), status)
}

/// Format seconds into human-readable uptime string (e.g. "3d 12h 5m").
fn format_uptime(total_secs: u64) -> String {
    let days = total_secs / 86400;
    let hours = (total_secs % 86400) / 3600;
    let mins = (total_secs % 3600) / 60;
    if days > 0 {
        format!("{}d {}h {}m", days, hours, mins)
    } else if hours > 0 {
        format!("{}h {}m", hours, mins)
    } else {
        format!("{}m", mins)
    }
}

/// Insert a validator endpoint, deduplicating by host address.
/// One host can only map to ONE current LOS address — if a validator
/// restarts with a new keypair, the old stale entry is removed.
/// The host can be any format: .onion, IP:port, domain:port.
fn insert_validator_endpoint(ve: &mut HashMap<String, String>, address: String, host: String) {
    // Remove any existing entry that maps to the same host
    // (stale address from previous restart of the same node)
    ve.retain(|existing_addr, existing_host| {
        if existing_host == &host && existing_addr != &address {
            println!(
                "🧹 Replacing stale endpoint: {} → {} (new: {})",
                get_short_addr(existing_addr),
                host,
                get_short_addr(&address)
            );
            false
        } else {
            true
        }
    });
    ve.insert(address, host);
}

/// Get this node's announced host address from environment.
/// Checks LOS_HOST_ADDRESS first (any format: IP, domain, .onion),
/// then falls back to LOS_ONION_ADDRESS for backward compatibility.
fn get_node_host_address() -> Option<String> {
    std::env::var("LOS_HOST_ADDRESS")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::var("LOS_ONION_ADDRESS")
                .ok()
                .filter(|s| !s.is_empty())
        })
}

/// Ensure a host string includes a port suffix.
/// If the host already contains `:port`, returns it unchanged.
/// Otherwise appends `:default_port`.
/// Examples:
///   `ensure_host_port("abc.onion", 3030)` → `"abc.onion:3030"`
///   `ensure_host_port("1.2.3.4:7030", 3030)` → `"1.2.3.4:7030"` (unchanged)
fn ensure_host_port(host: &str, default_port: u16) -> String {
    // Check if host already ends with `:digits`
    if let Some(last) = host.rsplit(':').next() {
        if last.chars().all(|c| c.is_ascii_digit()) && !last.is_empty() {
            return host.to_string(); // already has a port
        }
    }
    format!("{}:{}", host, default_port)
}

/// Resolve host address from a genesis wallet entry.
/// Prefers host_address, falls back to onion_address.
/// Appends rest_port if the host doesn't already have a port suffix.
fn resolve_genesis_host(wallet: &genesis::GenesisWallet) -> Option<String> {
    let host = wallet
        .host_address
        .as_ref()
        .filter(|s| !s.is_empty())
        .or(wallet.onion_address.as_ref().filter(|s| !s.is_empty()))
        .cloned();
    match host {
        Some(h) => {
            let port = wallet.rest_port.unwrap_or(3030);
            Some(ensure_host_port(&h, port))
        }
        None => None,
    }
}

/// Bootstrap nodes — resolved from env var OR auto-discovered from genesis config.
///
/// Priority:
///   1. LOS_BOOTSTRAP_NODES env var (operator override)
///   2. genesis_config.json bootstrap_nodes[].host_address or onion_address + p2p_port
///
/// Returns: Vec of dial-able addresses (onion:port, ip:port, or /ip4/.../tcp/...)
fn get_bootstrap_nodes() -> Vec<String> {
    // Priority 1: Explicit env var (operator override, e.g. for local dev or custom topology)
    if let Ok(val) = std::env::var("LOS_BOOTSTRAP_NODES") {
        if !val.trim().is_empty() {
            let nodes: Vec<String> = val
                .split(',')
                .map(|s| {
                    let trimmed = s.trim().to_string();
                    // Convert host:port format to libp2p multiaddr format.
                    // "127.0.0.1:4001" → "/ip4/127.0.0.1/tcp/4001"
                    // Already-valid multiaddrs (starting with /) are left as-is.
                    // .onion addresses are also left as-is for Tor handling.
                    if !trimmed.starts_with('/') && !trimmed.contains(".onion") {
                        if let Some((host, port)) = trimmed.split_once(':') {
                            format!("/ip4/{}/tcp/{}", host, port)
                        } else {
                            trimmed
                        }
                    } else {
                        trimmed
                    }
                })
                .filter(|s| !s.is_empty())
                .collect();
            if !nodes.is_empty() {
                return nodes;
            }
        }
    }

    // Priority 2: Auto-discover from genesis config bootstrap_nodes[].host_address or onion_address
    let genesis_path = if los_core::is_mainnet_build() {
        "genesis_config.json"
    } else {
        "testnet-genesis/testnet_wallets.json"
    };
    if let Ok(json_data) = std::fs::read_to_string(genesis_path) {
        if let Ok(config) = serde_json::from_str::<genesis::GenesisConfig>(&json_data) {
            if let Some(ref nodes) = config.bootstrap_nodes {
                // Filter out our own host address to avoid self-dialing
                let our_host = get_node_host_address();
                let peers: Vec<String> = nodes
                    .iter()
                    .filter_map(|node| {
                        resolve_genesis_host(node).and_then(|host| {
                            // Skip our own address
                            if let Some(ref ours) = our_host {
                                if &host == ours {
                                    return None;
                                }
                            }
                            let port = node.p2p_port.unwrap_or(4001);
                            // .onion addresses: return as host:port
                            // IP/domain addresses: convert to multiaddr
                            if host.contains(".onion") {
                                Some(format!("{}:{}", host, port))
                            } else if host.starts_with('/') {
                                Some(host) // Already a multiaddr
                            } else {
                                // Assume IP or domain
                                Some(format!("/ip4/{}/tcp/{}", host, port))
                            }
                        })
                    })
                    .collect();
                if !peers.is_empty() {
                    println!(
                        "📡 Auto-discovered {} bootstrap peers from genesis config",
                        peers.len()
                    );
                    return peers;
                }
            }
        }
    }

    Vec::new()
}

// Request body structure for sending LOS
#[derive(serde::Deserialize, serde::Serialize)]
struct SendRequest {
    from: Option<String>, // Sender address (if empty, use node's address)
    target: String,
    amount: u128,
    amount_cil: Option<u128>, // Amount already in CIL (skips ×CIL_PER_LOS). Used by client-signed blocks.
    signature: Option<String>, // Client-provided signature (if present, validate instead of signing)
    public_key: Option<String>, // Sender's public key (hex-encoded, REQUIRED for signature verification)
    previous: Option<String>,   // Previous block hash (for client-side signing)
    work: Option<u64>,          // PoW nonce (if client pre-computed)
    timestamp: Option<u64>,     // Client timestamp (used when client_signed to match signing_hash)
    fee: Option<u128>,          // Client fee (used when client_signed to match signing_hash)
}

#[derive(serde::Deserialize, serde::Serialize)]
struct DeployContractRequest {
    owner: String,
    bytecode: String, // base64 encoded WASM
    initial_state: Option<BTreeMap<String, String>>,
    amount_cil: Option<u128>,   // Initial CIL funding for contract
    signature: Option<String>,  // Client-signed: Dilithium5 sig
    public_key: Option<String>, // Client-signed: deployer's pubkey (hex)
    previous: Option<String>,   // Client-signed: previous block hash
    work: Option<u64>,          // Client-signed: PoW nonce
    timestamp: Option<u64>,     // Client-signed: block timestamp
    fee: Option<u128>,          // Client-signed: fee in CIL
}

#[derive(serde::Deserialize, serde::Serialize)]
struct CallContractRequest {
    contract_address: String,
    function: String,
    args: Vec<String>,
    gas_limit: Option<u64>,
    caller: Option<String>,    // Caller address (if empty, use node's address)
    amount_cil: Option<u128>,  // CIL to send to contract (msg.value)
    signature: Option<String>, // Client-signed: Dilithium5 sig
    public_key: Option<String>, // Client-signed: caller's pubkey (hex)
    previous: Option<String>,  // Client-signed: previous block hash
    work: Option<u64>,         // Client-signed: PoW nonce
    timestamp: Option<u64>,    // Client-signed: block timestamp
    fee: Option<u128>,         // Client-signed: fee in CIL
}

/// Per-address endpoint rate limiter
/// Tracks request timestamps per address for each endpoint type
#[derive(Clone)]
pub struct EndpointRateLimiter {
    /// Map of address -> list of request timestamps
    requests: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    /// Maximum requests allowed in the time window
    max_requests: u32,
    /// Time window duration
    window: Duration,
    /// Last time we cleaned up old entries
    last_cleanup: Arc<Mutex<Instant>>,
}

impl EndpointRateLimiter {
    pub fn new(max_requests: u32, window_secs: u64) -> Self {
        Self {
            requests: Arc::new(Mutex::new(HashMap::new())),
            max_requests,
            window: Duration::from_secs(window_secs),
            last_cleanup: Arc::new(Mutex::new(Instant::now())),
        }
    }

    /// Check if the address is within rate limit. Returns Ok(()) or Err(seconds until next allowed request).
    pub fn check_and_record(&self, address: &str) -> Result<(), u64> {
        let now = Instant::now();
        let mut requests = match self.requests.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(), // Recover from poisoned mutex
        };

        // Periodic cleanup (every 60s): remove entries older than window
        {
            let mut last = match self.last_cleanup.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            if now.duration_since(*last) > Duration::from_secs(60) {
                requests.retain(|_, timestamps| {
                    timestamps.retain(|t| now.duration_since(*t) < self.window);
                    !timestamps.is_empty()
                });
                *last = now;
            }
        }

        let timestamps = requests.entry(address.to_string()).or_default();

        // Remove expired timestamps for this address
        timestamps.retain(|t| now.duration_since(*t) < self.window);

        if timestamps.len() >= self.max_requests as usize {
            // Calculate wait time from oldest relevant request
            let oldest = timestamps[0];
            let elapsed = now.duration_since(oldest);
            let wait = if self.window > elapsed {
                (self.window - elapsed).as_secs() + 1
            } else {
                1
            };
            return Err(wait);
        }

        timestamps.push(now);
        Ok(())
    }

    /// Check rate limit WITHOUT recording. Use with record_success() for
    /// endpoints where the cooldown should only apply on successful operations.
    pub fn check_only(&self, address: &str) -> Result<(), u64> {
        let now = Instant::now();
        let mut requests = match self.requests.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        let timestamps = requests.entry(address.to_string()).or_default();
        timestamps.retain(|t| now.duration_since(*t) < self.window);

        if timestamps.len() >= self.max_requests as usize {
            let oldest = timestamps[0];
            let elapsed = now.duration_since(oldest);
            let wait = if self.window > elapsed {
                (self.window - elapsed).as_secs() + 1
            } else {
                1
            };
            return Err(wait);
        }
        Ok(())
    }

    /// Record a successful operation. Call AFTER the operation succeeds.
    pub fn record_success(&self, address: &str) {
        let now = Instant::now();
        let mut requests = match self.requests.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let timestamps = requests.entry(address.to_string()).or_default();
        timestamps.push(now);
    }
}

// Helper: sign message and hex-encode — returns Result instead of panicking.
// MAINNET SAFETY: A signing failure (corrupted key) no longer crashes the node.
fn try_sign_hex(msg: &[u8], sk: &[u8]) -> Result<String, String> {
    los_crypto::sign_message(msg, sk)
        .map(hex::encode)
        .map_err(|e| format!("Signing failed (key corrupted?): {:?}", e))
}

// Helper to inject state into route handlers
fn with_state<T: Clone + Send>(
    state: T,
) -> impl Filter<Extract = (T,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || state.clone())
}

/// Bundles all dependencies for the REST API server,
/// avoiding the `clippy::too_many_arguments` warning.
#[allow(clippy::type_complexity)]
pub struct ApiServerConfig {
    pub ledger: Arc<Mutex<Ledger>>,
    pub tx_out: mpsc::Sender<String>,
    pub pending_sends: Arc<Mutex<HashMap<String, (Block, u128)>>>,
    pub address_book: Arc<Mutex<HashMap<String, String>>>,
    pub my_address: String,
    pub secret_key: Zeroizing<Vec<u8>>,
    pub api_port: u16,
    pub metrics: Arc<LosMetrics>,
    pub database: Arc<LosDatabase>,
    pub slashing_manager: Arc<Mutex<SlashingManager>>,
    pub node_public_key: Vec<u8>,
    /// Bootstrap validator addresses loaded from genesis config (NOT hardcoded).
    /// On mainnet: from genesis_config.json bootstrap_nodes.
    /// On testnet: from testnet_wallets.json wallets with role="validator".
    pub bootstrap_validators: Vec<String>,
    /// Validator reward pool — epoch-based reward distribution engine
    pub reward_pool: Arc<Mutex<ValidatorRewardPool>>,
    /// Known validator host endpoints: validator_address → host_address
    /// Host can be .onion, IP:port, or domain:port. Tor is optional.
    /// Populated from LOS_HOST_ADDRESS/LOS_ONION_ADDRESS (self), VALIDATOR_REG gossip, and PEER_LIST exchange.
    pub validator_endpoints: Arc<Mutex<HashMap<String, String>>>,
    /// Mempool: tracks pending transactions with priority ordering and expiration.
    pub mempool_pool: Arc<Mutex<mempool::Mempool>>,
    /// aBFT Consensus Engine — shared between API server and main event loop
    pub abft_consensus: Arc<Mutex<ABFTConsensus>>,
    /// Tracks wallet addresses registered as validators through this node's API.
    /// Heartbeat loop records heartbeats for these addresses so they earn rewards.
    pub local_registered_validators: Arc<Mutex<HashSet<String>>>,
    /// WASM Smart Contract Engine — shared between API server and P2P event loop.
    /// Contracts deployed via REST are persisted to sled and replicated via gossip.
    pub wasm_engine: Arc<WasmEngine>,
    /// PoW Mint engine — tracks mining epochs, difficulty, and miner deduplication.
    pub mining_state: Arc<Mutex<MiningState>>,
    /// Whether background PoW mining is enabled (--mine flag).
    pub enable_mining: bool,
    /// Number of mining threads (--mine-threads N).
    pub mining_threads: usize,
}

#[allow(clippy::type_complexity)]
pub async fn start_api_server(cfg: ApiServerConfig) {
    let ApiServerConfig {
        ledger,
        tx_out,
        pending_sends,
        address_book,
        my_address,
        secret_key,
        api_port,
        metrics,
        database,
        slashing_manager,
        node_public_key,
        bootstrap_validators,
        reward_pool,
        validator_endpoints,
        mempool_pool,
        abft_consensus,
        local_registered_validators,
        wasm_engine,
        mining_state,
        enable_mining,
        mining_threads,
    } = cfg;
    // Rate Limiter: 100 req/sec per IP, burst 200
    let limiter = RateLimiter::new(100, Some(200));
    let rate_limit_filter = rate_limit(limiter.clone());

    // Track node startup time for uptime calculation
    let start_time = std::time::Instant::now();

    // Per-address endpoint rate limiters
    let send_limiter = Arc::new(EndpointRateLimiter::new(10, 60)); // /send: 10 tx per 60 seconds
    let faucet_limiter = Arc::new(EndpointRateLimiter::new(1, 120)); // /faucet: 1 per 2 minutes (testnet)

    // aBFT Consensus Engine — passed from main() via ApiServerConfig, shared with event loop
    // Initialize shared secret and validator set
    {
        let mut abft = safe_lock(&abft_consensus);
        // Set shared secret for MAC authentication (SHA3-256 of node's secret key)
        use sha3::{Digest as Sha3Digest, Sha3_256 as Sha3256Hasher};
        let mut hasher = Sha3256Hasher::new();
        hasher.update(&*secret_key);
        hasher.update(b"LOS_CONSENSUS_MAC_V1");
        abft.set_shared_secret(hasher.finalize().to_vec());

        // Populate validator set with real addresses for leader selection
        let l = safe_lock(&ledger);
        let mut validators: Vec<String> = l
            .accounts
            .iter()
            .filter(|(_, a)| a.balance >= MIN_VALIDATOR_REGISTER_CIL && a.is_validator)
            .map(|(addr, _)| addr.clone())
            .collect();
        validators.sort(); // Deterministic ordering across all nodes
        abft.update_validator_set(validators);
        drop(l);

        println!(
            "🔗 aBFT Consensus: n={}, f={}, quorum={}, safety={}",
            abft.total_validators,
            abft.f_max_faulty,
            abft.get_statistics().quorum_threshold,
            abft.is_byzantine_safe(0)
        );
    }

    // 1. GET /bal/:address
    let l_bal = ledger.clone();
    let balance_route = warp::path!("bal" / String).and(with_state(l_bal)).map(
        |addr: String, l: Arc<Mutex<Ledger>>| {
            let l_guard = safe_lock(&l);
            let full_addr = l_guard
                .accounts
                .keys()
                .find(|k| get_short_addr(k) == addr || **k == addr)
                .cloned()
                .unwrap_or(addr);
            let acct = l_guard.accounts.get(&full_addr);
            let bal = acct.map(|a| a.balance).unwrap_or(0);
            let head = acct.map(|a| a.head.as_str()).unwrap_or("0");
            let block_count = acct.map(|a| a.block_count).unwrap_or(0);
            api_json(serde_json::json!({
                "address": full_addr,
                "balance_los": format_balance_precise(bal),
                "balance_cil": bal,
                "balance_cil_str": bal.to_string(),
                "head": head,
                "block_count": block_count
            }))
        },
    );

    // 2. GET /supply
    let l_sup = ledger.clone();
    let supply_route = warp::path("supply")
        .and(with_state(l_sup))
        .map(|l: Arc<Mutex<Ledger>>| {
            let l_guard = safe_lock(&l);
            let total_supply_cil = TOTAL_SUPPLY_CIL;
            let remaining_cil = l_guard.distribution.remaining_supply;
            let circulating_cil = total_supply_cil.saturating_sub(remaining_cil);
            api_json(serde_json::json!({
                "total_supply": format_balance_precise(total_supply_cil),
                "total_supply_cil": total_supply_cil,
                "circulating_supply": format_balance_precise(circulating_cil),
                "circulating_supply_cil": circulating_cil,
                "remaining_supply": format_balance_precise(remaining_cil),
                "remaining_supply_cil": remaining_cil
            }))
        });

    // 3. GET /history/:address
    let l_his = ledger.clone();
    let ab_his = address_book.clone();
    let history_route = warp::path!("history" / String)
        .and(with_state((l_his, ab_his)))
        .map(#[allow(clippy::type_complexity)] |addr: String, (l, ab): (Arc<Mutex<Ledger>>, Arc<Mutex<HashMap<String, String>>>)| {
            let l_guard = safe_lock(&l);
            let target_full = if l_guard.accounts.contains_key(&addr) {
                Some(addr)
            } else {
                let ab_guard = safe_lock(&ab);
                if let Some(full) = ab_guard.get(&addr) {
                    Some(full.clone())
                } else {
                    l_guard.accounts.keys().find(|k| get_short_addr(k) == addr).cloned()
                }
            };

            let mut history = Vec::new();
            if let Some(full) = target_full {
                if let Some(acct) = l_guard.accounts.get(&full) {
                    let mut curr = acct.head.clone();
                    while curr != "0" {
                        if let Some(blk) = l_guard.blocks.get(&curr) {
                            // Resolve actual sender for Receive blocks
                            let from_addr = match blk.block_type {
                                BlockType::Send => blk.account.clone(),
                                BlockType::Receive => {
                                    l_guard.blocks.get(&blk.link)
                                        .map(|send_blk| send_blk.account.clone())
                                        .unwrap_or_else(|| "SYSTEM".to_string())
                                },
                                _ => "SYSTEM".to_string(),
                            };
                            let to_addr = match blk.block_type {
                                BlockType::Receive => blk.account.clone(),
                                _ => blk.link.clone(),
                            };
                            history.push(serde_json::json!({
                                "hash": curr,
                                "from": from_addr,
                                "to": to_addr,
                                "amount": format!("{}.{:011}", blk.amount / CIL_PER_LOS, blk.amount % CIL_PER_LOS),
                                "timestamp": blk.timestamp,
                                "type": format!("{:?}", blk.block_type).to_lowercase(),
                                "fee": blk.fee
                            }));
                            curr = blk.previous.clone();
                        } else { break; }
                    }
                }
            }
            api_json(serde_json::json!({"transactions": history}))
        });

    // 4. GET /peers — enhanced with validator endpoint discovery
    let ab_peer = address_book.clone();
    let ve_peer = validator_endpoints.clone();
    let l_peer = ledger.clone();
    let bv_peer = bootstrap_validators.clone();
    let my_addr_peer = my_address.clone();
    let peers_route = warp::path("peers")
        .and(with_state((ab_peer, ve_peer, l_peer)))
        .map(
            move |(ab, ve, l): (
                Arc<Mutex<HashMap<String, String>>>,
                Arc<Mutex<HashMap<String, String>>>,
                Arc<Mutex<Ledger>>,
            )| {
                let ab_guard = safe_lock(&ab);
                let ve_guard = safe_lock(&ve);
                let l_guard = safe_lock(&l);

                // Build enriched peer list from address_book (remote peers)
                let mut peers: Vec<serde_json::Value> = ab_guard
                    .iter()
                    .map(|(short, full)| {
                        let is_validator = l_guard
                            .accounts
                            .get(full)
                            .map(|a| a.is_validator)
                            .unwrap_or(false)
                            || bv_peer.contains(full)
                            || ve_guard.contains_key(full);
                        let onion = ve_guard.get(full).cloned();
                        let mut entry = serde_json::json!({
                            "short_address": short,
                            "address": full,
                            "is_validator": is_validator,
                        });
                        if let Some(o) = onion {
                            entry["host_address"] = serde_json::json!(&o);
                            entry["onion_address"] = serde_json::json!(o); // backward compat
                        }
                        entry
                    })
                    .collect();

                // Include THIS node (self) in the peers list so the operator
                // can see their own node listed alongside remote peers.
                {
                    let self_addr = &my_addr_peer;
                    let self_short = get_short_addr(self_addr);
                    let self_is_validator = l_guard
                        .accounts
                        .get(self_addr)
                        .map(|a| a.is_validator)
                        .unwrap_or(false)
                        || bv_peer.contains(self_addr)
                        || ve_guard.contains_key(self_addr);
                    let self_onion = ve_guard.get(self_addr).cloned();
                    let mut self_entry = serde_json::json!({
                        "short_address": self_short,
                        "address": self_addr,
                        "is_validator": self_is_validator,
                        "self": true,
                    });
                    if let Some(o) = self_onion {
                        self_entry["host_address"] = serde_json::json!(&o);
                        self_entry["onion_address"] = serde_json::json!(o); // backward compat
                    }
                    // Insert self at the beginning of the list
                    peers.insert(0, self_entry);
                }

                // Collect all known validator endpoints for discovery
                let validator_endpoints: Vec<serde_json::Value> = ve_guard
                    .iter()
                    .map(|(addr, host)| {
                        serde_json::json!({
                            "address": addr,
                            "host_address": host,
                            "onion_address": host, // backward compat
                        })
                    })
                    .collect();

                api_json(serde_json::json!({
                    "peers": peers,
                    "peer_count": peers.len(),
                    "validator_endpoints": validator_endpoints,
                    "validator_endpoint_count": validator_endpoints.len(),
                }))
            },
        );

    // 5. POST /send (WEIGHTED INITIAL POWER + BASE FEE)
    let l_send = ledger.clone();
    let p_send = pending_sends.clone();
    let tx_send = tx_out.clone();
    let sl_send = send_limiter.clone();
    let pk_send = node_public_key.clone();
    let mp_send = mempool_pool.clone();
    let send_route = warp::path("send")
        .and(warp::post())
        .and(warp::body::bytes())
        .and(with_state((l_send, tx_send, p_send, my_address.clone(), secret_key.clone(), sl_send, pk_send, mp_send)))
        .then(#[allow(clippy::type_complexity)] |body: bytes::Bytes, (l, tx, p, my_addr, key, rate_lim, node_pk, mp): (Arc<Mutex<Ledger>>, mpsc::Sender<String>, Arc<Mutex<HashMap<String, (Block, u128)>>>, String, Zeroizing<Vec<u8>>, Arc<EndpointRateLimiter>, Vec<u8>, Arc<Mutex<mempool::Mempool>>)| async move {
            // Parse JSON manually to return proper 400 instead of 500
            let req: SendRequest = match serde_json::from_slice(&body) {
                Ok(r) => r,
                Err(e) => {
                    return api_json(serde_json::json!({
                        "status": "error",
                        "code": 400,
                        "msg": format!("Invalid request body: {}", e)
                    }));
                }
            };
            // Determine sender: use req.from if provided, otherwise node's address
            let sender_addr = req.from.clone().unwrap_or(my_addr.clone());

            // RATE LIMIT: 10 transactions per minute per sender address
            if let Err(wait_secs) = rate_lim.check_and_record(&sender_addr) {
                return api_json(serde_json::json!({
                    "status": "error",
                    "code": 429,
                    "msg": format!("Rate limit exceeded: max 10 transactions per minute. Try again in {} seconds.", wait_secs)
                }));
            }

            // CRITICAL: Validate sender address format (Base58Check)
            if !los_crypto::validate_address(&sender_addr) {
                return api_json(serde_json::json!({
                    "status": "error",
                    "msg": "Invalid sender address format. Must be Base58Check with LOS prefix."
                }));
            }

            // Validate target address format (Base58Check)
            if !los_crypto::validate_address(&req.target) {
                return api_json(serde_json::json!({
                    "status": "error",
                    "msg": "Invalid target address format. Must be Base58Check with LOS prefix."
                }));
            }

            // SECURITY: Reject zero-amount transactions (spam prevention)
            let effective_amount = req.amount_cil.unwrap_or(req.amount * CIL_PER_LOS);
            if effective_amount == 0 {
                return api_json(serde_json::json!({
                    "status": "error",
                    "msg": "Amount must be greater than 0."
                }));
            }

            // SECURITY: Reject self-sends (no economic purpose, wastes consensus)
            if req.target == sender_addr {
                return api_json(serde_json::json!({
                    "status": "error",
                    "msg": "Cannot send to your own address."
                }));
            }

            // Client-side signing: if signature provided, validate it instead of signing with node key
            let client_signed = req.signature.is_some();

            let target_addr = {
                let l_guard = safe_lock(&l);
                // First: check existing accounts (supports short address lookup)
                if let Some(found) = l_guard.accounts.keys()
                    .find(|k| get_short_addr(k) == req.target || **k == req.target).cloned() {
                    Some(found)
                // Allow sending to new addresses not yet in ledger
                // In block-lattice, Send only records target in `link`; recipient
                // creates their own Receive block later.
                } else if los_crypto::validate_address(&req.target) {
                    Some(req.target.clone())
                } else {
                    None
                }
            };
            if let Some(target) = target_addr {
                // Checked multiplication to prevent u128 wrapping overflow
                // If amount_cil is provided (client-signed with sub-LOS precision),
                // use it directly. Otherwise multiply LOS × CIL_PER_LOS.
                let amt = if let Some(cil_amt) = req.amount_cil {
                    cil_amt
                } else {
                    match req.amount.checked_mul(CIL_PER_LOS) {
                        Some(v) => v,
                        None => {
                            return api_json(serde_json::json!({
                                "status": "error",
                                "msg": "Amount overflow: value too large"
                            }));
                        }
                    }
                };

                // CRITICAL: For client-signed transactions, public_key is REQUIRED
                let pubkey = if client_signed {
                    if let Some(pk) = req.public_key.clone() {
                        pk
                    } else {
                        return api_json(serde_json::json!({
                            "status": "error",
                            "msg": "public_key field is REQUIRED when providing signature"
                        }));
                    }
                } else {
                    hex::encode(&node_pk) // Node's own public key
                };

                let mut blk = Block {
                    account: sender_addr.clone(),
                    previous: req.previous.clone().unwrap_or("0".to_string()),
                    block_type: BlockType::Send,
                    amount: amt,
                    link: target.clone(),
                    signature: "".to_string(),
                    public_key: pubkey,
                    work: req.work.unwrap_or(0),
                    // When client-signed, use client's timestamp (part of signing_hash)
                    timestamp: if client_signed {
                        req.timestamp.unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs())
                    } else {
                        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
                    },
                    // When client-signed, use client's fee (part of signing_hash)
                    // Server still validates the fee is >= base_fee
                    fee: if client_signed { req.fee.unwrap_or(0) } else { 0 },
                };

                let initial_power: u128;
                let base_fee = los_core::BASE_FEE_CIL; // Protocol constant from los-core
                let final_fee: u128;

                // DEADLOCK Never hold L and AW simultaneously.
                // Step 1: Read state from Ledger, drop lock
                let sender_state = {
                    let l_guard = safe_lock(&l);
                    l_guard.accounts.get(&sender_addr).cloned()
                }; // L dropped

                if let Some(st) = sender_state {
                    if req.previous.is_none() {
                        blk.previous = st.head.clone();
                    }

                    // Fee = protocol base fee (flat, no dynamic scaling)
                    final_fee = base_fee;

                    // Step 3: Check balance INCLUDING pending transactions (TOCTOU prevention)
                    let pending_total: u128 = {
                        let ps = safe_lock(&p);
                        ps.values()
                            .filter(|(b, _)| b.account == sender_addr)
                            .map(|(b, _)| b.amount)
                            .sum()
                    };
                    // Use checked_add to prevent u128 overflow
                    let total_needed = match amt.checked_add(final_fee).and_then(|v| v.checked_add(pending_total)) {
                        Some(total) => total,
                        None => {
                            return api_json(serde_json::json!({
                                "status": "error",
                                "msg": "Overflow: total transaction cost exceeds maximum"
                            }));
                        }
                    };
                    if st.balance < total_needed {
                        return api_json(serde_json::json!({
                            "status":"error",
                            "msg": format!("Insufficient balance (need {} CIL for tx + {} CIL fee + {} CIL pending)", amt, final_fee, pending_total)
                        }));
                    }
                    initial_power = st.balance / CIL_PER_LOS;
                } else {
                    return api_json(serde_json::json!({"status":"error","msg":"Sender account not found"}));
                }

                // Set fee on block BEFORE PoW/signing (fee is part of signing_hash)
                // When client-signed, validate that client fee >= server-calculated fee
                if client_signed {
                    let client_fee = blk.fee;
                    if client_fee < final_fee {
                        return api_json(serde_json::json!({
                            "status": "error",
                            "msg": format!("Client fee {} CIL is below minimum required fee {} CIL", client_fee, final_fee)
                        }));
                    }
                    // Keep client's fee (already set on blk) — it's part of their signing_hash
                } else {
                    blk.fee = final_fee;
                }

                // Compute PoW if not provided by client
                if req.work.is_none() {
                    solve_pow(&mut blk);
                }

                // If client provided signature, validate it
                if client_signed {
                    // MAINNET SAFETY: use if-let instead of .unwrap() for defense-in-depth
                    if let Some(sig) = req.signature {
                        blk.signature = sig;
                    } else {
                        return api_json(serde_json::json!({
                            "status": "error",
                            "msg": "Internal error: client_signed=true but signature missing"
                        }));
                    }

                    // CRITICAL: Verify signature with public key (not address!)
                    if !blk.verify_signature() {
                        let sh = blk.signing_hash();
                        let sig_len = blk.signature.len() / 2; // hex → bytes
                        let pk_len = blk.public_key.len() / 2;
                        // Log all block fields to help client diagnose signing_hash mismatch
                        eprintln!("❌ [SIGN_FAIL] account={}", blk.account);
                        eprintln!("❌ [SIGN_FAIL] previous={}", blk.previous);
                        eprintln!("❌ [SIGN_FAIL] block_type={:?}", blk.block_type);
                        eprintln!("❌ [SIGN_FAIL] amount={} CIL", blk.amount);
                        eprintln!("❌ [SIGN_FAIL] link={}", blk.link);
                        eprintln!("❌ [SIGN_FAIL] public_key_len={} chars ({} bytes)", blk.public_key.len(), pk_len);
                        eprintln!("❌ [SIGN_FAIL] work={}", blk.work);
                        eprintln!("❌ [SIGN_FAIL] timestamp={}", blk.timestamp);
                        eprintln!("❌ [SIGN_FAIL] fee={} CIL", blk.fee);
                        eprintln!("❌ [SIGN_FAIL] chain_id={} (CHAIN_ID constant)", los_core::CHAIN_ID);
                        eprintln!("❌ [SIGN_FAIL] signing_hash={}", sh);
                        return api_json(serde_json::json!({
                            "status": "error",
                            "msg": format!("Invalid signature: verification failed (sig={} bytes, pk={} bytes). signing_hash={}", sig_len, pk_len, sh)
                        }));
                    }
                    println!("✅ Client signature verified successfully");
                } else {
                    // MAINNET SAFETY: On Production level, ALL transactions MUST be client-signed.
                    // Node auto-signing (even for its own address) is a testnet convenience only.
                    // On mainnet, the API caller must prove key ownership via signature.
                    if los_core::is_mainnet_build() {
                        return api_json(serde_json::json!({
                            "status": "error",
                            "msg": "Mainnet requires client-side signature. Provide signature + public_key fields."
                        }));
                    }

                    // TESTNET: Node signs with its own key
                    // On consensus/production testnet, only allow node to sign for its OWN address
                    if sender_addr != my_addr && testnet_config::get_testnet_config().should_validate_signatures() {
                        return api_json(serde_json::json!({
                            "status": "error",
                            "msg": "External address requires client-side Dilithium5 signature. Provide signature + public_key fields."
                        }));
                    }
                    if sender_addr != my_addr {
                        println!("🧪 TESTNET functional: node signing on behalf of external address {}", sender_addr);
                    } else {
                        println!("🔑 Node auto-signing for own address (testnet convenience)");
                    }
                    blk.signature = match try_sign_hex(blk.signing_hash().as_bytes(), &key) {
                        Ok(sig) => sig,
                        Err(e) => return api_json(serde_json::json!({"status": "error", "msg": e})),
                    };
                }

                // Block ID sekarang mencakup signature
                let hash = blk.calculate_hash();

                // Finalize immediately when:
                //   (a) Functional testnet (no consensus needed)
                //
                // DESIGN Client-signed blocks MUST go through consensus on mainnet.
                // Previously, client_signed=true skipped consensus entirely, allowing a
                // malicious user to submit the same signed block to multiple nodes
                // simultaneously — each would independently apply it, causing permanent
                // state divergence (double-spend). The chain-fork check (head != previous)
                // only protects the LOCAL node; without network consensus, remote nodes
                // have no coordination.
                //
                // On mainnet: ALL sends go through CONFIRM_REQ/CONFIRM_RES voting.
                // On testnet (functional mode): skip consensus for rapid testing.
                let skip_consensus = !testnet_config::get_testnet_config().should_enable_consensus();
                if skip_consensus {
                    {
                        let mut l_guard = safe_lock(&l);
                        // Debit sender: amount + fee
                        // Use blk.fee (not final_fee) because for client-signed blocks,
                        // blk.fee is what's in the signed block (may be >= final_fee)
                        let actual_fee = blk.fee;
                        if let Some(sender_state) = l_guard.accounts.get_mut(&sender_addr) {
                            // Chain-sequence validation — prevents double-spend.
                            // In block-lattice, each block references its predecessor. If two
                            // blocks claim the same `previous`, only the first can be applied.
                            // Without this check, a malicious client could submit conflicting
                            // sends to the same node and both would succeed (balance check
                            // alone is insufficient if the first tx hasn't been processed yet).
                            if sender_state.head != blk.previous {
                                return api_json(serde_json::json!({
                                    "status": "error",
                                    "msg": format!("Chain sequence error: expected previous={}, got={}",
                                        sender_state.head, blk.previous)
                                }));
                            }
                            let total_debit = amt.saturating_add(actual_fee);
                            if sender_state.balance < total_debit {
                                return api_json(serde_json::json!({
                                    "status": "error",
                                    "msg": "Insufficient balance for amount + fee"
                                }));
                            }
                            sender_state.balance -= total_debit;
                            sender_state.head = hash.clone();
                            sender_state.block_count += 1;
                        } else {
                            return api_json(serde_json::json!({
                                "status":"error","msg":"Sender account not found"
                            }));
                        }
                        // Insert block
                        l_guard.blocks.insert(hash.clone(), blk.clone());
                        // Accumulate fees
                        l_guard.accumulated_fees_cil = l_guard.accumulated_fees_cil.saturating_add(actual_fee);
                    }
                    SAVE_DIRTY.store(true, Ordering::Release);
                    let reason = if client_signed { "client-signed" } else { "functional testnet" };
                    println!("✅ Send finalized immediately ({}): {} → {} ({} LOS, fee {} CIL)",
                        reason, get_short_addr(&sender_addr), get_short_addr(&target), amt / CIL_PER_LOS, blk.fee);

                    // GOSSIP Broadcast finalized Send block to peers using
                    // BLOCK_CONFIRMED format so all nodes apply via the P2P handler.
                    // Raw JSON gossip was silently ignored — peers only parse BLOCK_CONFIRMED.
                    let send_json = serde_json::to_string(&blk).unwrap_or_default();
                    let send_b64_for_gossip = base64::engine::general_purpose::STANDARD.encode(send_json.as_bytes());

                    // AUTO-UNREGISTER: If sender was a validator and balance dropped below
                    // minimum registration stake (1 LOS) after this send, automatically unregister them.
                    {
                        let mut l_guard = safe_lock(&l);
                        if let Some(sender_acct) = l_guard.accounts.get_mut(&sender_addr) {
                            if sender_acct.is_validator && sender_acct.balance < MIN_VALIDATOR_REGISTER_CIL {
                                sender_acct.is_validator = false;
                                SAVE_DIRTY.store(true, Ordering::Release);
                                println!("⚠️ Auto-unregistered validator {}: balance {} < minimum registration stake {} LOS",
                                    get_short_addr(&sender_addr),
                                    sender_acct.balance / CIL_PER_LOS,
                                    MIN_VALIDATOR_REGISTER_CIL / CIL_PER_LOS);
                            }
                        }
                    }

                    // Auto-receive for recipient
                    // In block-lattice, the recipient needs their own Receive block.
                    // The node creates it and gossips to all peers.
                    let recv_gossip: Option<String> = {
                        let mut l_guard = safe_lock(&l);
                        if !l_guard.accounts.contains_key(&target) {
                            l_guard.accounts.insert(target.clone(), AccountState {
                                head: "0".to_string(), balance: 0, block_count: 0, is_validator: false,
                            });
                        }
                        if let Some(recv_state) = l_guard.accounts.get(&target).cloned() {
                            let mut recv_blk = Block {
                                account: target.clone(),
                                previous: recv_state.head,
                                block_type: BlockType::Receive,
                                amount: amt,
                                link: hash.clone(),
                                signature: "".to_string(),
                                public_key: hex::encode(&node_pk),
                                work: 0,
                                timestamp: std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs(),
                                fee: 0,
                            };
                            solve_pow(&mut recv_blk);
                            recv_blk.signature = match try_sign_hex(recv_blk.signing_hash().as_bytes(), &key) {
                                Ok(sig) => sig,
                                Err(e) => { eprintln!("❌ Auto-Receive signing failed: {}", e); return api_json(serde_json::json!({"status": "error", "msg": e})); }
                            };
                            // Direct ledger manipulation for Receive block — bypass process_block()
                            // because the node's public_key doesn't match the target's account address.
                            let recv_hash = recv_blk.calculate_hash();
                            if let Some(recv_acct) = l_guard.accounts.get_mut(&target) {
                                recv_acct.balance = recv_acct.balance.saturating_add(amt);
                                recv_acct.head = recv_hash.clone();
                                recv_acct.block_count += 1;
                            }
                            l_guard.blocks.insert(recv_hash.clone(), recv_blk.clone());
                            // Track claimed Send for double-receive prevention.
                            // Direct ledger manipulation bypasses process_block() which normally
                            // inserts into claimed_sends. Without this, a second Receive referencing
                            // the same Send could pass the claimed_sends check in process_block().
                            l_guard.claimed_sends.insert(hash.clone());
                            SAVE_DIRTY.store(true, Ordering::Release);
                            println!("✅ Auto-Receive created for {} ({} CIL)", get_short_addr(&target), amt);
                            let recv_json = serde_json::to_string(&recv_blk).unwrap_or_default();
                            let recv_b64_for_gossip = base64::engine::general_purpose::STANDARD.encode(recv_json.as_bytes());
                            Some(recv_b64_for_gossip)
                        } else { None }
                    }; // l_guard dropped

                    // Gossip as BLOCK_CONFIRMED:send_b64:recv_b64 so peers apply via P2P handler.
                    // Raw block JSON was silently ignored — only BLOCK_CONFIRMED is parsed.
                    if let Some(recv_b64) = recv_gossip {
                        let confirmed_msg = format!("BLOCK_CONFIRMED:{}:{}", send_b64_for_gossip, recv_b64);
                        let _ = tx.send(confirmed_msg).await;
                    }

                    return api_json(serde_json::json!({
                        "status":"success",
                        "tx_hash":hash,
                        "initial_power": initial_power,
                        "fee_paid_cil": blk.fee,
                        "fee_multiplier_bps": if base_fee > 0 { blk.fee * 10_000 / base_fee } else { 10_000 }
                    }));
                }

                // Start total_power_votes at 0 instead of initial_power.
                // The sender doesn't self-vote — only distinct external validators contribute
                // voting power via CONFIRM_RES. initial_power is kept for API response & mempool.
                safe_lock(&p).insert(hash.clone(), (blk.clone(), 0u128));

                // Serialize block BEFORE mempool takes ownership.
                // Include block data (base64) so peers can validate and vote.
                // Without this, peers receive CONFIRM_REQ but can't verify the block
                // because it only exists locally in pending_sends — causing zero votes.
                let block_json = serde_json::to_string(&blk).unwrap_or_default();
                let block_b64 = base64::engine::general_purpose::STANDARD.encode(block_json.as_bytes());

                // Track in mempool for stats and future block assembly
                {
                    let fee_u64 = blk.fee as u64;
                    let priority = initial_power as u64;
                    let _ = safe_lock(&mp).add_transaction(blk, fee_u64, priority);
                }

                let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
                let _ = tx.send(format!("CONFIRM_REQ:{}:{}:{}:{}:{}", hash, sender_addr, amt, ts, block_b64)).await;
                api_json(serde_json::json!({
                    "status":"success",
                    "tx_hash":hash,
                    "initial_power": initial_power,
                    "fee_paid_cil": final_fee,
                    "fee_multiplier_bps": if base_fee > 0 { final_fee * 10_000 / base_fee } else { 10_000 }
                }))
            } else {
                api_json(serde_json::json!({"status":"error","msg":"Address not found"}))
            }
        });

    // 7. POST /deploy-contract (PERMISSIONLESS — create ContractDeploy block)
    let deploy_route = {
        let l_deploy = ledger.clone();
        let tx_deploy = tx_out.clone();
        let sk_deploy = secret_key.clone();
        let pk_deploy = node_public_key.clone();
        let addr_deploy = my_address.clone();
        let engine_deploy = wasm_engine.clone();
        let db_deploy = database.clone();
        let m_deploy = metrics.clone();
        let deploy = warp::path("deploy-contract")
            .and(warp::post())
            .and(warp::body::bytes())
            .and(with_state((l_deploy, tx_deploy, sk_deploy, pk_deploy, addr_deploy, engine_deploy, db_deploy, m_deploy)))
            .then(|body: bytes::Bytes, state: (Arc<Mutex<Ledger>>, mpsc::Sender<String>, Zeroizing<Vec<u8>>, Vec<u8>, String, Arc<WasmEngine>, Arc<LosDatabase>, Arc<LosMetrics>)| async move {
                let (l, tx, sk, pk, my_addr, engine, db, metrics) = state;
                let req: DeployContractRequest = match serde_json::from_slice(&body) {
                    Ok(r) => r,
                    Err(e) => {
                        return api_json(serde_json::json!({
                            "status": "error", "code": 400,
                            "msg": format!("Invalid request body: {}", e)
                        }))
                    }
                };
                // Decode base64 WASM bytecode
                let bytecode = match base64::engine::general_purpose::STANDARD.decode(&req.bytecode) {
                    Ok(bytes) => bytes,
                    Err(_) => {
                        return api_json(serde_json::json!({"status":"error","msg":"Invalid base64 bytecode"}))
                    }
                };
                // Compute code hash for block link
                let code_hash = WasmEngine::compute_code_hash(&bytecode);
                let link = format!("DEPLOY:{}", code_hash);
                let amount_cil = req.amount_cil.unwrap_or(0);
                let is_client_signed = req.signature.is_some() && req.public_key.is_some();

                // MAINNET GUARD: Server-signed deploys are disabled on mainnet.
                // All contract deployments on mainnet MUST be client-signed (with signature + public_key).
                // This prevents the node from deploying contracts with its own key on behalf of anonymous callers.
                if los_core::is_mainnet_build() && !is_client_signed {
                    return api_json(serde_json::json!({
                        "status": "error",
                        "code": 403,
                        "msg": "Server-signed contract deployment is disabled on mainnet. Provide signature and public_key."
                    }));
                }

                let fee = req.fee.unwrap_or(los_core::MIN_DEPLOY_FEE_CIL);
                let now_ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                let (account, pub_key_hex) = if is_client_signed {
                    let pk_hex = req.public_key.clone().unwrap_or_default();
                    let pk_bytes = hex::decode(&pk_hex).unwrap_or_default();
                    let derived = los_crypto::public_key_to_address(&pk_bytes);
                    (derived, pk_hex)
                } else {
                    (my_addr.clone(), hex::encode(&pk))
                };

                let previous = if is_client_signed {
                    req.previous.unwrap_or_else(|| {
                        let l_guard = safe_lock(&l);
                        l_guard.accounts.get(&account).map(|a| a.head.clone()).unwrap_or_else(|| "0".to_string())
                    })
                } else {
                    let l_guard = safe_lock(&l);
                    l_guard.accounts.get(&account).map(|a| a.head.clone()).unwrap_or_else(|| "0".to_string())
                };

                let mut block = Block {
                    account: account.clone(),
                    previous,
                    block_type: BlockType::ContractDeploy,
                    amount: amount_cil,
                    link: link.clone(),
                    signature: String::new(),
                    public_key: pub_key_hex,
                    work: req.work.unwrap_or(0),
                    timestamp: req.timestamp.unwrap_or(now_ts),
                    fee,
                };

                // PoW + Signing
                if is_client_signed {
                    block.signature = req.signature.unwrap_or_default();
                } else {
                    solve_pow(&mut block);
                    block.signature = match try_sign_hex(block.signing_hash().as_bytes(), &sk) {
                        Ok(sig) => sig,
                        Err(e) => {
                            return api_json(serde_json::json!({"status":"error","msg":format!("Signing failed: {}", e)}))
                        }
                    };
                }

                // Process block through ledger (debit fees + optional funding)
                let block_hash = {
                    let mut l_guard = safe_lock(&l);
                    match l_guard.process_block(&block) {
                        Ok(result) => result.into_hash(),
                        Err(e) => {
                            return api_json(serde_json::json!({"status":"error","msg":e}))
                        }
                    }
                };

                // Deploy bytecode to WASM engine
                let contract_addr = match engine.deploy_contract(
                    account.clone(),
                    bytecode.clone(),
                    req.initial_state.unwrap_or_default(),
                    now_ts,
                ) {
                    Ok(addr) => addr,
                    Err(e) => {
                        return api_json(serde_json::json!({"status":"error","msg":format!("VM deploy failed: {}", e)}))
                    }
                };

                // Fund contract if amount > 0
                if amount_cil > 0 {
                    if let Err(e) = engine.send_to_contract(&contract_addr, amount_cil) {
                        eprintln!("Warning: Failed to fund contract: {}", e);
                    }
                }

                // Persist VM state to DB
                if let Ok(vm_data) = engine.serialize_all() {
                    let _ = db.save_contracts(&vm_data);
                }

                // Gossip to peers: CONTRACT_DEPLOYED:{block_b64}:{bytecode_b64}:{initial_state_b64}
                let block_b64 = base64::engine::general_purpose::STANDARD.encode(
                    serde_json::to_vec(&block).unwrap_or_default()
                );
                let bytecode_b64 = base64::engine::general_purpose::STANDARD.encode(&bytecode);
                let gossip = format!("CONTRACT_DEPLOYED:{}:{}:{}", block_b64, bytecode_b64, contract_addr);
                let _ = tx.send(gossip).await;

                SAVE_DIRTY.store(true, Ordering::Release);
                metrics.contracts_deployed_total.inc();

                api_json(serde_json::json!({
                    "status": "success",
                    "contract_address": contract_addr,
                    "code_hash": code_hash,
                    "block_hash": block_hash,
                    "owner": account,
                    "fee_cil": fee,
                    "deployed_at": now_ts
                }))
            });

        // 8. POST /call-contract (create ContractCall block + execute)
        let l_call = ledger.clone();
        let tx_call = tx_out.clone();
        let sk_call = secret_key.clone();
        let pk_call = node_public_key.clone();
        let addr_call = my_address.clone();
        let engine_call = wasm_engine.clone();
        let db_call = database.clone();
        let m_call = metrics.clone();
        let call = warp::path("call-contract")
            .and(warp::post())
            .and(warp::body::bytes())
            .and(with_state((l_call, tx_call, sk_call, pk_call, addr_call, engine_call, db_call, m_call)))
            .then(|body: bytes::Bytes, state: (Arc<Mutex<Ledger>>, mpsc::Sender<String>, Zeroizing<Vec<u8>>, Vec<u8>, String, Arc<WasmEngine>, Arc<LosDatabase>, Arc<LosMetrics>)| async move {
                let (l, tx, sk, pk, my_addr, engine, db, metrics) = state;
                let req: CallContractRequest = match serde_json::from_slice(&body) {
                    Ok(r) => r,
                    Err(e) => {
                        return api_json(serde_json::json!({
                            "status": "error", "code": 400,
                            "msg": format!("Invalid request body: {}", e)
                        }))
                    }
                };
                let gas_limit = req.gas_limit.unwrap_or(los_core::DEFAULT_GAS_LIMIT);
                let amount_cil = req.amount_cil.unwrap_or(0);
                let fee = req.fee.unwrap_or(los_core::MIN_CALL_FEE_CIL.max(
                    (gas_limit as u128).saturating_mul(los_core::GAS_PRICE_CIL)
                ));
                let is_client_signed = req.signature.is_some() && req.public_key.is_some();

                // MAINNET GUARD: Server-signed contract calls are disabled on mainnet.
                // All contract calls on mainnet MUST be client-signed (with signature + public_key).
                // This prevents the node from signing transactions on behalf of anonymous callers.
                if los_core::is_mainnet_build() && !is_client_signed {
                    return api_json(serde_json::json!({
                        "status": "error",
                        "code": 403,
                        "msg": "Server-signed contract calls are disabled on mainnet. Provide signature and public_key."
                    }));
                }

                let now_ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                // Encode args as base64 JSON for deterministic link field
                let args_json = serde_json::to_string(&req.args).unwrap_or_else(|_| "[]".to_string());
                let args_b64 = base64::engine::general_purpose::STANDARD.encode(args_json.as_bytes());
                let link = format!("CALL:{}:{}:{}", req.contract_address, req.function, args_b64);

                let (account, pub_key_hex) = if is_client_signed {
                    let pk_hex = req.public_key.clone().unwrap_or_default();
                    let pk_bytes = hex::decode(&pk_hex).unwrap_or_default();
                    let derived = los_crypto::public_key_to_address(&pk_bytes);
                    (derived, pk_hex)
                } else {
                    let caller = req.caller.clone().unwrap_or_else(|| my_addr.clone());
                    // If caller != node, still use node's key (node-signed on behalf)
                    (caller, hex::encode(&pk))
                };

                let previous = if is_client_signed {
                    req.previous.unwrap_or_else(|| {
                        let l_guard = safe_lock(&l);
                        l_guard.accounts.get(&account).map(|a| a.head.clone()).unwrap_or_else(|| "0".to_string())
                    })
                } else {
                    let l_guard = safe_lock(&l);
                    l_guard.accounts.get(&account).map(|a| a.head.clone()).unwrap_or_else(|| "0".to_string())
                };

                let mut block = Block {
                    account: account.clone(),
                    previous,
                    block_type: BlockType::ContractCall,
                    amount: amount_cil,
                    link: link.clone(),
                    signature: String::new(),
                    public_key: pub_key_hex,
                    work: req.work.unwrap_or(0),
                    timestamp: req.timestamp.unwrap_or(now_ts),
                    fee,
                };

                if is_client_signed {
                    block.signature = req.signature.unwrap_or_default();
                } else {
                    solve_pow(&mut block);
                    block.signature = match try_sign_hex(block.signing_hash().as_bytes(), &sk) {
                        Ok(sig) => sig,
                        Err(e) => {
                            return api_json(serde_json::json!({"status":"error","msg":format!("Signing failed: {}", e)}))
                        }
                    };
                }

                // Process block through ledger (debit fee + value)
                let block_hash = {
                    let mut l_guard = safe_lock(&l);
                    match l_guard.process_block(&block) {
                        Ok(result) => result.into_hash(),
                        Err(e) => {
                            return api_json(serde_json::json!({"status":"error","msg":e}))
                        }
                    }
                };

                // Send CIL to contract if amount > 0
                if amount_cil > 0 {
                    if let Err(e) = engine.send_to_contract(&req.contract_address, amount_cil) {
                        return api_json(serde_json::json!({"status":"error","msg":format!("Value transfer failed: {}", e)}));
                    }
                }

                // Execute contract call on WASM engine
                let call = ContractCall {
                    contract: req.contract_address.clone(),
                    function: req.function.clone(),
                    args: req.args.clone(),
                    gas_limit,
                    caller: account.clone(),
                    block_timestamp: block.timestamp,
                };

                let exec_result = match engine.call_contract(call) {
                    Ok(result) => result,
                    Err(e) => {
                        return api_json(serde_json::json!({"status":"error","msg":format!("Execution failed: {}", e)}))
                    }
                };

                // Persist VM state to DB
                if let Ok(vm_data) = engine.serialize_all() {
                    let _ = db.save_contracts(&vm_data);
                }

                // CRITICAL: Credit recipients from contract transfers.
                // host_transfer() already decremented the contract's balance in the VM.
                // Without this, transferred CIL is burned (never credited to recipients).
                if !exec_result.transfers.is_empty() {
                    let mut l_guard = safe_lock(&l);
                    for (recipient, amount) in &exec_result.transfers {
                        if let Some(recv_acc) = l_guard.accounts.get_mut(recipient) {
                            recv_acc.balance = recv_acc.balance.saturating_add(*amount);
                        } else {
                            // Create new account if recipient doesn't exist yet
                            l_guard.accounts.insert(recipient.clone(), los_core::AccountState {
                                head: "0".to_string(),
                                balance: *amount,
                                block_count: 0,
                                is_validator: false,
                            });
                        }
                    }
                }

                // Gossip to peers
                let block_b64 = base64::engine::general_purpose::STANDARD.encode(
                    serde_json::to_vec(&block).unwrap_or_default()
                );
                let gossip = format!("CONTRACT_CALLED:{}", block_b64);
                let _ = tx.send(gossip).await;

                SAVE_DIRTY.store(true, Ordering::Release);
                metrics.contract_executions_total.inc();

                api_json(serde_json::json!({
                    "status": "success",
                    "block_hash": block_hash,
                    "result": {
                        "success": exec_result.success,
                        "output": exec_result.output,
                        "gas_used": exec_result.gas_used,
                        "state_changes": exec_result.state_changes,
                        "events": exec_result.events,
                        "transfers": exec_result.transfers.iter()
                            .map(|(addr, amt)| serde_json::json!({"recipient": addr, "amount_cil": amt}))
                            .collect::<Vec<_>>()
                    },
                    "fee_cil": fee,
                    "caller": account
                }))
            });

        // 9. GET /contract/:address
        let engine_get = wasm_engine.clone();
        let get_contract = warp::path!("contract" / String)
            .and(with_state(engine_get))
            .map(
                |addr: String, engine: Arc<WasmEngine>| match engine.get_contract(&addr) {
                    Ok(contract) => api_json(serde_json::json!({
                        "status": "success",
                        "contract": {
                            "address": contract.address,
                            "code_hash": contract.code_hash,
                            "balance": contract.balance,
                            "owner": contract.owner,
                            "created_at_block": contract.created_at_block,
                            "state": contract.state
                        }
                    })),
                    Err(e) => api_json(serde_json::json!({
                        "status": "error",
                        "msg": e
                    })),
                },
            );

        // 9b. GET /contracts (list all deployed contracts)
        let engine_list = wasm_engine.clone();
        let list_contracts_route =
            warp::path("contracts")
                .and(with_state(engine_list))
                .map(|engine: Arc<WasmEngine>| match engine.list_contracts() {
                    Ok(addrs) => api_json(serde_json::json!({
                        "status": "success",
                        "count": addrs.len(),
                        "contracts": addrs
                    })),
                    Err(e) => api_json(serde_json::json!({"status":"error","msg":e})),
                });

        deploy
            .boxed()
            .or(call.boxed())
            .or(get_contract.boxed())
            .or(list_contracts_route.boxed())
            .boxed()
    };

    // ── USP-01 Token Routes ──

    // GET /tokens — List all deployed USP-01 tokens
    let engine_tokens = wasm_engine.clone();
    let list_tokens_route = warp::path("tokens")
        .and(warp::path::end())
        .and(warp::get())
        .and(with_state(engine_tokens))
        .map(|engine: Arc<WasmEngine>| {
            let tokens = token_registry::list_usp01_tokens(&engine);
            api_json(serde_json::json!({
                "status": "success",
                "count": tokens.len(),
                "tokens": tokens
            }))
        });

    // GET /token/:address — Get USP-01 token metadata
    let engine_token_info = wasm_engine.clone();
    let token_info_route = warp::path!("token" / String)
        .and(warp::get())
        .and(with_state(engine_token_info))
        .map(|addr: String, engine: Arc<WasmEngine>| {
            match token_registry::query_token_info(&engine, &addr) {
                Some(info) => api_json(serde_json::json!({
                    "status": "success",
                    "token": info
                })),
                None => api_json(serde_json::json!({
                    "status": "error",
                    "msg": "Contract not found or not a USP-01 token"
                })),
            }
        });

    // GET /token/:address/balance/:holder — Get token balance for a holder
    let engine_token_bal = wasm_engine.clone();
    let token_balance_route = warp::path!("token" / String / "balance" / String)
        .and(with_state(engine_token_bal))
        .map(
            |contract: String, holder: String, engine: Arc<WasmEngine>| {
                match token_registry::query_token_balance(&engine, &contract, &holder) {
                    Ok(balance) => api_json(serde_json::json!({
                        "status": "success",
                        "contract": contract,
                        "holder": holder,
                        "balance": balance.to_string()
                    })),
                    Err(e) => api_json(serde_json::json!({
                        "status": "error",
                        "msg": e
                    })),
                }
            },
        );

    // GET /token/:address/allowance/:owner/:spender — Get token allowance
    let engine_token_allow = wasm_engine.clone();
    let token_allowance_route = warp::path!("token" / String / "allowance" / String / String)
        .and(with_state(engine_token_allow))
        .map(
            |contract: String, owner: String, spender: String, engine: Arc<WasmEngine>| {
                match token_registry::query_token_allowance(&engine, &contract, &owner, &spender) {
                    Ok(allowance) => api_json(serde_json::json!({
                        "status": "success",
                        "contract": contract,
                        "owner": owner,
                        "spender": spender,
                        "allowance": allowance.to_string()
                    })),
                    Err(e) => api_json(serde_json::json!({
                        "status": "error",
                        "msg": e
                    })),
                }
            },
        );

    // ── DEX Routes ──

    // GET /dex/pools — List all DEX pools across all contracts
    let engine_dex_pools = wasm_engine.clone();
    let dex_list_pools_route = warp::path!("dex" / "pools")
        .and(warp::get())
        .and(with_state(engine_dex_pools))
        .map(|engine: Arc<WasmEngine>| {
            let pools = dex_registry::list_all_dex_pools(&engine);
            api_json(serde_json::json!({
                "status": "success",
                "count": pools.len(),
                "pools": pools
            }))
        });

    // GET /dex/pool/:contract/:pool_id — Get pool info
    let engine_dex_pool = wasm_engine.clone();
    let dex_pool_info_route = warp::path!("dex" / "pool" / String / String)
        .and(warp::get())
        .and(with_state(engine_dex_pool))
        .map(
            |contract: String, pool_id: String, engine: Arc<WasmEngine>| {
                match dex_registry::query_pool_info(&engine, &contract, &pool_id) {
                    Some(info) => api_json(serde_json::json!({
                        "status": "success",
                        "pool": info
                    })),
                    None => api_json(serde_json::json!({
                        "status": "error",
                        "msg": "Pool not found or contract is not a DEX"
                    })),
                }
            },
        );

    // GET /dex/quote/:contract/:pool_id/:token_in/:amount_in — Swap quote
    let engine_dex_quote = wasm_engine.clone();
    let dex_quote_route = warp::path!("dex" / "quote" / String / String / String / String)
        .and(with_state(engine_dex_quote))
        .map(
            |contract: String,
             pool_id: String,
             token_in: String,
             amount_str: String,
             engine: Arc<WasmEngine>| {
                let amount_in: u128 = amount_str.parse().unwrap_or(0);
                match dex_registry::compute_quote(
                    &engine, &contract, &pool_id, &token_in, amount_in,
                ) {
                    Ok((amount_out, fee, impact_bps)) => api_json(serde_json::json!({
                        "status": "success",
                        "quote": {
                            "amount_out": amount_out.to_string(),
                            "fee": fee.to_string(),
                            "price_impact_bps": impact_bps.to_string()
                        }
                    })),
                    Err(e) => api_json(serde_json::json!({
                        "status": "error",
                        "msg": e
                    })),
                }
            },
        );

    // GET /dex/position/:contract/:pool_id/:user — LP position
    let engine_dex_pos = wasm_engine.clone();
    let dex_position_route = warp::path!("dex" / "position" / String / String / String)
        .and(with_state(engine_dex_pos))
        .map(
            |contract: String, pool_id: String, user: String, engine: Arc<WasmEngine>| {
                match dex_registry::query_lp_position(&engine, &contract, &pool_id, &user) {
                    Ok(shares) => api_json(serde_json::json!({
                        "status": "success",
                        "contract": contract,
                        "pool_id": pool_id,
                        "user": user,
                        "lp_shares": shares.to_string()
                    })),
                    Err(e) => api_json(serde_json::json!({
                        "status": "error",
                        "msg": e
                    })),
                }
            },
        );

    // 10. GET /metrics (Prometheus endpoint)
    let metrics_clone = metrics.clone();
    let ledger_metrics = ledger.clone();
    let db_metrics = database.clone();
    let metrics_route = warp::path("metrics")
        .and(with_state((metrics_clone, ledger_metrics, db_metrics)))
        .map(
            |(m, l, db): (Arc<LosMetrics>, Arc<Mutex<Ledger>>, Arc<LosDatabase>)| {
                // Update blockchain metrics before export
                {
                    let ledger_guard = safe_lock(&l);
                    m.update_blockchain_metrics(&ledger_guard);
                }

                // Update database metrics
                let stats = db.stats();
                m.update_db_metrics(&stats);

                // Export all metrics
                match m.export() {
                    Ok(output) => warp::reply::with_header(
                        output,
                        "Content-Type",
                        "text/plain; version=0.0.4",
                    ),
                    Err(e) => warp::reply::with_header(
                        format!("# Error exporting metrics: {}", e),
                        "Content-Type",
                        "text/plain",
                    ),
                }
            },
        );

    // 11. GET /node-info (Network metadata for CLI)
    let l_info = ledger.clone();
    let ab_info = address_book.clone();
    let my_addr_info = my_address.clone();
    let node_info_route = warp::path("node-info")
        .and(with_state((l_info, ab_info)))
        .map(
            move |(l, ab): (Arc<Mutex<Ledger>>, Arc<Mutex<HashMap<String, String>>>)| {
                let l_guard = safe_lock(&l);
                // Protocol constant: 21,936,236 LOS total supply (immutable)
                // Validated against genesis_config.json on mainnet startup
                let total_supply = TOTAL_SUPPLY_CIL;
                let circulating = total_supply - l_guard.distribution.remaining_supply;

                // Validator count = ALL registered validators (genesis + dynamically registered)
                // Counts every account with is_validator == true in the ledger
                let validator_count = l_guard
                    .accounts
                    .values()
                    .filter(|acc| acc.is_validator)
                    .count();
                let peer_count = safe_lock(&ab).len();
                let network = if los_core::CHAIN_ID == 1 {
                    "los-mainnet"
                } else {
                    "los-testnet"
                };

                // Calculate TPS from blocks in the last 60 seconds
                let now_ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let window_secs: u64 = 60;
                let recent_tx_count = l_guard
                    .blocks
                    .values()
                    .filter(|b| b.timestamp > now_ts.saturating_sub(window_secs))
                    .count() as u64;
                let network_tps = if window_secs > 0 {
                    recent_tx_count / window_secs
                } else {
                    0
                };

                api_json(serde_json::json!({
                    "chain_id": network,
                    "network": network,
                    "address": my_addr_info,
                    "version": env!("CARGO_PKG_VERSION"),
                    "block_height": l_guard.total_chain_blocks(),
                    "validator_count": validator_count,
                    "peer_count": peer_count,
                    "total_supply": format_balance_precise(total_supply),
                    "circulating_supply": format_balance_precise(circulating),
                    "network_tps": network_tps,
                    "protocol": {
                        "base_fee_cil": los_core::BASE_FEE_CIL,
                        "pow_difficulty_bits": los_core::MIN_POW_DIFFICULTY_BITS,
                        "cil_per_los": los_core::CIL_PER_LOS,
                        "chain_id_numeric": los_core::CHAIN_ID
                    }
                }))
            },
        );

    // 12. GET /validators (List ALL registered validators — genesis + dynamically registered)
    // Active status is determined by actual connectivity (is_self || in_peers),
    // NOT just by having sufficient balance. Uptime comes from real heartbeat data.
    let l_validators = ledger.clone();
    let ab_validators = address_book.clone();
    let my_addr_validators = my_address.clone();
    let bv_validators = bootstrap_validators.clone();
    let sm_validators = slashing_manager.clone();
    let rp_validators = reward_pool.clone();
    let ve_validators = validator_endpoints.clone();
    let validators_route = warp::path("validators")
        .and(with_state((l_validators, ab_validators)))
        .map(
            move |(l, ab): (Arc<Mutex<Ledger>>, Arc<Mutex<HashMap<String, String>>>)| {
                let l_guard = safe_lock(&l);
                let ab_guard = safe_lock(&ab);

                // Collect ALL validator addresses: genesis bootstrap + slashing (active only) + ledger is_validator
                let mut all_validator_addrs: Vec<String> = bv_validators.clone();
                {
                    let sm_guard = safe_lock(&sm_validators);
                    for addr in sm_guard.get_all_validator_addresses() {
                        // Skip validators that have been unstaked or banned — they should not appear
                        if let Some(status) = sm_guard.get_status(&addr) {
                            if status == los_consensus::slashing::ValidatorStatus::Unstaking
                                || status == los_consensus::slashing::ValidatorStatus::Banned
                            {
                                continue;
                            }
                        }
                        if !all_validator_addrs.contains(&addr) {
                            all_validator_addrs.push(addr);
                        }
                    }
                }
                // Also include accounts explicitly marked as validators (user-registered)
                for (addr, acc) in &l_guard.accounts {
                    if acc.is_validator && !all_validator_addrs.contains(addr) {
                        all_validator_addrs.push(addr.clone());
                    }
                }

                // Get real uptime data from reward pool
                let rp_guard = safe_lock(&rp_validators);
                let ve_guard = safe_lock(&ve_validators);

                let validators: Vec<serde_json::Value> = all_validator_addrs
                    .iter()
                    .filter_map(|addr| {
                        l_guard.accounts.get(addr.as_str()).and_then(|acc| {
                            // Skip non-validators that are no longer active (e.g. unstaked)
                            // Bootstrap validators are always shown regardless of flag
                            let is_genesis = bv_validators.contains(addr);
                            if !acc.is_validator && !is_genesis {
                                return None;
                            }
                            // ACTIVE = verified validator with sufficient stake
                            let is_self = addr == &my_addr_validators;
                            let in_peers = ab_guard.values().any(|v| v.contains(addr.as_str()));
                            // Also consider validator as connected if their address is
                            // known in validator_endpoints (announced via VALIDATOR_REG
                            // or seeded from genesis onion data)
                            let in_endpoints = ve_guard.contains_key(addr.as_str());
                            let has_min_stake = acc.balance >= MIN_VALIDATOR_STAKE_CIL;
                            // Connected = evidence of P2P liveness (online indicator)
                            let connected = is_self || in_peers || in_endpoints;
                            // in_reward_pool = registered via verified Dilithium5 signature
                            // (registration checks: valid sig + address match + min stake)
                            let in_reward_pool = rp_guard.validators.contains_key(addr.as_str());
                            // ACTIVE requires: registered + staked + evidence of legitimacy
                            // - connected: appeared in P2P address book → online
                            // - is_genesis: infrastructure bootstrap nodes → assumed active
                            // - in_reward_pool: verified registration (Dilithium5 proof) → active
                            let active = has_min_stake
                                && acc.is_validator
                                && (connected || is_genesis || in_reward_pool);

                            // Real uptime from heartbeat data (not hardcoded)
                            // Uses display_uptime_pct() which shows max(current_epoch, last_epoch)
                            // to avoid misleading 0% at epoch start.
                            // MAINNET SAFETY: Integer only (u64 percent)
                            let uptime_pct: u64 = rp_guard
                                .validators
                                .get(addr.as_str())
                                .map(|vs| vs.display_uptime_pct())
                                .unwrap_or(if is_self { 100 } else { 0 });

                            // Include host endpoint if known (for peer discovery)
                            let host_ep = ve_guard.get(addr.as_str()).cloned();

                            let mut entry = serde_json::json!({
                                "address": addr,
                                "stake": acc.balance / CIL_PER_LOS,
                                "is_active": active,
                                "active": active,
                                "connected": connected,
                                "is_genesis": is_genesis,
                                "uptime_percentage": uptime_pct,
                                "has_min_stake": has_min_stake,
                            });
                            if let Some(h) = host_ep {
                                entry["host_address"] = serde_json::json!(&h);
                                entry["onion_address"] = serde_json::json!(h); // backward compat
                            }
                            Some(entry)
                        })
                    })
                    .collect();

                api_json(serde_json::json!({
                    "validators": validators
                }))
            },
        );

    // 13. GET /balance/:address (Check balance - alias for CLI compatibility)
    let l_balance_alias = ledger.clone();
    let balance_alias_route = warp::path!("balance" / String)
        .and(with_state(l_balance_alias))
        .map(|addr: String, l: Arc<Mutex<Ledger>>| {
            let l_guard = safe_lock(&l);
            let full_addr = l_guard
                .accounts
                .keys()
                .find(|k| get_short_addr(k) == addr || **k == addr)
                .cloned()
                .unwrap_or(addr.clone());
            let acct = l_guard.accounts.get(&full_addr);
            let bal = acct.map(|a| a.balance).unwrap_or(0);
            let head = acct.map(|a| a.head.as_str()).unwrap_or("0");
            let block_count = acct.map(|a| a.block_count).unwrap_or(0);
            api_json(serde_json::json!({
                "address": full_addr,
                "balance": format_balance_precise(bal),
                "balance_los": format_balance_precise(bal),
                "balance_cil": bal,
                "balance_cil_str": bal.to_string(),
                "head": head,
                "block_count": block_count
            }))
        });

    // 13b. GET /fee-estimate/:address (returns flat base fee — no dynamic scaling)
    let fee_estimate_route = warp::path!("fee-estimate" / String).map(|addr: String| {
        // Validate address format (Base58Check with LOS prefix)
        if !los_crypto::validate_address(&addr) {
            return api_json(serde_json::json!({
                "status": "error",
                "code": 400,
                "msg": "Invalid address format. Must be Base58Check with LOS prefix."
            }));
        }
        let base_fee = los_core::BASE_FEE_CIL;
        api_json(serde_json::json!({
            "address": addr,
            "base_fee_cil": base_fee,
            "estimated_fee_cil": base_fee,
            "fee_multiplier": 1,
            "fee_multiplier_bps": 10_000
        }))
    });

    // ──────────────────────────────────────────────────────────────────
    // 13c. GET /mining-info — Current epoch, difficulty, reward for miners
    // ──────────────────────────────────────────────────────────────────
    let ms_info = mining_state.clone();
    let l_mining_info = ledger.clone();
    let mining_info_route = warp::path("mining-info")
        .and(warp::path::end())
        .and(warp::get())
        .and(with_state((ms_info, l_mining_info)))
        .map(|state: (Arc<Mutex<MiningState>>, Arc<Mutex<Ledger>>)| {
            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let mut ms = safe_lock(&state.0);
            ms.maybe_advance_epoch(now_secs);
            let remaining = safe_lock(&state.1).distribution.remaining_supply;
            let info = ms.get_mining_info(now_secs, remaining);
            api_json(serde_json::json!({
                "epoch": info.epoch,
                "difficulty_bits": info.difficulty_bits,
                "reward_per_epoch_cil": info.reward_per_epoch_cil.to_string(),
                "reward_per_epoch_los": info.reward_per_epoch_cil / CIL_PER_LOS,
                "remaining_supply_cil": info.remaining_supply_cil.to_string(),
                "remaining_supply_los": info.remaining_supply_cil / CIL_PER_LOS,
                "epoch_remaining_secs": info.epoch_remaining_secs,
                "miners_this_epoch": info.miners_this_epoch,
                "chain_id": info.chain_id
            }))
        });

    // ──────────────────────────────────────────────────────────────────
    // 13d. POST /mine — REMOVED
    // Mining is now exclusively via the --mine flag on the node binary.
    // Miners MUST run a full node. There is no external mining API.
    // See: background_mining_thread() for the integrated mining loop.
    // ──────────────────────────────────────────────────────────────────

    // 14. GET /block (Latest block) — path::end() prevents /block/{hash} route conflict
    let l_block = ledger.clone();
    let block_route = warp::path("block")
        .and(warp::path::end())
        .and(with_state(l_block))
        .map(|l: Arc<Mutex<Ledger>>| {
            let l_guard = safe_lock(&l);
            // Get latest block by timestamp (HashMap has no guaranteed order)
            let latest = l_guard.blocks.values().max_by_key(|b| b.timestamp);
            if let Some(b) = latest {
                api_json(serde_json::json!({
                    "height": l_guard.total_chain_blocks(),
                    "hash": b.calculate_hash(),
                    "account": b.account,
                    "previous": b.previous,
                    "amount": b.amount / CIL_PER_LOS,
                    "block_type": format!("{:?}", b.block_type)
                }))
            } else {
                api_json(serde_json::json!({"error": "No blocks yet"}))
            }
        });

    // 15. POST /faucet (TESTNET ONLY - Free LOS for testing)
    // GOSSIP: Faucet Mint blocks are now broadcast to all peers so every
    // node sees the faucet balance. This is testnet-only — faucet is disabled
    // on mainnet by should_enable_faucet() which returns false (compiler DCE).
    let l_faucet = ledger.clone();
    let db_faucet = database.clone();
    let fl_faucet = faucet_limiter.clone();
    let pk_faucet = node_public_key.clone();
    let sk_faucet = secret_key.clone();
    let tx_faucet = tx_out.clone();
    let faucet_route = warp::path("faucet")
        .and(warp::post())
        .and(warp::body::bytes())
        .and(with_state((l_faucet, db_faucet, fl_faucet, pk_faucet, sk_faucet, tx_faucet)))
        .then(#[allow(clippy::type_complexity)] |body: bytes::Bytes, (l, db, rate_lim, node_pk, node_sk, tx): (Arc<Mutex<Ledger>>, Arc<LosDatabase>, Arc<EndpointRateLimiter>, Vec<u8>, Zeroizing<Vec<u8>>, mpsc::Sender<String>)| async move {
            // Parse JSON manually to return proper error instead of 500
            let req: serde_json::Value = match serde_json::from_slice(&body) {
                Ok(r) => r,
                Err(e) => {
                    return api_json(serde_json::json!({
                        "status": "error",
                        "code": 400,
                        "msg": format!("Invalid request body: {}", e)
                    }));
                }
            };
            // BELT-AND-SUSPENDERS: Explicit compile-time mainnet guard.
            if los_core::is_mainnet_build() {
                return api_json(serde_json::json!({
                    "status": "error",
                    "code": 403,
                    "msg": "Faucet is permanently disabled on mainnet"
                }));
            }

            if !testnet_config::get_testnet_config().should_enable_faucet() {
                return api_json(serde_json::json!({
                    "status": "error",
                    "code": 403,
                    "msg": "Faucet only available in Functional/Consensus testnet modes"
                }));
            }

            let address = req["address"].as_str().unwrap_or("");
            if address.is_empty() {
                return api_json(serde_json::json!({
                    "status": "error",
                    "code": 400,
                    "msg": "Address required"
                }));
            }

            // Validate address format (Base58Check with LOS prefix)
            if !los_crypto::validate_address(address) {
                return api_json(serde_json::json!({
                    "status": "error",
                    "code": 400,
                    "msg": "Invalid address format. Must be Base58Check with LOS prefix."
                }));
            }

            // PERSISTENT cooldown: 1 faucet claim per 2 minutes per address (survives restart)
            const FAUCET_COOLDOWN_SECS: u64 = 120; // 2 minutes (testnet-friendly)
            if let Err(remaining) = db.check_faucet_cooldown(address, FAUCET_COOLDOWN_SECS) {
                return api_json(serde_json::json!({
                    "status": "error",
                    "code": 429,
                    "msg": format!("Faucet cooldown active: try again in {} seconds", remaining)
                }));
            }

            // In-memory rate limit as secondary protection
            if let Err(wait_secs) = rate_lim.check_and_record(address) {
                return api_json(serde_json::json!({
                    "status": "error",
                    "code": 429,
                    "msg": format!("Rate limit exceeded: max 1 faucet claim per 2 minutes. Try again in {} seconds.", wait_secs)
                }));
            }

            let faucet_amount = FAUCET_AMOUNT_CIL; // 5k LOS per faucet request (testnet only)

            // All ledger work in a contained scope so MutexGuard is dropped before .await
            let faucet_result: Result<(String, String, u128, u128), String> = {
                let mut l_guard = safe_lock(&l);

                // Ensure account exists
                if !l_guard.accounts.contains_key(address) {
                    l_guard.accounts.insert(address.to_string(), AccountState {
                        head: "0".to_string(),
                        balance: 0,
                        block_count: 0,
                        is_validator: false,
                    });
                }

                let state = l_guard.accounts.get(address).cloned().unwrap_or(AccountState {
                    head: "0".to_string(),
                    balance: 0,
                    block_count: 0,
                    is_validator: false,
                });

                // Create proper Mint block with PoW + signature, use process_block()
                // This ensures remaining_supply is properly deducted
                let mut faucet_block = Block {
                    account: address.to_string(),
                    previous: state.head.clone(),
                    block_type: BlockType::Mint,
                    amount: faucet_amount,
                    link: format!("FAUCET:TESTNET:{}", std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()),
                    signature: "".to_string(),
                    public_key: hex::encode(&node_pk),
                    work: 0,
                    timestamp: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs(),
                    fee: 0,
                };

                solve_pow(&mut faucet_block);
                faucet_block.signature = match try_sign_hex(faucet_block.signing_hash().as_bytes(), &node_sk) {
                    Ok(sig) => sig,
                    Err(e) => {
                        let _err_msg = format!("Faucet signing failed: {}", e);
                        return api_json(serde_json::json!({"status": "error", "code": 500, "msg": _err_msg}));
                    }
                };

                match l_guard.process_block(&faucet_block) {
                    Ok(result) => {
                        let hash = result.into_hash();
                        let new_balance = l_guard.accounts.get(address)
                            .map(|a| a.balance).unwrap_or(0);
                        let _ = db.record_faucet_claim(address);
                        SAVE_DIRTY.store(true, Ordering::Release);
                        let gossip_msg = serde_json::to_string(&faucet_block).unwrap_or_default();
                        Ok((hash, gossip_msg, faucet_amount / CIL_PER_LOS, new_balance / CIL_PER_LOS))
                    }
                    Err(e) => Err(format!("Faucet mint failed: {}", e))
                }
            }; // l_guard dropped here — safe to .await below

            match faucet_result {
                Ok((hash, gossip_msg, amount_los, balance_los)) => {
                    // GOSSIP: Broadcast faucet Mint block to all peers
                    let _ = tx.send(gossip_msg).await;

                    api_json(serde_json::json!({
                        "status": "success",
                        "msg": "Faucet claim successful",
                        "amount": amount_los,
                        "new_balance": balance_los,
                        "block_hash": hash
                    }))
                }
                Err(e) => {
                    api_json(serde_json::json!({
                        "status": "error",
                        "code": 500,
                        "msg": e
                    }))
                }
            }
        });

    // 16. GET /blocks/recent (Recent blocks for validator dashboard)
    let l_blocks = ledger.clone();
    let blocks_recent_route = warp::path!("blocks" / "recent")
        .and(with_state(l_blocks))
        .map(|l: Arc<Mutex<Ledger>>| {
            let l_guard = safe_lock(&l);
            // Collect only blocks that are part of valid account chains.
            // Walk each account's chain from head backwards to gather chain block hashes.
            let mut chain_hashes: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            for acct in l_guard.accounts.values() {
                let mut current = acct.head.clone();
                while current != "0" && !current.is_empty() {
                    if !chain_hashes.insert(current.clone()) {
                        break;
                    }
                    if let Some(blk) = l_guard.blocks.get(&current) {
                        current = blk.previous.clone();
                    } else {
                        break;
                    }
                }
            }
            // Filter and sort chain blocks by timestamp descending
            let mut block_list: Vec<(&String, &Block)> = l_guard
                .blocks
                .iter()
                .filter(|(hash, _)| chain_hashes.contains(*hash))
                .collect();
            block_list.sort_by(|a, b| b.1.timestamp.cmp(&a.1.timestamp));
            // total_blocks = sum of all account chain block counts (excludes orphans)
            let total_blocks = l_guard.total_chain_blocks();
            let blocks: Vec<serde_json::Value> = block_list
                .iter()
                .take(10) // Last 10 blocks by timestamp
                .enumerate()
                .map(|(i, (hash, b))| {
                    // Per-account block count as individual height (block-lattice = per-account chain)
                    let account_block_count = l_guard
                        .accounts
                        .get(&b.account)
                        .map(|a| a.block_count)
                        .unwrap_or(0);
                    serde_json::json!({
                        "hash": hash,
                        "height": account_block_count,
                        "global_index": total_blocks as usize - i,
                        "timestamp": b.timestamp,
                        "transactions_count": 1,
                        "account": b.account,
                        "amount": b.amount,
                        "amount_los": b.amount / CIL_PER_LOS,
                        "block_type": format!("{:?}", b.block_type).to_lowercase()
                    })
                })
                .collect();
            api_json(serde_json::json!({
                "blocks": blocks,
                "total_blocks": total_blocks
            }))
        });

    // 17. GET /whoami (Get node's internal signing address)
    let whoami_route = warp::path("whoami")
        .and(with_state(my_address.clone()))
        .map(|addr: String| {
            api_json(serde_json::json!({
                "address": addr,
                "short": get_short_addr(&addr),
                "format": "hex-encoded"
            }))
        });

    // 18. GET /account/:address (Account details - balance + history combined)
    let l_account = ledger.clone();
    let account_route = warp::path!("account" / String)
        .and(with_state(l_account))
        .map(|addr: String, l: Arc<Mutex<Ledger>>| {
            let l_guard = safe_lock(&l);
            let state = l_guard
                .accounts
                .get(&addr)
                .cloned()
                .unwrap_or(AccountState {
                    head: "0".to_string(),
                    balance: 0,
                    block_count: 0,
                    is_validator: false,
                });

            // Get transaction history for this account
            // Walk the account chain from head backwards via `previous` links.
            // This ensures ONLY blocks in the valid chain are shown (no orphans).
            let mut transactions: Vec<serde_json::Value> = Vec::new();
            let mut chain_blocks: Vec<(String, Block)> = Vec::new();
            {
                let mut current = state.head.clone();
                while current != "0" && !current.is_empty() {
                    if let Some(blk) = l_guard.blocks.get(&current) {
                        chain_blocks.push((current.clone(), blk.clone()));
                        current = blk.previous.clone();
                    } else {
                        break;
                    }
                }
            }
            for (hash, block) in chain_blocks.iter() {
                if block.account == addr {
                    // Resolve `from` address: for Receive blocks, look up the Send block
                    // to get the actual sender instead of showing "SYSTEM"
                    let from_addr = match block.block_type {
                        BlockType::Send => block.account.clone(),
                        BlockType::Receive => {
                            // block.link = hash of the Send block that funded this Receive
                            l_guard.blocks.get(&block.link)
                                .map(|send_blk| send_blk.account.clone())
                                .unwrap_or_else(|| "SYSTEM".to_string())
                        },
                        _ => "SYSTEM".to_string(), // Mint, Slash, Change
                    };
                    // Resolve `to` address
                    let to_addr = match block.block_type {
                        BlockType::Receive => block.account.clone(),
                        _ => block.link.clone(), // Send→recipient, Mint→link
                    };
                    transactions.push(serde_json::json!({
                        "hash": hash,
                        "from": from_addr,
                        "to": to_addr,
                        "type": format!("{:?}", block.block_type).to_lowercase(),
                        "amount": format!("{}.{:011}", block.amount / CIL_PER_LOS, block.amount % CIL_PER_LOS),
                        "timestamp": block.timestamp,
                        "link": block.link,
                        "previous": block.previous,
                        "fee": block.fee
                    }));
                }
            }

            api_json(serde_json::json!({
                "address": addr,
                "balance": format_balance_precise(state.balance),
                "balance_los": format_balance_precise(state.balance),
                "balance_cil": state.balance,
                "balance_cil_str": state.balance.to_string(),
                "block_count": state.block_count,
                "head_block": state.head,
                "is_validator": state.is_validator,
                "transactions": transactions,
                "transaction_count": transactions.len()
            }))
        });

    // 19. GET / (Root endpoint - API welcome)
    let root_route = warp::path::end().map(|| {
        let network_label = if los_core::is_mainnet_build() {
            "mainnet"
        } else {
            "testnet"
        };
        api_json(serde_json::json!({
            "name": "Unauthority (LOS) Blockchain API",
            "version": env!("CARGO_PKG_VERSION"),
            "network": network_label,
            "description": "Decentralized blockchain with aBFT consensus",
            "endpoints": {
                "health": "GET /health - Health check",
                "node_info": "GET /node-info - Node information",
                "bal": "GET /bal/{address} - Account balance (short alias)",
                "balance": "GET /balance/{address} - Account balance",
                "supply": "GET /supply - Total supply, circulating, remaining",
                "fee_estimate": "GET /fee-estimate/{address} - Fee estimate (flat base fee)",
                "mining_info": "GET /mining-info - PoW mining epoch, difficulty, reward info",

                "account": "GET /account/{address} - Account details + history",
                "history": "GET /history/{address} - Transaction history",
                "validators": "GET /validators - Active validators",
                "peers": "GET /peers - Connected peers + validator endpoints",
                "network_peers": "GET /network/peers - Validator .onion endpoint discovery",
                "block": "GET /block - Latest block",
                "block_by_hash": "GET /block/{hash} - Block by hash",
                "blocks_recent": "GET /blocks/recent - Recent blocks",
                "transaction": "GET /transaction/{hash} - Transaction by hash",
                "search": "GET /search/{query} - Search addresses, blocks, transactions",
                "whoami": "GET /whoami - Node's signing address",
                "consensus": "GET /consensus - aBFT consensus parameters and safety status",
                "reward_info": "GET /reward-info - Validator reward pool status and epoch info",
                "slashing": "GET /slashing - Slashing statistics",
                "slashing_profile": "GET /slashing/{address} - Validator slashing profile",
                "sync": "GET /sync - Node sync status",
                "metrics": "GET /metrics - Prometheus metrics",
                "mempool_stats": "GET /mempool/stats - Mempool statistics",
                "send": "POST /send {from, target, amount} - Send transaction",
                "faucet": "POST /faucet {address} - Claim testnet tokens",
                "register_validator": "POST /register-validator - Register as validator",
                "unregister_validator": "POST /unregister-validator - Unregister validator",
                "deploy_contract": "POST /deploy-contract - Deploy WASM smart contract",
                "call_contract": "POST /call-contract - Call smart contract method",
                "contract": "GET /contract/{address} - Contract info and state",
                "tokens": "GET /tokens - List all USP-01 tokens",
                "token_info": "GET /token/{address} - USP-01 token metadata",
                "token_balance": "GET /token/{address}/balance/{holder} - Token balance",
                "token_allowance": "GET /token/{address}/allowance/{owner}/{spender} - Token allowance",
                "dex_pools": "GET /dex/pools - List all DEX pools",
                "dex_pool": "GET /dex/pool/{contract}/{pool_id} - Pool info",
                "dex_quote": "GET /dex/quote/{contract}/{pool_id}/{token_in}/{amount} - Swap quote",
                "dex_position": "GET /dex/position/{contract}/{pool_id}/{user} - LP position"
            },
            "docs": "https://github.com/monkey-king-code/unauthority-core",
            "status": "operational"
        }))
    });

    // 20. GET /slashing (Slashing statistics and validator safety)
    let sm_stats = slashing_manager.clone();
    let slashing_route =
        warp::path!("slashing")
            .and(with_state(sm_stats))
            .map(|sm: Arc<Mutex<SlashingManager>>| {
                let sm_guard = safe_lock(&sm);
                let stats = sm_guard.get_safety_stats();
                let banned = sm_guard.get_banned_validators();
                let slashed = sm_guard.get_slashed_validators();
                let events = sm_guard.get_all_slash_events();

                let events_json: Vec<serde_json::Value> = events
                    .iter()
                    .map(|e| {
                        serde_json::json!({
                            "block_height": e.block_height,
                            "validator": e.validator_address,
                            "violation": format!("{:?}", e.violation_type),
                            "slash_amount_cil": e.slash_amount_cil,
                            "slash_bps": e.slash_bps,
                            "timestamp": e.timestamp
                        })
                    })
                    .collect();

                api_json(serde_json::json!({
                    "safety_stats": {
                        "total_validators": stats.total_validators,
                        "active_validators": stats.active_validators,
                        "banned_count": stats.banned_count,
                        "slashed_count": stats.slashed_count,
                        "total_slashed_cil": stats.total_slashed_cil,
                        "total_slash_events": stats.total_slash_events
                    },
                    "banned_validators": banned,
                    "slashed_validators": slashed,
                    "recent_events": events_json
                }))
            });

    // 21. GET /slashing/:address (Validator-specific slashing info)
    let sm_profile = slashing_manager.clone();
    let slashing_profile_route = warp::path!("slashing" / String)
        .and(with_state(sm_profile))
        .map(|addr: String, sm: Arc<Mutex<SlashingManager>>| {
            let sm_guard = safe_lock(&sm);
            if let Some(profile) = sm_guard.get_profile(&addr) {
                let history: Vec<serde_json::Value> = profile
                    .slash_history
                    .iter()
                    .map(|e| {
                        serde_json::json!({
                            "block_height": e.block_height,
                            "violation": format!("{:?}", e.violation_type),
                            "slash_amount_cil": e.slash_amount_cil,
                            "slash_bps": e.slash_bps,
                            "timestamp": e.timestamp
                        })
                    })
                    .collect();

                api_json(serde_json::json!({
                    "address": addr,
                    "status": format!("{:?}", profile.status),
                    "uptime_bps": profile.get_uptime_bps(),
                    "uptime_percent_x100": profile.get_uptime_bps(),  // 9500 = 95.00%
                    "total_slashed_cil": profile.total_slashed_cil,
                    "violation_count": profile.violation_count,
                    "blocks_participated": profile.blocks_participated,
                    "total_blocks_observed": profile.total_blocks_observed,
                    "slash_history": history
                }))
            } else {
                api_json(serde_json::json!({
                    "error": "Validator not found in slashing manager",
                    "address": addr
                }))
            }
        });

    // 22. GET /health (Health check endpoint)
    let l_health = ledger.clone();
    let db_health = database.clone();
    let health_route = warp::path("health")
        .and(with_state((l_health, db_health)))
        .map(move |(l, db): (Arc<Mutex<Ledger>>, Arc<LosDatabase>)| {
            let l_guard = safe_lock(&l);
            let db_stats = db.stats();

            // Check system health
            let is_healthy = !l_guard.accounts.is_empty() && db_stats.accounts_count > 0;
            let status = if is_healthy { "healthy" } else { "degraded" };

            api_json(serde_json::json!({
                "status": status,
                "uptime_seconds": start_time.elapsed().as_secs(),
                "chain": {
                    "id": if los_core::is_mainnet_build() { "los-mainnet" } else { "los-testnet" },
                    "accounts": l_guard.accounts.len(),
                    "blocks": l_guard.total_chain_blocks()
                },
                "database": {
                    "accounts_count": db_stats.accounts_count,
                    "blocks_count": db_stats.blocks_count,
                    "size_on_disk": db_stats.size_on_disk
                },
                "version": env!("CARGO_PKG_VERSION"),
                "timestamp": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            }))
        });

    // 22b. GET /tor-health (Tor Hidden Service reachability status)
    let tor_health_m = metrics.clone();
    let tor_health_route =
        warp::path("tor-health")
            .and(with_state(tor_health_m))
            .map(move |m: Arc<LosMetrics>| {
                let reachable = m.tor_onion_reachable.get();
                let consecutive_failures = m.tor_consecutive_failures.get();
                let total_pings = m.tor_self_ping_total.get();
                let total_failures = m.tor_self_ping_failures_total.get();
                let onion_addr = std::env::var("LOS_ONION_ADDRESS").unwrap_or_default();

                let status = match reachable {
                    1 => "reachable",
                    0 => "unreachable",
                    _ => "not_configured",
                };

                api_json(serde_json::json!({
                    "status": status,
                    "onion_address": onion_addr,
                    "reachable": reachable == 1,
                    "consecutive_failures": consecutive_failures,
                    "total_pings": total_pings,
                    "total_failures": total_failures,
                    "timestamp": std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                }))
            });

    // 23. GET /block/:hash (Block explorer - get block by hash)
    let l_block_hash = ledger.clone();
    let block_by_hash_route = warp::path!("block" / String)
        .and(with_state(l_block_hash))
        .map(|hash: String, l: Arc<Mutex<Ledger>>| {
            let l_guard = safe_lock(&l);
            if let Some(block) = l_guard.blocks.get(&hash) {
                api_json(serde_json::json!({
                    "status": "success",
                    "block": {
                        "hash": hash,
                        "account": block.account,
                        "previous": block.previous,
                        "type": format!("{:?}", block.block_type),
                        "amount": block.amount / CIL_PER_LOS,
                        "amount_cil": block.amount,
                        "link": block.link,
                        "signature": block.signature,
                        "public_key": block.public_key,
                        "work": block.work,
                        "timestamp": block.timestamp
                    }
                }))
            } else {
                api_json(serde_json::json!({
                    "status": "error",
                    "msg": format!("Block not found: {}", hash)
                }))
            }
        });

    // 24. GET /transaction/:hash (Alias for block by hash - block explorer compatibility)
    let l_tx_hash = ledger.clone();
    let tx_by_hash_route = warp::path!("transaction" / String)
        .and(with_state(l_tx_hash))
        .map(|hash: String, l: Arc<Mutex<Ledger>>| {
            let l_guard = safe_lock(&l);
            if let Some(block) = l_guard.blocks.get(&hash) {
                api_json(serde_json::json!({
                    "status": "success",
                    "transaction": {
                        "hash": hash,
                        "from": block.account.clone(),
                        "to": if block.block_type == BlockType::Send { block.link.clone() } else { block.account.clone() },
                        "type": format!("{:?}", block.block_type),
                        "amount": block.amount / CIL_PER_LOS,
                        "amount_cil": block.amount,
                        "timestamp": block.timestamp,
                        "signature": block.signature,
                        "confirmed": true
                    }
                }))
            } else {
                api_json(serde_json::json!({
                    "status": "error",
                    "msg": format!("Transaction not found: {}", hash)
                }))
            }
        });

    // 25. GET /search/:query (Block explorer - search for address, block, or transaction)
    let l_search = ledger.clone();
    let ab_search = address_book.clone();
    let search_route = warp::path!("search" / String)
        .and(with_state((l_search, ab_search)))
        .map(
            #[allow(clippy::type_complexity)]
            |query: String, (l, ab): (Arc<Mutex<Ledger>>, Arc<Mutex<HashMap<String, String>>>)| {
                let l_guard = safe_lock(&l);
                let mut results = Vec::new();

                // Check if it's a full address
                if l_guard.accounts.contains_key(&query) {
                    if let Some(acc) = l_guard.accounts.get(&query) {
                        results.push(serde_json::json!({
                            "type": "account",
                            "address": query,
                            "balance": acc.balance / CIL_PER_LOS,
                            "block_count": acc.block_count
                        }));
                    }
                }

                // Check if it's a block hash
                if l_guard.blocks.contains_key(&query) {
                    results.push(serde_json::json!({
                        "type": "block",
                        "hash": query
                    }));
                }

                // Check if it's a short address
                let ab_guard = safe_lock(&ab);
                if let Some(full) = ab_guard.get(&query) {
                    if let Some(acc) = l_guard.accounts.get(full) {
                        results.push(serde_json::json!({
                            "type": "account",
                            "address": full,
                            "short_address": query,
                            "balance": acc.balance / CIL_PER_LOS,
                            "block_count": acc.block_count
                        }));
                    }
                }

                // Partial match on addresses
                if results.is_empty() {
                    for (addr, acc) in l_guard.accounts.iter() {
                        if addr.contains(&query) {
                            results.push(serde_json::json!({
                                "type": "account",
                                "address": addr,
                                "balance": acc.balance / CIL_PER_LOS,
                                "block_count": acc.block_count
                            }));
                            if results.len() >= 10 {
                                break;
                            } // Limit to 10 results
                        }
                    }
                }

                api_json(serde_json::json!({
                    "query": query,
                    "results": results,
                    "count": results.len()
                }))
            },
        );

    // CORS configuration
    // SECURITY: Behind Tor hidden service, browser requests come from .onion origin.
    // Allow any origin since Tor hidden services are already access-controlled by
    // the .onion address itself. Same-origin would block legitimate Tor Browser users.
    let cors = if los_core::is_mainnet_build() {
        warp::cors()
            .allow_any_origin() // .onion addresses serve as access control
            .allow_methods(vec!["GET", "POST", "OPTIONS"])
            .allow_headers(vec!["Content-Type", "Accept"])
    } else {
        warp::cors()
            .allow_any_origin()
            .allow_methods(vec!["GET", "POST", "PUT", "DELETE", "OPTIONS"])
            .allow_headers(vec!["Content-Type", "Authorization", "Accept"])
    };

    // 26. GET /sync (HTTP-based state sync for Tor peers)
    // Returns JSON ledger state for peers that connect via HTTP.
    // P2P gossip uses SYNC_GZIP (base64-encoded gzip) for peer-to-peer sync.
    let l_sync = ledger.clone();
    let sync_route = warp::path("sync")
        .and(warp::query::<std::collections::HashMap<String, String>>())
        .and(with_state(l_sync))
        .map(
            |params: std::collections::HashMap<String, String>, l: Arc<Mutex<Ledger>>| {
                let their_blocks: usize = params
                    .get("blocks")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);

                let l_guard = safe_lock(&l);
                let our_blocks = l_guard.blocks.len();

                // Only send state if we have more blocks
                if our_blocks <= their_blocks {
                    return api_json(serde_json::json!({
                        "status": "up_to_date",
                        "blocks": our_blocks
                    }));
                }

                // Collect non-Mint/Slash blocks only (those must go through consensus)
                let sync_blocks: std::collections::HashMap<String, &los_core::Block> = l_guard
                    .blocks
                    .iter()
                    .filter(|(_, b)| !matches!(b.block_type, BlockType::Mint | BlockType::Slash))
                    .take(5000) // Cap at 5000 blocks per sync
                    .map(|(k, v)| (k.clone(), v))
                    .collect();

                let accounts_snapshot: std::collections::HashMap<&String, &AccountState> =
                    l_guard.accounts.iter().collect();

                api_json(serde_json::json!({
                    "status": "sync",
                    "blocks": sync_blocks,
                    "accounts": accounts_snapshot,
                    "our_block_count": our_blocks,
                    "distribution": l_guard.distribution
                }))
            },
        );

    // 26b. GET /sync/full — Streaming gzip-compressed full ledger state.
    // Unlike SYNC_GZIP (gossip, capped at 8MB), this has NO size limit.
    // Used by REST-based sync fallback when state exceeds gossip capacity.
    // Returns: Content-Encoding: gzip, Content-Type: application/octet-stream
    let l_sync_full = ledger.clone();
    let sync_full_route = warp::path!("sync" / "full")
        .and(warp::get())
        .and(warp::query::<std::collections::HashMap<String, String>>())
        .and(with_state(l_sync_full))
        .map(
            |params: std::collections::HashMap<String, String>, l: Arc<Mutex<Ledger>>| {
                let their_blocks: usize = params
                    .get("blocks")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);

                let l_guard = safe_lock(&l);
                let our_blocks = l_guard.blocks.len();

                // Only send state if we have more blocks
                if our_blocks <= their_blocks {
                    let body = serde_json::json!({"status": "up_to_date", "blocks": our_blocks})
                        .to_string();
                    return warp::http::Response::builder()
                        .header("Content-Type", "application/json")
                        .body(body.into_bytes())
                        .unwrap_or_default();
                }

                // Serialize full ledger state and gzip compress
                let json = serde_json::to_string(&*l_guard).unwrap_or_default();
                drop(l_guard); // Release lock before compression

                use flate2::write::GzEncoder;
                use flate2::Compression;
                use std::io::Write;
                let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
                let _ = encoder.write_all(json.as_bytes());
                let compressed = encoder.finish().unwrap_or_default();

                println!(
                    "📤 REST /sync/full: {} blocks, {:.1} KB compressed",
                    our_blocks,
                    compressed.len() as f64 / 1024.0
                );

                warp::http::Response::builder()
                    .header("Content-Type", "application/octet-stream")
                    .header("Content-Encoding", "gzip")
                    .header("X-Block-Count", our_blocks.to_string())
                    .body(compressed)
                    .unwrap_or_default()
            },
        );

    // 27. GET /consensus (aBFT consensus parameters and safety status)
    let abft_consensus_route = abft_consensus.clone();
    let l_consensus = ledger.clone();
    let consensus_route = warp::path("consensus")
        .and(with_state((abft_consensus_route, l_consensus)))
        .map(
            |(abft, l): (Arc<Mutex<ABFTConsensus>>, Arc<Mutex<Ledger>>)| {
                let abft_guard = safe_lock(&abft);
                let l_guard = safe_lock(&l);
                let stats = abft_guard.get_statistics();
                // active_validators must only count accounts that are BOTH
                // registered validators AND have sufficient stake. Previously this
                // counted all accounts with enough balance, inflating the number.
                let active_validators = l_guard
                    .accounts
                    .iter()
                    .filter(|(_, a)| a.is_validator && a.balance >= MIN_VALIDATOR_STAKE_CIL)
                    .count();

                api_json(serde_json::json!({
                    "protocol": "aBFT (Weighted Confirmation)",
                    "safety": {
                        "byzantine_safe": abft_guard.is_byzantine_safe(0),
                        "total_validators": stats.total_validators,
                        "active_validators": active_validators,
                        "max_faulty": stats.max_faulty_validators,
                        "quorum_threshold": stats.quorum_threshold,
                        "formula": "f < n/3, quorum = 2f+1"
                    },
                    "confirmation": {
                        "send_threshold": SEND_CONSENSUS_THRESHOLD,
                        "voting_model": "linear (stake_cil)",
                        "signature_scheme": "Dilithium5 (post-quantum)"
                    },
                    "finality": {
                        "target_ms": 3000,
                        "blocks_finalized": stats.blocks_finalized,
                        "current_view": stats.current_view,
                        "current_sequence": stats.current_sequence
                    }
                }))
            },
        );

    // 28. GET /reward-info (Validator reward pool status)
    let rp_info = reward_pool.clone();
    let reward_info_route = warp::path("reward-info").and(with_state(rp_info)).map(
        |rp: Arc<Mutex<ValidatorRewardPool>>| {
            let pool = safe_lock(&rp);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let summary = pool.pool_summary();
            let remaining_secs = pool.epoch_remaining_secs(now);

            // Per-validator reward details
            let epoch_elapsed_secs = now.saturating_sub(pool.epoch_start_timestamp);
            let validators_json: Vec<serde_json::Value> = pool
                .validators
                .iter()
                .map(|(addr, v)| {
                    // Show whether this validator joined mid-epoch
                    let joined_this_epoch = v.join_epoch == pool.current_epoch;
                    serde_json::json!({
                        "address": addr,
                        "is_genesis": v.is_genesis,
                        "join_epoch": v.join_epoch,
                        "joined_this_epoch": joined_this_epoch,
                        "stake_cil": v.stake_cil,
                        "uptime_pct": v.display_uptime_pct(),
                        "cumulative_rewards_cil": v.cumulative_rewards_cil,
                        "eligible": v.is_eligible(pool.current_epoch),
                        "heartbeats_current_epoch": v.heartbeats_current_epoch,
                        "expected_heartbeats": v.expected_heartbeats,
                    })
                })
                .collect();

            api_json(serde_json::json!({
                "pool": {
                    "remaining_cil": summary.remaining_cil,
                    "remaining_los": format_balance_precise(summary.remaining_cil),
                    "total_distributed_cil": summary.total_distributed_cil,
                    "total_distributed_los": format_balance_precise(summary.total_distributed_cil),
                    "pool_exhaustion_bps": summary.pool_exhaustion_bps,
                },
                "epoch": {
                    "current_epoch": summary.current_epoch,
                    "epoch_reward_rate_cil": summary.epoch_reward_rate_cil,
                    "epoch_reward_rate_los": format_balance_precise(summary.epoch_reward_rate_cil),
                    "halvings_occurred": summary.halvings_occurred,
                    "epoch_elapsed_secs": epoch_elapsed_secs,
                    "epoch_remaining_secs": remaining_secs,
                    "epoch_duration_secs": pool.epoch_duration_secs,
                },
                "validators": {
                    "total": summary.total_validators,
                    "eligible": summary.eligible_validators,
                    "details": validators_json,
                },
                "config": {
                    "min_uptime_pct": los_core::REWARD_MIN_UPTIME_PCT,
                    "probation_epochs": los_core::REWARD_PROBATION_EPOCHS,
                    "halving_interval_epochs": los_core::REWARD_HALVING_INTERVAL_EPOCHS,
                    "distribution_model": "linear stake-weighted proportional",
                    "genesis_excluded": false,
                }
            }))
        },
    );

    // 29. POST /register-validator (Register as an active validator)
    // Requires proof of ownership via Dilithium5 signature + minimum stake.
    // Sets is_validator = true, registers in SlashingManager and RewardPool,
    // updates aBFT validator set dynamically, and broadcasts to peers.
    let l_regval = ledger.clone();
    let sm_regval = slashing_manager.clone();
    let rp_regval = reward_pool.clone();
    let tx_regval = tx_out.clone();
    let bv_regval = bootstrap_validators.clone();
    let db_regval = database.clone();
    let abft_regval = abft_consensus.clone();
    let ve_regval = validator_endpoints.clone();
    let lrv_regval = local_registered_validators.clone();
    let register_validator_route = warp::path("register-validator")
        .and(warp::post())
        .and(warp::body::bytes())
        .and(with_state((l_regval, sm_regval, rp_regval, tx_regval, db_regval)))
        .then(#[allow(clippy::type_complexity)] move |body: bytes::Bytes, (l, sm, rp, tx, db): (Arc<Mutex<Ledger>>, Arc<Mutex<SlashingManager>>, Arc<Mutex<ValidatorRewardPool>>, mpsc::Sender<String>, Arc<LosDatabase>)| {
            let bv_inner = bv_regval.clone();
            let abft_inner = abft_regval.clone();
            let ve_inner = ve_regval.clone();
            let lrv_inner = lrv_regval.clone();
            async move {
            // Parse JSON manually to return proper 400 instead of 500
            let req: serde_json::Value = match serde_json::from_slice(&body) {
                Ok(r) => r,
                Err(e) => {
                    return api_json(serde_json::json!({
                        "status": "error",
                        "code": 400,
                        "msg": format!("Invalid request body: {}", e)
                    }));
                }
            };
            // Parse required fields
            let address = match req["address"].as_str() {
                Some(a) if !a.is_empty() => a.to_string(),
                _ => return api_json(serde_json::json!({
                    "status": "error",
                    "msg": "Missing 'address' field"
                })),
            };
            let public_key = match req["public_key"].as_str() {
                Some(pk) if !pk.is_empty() => pk.to_string(),
                _ => return api_json(serde_json::json!({
                    "status": "error",
                    "msg": "Missing 'public_key' field"
                })),
            };
            let signature = match req["signature"].as_str() {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => return api_json(serde_json::json!({
                    "status": "error",
                    "msg": "Missing 'signature' field"
                })),
            };
            let timestamp = req["timestamp"].as_u64().unwrap_or(0);

            // 1. Validate address format
            if !los_crypto::validate_address(&address) {
                return api_json(serde_json::json!({
                    "status": "error",
                    "msg": "Invalid address format"
                }));
            }

            // 2. Verify public_key derives to address (proves key ownership)
            let pk_bytes = match hex::decode(&public_key) {
                Ok(b) => b,
                Err(_) => return api_json(serde_json::json!({
                    "status": "error",
                    "msg": "Invalid public_key hex encoding"
                })),
            };
            let derived_addr = los_crypto::public_key_to_address(&pk_bytes);
            if derived_addr != address {
                return api_json(serde_json::json!({
                    "status": "error",
                    "msg": "public_key does not match address"
                }));
            }

            // 3. Verify signature (message = "REGISTER_VALIDATOR:<address>:<timestamp>")
            let message = format!("REGISTER_VALIDATOR:{}:{}", address, timestamp);
            let sig_bytes = match hex::decode(&signature) {
                Ok(b) => b,
                Err(_) => return api_json(serde_json::json!({
                    "status": "error",
                    "msg": "Invalid signature hex encoding"
                })),
            };
            if !los_crypto::verify_signature(message.as_bytes(), &sig_bytes, &pk_bytes) {
                return api_json(serde_json::json!({
                    "status": "error",
                    "msg": "Signature verification failed"
                }));
            }

            // 4. Timestamp freshness check (prevent replay attacks, allow 5 min window)
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if timestamp == 0 || now.abs_diff(timestamp) > 300 {
                return api_json(serde_json::json!({
                    "status": "error",
                    "msg": "Timestamp too old or missing (max 5 minute window)"
                }));
            }

            // 5. Check balance & register atomically (single lock scope prevents TOCTOU race)
            let reg_result = {
                let mut l_guard = safe_lock(&l);
                match l_guard.accounts.get_mut(&address) {
                    Some(acc) => {
                        if acc.is_validator || bv_inner.contains(&address) {
                            Err("already_validator")
                        } else if acc.balance < MIN_VALIDATOR_REGISTER_CIL {
                            Err("insufficient_stake")
                        } else {
                            // 6. Set is_validator = true atomically with the check
                            acc.is_validator = true;
                            Ok(acc.balance)
                        }
                    }
                    None => Err("insufficient_stake"), // balance = 0
                }
            };

            let balance = match reg_result {
                Err("already_validator") => {
                    return api_json(serde_json::json!({
                        "status": "ok",
                        "msg": "Already registered as validator",
                        "address": address,
                        "is_validator": true,
                        "is_genesis": bv_inner.contains(&address),
                    }));
                }
                Err(_) => {
                    let min_los = MIN_VALIDATOR_REGISTER_CIL / CIL_PER_LOS;
                    return api_json(serde_json::json!({
                        "status": "error",
                        "msg": format!("Insufficient stake: need {} LOS, have 0 LOS", min_los)
                    }));
                }
                Ok(balance) => balance,
            };

            // 7. Register in SlashingManager
            {
                let mut sm_guard = safe_lock(&sm);
                if sm_guard.get_profile(&address).is_none() {
                    sm_guard.register_validator(address.clone());
                }
            }

            // 8. Register in RewardPool (non-genesis = registered via API)
            {
                let mut rp_guard = safe_lock(&rp);
                rp_guard.register_validator(&address, false, balance);
            }

            // 8b. Track as locally-registered validator for heartbeat forwarding.
            // This node's liveness proves the registered wallet's liveness,
            // so the heartbeat loop will record heartbeats for this address.
            {
                let mut lrv: std::sync::MutexGuard<'_, HashSet<String>> = safe_lock(&lrv_inner);
                lrv.insert(address.clone());
            }

            // 9. Mark ledger dirty for persistence
            SAVE_DIRTY.store(true, Ordering::Release);

            // 9b. Dynamically update aBFT validator set so new validator
            // participates in consensus immediately (no restart required).
            {
                let l_guard = safe_lock(&l);
                let mut validators: Vec<String> = l_guard
                    .accounts
                    .iter()
                    .filter(|(_, a)| a.balance >= MIN_VALIDATOR_REGISTER_CIL && a.is_validator)
                    .map(|(addr, _)| addr.clone())
                    .collect();
                validators.sort();
                safe_lock(&abft_inner).update_validator_set(validators);
            }

            // 10. Broadcast to peers so they also register this validator
            // Use the registering validator's host_address if provided in the request,
            // then try onion_address, then fall back to this node's own host address.
            let raw_host_addr = req["host_address"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .or_else(|| req["onion_address"].as_str().filter(|s| !s.is_empty()).map(|s| s.to_string()))
                .or_else(get_node_host_address);
            // Ensure host includes port for peer discovery
            let host_addr = raw_host_addr.map(|h| ensure_host_port(&h, api_port));
            let reg_msg = serde_json::json!({
                "type": "VALIDATOR_REG",
                "address": address,
                "public_key": public_key,
                "signature": signature,
                "timestamp": timestamp,
                "host_address": host_addr,
                "onion_address": host_addr, // backward compat for older nodes
                "rest_port": api_port,
            });
            let _ = tx.send(format!("VALIDATOR_REG:{}", reg_msg)).await;

            // 10b. Store the validator's host address in our own endpoint map
            if let Some(ref host) = host_addr {
                if !host.is_empty() {
                    insert_validator_endpoint(&mut safe_lock(&ve_inner), address.clone(), host.clone());
                    println!("🌐 Stored validator endpoint: {} → {}", get_short_addr(&address), host);
                }
            }

            println!("✅ New validator registered: {} (stake: {} LOS)", get_short_addr(&address), balance / CIL_PER_LOS);

            // Persist immediately
            let _ = db.save_ledger(&safe_lock(&l));

            api_json(serde_json::json!({
                "status": "ok",
                "msg": "Validator registered successfully",
                "address": address,
                "stake_los": balance / CIL_PER_LOS,
                "is_validator": true,
                "is_genesis": false,
            }))
        }});

    // 29b. POST /unregister-validator (Voluntary validator exit / unstake)
    // Requires proof of ownership via Dilithium5 signature.
    // Sets is_validator = false, marks Unstaking in SlashingManager, removes from RewardPool,
    // updates aBFT validator set, and broadcasts to peers.
    // Also available as /unregister_validator (underscore) for CLI compatibility.
    let bv_unregval = bootstrap_validators.clone();
    let abft_unregval = abft_consensus.clone();
    let lrv_unregval = local_registered_validators.clone();
    let ve_unregval = validator_endpoints.clone();
    let unregister_handler = move |body: bytes::Bytes,
                                   (l, sm, rp, tx, db): (
        Arc<Mutex<Ledger>>,
        Arc<Mutex<SlashingManager>>,
        Arc<Mutex<ValidatorRewardPool>>,
        mpsc::Sender<String>,
        Arc<LosDatabase>,
    )| {
        let bv_inner = bv_unregval.clone();
        let abft_inner = abft_unregval.clone();
        let lrv_inner = lrv_unregval.clone();
        let ve_inner = ve_unregval.clone();
        async move {
            // Parse JSON
            let req: serde_json::Value = match serde_json::from_slice(&body) {
                Ok(r) => r,
                Err(e) => {
                    return api_json(serde_json::json!({
                        "status": "error",
                        "code": 400,
                        "msg": format!("Invalid request body: {}", e)
                    }));
                }
            };

            let address = match req["address"].as_str() {
                Some(a) if !a.is_empty() => a.to_string(),
                _ => {
                    return api_json(serde_json::json!({
                        "status": "error",
                        "msg": "Missing 'address' field"
                    }))
                }
            };
            let public_key = match req["public_key"].as_str() {
                Some(pk) if !pk.is_empty() => pk.to_string(),
                _ => {
                    return api_json(serde_json::json!({
                        "status": "error",
                        "msg": "Missing 'public_key' field"
                    }))
                }
            };
            let signature = match req["signature"].as_str() {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => {
                    return api_json(serde_json::json!({
                        "status": "error",
                        "msg": "Missing 'signature' field"
                    }))
                }
            };
            let timestamp = req["timestamp"].as_u64().unwrap_or(0);

            // 1. Validate address format
            if !los_crypto::validate_address(&address) {
                return api_json(serde_json::json!({
                    "status": "error",
                    "msg": "Invalid address format"
                }));
            }

            // 2. Verify public_key derives to address
            let pk_bytes = match hex::decode(&public_key) {
                Ok(b) => b,
                Err(_) => {
                    return api_json(serde_json::json!({
                        "status": "error",
                        "msg": "Invalid public_key hex encoding"
                    }))
                }
            };
            let derived_addr = los_crypto::public_key_to_address(&pk_bytes);
            if derived_addr != address {
                return api_json(serde_json::json!({
                    "status": "error",
                    "msg": "public_key does not match address"
                }));
            }

            // 3. Verify signature (message = "UNREGISTER_VALIDATOR:<address>:<timestamp>")
            let message = format!("UNREGISTER_VALIDATOR:{}:{}", address, timestamp);
            let sig_bytes = match hex::decode(&signature) {
                Ok(b) => b,
                Err(_) => {
                    return api_json(serde_json::json!({
                        "status": "error",
                        "msg": "Invalid signature hex encoding"
                    }))
                }
            };
            if !los_crypto::verify_signature(message.as_bytes(), &sig_bytes, &pk_bytes) {
                return api_json(serde_json::json!({
                    "status": "error",
                    "msg": "Signature verification failed"
                }));
            }

            // 4. Timestamp freshness (5 minute window)
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if timestamp == 0 || now.abs_diff(timestamp) > 300 {
                return api_json(serde_json::json!({
                    "status": "error",
                    "msg": "Timestamp too old or missing (max 5 minute window)"
                }));
            }

            // 5. Prevent genesis/bootstrap validators from unregistering
            if bv_inner.contains(&address) {
                return api_json(serde_json::json!({
                    "status": "error",
                    "msg": "Bootstrap validators cannot unregister"
                }));
            }

            // 6. Check that address is currently a validator
            let (is_validator, balance) = {
                let l_guard = safe_lock(&l);
                match l_guard.accounts.get(&address) {
                    Some(acc) => (acc.is_validator, acc.balance),
                    None => (false, 0),
                }
            };

            if !is_validator {
                return api_json(serde_json::json!({
                    "status": "error",
                    "msg": "Address is not a registered validator"
                }));
            }

            // 7. Set is_validator = false in ledger
            {
                let mut l_guard = safe_lock(&l);
                if let Some(acc) = l_guard.accounts.get_mut(&address) {
                    acc.is_validator = false;
                }
            }

            // 8. Remove from SlashingManager (full cleanup, not just unstaking)
            {
                let mut sm_guard = safe_lock(&sm);
                sm_guard.remove_validator(&address);
            }

            // 9. Remove from RewardPool
            {
                let mut rp_guard = safe_lock(&rp);
                rp_guard.unregister_validator(&address);
            }

            // 9b. Remove from local registered validators (stop heartbeat forwarding)
            {
                let mut lrv: std::sync::MutexGuard<'_, HashSet<String>> = safe_lock(&lrv_inner);
                lrv.remove(&address);
            }

            // 9c. Remove from validator_endpoints (so /peers and /validators stop showing it)
            {
                let mut ve = safe_lock(&ve_inner);
                ve.remove(&address);
            }

            // 10. Update aBFT validator set
            {
                let l_guard = safe_lock(&l);
                let mut validators: Vec<String> = l_guard
                    .accounts
                    .iter()
                    .filter(|(_, a)| a.balance >= MIN_VALIDATOR_REGISTER_CIL && a.is_validator)
                    .map(|(addr, _)| addr.clone())
                    .collect();
                validators.sort();
                safe_lock(&abft_inner).update_validator_set(validators);
            }

            SAVE_DIRTY.store(true, Ordering::Release);

            // 11. Broadcast to peers
            let unreg_msg = serde_json::json!({
                "type": "VALIDATOR_UNREG",
                "address": address,
                "public_key": public_key,
                "signature": signature,
                "timestamp": timestamp,
            });
            let _ = tx.send(format!("VALIDATOR_UNREG:{}", unreg_msg)).await;

            println!(
                "🔻 Validator unregistered: {} (balance: {} LOS)",
                get_short_addr(&address),
                balance / CIL_PER_LOS
            );

            // Persist immediately
            let _ = db.save_ledger(&safe_lock(&l));

            api_json(serde_json::json!({
                "status": "ok",
                "msg": "Validator unregistered successfully",
                "address": address,
                "balance_los": balance / CIL_PER_LOS,
                "is_validator": false,
            }))
        }
    };

    // Route 1: /unregister-validator (hyphenated — canonical)
    let l_unregval1 = ledger.clone();
    let sm_unregval1 = slashing_manager.clone();
    let rp_unregval1 = reward_pool.clone();
    let tx_unregval1 = tx_out.clone();
    let db_unregval1 = database.clone();
    let handler1 = unregister_handler.clone();
    let unregister_validator_route = warp::path("unregister-validator")
        .and(warp::post())
        .and(warp::body::bytes())
        .and(with_state((
            l_unregval1,
            sm_unregval1,
            rp_unregval1,
            tx_unregval1,
            db_unregval1,
        )))
        .then(handler1);

    // Route 2: /unregister_validator (underscore — CLI compatibility)
    let l_unregval2 = ledger.clone();
    let sm_unregval2 = slashing_manager.clone();
    let rp_unregval2 = reward_pool.clone();
    let tx_unregval2 = tx_out.clone();
    let db_unregval2 = database.clone();
    let unregister_validator_underscore_route = warp::path("unregister_validator")
        .and(warp::post())
        .and(warp::body::bytes())
        .and(with_state((
            l_unregval2,
            sm_unregval2,
            rp_unregval2,
            tx_unregval2,
            db_unregval2,
        )))
        .then(unregister_handler);

    // 30. GET /network/peers — Lightweight endpoint for Flutter peer discovery.
    // Returns all known validator endpoints (clearnet and/or onion) so Flutter apps
    // can discover new nodes beyond the hardcoded bootstrap list.
    // Each entry includes `transport` field ("onion" or "clearnet") so clients
    // can decide whether Tor SOCKS5 proxy is needed.
    let ve_discovery = validator_endpoints.clone();
    let ab_discovery = address_book.clone();
    let l_discovery = ledger.clone();
    let my_addr_discovery = my_address.clone();
    let network_peers_route = warp::path!("network" / "peers")
        .and(with_state((ve_discovery, ab_discovery, l_discovery)))
        .map(
            move |(ve, ab, l): (
                Arc<Mutex<HashMap<String, String>>>,
                Arc<Mutex<HashMap<String, String>>>,
                Arc<Mutex<Ledger>>,
            )| {
                // Lock order MUST be ab → ve → l (same as /peers route).
                // Inconsistent ordering causes ABBA deadlock.
                let ab_guard = safe_lock(&ab);
                let ve_guard = safe_lock(&ve);
                let l_guard = safe_lock(&l);

                // All known validator endpoints (clearnet and/or onion)
                let endpoints: Vec<serde_json::Value> = ve_guard
                    .iter()
                    .map(|(addr, host)| {
                        let stake = l_guard
                            .accounts
                            .get(addr)
                            .map(|a| a.balance / CIL_PER_LOS)
                            .unwrap_or(0);
                        let in_peers = ab_guard.values().any(|v| v.contains(addr.as_str()));
                        let is_self = addr == &my_addr_discovery;
                        let transport = if host.contains(".onion") {
                            "onion"
                        } else {
                            "clearnet"
                        };
                        // Extract rest_port from host string (e.g. "127.0.0.1:7030" → 7030)
                        let rest_port: u16 = host
                            .rsplit(':')
                            .next()
                            .and_then(|p| p.parse().ok())
                            .unwrap_or(3030);
                        serde_json::json!({
                            "address": addr,
                            "host_address": host,
                            "onion_address": host, // backward compat
                            "transport": transport,
                            "rest_port": rest_port,
                            "stake_los": stake,
                            "reachable": is_self || in_peers,
                        })
                    })
                    .collect();

                api_json(serde_json::json!({
                    "version": 1,
                    "endpoints": endpoints,
                    "total": endpoints.len(),
                    "timestamp": std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                }))
            },
        );

    // GET /mempool/stats — Real-time mempool statistics
    let mp_stats = mempool_pool.clone();
    let mempool_stats_route = warp::path!("mempool" / "stats")
        .and(warp::get())
        .map(move || {
            let mut mp = safe_lock(&mp_stats);
            // Expire old transactions while we're here
            let expired = mp.remove_expired();
            let stats = mp.stats();
            api_json(serde_json::json!({
                "status": "ok",
                "mempool": {
                    "pending": stats.size,
                    "total_received": stats.total_received,
                    "total_accepted": stats.total_accepted,
                    "total_rejected": stats.total_rejected,
                    "total_expired": stats.total_expired,
                    "unique_senders": stats.unique_senders,
                    "just_expired": expired,
                }
            }))
        });

    // ── PEER DIRECTORY ──────────────────────────────────────────────
    // Embedded peer directory: every validator serves a live HTML page
    // listing all known validators with their current status.
    // No separate server/VPS needed — runs forever as part of the node.
    //
    // Routes:
    //   GET /directory              → HTML page (human-readable)
    //   GET /directory/api/peers    → All peers JSON (for apps)
    //   GET /directory/api/active   → Active peers only JSON
    // ────────────────────────────────────────────────────────────────

    // GET /directory — Dark-themed HTML page showing all known validators
    let ve_dir_html = validator_endpoints.clone();
    let ab_dir_html = address_book.clone();
    let l_dir_html = ledger.clone();
    let bv_dir_html = bootstrap_validators.clone();
    let my_addr_dir_html = my_address.clone();
    let directory_html_route = warp::path("directory")
        .and(warp::path::end())
        .and(with_state((ve_dir_html, ab_dir_html, l_dir_html)))
        .map(
            move |(ve, ab, l): (
                Arc<Mutex<HashMap<String, String>>>,
                Arc<Mutex<HashMap<String, String>>>,
                Arc<Mutex<Ledger>>,
            )| {
                let ab_guard = safe_lock(&ab);
                let ve_guard = safe_lock(&ve);
                let l_guard = safe_lock(&l);

                let network = if los_core::is_mainnet_build() { "Mainnet" } else { "Testnet" };
                let uptime_secs = start_time.elapsed().as_secs();
                let uptime_str = format_uptime(uptime_secs);

                // Build peer list from validator_endpoints
                let mut peers: Vec<(String, String, String, bool, u128, bool)> = Vec::new();

                for (addr, host) in ve_guard.iter() {
                    let in_peers = ab_guard.values().any(|v| v == addr);
                    let is_self = addr == &my_addr_dir_html;
                    let active = is_self || in_peers;
                    let stake = l_guard.accounts.get(addr)
                        .map(|a| a.balance / CIL_PER_LOS)
                        .unwrap_or(0);
                    let transport = if host.contains(".onion") { "onion" } else { "clearnet" };
                    let is_bootstrap = bv_dir_html.contains(addr);
                    peers.push((addr.clone(), host.clone(), transport.to_string(), active, stake, is_bootstrap));
                }

                // Sort: active first, then by stake
                peers.sort_by(|a, b| b.3.cmp(&a.3).then_with(|| b.4.cmp(&a.4)));

                let active_count = peers.iter().filter(|p| p.3).count();
                let total = peers.len();

                // Build table rows
                let mut rows = String::new();
                for (addr, host, transport, active, stake, is_bootstrap) in &peers {
                    let status_dot = if *active {
                        r#"<span class="dot active"></span>"#
                    } else {
                        r#"<span class="dot inactive"></span>"#
                    };
                    let transport_badge = if transport == "onion" {
                        r#"<span class="badge onion">🧅 onion</span>"#
                    } else {
                        r#"<span class="badge clearnet">🌐 clearnet</span>"#
                    };
                    let bootstrap_badge = if *is_bootstrap {
                        r#" <span class="badge bootstrap">⚡ genesis</span>"#
                    } else { "" };

                    let addr_short = if addr.len() > 16 {
                        format!("{}…{}", &addr[..8], &addr[addr.len()-6..])
                    } else { addr.clone() };

                    let host_display = if host.len() > 40 {
                        format!("{}…{}", &host[..20], &host[host.len()-12..])
                    } else { host.clone() };

                    let rest_url = if host.contains("://") { host.clone() } else { format!("http://{}", host) };

                    rows.push_str(&format!(
                        r#"<tr>
                            <td>{status_dot}</td>
                            <td class="mono addr" title="{addr}">{addr_short}{bootstrap_badge}</td>
                            <td class="mono host" title="{rest_url}">
                                <span>{host_display}</span>
                                <button class="copy-btn" onclick="copyText('{rest_url}')">📋</button>
                            </td>
                            <td>{transport_badge}</td>
                            <td class="mono">{stake} LOS</td>
                        </tr>"#
                    ));
                }

                let now_str = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();

                let html = format!(
                    r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<meta http-equiv="refresh" content="60">
<title>LOS Peer Directory — {network}</title>
<style>
*{{margin:0;padding:0;box-sizing:border-box;}}
body{{font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,monospace;background:#0d1117;color:#c9d1d9;min-height:100vh;}}
.container{{max-width:1100px;margin:0 auto;padding:20px;}}
h1{{font-size:24px;color:#58a6ff;margin-bottom:4px;}}
.subtitle{{color:#8b949e;margin-bottom:20px;font-size:14px;}}
.stats{{display:flex;gap:24px;flex-wrap:wrap;margin-bottom:20px;padding:12px 16px;background:#161b22;border:1px solid #30363d;border-radius:6px;}}
.stat{{display:flex;flex-direction:column;}}
.stat-label{{font-size:11px;color:#8b949e;text-transform:uppercase;}}
.stat-value{{font-size:18px;font-weight:600;color:#f0f6fc;}}
.stat-value.green{{color:#3fb950;}}
table{{width:100%;border-collapse:collapse;background:#161b22;border:1px solid #30363d;border-radius:6px;overflow:hidden;}}
th{{text-align:left;padding:10px 12px;background:#21262d;color:#8b949e;font-size:12px;text-transform:uppercase;border-bottom:1px solid #30363d;}}
td{{padding:8px 12px;border-bottom:1px solid #21262d;font-size:13px;}}
tr:hover{{background:#1c2128;}}
.mono{{font-family:'SF Mono','Fira Code',monospace;font-size:12px;}}
.dot{{display:inline-block;width:10px;height:10px;border-radius:50%;margin-right:4px;}}
.dot.active{{background:#3fb950;box-shadow:0 0 6px #3fb950;}}
.dot.inactive{{background:#484f58;}}
.badge{{padding:2px 8px;border-radius:10px;font-size:11px;font-weight:500;}}
.badge.onion{{background:#2d1f4e;color:#a371f7;}}
.badge.clearnet{{background:#0d2818;color:#3fb950;}}
.badge.bootstrap{{background:#1c2541;color:#58a6ff;}}
.host{{position:relative;}}
.copy-btn{{background:none;border:none;cursor:pointer;font-size:12px;opacity:0.4;margin-left:4px;padding:2px;}}
.copy-btn:hover{{opacity:1;}}
.api-section{{margin-top:24px;padding:16px;background:#161b22;border:1px solid #30363d;border-radius:6px;}}
.api-section h3{{color:#58a6ff;font-size:14px;margin-bottom:8px;}}
.api-section code{{background:#0d1117;padding:3px 8px;border-radius:4px;color:#79c0ff;font-size:12px;}}
.api-row{{margin:6px 0;}}
footer{{margin-top:24px;text-align:center;color:#484f58;font-size:12px;}}
footer a{{color:#58a6ff;text-decoration:none;}}
.toast{{position:fixed;bottom:20px;right:20px;background:#238636;color:#fff;padding:10px 20px;border-radius:6px;display:none;font-size:13px;z-index:100;}}
@media(max-width:700px){{.addr{{display:none;}}.stats{{gap:12px;}}}}
</style>
</head>
<body>
<div class="container">
    <h1>🔗 LOS Peer Directory</h1>
    <p class="subtitle">Unauthority Network — {network} | Auto-refreshes every 60s | Served by this validator node</p>
    <div class="stats">
        <div class="stat"><span class="stat-label">Active Nodes</span><span class="stat-value green">{active_count} / {total}</span></div>
        <div class="stat"><span class="stat-label">Last Updated</span><span class="stat-value">{now_str}</span></div>
        <div class="stat"><span class="stat-label">Node Uptime</span><span class="stat-value">{uptime_str}</span></div>
    </div>
    <table>
        <thead><tr><th>Status</th><th class="addr">Address</th><th>Host</th><th>Type</th><th>Stake</th></tr></thead>
        <tbody>{rows}</tbody>
    </table>
    <div class="api-section">
        <h3>📡 API Endpoints (for apps &amp; scripts)</h3>
        <div class="api-row"><code>GET /directory/api/peers</code> — All known peers (JSON)</div>
        <div class="api-row"><code>GET /directory/api/active</code> — Active peers only (JSON) — <em>use this in your app</em></div>
        <div class="api-row"><code>GET /network/peers</code> — Full peer data with stake info</div>
    </div>
    <footer>
        <p>LOS Peer Directory — embedded in every validator node</p>
        <p>Any validator's .onion address + /directory = this page. No extra server needed.</p>
        <p><a href="https://github.com/AurelMoonworker/unauthority-core" target="_blank">GitHub</a></p>
    </footer>
</div>
<div class="toast" id="toast">✅ Copied!</div>
<script>
function copyText(text){{navigator.clipboard.writeText(text).then(function(){{var t=document.getElementById('toast');t.style.display='block';setTimeout(function(){{t.style.display='none';}},1500);}});}}
</script>
</body>
</html>"##);

                warp::reply::html(html)
            },
        );

    // GET /directory/api/peers — All known peers as JSON
    let ve_dir_api = validator_endpoints.clone();
    let ab_dir_api = address_book.clone();
    let l_dir_api = ledger.clone();
    let bv_dir_api = bootstrap_validators.clone();
    let my_addr_dir_api = my_address.clone();
    let directory_api_peers_route = warp::path!("directory" / "api" / "peers")
        .and(with_state((ve_dir_api, ab_dir_api, l_dir_api)))
        .map(
            move |(ve, ab, l): (
                Arc<Mutex<HashMap<String, String>>>,
                Arc<Mutex<HashMap<String, String>>>,
                Arc<Mutex<Ledger>>,
            )| {
                let ab_guard = safe_lock(&ab);
                let ve_guard = safe_lock(&ve);
                let l_guard = safe_lock(&l);

                let network = if los_core::is_mainnet_build() { "mainnet" } else { "testnet" };
                let mut peers: Vec<serde_json::Value> = Vec::new();

                for (addr, host) in ve_guard.iter() {
                    let in_peers = ab_guard.values().any(|v| v == addr);
                    let is_self = addr == &my_addr_dir_api;
                    let active = is_self || in_peers;
                    let stake = l_guard.accounts.get(addr)
                        .map(|a| a.balance / CIL_PER_LOS)
                        .unwrap_or(0);
                    let transport = if host.contains(".onion") { "onion" } else { "clearnet" };
                    let rest_port: u16 = host.rsplit(':').next()
                        .and_then(|p| p.parse().ok())
                        .unwrap_or(3030);
                    let is_bootstrap = bv_dir_api.contains(addr);

                    peers.push(serde_json::json!({
                        "address": addr,
                        "host": if host.contains("://") { host.clone() } else { format!("http://{}", host) },
                        "transport": transport,
                        "active": active,
                        "stake_los": stake,
                        "rest_port": rest_port,
                        "is_bootstrap": is_bootstrap,
                    }));
                }

                let active_count = peers.iter().filter(|p| p["active"].as_bool().unwrap_or(false)).count();
                api_json(serde_json::json!({
                    "network": network,
                    "active_count": active_count,
                    "total_count": peers.len(),
                    "peers": peers,
                    "updated_at": chrono::Utc::now().to_rfc3339(),
                }))
            },
        );

    // GET /directory/api/active — Active peers only JSON (for app bootstrapping)
    let ve_dir_active = validator_endpoints.clone();
    let ab_dir_active = address_book.clone();
    let l_dir_active = ledger.clone();
    let my_addr_dir_active = my_address.clone();
    let directory_api_active_route = warp::path!("directory" / "api" / "active")
        .and(with_state((ve_dir_active, ab_dir_active, l_dir_active)))
        .map(
            move |(ve, ab, l): (
                Arc<Mutex<HashMap<String, String>>>,
                Arc<Mutex<HashMap<String, String>>>,
                Arc<Mutex<Ledger>>,
            )| {
                let ab_guard = safe_lock(&ab);
                let ve_guard = safe_lock(&ve);
                let _l_guard = safe_lock(&l);

                let network = if los_core::is_mainnet_build() { "mainnet" } else { "testnet" };
                let mut active_peers: Vec<serde_json::Value> = Vec::new();

                for (addr, host) in ve_guard.iter() {
                    let in_peers = ab_guard.values().any(|v| v == addr);
                    let is_self = addr == &my_addr_dir_active;
                    if !is_self && !in_peers { continue; }

                    let rest_port: u16 = host.rsplit(':').next()
                        .and_then(|p| p.parse().ok())
                        .unwrap_or(3030);

                    active_peers.push(serde_json::json!({
                        "host": if host.contains("://") { host.clone() } else { format!("http://{}", host) },
                        "address": addr,
                        "transport": if host.contains(".onion") { "onion" } else { "clearnet" },
                        "rest_port": rest_port,
                    }));
                }

                api_json(serde_json::json!({
                    "network": network,
                    "active_count": active_peers.len(),
                    "peers": active_peers,
                    "updated_at": chrono::Utc::now().to_rfc3339(),
                }))
            },
        );

    // Combine all routes with rate limiting
    // NOTE: Each route is .boxed() to prevent warp type recursion overflow (E0275)
    // when compiling in release mode. This breaks the deeply nested type chain.
    let group1 = root_route
        .boxed()
        .or(balance_route.boxed())
        .or(supply_route.boxed())
        .or(history_route.boxed())
        .or(peers_route.boxed())
        .or(send_route.boxed())
        .boxed();

    let group2 = deploy_route
        .boxed()
        .or(metrics_route.boxed())
        .or(node_info_route.boxed())
        .boxed();

    let group3 = validators_route
        .boxed()
        .or(balance_alias_route.boxed())
        .or(fee_estimate_route.boxed())
        .or(mining_info_route.boxed())
        .or(block_route.boxed())
        .or(faucet_route.boxed())
        .or(blocks_recent_route.boxed())
        .or(whoami_route.boxed())
        .boxed();

    let group4 = account_route
        .boxed()
        .or(health_route.boxed())
        .or(tor_health_route.boxed())
        .or(slashing_route.boxed())
        .or(slashing_profile_route.boxed())
        .or(block_by_hash_route.boxed())
        .or(tx_by_hash_route.boxed())
        .or(search_route.boxed())
        .or(sync_full_route.boxed())
        .or(sync_route.boxed())
        .or(consensus_route.boxed())
        .or(reward_info_route.boxed())
        .or(register_validator_route.boxed())
        .or(unregister_validator_route.boxed())
        .or(unregister_validator_underscore_route.boxed())
        .or(network_peers_route.boxed())
        .or(mempool_stats_route.boxed())
        .or(validator_api::validator_routes().boxed())
        .boxed();

    // Token routes (USP-01)
    let group5 = list_tokens_route
        .boxed()
        .or(token_balance_route.boxed())
        .or(token_allowance_route.boxed())
        .or(token_info_route.boxed())
        .boxed();

    // DEX routes
    let group6 = dex_list_pools_route
        .boxed()
        .or(dex_pool_info_route.boxed())
        .or(dex_quote_route.boxed())
        .or(dex_position_route.boxed())
        .boxed();

    // Peer Directory routes (embedded in every validator)
    let group7 = directory_html_route
        .boxed()
        .or(directory_api_peers_route.boxed())
        .or(directory_api_active_route.boxed())
        .boxed();

    let routes = group1
        .or(group2)
        .or(group3)
        .or(group4)
        .or(group5)
        .or(group6)
        .or(group7)
        .with(cors) // Apply CORS
        .with(warp::log("api"))
        .recover(handle_rejection);

    // Apply rate limiting globally
    let routes_with_limit = rate_limit_filter.and(routes);

    // ── PoW MINING BACKGROUND THREAD ──────────────────────────────────
    // When --mine is set, spawn background threads that grind SHA3 hashes
    // to mine new LOS tokens. Mining is independent of consensus — it only
    // submits proofs to the local API which creates Mint blocks.
    if enable_mining {
        // Genesis bootstrap validators are excluded from mining rewards.
        // All mining rewards go to public miners for fair distribution.
        let is_genesis_miner = bootstrap_validators.contains(&my_address);
        if is_genesis_miner {
            println!("⛏️  Mining DISABLED: genesis bootstrap validators cannot mine.");
            println!("   All mining rewards are reserved for public miners.");
        }

        let ms_bg = mining_state.clone();
        let l_bg = ledger.clone();
        let db_bg = database.clone();
        let pk_bg = node_public_key.clone();
        let sk_bg = secret_key.clone();
        let tx_bg = tx_out.clone();
        let my_addr_bg = my_address.clone();
        // Arcs for auto self-registration as validator after first mine
        let sm_bg = slashing_manager.clone();
        let rp_bg = reward_pool.clone();
        let abft_bg = abft_consensus.clone();
        let ve_bg = validator_endpoints.clone();
        let bv_bg = bootstrap_validators.clone();
        let lrv_bg = local_registered_validators.clone();

        if !is_genesis_miner {
            tokio::spawn(async move {
                println!(
                    "⛏️  Mining thread started (address: {})",
                    get_short_addr(&my_addr_bg)
                );
                let cancel = Arc::new(AtomicBool::new(false));

                loop {
                    // Get current epoch and difficulty
                    let (epoch, difficulty_bits, remaining) = {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        let mut ms: std::sync::MutexGuard<'_, MiningState> = safe_lock(&ms_bg);
                        ms.maybe_advance_epoch(now);
                        let remaining = safe_lock(&l_bg).distribution.remaining_supply;
                        (ms.current_epoch, ms.difficulty_bits, remaining)
                    };

                    // Check if we already mined this epoch
                    let already_mined_wait = {
                        let ms: std::sync::MutexGuard<'_, MiningState> = safe_lock(&ms_bg);
                        if ms.current_epoch_miners.contains(&my_addr_bg) {
                            let now = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();
                            Some(ms.epoch_remaining_secs(now))
                        } else {
                            None
                        }
                    }; // ms guard dropped here

                    if let Some(remaining_secs) = already_mined_wait {
                        println!(
                            "⛏️  Already mined epoch {} — waiting {}s for next epoch",
                            epoch, remaining_secs
                        );
                        tokio::time::sleep(Duration::from_secs(remaining_secs.max(5))).await;
                        continue;
                    }

                    if remaining == 0 {
                        println!("⛏️  Public supply exhausted — mining stopped");
                        break;
                    }

                    // Mine using CPU thread(s)
                    let addr_clone = my_addr_bg.clone();
                    let cancel_clone = cancel.clone();
                    cancel_clone.store(false, Ordering::Release);

                    // Spawn blocking mining work
                    let mining_threads_count = mining_threads;
                    let found = tokio::task::spawn_blocking(move || {
                        if mining_threads_count <= 1 {
                            los_core::pow_mint::mine(
                                &addr_clone,
                                epoch,
                                difficulty_bits,
                                &cancel_clone,
                            )
                        } else {
                            // Multi-threaded mining: first thread to find a nonce cancels others
                            use std::sync::mpsc as std_mpsc;
                            let (sender, receiver) = std_mpsc::channel();
                            let threads: Vec<_> = (0..mining_threads_count)
                                .map(|_t| {
                                    let addr = addr_clone.clone();
                                    let cancel = cancel_clone.clone();
                                    let tx = sender.clone();
                                    std::thread::spawn(move || {
                                        if let Some(nonce) = los_core::pow_mint::mine(
                                            &addr,
                                            epoch,
                                            difficulty_bits,
                                            &cancel,
                                        ) {
                                            let _ = tx.send(nonce);
                                            cancel.store(true, Ordering::Release);
                                            // Cancel other threads
                                        }
                                    })
                                })
                                .collect();
                            drop(sender);
                            let result = receiver.recv().ok();
                            cancel_clone.store(true, Ordering::Release); // Ensure all threads stop
                            for t in threads {
                                let _ = t.join();
                            }
                            result
                        }
                    })
                    .await
                    .unwrap_or(None);

                    if let Some(nonce) = found {
                        println!(
                            "⛏️  Found valid nonce {} for epoch {} (difficulty: {} bits)",
                            nonce, epoch, difficulty_bits
                        );

                        // Submit proof to mining state
                        let proof = los_core::pow_mint::MiningProof {
                            address: my_addr_bg.clone(),
                            epoch,
                            nonce,
                        };

                        let now_secs = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();

                        let reward_cil = {
                            let mut ms: std::sync::MutexGuard<'_, MiningState> = safe_lock(&ms_bg);
                            let remaining = safe_lock(&l_bg).distribution.remaining_supply;
                            match ms.verify_proof(&proof, now_secs, remaining) {
                                Ok(r) => r,
                                Err(e) => {
                                    println!("⛏️  Proof rejected (stale epoch?): {}", e);
                                    continue;
                                }
                            }
                        };

                        // Create and process Mint block
                        let link = format!("MINE:{}:{}", epoch, nonce);
                        let (head, _bc) = {
                            let l = safe_lock(&l_bg);
                            let acc = l.accounts.get(&my_addr_bg);
                            (
                                acc.map(|a| a.head.clone())
                                    .unwrap_or_else(|| "0".to_string()),
                                acc.map(|a| a.block_count).unwrap_or(0),
                            )
                        };

                        let mut mint_block = Block {
                            account: my_addr_bg.clone(),
                            previous: head,
                            block_type: BlockType::Mint,
                            amount: reward_cil,
                            link,
                            signature: String::new(),
                            public_key: hex::encode(&pk_bg),
                            work: 0,
                            timestamp: now_secs,
                            fee: 0,
                        };

                        // Anti-spam PoW on block
                        solve_pow(&mut mint_block);

                        // Sign block
                        mint_block.signature =
                            match try_sign_hex(mint_block.signing_hash().as_bytes(), &sk_bg) {
                                Ok(sig) => sig,
                                Err(e) => {
                                    eprintln!("⛏️  Signing failed: {} — skipping", e);
                                    continue;
                                }
                            };

                        // Process locally
                        let process_ok = {
                            let mut l = safe_lock(&l_bg);
                            match l.process_block(&mint_block) {
                                Ok(_) => {
                                    SAVE_DIRTY.store(true, Ordering::Release);
                                    true
                                }
                                Err(e) => {
                                    eprintln!("⛏️  Mint block rejected: {}", e);
                                    // Revert mining state
                                    let mut ms: std::sync::MutexGuard<'_, MiningState> =
                                        safe_lock(&ms_bg);
                                    ms.current_epoch_miners.remove(&my_addr_bg);
                                    false
                                }
                            }
                        };

                        if process_ok {
                            let hash = mint_block.calculate_hash();
                            if let Err(e) = db_bg.save_block(&hash, &mint_block) {
                                eprintln!("⚠️ DB save error for mined block: {}", e);
                            }
                            // Broadcast to network
                            if let Ok(json) = serde_json::to_string(&mint_block) {
                                let gossip_msg = format!("MINE_BLOCK:{}", json);
                                let _ = tx_bg.send(gossip_msg).await;
                            }
                            let reward_los = reward_cil / CIL_PER_LOS;
                            let reward_remainder =
                                (reward_cil % CIL_PER_LOS) / (CIL_PER_LOS / 10000); // 4 decimal places
                            println!(
                                "⛏️  Mined {}.{:04} LOS in epoch {} ✓",
                                reward_los, reward_remainder, epoch
                            );

                            // ── AUTO SELF-REGISTER AS VALIDATOR ──
                            // After first successful mine, auto-register this node as a validator
                            // so it participates in consensus and is discoverable by peers.
                            // Requires balance >= MIN_VALIDATOR_REGISTER_CIL (1 LOS).
                            let needs_register = {
                                let l = safe_lock(&l_bg);
                                match l.accounts.get(&my_addr_bg) {
                                    Some(acc) => {
                                        !acc.is_validator
                                            && acc.balance >= MIN_VALIDATOR_REGISTER_CIL
                                            && !bv_bg.contains(&my_addr_bg)
                                    }
                                    None => false,
                                }
                            };
                            if needs_register {
                                // 1. Set is_validator = true in ledger
                                {
                                    let mut l = safe_lock(&l_bg);
                                    if let Some(acc) = l.accounts.get_mut(&my_addr_bg) {
                                        acc.is_validator = true;
                                    }
                                }
                                // 2. Register in SlashingManager
                                {
                                    let mut sm = safe_lock(&sm_bg);
                                    if sm.get_profile(&my_addr_bg).is_none() {
                                        sm.register_validator(my_addr_bg.clone());
                                    }
                                }
                                // 3. Register in RewardPool (non-genesis)
                                {
                                    let balance = safe_lock(&l_bg)
                                        .accounts
                                        .get(&my_addr_bg)
                                        .map(|a| a.balance)
                                        .unwrap_or(0);
                                    let mut rp = safe_lock(&rp_bg);
                                    rp.register_validator(&my_addr_bg, false, balance);
                                }
                                // 4. Track as locally-registered validator for heartbeats
                                {
                                    let mut lrv = safe_lock(&lrv_bg);
                                    lrv.insert(my_addr_bg.clone());
                                }
                                // 5. Update aBFT validator set dynamically
                                {
                                    let l = safe_lock(&l_bg);
                                    let mut validators: Vec<String> = l
                                        .accounts
                                        .iter()
                                        .filter(|(_, a)| {
                                            a.balance >= MIN_VALIDATOR_REGISTER_CIL
                                                && a.is_validator
                                        })
                                        .map(|(addr, _)| addr.clone())
                                        .collect();
                                    validators.sort();
                                    safe_lock(&abft_bg).update_validator_set(validators);
                                }
                                // 6. Store our onion address in validator endpoints
                                let host_addr =
                                    get_node_host_address().map(|h| ensure_host_port(&h, api_port));
                                if let Some(ref host) = host_addr {
                                    insert_validator_endpoint(
                                        &mut safe_lock(&ve_bg),
                                        my_addr_bg.clone(),
                                        host.clone(),
                                    );
                                }
                                // 7. Broadcast VALIDATOR_REG to peers
                                let ts = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs();
                                let reg_message =
                                    format!("REGISTER_VALIDATOR:{}:{}", my_addr_bg, ts);
                                if let Ok(sig) =
                                    los_crypto::sign_message(reg_message.as_bytes(), &sk_bg)
                                {
                                    let reg_msg = serde_json::json!({
                                        "type": "VALIDATOR_REG",
                                        "address": my_addr_bg,
                                        "public_key": hex::encode(&pk_bg),
                                        "signature": hex::encode(&sig),
                                        "timestamp": ts,
                                        "host_address": host_addr,
                                        "onion_address": host_addr,
                                        "rest_port": api_port,
                                    });
                                    let _ = tx_bg.send(format!("VALIDATOR_REG:{}", reg_msg)).await;
                                }
                                SAVE_DIRTY.store(true, Ordering::Release);
                                let stake_los = safe_lock(&l_bg)
                                    .accounts
                                    .get(&my_addr_bg)
                                    .map(|a| a.balance / CIL_PER_LOS)
                                    .unwrap_or(0);
                                println!(
                                    "✅ Auto-registered as validator: {} (stake: {} LOS, host: {})",
                                    get_short_addr(&my_addr_bg),
                                    stake_los,
                                    host_addr.as_deref().unwrap_or("none")
                                );
                            } else {
                                // Already registered — re-broadcast VALIDATOR_REG to ensure
                                // all peers know our endpoint (gossip is idempotent).
                                // Receivers skip if already known, so this is safe.
                                let is_val = safe_lock(&l_bg)
                                    .accounts
                                    .get(&my_addr_bg)
                                    .map(|a| a.is_validator)
                                    .unwrap_or(false);
                                if is_val {
                                    let host_addr = get_node_host_address()
                                        .map(|h| ensure_host_port(&h, api_port));
                                    let ts = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_secs();
                                    let reg_message =
                                        format!("REGISTER_VALIDATOR:{}:{}", my_addr_bg, ts);
                                    if let Ok(sig) =
                                        los_crypto::sign_message(reg_message.as_bytes(), &sk_bg)
                                    {
                                        let reg_msg = serde_json::json!({
                                            "type": "VALIDATOR_REG",
                                            "address": my_addr_bg,
                                            "public_key": hex::encode(&pk_bg),
                                            "signature": hex::encode(&sig),
                                            "timestamp": ts,
                                            "host_address": host_addr,
                                            "onion_address": host_addr,
                                            "rest_port": api_port,
                                        });
                                        let _ =
                                            tx_bg.send(format!("VALIDATOR_REG:{}", reg_msg)).await;
                                    }
                                    // Also ensure our endpoint is stored locally
                                    if let Some(ref host) = host_addr {
                                        insert_validator_endpoint(
                                            &mut safe_lock(&ve_bg),
                                            my_addr_bg.clone(),
                                            host.clone(),
                                        );
                                    }
                                }
                            }
                        }
                    } else {
                        // Mining was cancelled (epoch changed) — retry immediately
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            });
        } // end if !is_genesis_miner
    }

    // Bind to 127.0.0.1 for Tor/production (prevents IP leak)
    // Set LOS_BIND_ALL=1 for local dev with multiple machines
    // Check for "1" specifically to prevent accidental exposure (e.g., LOS_BIND_ALL=0)
    let bind_addr: [u8; 4] = if std::env::var("LOS_BIND_ALL").unwrap_or_default() == "1" {
        [0, 0, 0, 0]
    } else {
        [127, 0, 0, 1] // Default: localhost only (safe for Tor hidden service)
    };
    println!(
        "🌍 API Server running at http://{}:{} (Rate Limit: 100 req/sec per IP)",
        if bind_addr == [0, 0, 0, 0] {
            "0.0.0.0"
        } else {
            "127.0.0.1"
        },
        api_port
    );
    // Flush stdout — when spawned from Flutter, stdout is a pipe (fully buffered)
    {
        use std::io::Write;
        let _ = std::io::stdout().flush();
    }
    warp::serve(routes_with_limit)
        .run((bind_addr, api_port))
        .await;
}

// Rate limit rejection handler
async fn handle_rejection(
    err: warp::Rejection,
) -> Result<impl warp::Reply, std::convert::Infallible> {
    if let Some(rate_limiter::filters::RateLimitExceeded { ip }) = err.find() {
        let json = warp::reply::json(&serde_json::json!({
            "status": "error",
            "code": 429,
            "msg": "Rate limit exceeded. Please slow down your requests.",
            "ip": ip.to_string()
        }));
        Ok(warp::reply::with_status(
            json,
            warp::http::StatusCode::TOO_MANY_REQUESTS,
        ))
    } else if err.is_not_found() {
        let json = warp::reply::json(&serde_json::json!({
            "status": "error",
            "code": 404,
            "msg": "Endpoint not found"
        }));
        Ok(warp::reply::with_status(
            json,
            warp::http::StatusCode::NOT_FOUND,
        ))
    } else if let Some(e) = err.find::<warp::filters::body::BodyDeserializeError>() {
        // Return proper 400 for malformed JSON / type errors
        // (negative amounts, floats for u128, null fields, missing fields, etc.)
        let detail = e.to_string();
        let json = warp::reply::json(&serde_json::json!({
            "status": "error",
            "code": 400,
            "msg": format!("Invalid request body: {}", detail)
        }));
        Ok(warp::reply::with_status(
            json,
            warp::http::StatusCode::BAD_REQUEST,
        ))
    } else if err.find::<warp::reject::MethodNotAllowed>().is_some() {
        let json = warp::reply::json(&serde_json::json!({
            "status": "error",
            "code": 405,
            "msg": "Method not allowed"
        }));
        Ok(warp::reply::with_status(
            json,
            warp::http::StatusCode::METHOD_NOT_ALLOWED,
        ))
    } else {
        eprintln!("⚠️ Unhandled rejection: {:?}", err);
        let json = warp::reply::json(&serde_json::json!({
            "status": "error",
            "code": 500,
            "msg": "Internal server error"
        }));
        Ok(warp::reply::with_status(
            json,
            warp::http::StatusCode::INTERNAL_SERVER_ERROR,
        ))
    }
}

// ══════════════════════════════════════════════════════════════════════
// REST-BASED STATE SYNC — Fallback when gossip SYNC_GZIP exceeds 8MB
// ══════════════════════════════════════════════════════════════════════
// When state grows beyond the gossip message size limit (8MB compressed),
// nodes fall back to direct HTTP REST sync via GET /sync/full.
// This supports .onion peers via SOCKS5 proxy and has NO size limit.
//
// Flow:
//   1. SYNC_REQUEST → responder detects state > 8MB
//   2. Responder sends SYNC_VIA_REST:<host>:<blocks> via gossip
//   3. Requester calls rest_sync_from_peer() → HTTP GET /sync/full?blocks=N
//   4. Response is gzip-compressed full ledger state (binary)
//   5. Apply using same fast-path as SYNC_GZIP (crypto validation, direct adoption)
//
// Also used by the background stale-state detector (runs every 2 min):
//   If block count unchanged for 4+ minutes, iterate known peer endpoints
//   and attempt REST sync from each until one succeeds.
//
// SECURITY:
//   - All blocks are cryptographically validated (PoW + signature)
//   - State only adopted if <10% of blocks fail validation
//   - Rate limited: one REST sync attempt per 60 seconds
//   - Decompression capped at 500MB to prevent decompression bombs

/// Perform REST-based state sync from a specific peer.
/// Returns the number of new blocks merged on success.
async fn rest_sync_from_peer(
    peer_host: &str,
    our_blocks: usize,
    ledger: &Arc<Mutex<Ledger>>,
    reward_pool: &Arc<Mutex<ValidatorRewardPool>>,
    slashing_mgr: &Arc<Mutex<los_consensus::slashing::SlashingManager>>,
    _database: &Arc<LosDatabase>,
) -> Result<usize, String> {
    // Build HTTP client (with SOCKS5 proxy for .onion addresses)
    let client = if peer_host.contains(".onion") {
        let socks_url = std::env::var("LOS_SOCKS5_PROXY")
            .or_else(|_| std::env::var("LOS_TOR_SOCKS5"))
            .unwrap_or_else(|_| "socks5h://127.0.0.1:9050".to_string());
        let proxy =
            reqwest::Proxy::all(&socks_url).map_err(|e| format!("SOCKS5 proxy error: {}", e))?;
        reqwest::Client::builder()
            .proxy(proxy)
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| format!("HTTP client error: {}", e))?
    } else {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| format!("HTTP client error: {}", e))?
    };

    let url = format!("http://{}/sync/full?blocks={}", peer_host, our_blocks);
    println!("📡 REST sync: fetching {}", url);

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {} from peer", resp.status()));
    }

    // Check Content-Type — if JSON, peer says we're up-to-date
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let body_bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Failed to read response body: {}", e))?;

    if content_type.contains("application/json") {
        // Peer says we're up-to-date
        return Ok(0);
    }

    // Decompress gzip body
    use flate2::read::GzDecoder;
    use std::io::Read;

    const MAX_DECOMPRESSED: u64 = 500 * 1024 * 1024; // 500 MB max
    let decoder = GzDecoder::new(&body_bytes[..]);
    let mut limited = decoder.take(MAX_DECOMPRESSED);
    let mut json_str = String::new();
    limited
        .read_to_string(&mut json_str)
        .map_err(|e| format!("Decompression failed: {}", e))?;

    let incoming: Ledger =
        serde_json::from_str(&json_str).map_err(|e| format!("JSON parse failed: {}", e))?;

    // Compare state roots — skip if identical
    let incoming_root = incoming.compute_state_root();
    let our_root = {
        let l = safe_lock(ledger);
        l.compute_state_root()
    };
    if incoming_root == our_root {
        println!("📦 REST sync: state roots match — already in sync");
        return Ok(0);
    }

    // Validate incoming blocks cryptographically
    let incoming_block_count = incoming.blocks.len();
    let mut crypto_invalid = 0usize;
    for blk in incoming.blocks.values() {
        if !blk.verify_pow() || !blk.verify_signature() {
            crypto_invalid += 1;
        }
    }

    let max_invalid = (incoming_block_count / 10).max(3);
    if crypto_invalid > max_invalid {
        return Err(format!(
            "Too many invalid blocks: {}/{} (max allowed: {})",
            crypto_invalid, incoming_block_count, max_invalid
        ));
    }

    // Apply state — same logic as SYNC_GZIP fast-path
    let mut added_count = 0;
    {
        let mut l = safe_lock(ledger);

        // Adopt account states where peer is more advanced
        for (addr, incoming_acct) in &incoming.accounts {
            let dominated = match l.accounts.get(addr) {
                Some(ours) => incoming_acct.block_count > ours.block_count,
                None => true,
            };
            if dominated {
                l.accounts.insert(addr.clone(), incoming_acct.clone());
            }
        }

        // Merge missing blocks
        for (hash, blk) in &incoming.blocks {
            if !l.blocks.contains_key(hash) {
                l.blocks.insert(hash.clone(), blk.clone());
                added_count += 1;
            }
        }

        // Adopt distribution state (lower remaining = more distributed)
        if incoming.distribution.remaining_supply < l.distribution.remaining_supply {
            l.distribution = incoming.distribution.clone();
        }

        // Merge claimed sends
        for claimed in &incoming.claimed_sends {
            l.claimed_sends.insert(claimed.clone());
        }

        // Adopt accumulated fees
        if incoming.accumulated_fees_cil > l.accumulated_fees_cil {
            l.accumulated_fees_cil = incoming.accumulated_fees_cil;
        }

        // Sanitize: remove orphaned blocks after merging
        let orphans = l.remove_orphaned_blocks();
        if orphans > 0 {
            println!("🧹 REST sync: removed {} orphaned block(s)", orphans);
        }

        SAVE_DIRTY.store(true, Ordering::Release);
    }

    // Sync reward pool for reward/fee blocks
    for blk in incoming.blocks.values() {
        if blk.block_type == BlockType::Mint
            && (blk.link.starts_with("REWARD:EPOCH:") || blk.link.starts_with("FEE_REWARD:EPOCH:"))
        {
            let mut pool = safe_lock(reward_pool);
            pool.sync_reward_from_gossip(&blk.account, blk.amount);
        }
    }

    // Update slashing participation
    {
        let l = safe_lock(ledger);
        let mut sm = safe_lock(slashing_mgr);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        for (addr, acc) in &l.accounts {
            if acc.balance >= MIN_VALIDATOR_STAKE_CIL {
                if sm.get_profile(addr).is_none() {
                    sm.register_validator(addr.clone());
                }
                let _ = sm.record_block_participation(addr, l.blocks.len() as u64, timestamp);
            }
        }
    }

    if crypto_invalid > 0 {
        println!(
            "⚠️ REST sync: {} blocks failed crypto validation (skipped)",
            crypto_invalid
        );
    }

    Ok(added_count)
}

// --- UTILS & FORMATTING ---

fn get_short_addr(full_addr: &str) -> String {
    if full_addr.len() < 12 {
        return full_addr.to_string();
    }
    // Skip "LOS" prefix (3 chars), take next 8 chars of base58
    format!("los_{}", &full_addr[3..11])
}

/// Format CIL balance as precise LOS string
/// Prevents integer division hiding sub-LOS amounts (e.g., 0.5 LOS → "0" with integer division)
fn format_balance_precise(cil_amount: u128) -> String {
    format!(
        "{}.{:011}",
        cil_amount / CIL_PER_LOS,
        cil_amount % CIL_PER_LOS
    )
}

fn format_u128(n: u128) -> String {
    let s = n.to_string();
    if s.len() > 3 {
        let mut result = String::new();
        for (count, c) in s.chars().rev().enumerate() {
            if count > 0 && count.is_multiple_of(3) {
                result.push('.');
            }
            result.push(c);
        }
        result.chars().rev().collect()
    } else {
        s
    }
}

// DEPRECATED: Old JSON-based save (kept for emergency backup)
#[allow(dead_code)]
fn save_to_disk_legacy(ledger: &Ledger) {
    if let Ok(data) = serde_json::to_string_pretty(ledger) {
        let _ = fs::write(LEDGER_FILE, &data);
        let _ = fs::create_dir_all("backups");
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let backup_path = format!("backups/ledger_{}.json", ts % 100);
        let _ = fs::write(backup_path, data);
    }
}

// Database-based save (ACID-compliant) with race condition protection
#[allow(dead_code)]
fn save_to_disk(ledger: &Ledger, db: &LosDatabase) {
    save_to_disk_internal(ledger, db, false);
}

// Internal save with force option
fn save_to_disk_internal(ledger: &Ledger, db: &LosDatabase, force: bool) {
    // Atomic check-and-set: prevents race condition where two tasks both pass the check
    if !force {
        if SAVE_IN_PROGRESS
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::Relaxed)
            .is_err()
        {
            // Another task is already saving — mark dirty so it will be retried
            SAVE_DIRTY.store(true, Ordering::Release);
            return;
        }
    } else {
        SAVE_IN_PROGRESS.store(true, Ordering::SeqCst);
    }

    if let Err(e) = db.save_ledger(ledger) {
        eprintln!("❌ Database save failed: {}", e);
        // Fallback to JSON backup
        save_to_disk_legacy(ledger);
    }

    SAVE_IN_PROGRESS.store(false, Ordering::SeqCst);
    SAVE_DIRTY.store(false, Ordering::Release);
}

// Load from database with JSON migration fallback
fn load_from_disk(db: &LosDatabase) -> Ledger {
    // Try loading from database first
    if !db.is_empty() {
        match db.load_ledger() {
            Ok(ledger) => {
                println!("✅ Loaded ledger from database");
                return ledger;
            }
            Err(e) => {
                eprintln!("⚠️  Database load failed: {}", e);
            }
        }
    }

    // One-time migration: if legacy JSON file exists, migrate to DB then remove
    if std::path::Path::new(LEDGER_FILE).exists() {
        if let Ok(data) = fs::read_to_string(LEDGER_FILE) {
            if let Ok(ledger) = serde_json::from_str::<Ledger>(&data) {
                println!("📦 Migrating legacy JSON to database...");
                if let Err(e) = db.save_ledger(&ledger) {
                    eprintln!("❌ Migration failed: {}", e);
                } else {
                    println!(
                        "✅ Migration complete: {} accounts, {} blocks",
                        ledger.accounts.len(),
                        ledger.blocks.len()
                    );
                    let _ = fs::rename(LEDGER_FILE, format!("{}.migrated", LEDGER_FILE));
                }
                return ledger;
            }
        }
    }

    println!("🆕 Creating new ledger");
    Ledger::new()
}

/// Maximum PoW iterations before giving up (safety limit)
/// 16 zero bits should typically be found within ~200k attempts
const MAX_POW_ITERATIONS: u64 = 10_000_000;

fn solve_pow(block: &mut los_core::Block) {
    println!(
        "⏳ Calculating PoW (Anti-Spam: 16 zero bits, limit: {}M iterations)...",
        MAX_POW_ITERATIONS / 1_000_000
    );
    let mut nonce: u64 = 0;
    loop {
        block.work = nonce;

        // Show progress every 100k attempts
        if nonce.is_multiple_of(100_000) && nonce > 0 {
            println!("   ... trying nonce #{}", nonce);
        }

        // Use the same validation logic as process_block (16 leading zero bits)
        if block.verify_pow() {
            break;
        }
        nonce += 1;

        // Safety limit: prevent infinite loop on malformed blocks
        if nonce >= MAX_POW_ITERATIONS {
            eprintln!(
                "⚠️ PoW safety limit reached ({} iterations). Using best nonce found.",
                MAX_POW_ITERATIONS
            );
            break;
        }
    }
    if nonce < MAX_POW_ITERATIONS {
        println!("✅ PoW found in {} iterations", nonce);
    }
}

/// PERF: Async PoW solver — offloads CPU-intensive mining to a blocking thread
/// so it doesn't stall tokio worker threads during concurrent API handling.
#[allow(dead_code)]
async fn solve_pow_async(mut block: los_core::Block) -> los_core::Block {
    let fallback = block.clone();
    match tokio::task::spawn_blocking(move || {
        solve_pow(&mut block);
        block
    })
    .await
    {
        Ok(solved) => solved,
        Err(e) => {
            eprintln!("[ERROR] PoW spawn_blocking task failed: {e}. Returning unsolved block.");
            fallback
        }
    }
}

/// Quiet PoW computation for system-generated blocks (rewards, etc.)
/// Same logic as solve_pow but without verbose logging.
fn compute_pow_inline(block: &mut los_core::Block, _difficulty_bits: u32) {
    let mut nonce: u64 = 0;
    loop {
        block.work = nonce;
        if block.verify_pow() {
            break;
        }
        nonce += 1;
        if nonce >= MAX_POW_ITERATIONS {
            break;
        }
    }
}

// --- VISUALIZATION ---

fn print_history_table(blocks: Vec<&Block>) {
    println!("\n📜 TRANSACTION HISTORY (Newest -> Oldest)");
    println!(
        "+----------------+----------------+--------------------------+------------------------+"
    );
    println!(
        "| {:<14} | {:<14} | {:<24} | {:<22} |",
        "TYPE", "AMOUNT (LOS)", "DETAIL / LINK", "HASH"
    );
    println!(
        "+----------------+----------------+--------------------------+------------------------+"
    );

    for b in blocks {
        let amount_los = b.amount / CIL_PER_LOS;
        let amt_str = format_u128(amount_los);

        let (type_str, amt_display, info) = match b.block_type {
            BlockType::Mint => (
                "🔥 MINT",
                format!("+{}", amt_str),
                format!("Src: {}", &b.link[..10.min(b.link.len())]),
            ),
            BlockType::Send => (
                "📤 SEND",
                format!("-{}", amt_str),
                format!("To: {}", get_short_addr(&b.link)),
            ),
            BlockType::Receive => (
                "📥 RECEIVE",
                format!("+{}", amt_str),
                format!("From Hash: {}", &b.link[..8.min(b.link.len())]),
            ),
            BlockType::Change => (
                "🔄 CHANGE",
                "0".to_string(),
                format!("Rep: {}", get_short_addr(&b.link)),
            ),
            BlockType::Slash => (
                "⚖️ SLASH",
                format!("-{}", amt_str),
                format!("Evidence: {}", &b.link[..10.min(b.link.len())]),
            ),
            BlockType::ContractDeploy => (
                "📦 DEPLOY",
                format!("-{}", amt_str),
                format!("Code: {}", &b.link[..16.min(b.link.len())]),
            ),
            BlockType::ContractCall => (
                "⚙️ CALL",
                format!("-{}", amt_str),
                format!("Contract: {}", &b.link[..16.min(b.link.len())]),
            ),
        };

        let hash_short = if b.calculate_hash().len() > 8 {
            format!("...{}", &b.calculate_hash()[..8])
        } else {
            "-".to_string()
        };

        println!(
            "| {:<14} | {:<14} | {:<24} | {:<22} |",
            type_str, amt_display, info, hash_short
        );
    }
    println!(
        "+----------------+----------------+--------------------------+------------------------+\n"
    );
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Ensure panics in spawned tasks are logged to stderr
    std::panic::set_hook(Box::new(|panic_info| {
        eprintln!("❌ PANIC in spawned task: {}", panic_info);
    }));

    // --- Dynamic port assignment ---
    // Parse command line arguments
    let args: Vec<String> = std::env::args().collect();

    // Extended CLI arguments for Flutter Validator launcher
    let mut api_port: u16 = 3030;
    let mut data_dir_override: Option<String> = None;
    let mut node_id_override: Option<String> = None;
    let mut json_log = false; // Machine-readable logs for Flutter
    let mut mainnet_flag = false; // Runtime --mainnet flag
    let mut enable_mining = false; // --mine: enable background PoW mining
    let mut mining_threads: usize = 1; // --mine-threads N: parallel mining threads

    {
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--mainnet" => {
                    mainnet_flag = true;
                }
                "--port" => {
                    if let Some(v) = args.get(i + 1) {
                        match v.parse::<u16>() {
                            Ok(p) => api_port = p,
                            Err(_) => eprintln!(
                                "⚠️  Invalid --port value '{}', using default {}",
                                v, api_port
                            ),
                        }
                        i += 1;
                    }
                }
                "--data-dir" => {
                    if let Some(v) = args.get(i + 1) {
                        data_dir_override = Some(v.clone());
                        i += 1;
                    }
                }
                "--node-id" => {
                    if let Some(v) = args.get(i + 1) {
                        node_id_override = Some(v.clone());
                        i += 1;
                    }
                }
                "--json-log" => {
                    json_log = true;
                }
                "--config" => {
                    // Legacy: load from validator.toml
                    if let Some(config_path) = args.get(i + 1) {
                        if let Ok(config_content) = fs::read_to_string(config_path) {
                            if let Some(line) = config_content
                                .lines()
                                .find(|l| l.trim().starts_with("rest_port"))
                            {
                                if let Some(port_str) = line.split('=').nth(1) {
                                    match port_str.trim().parse::<u16>() {
                                        Ok(p) => api_port = p,
                                        Err(_) => eprintln!("⚠️  Invalid rest_port in config: '{}', using default {}", port_str.trim(), api_port),
                                    }
                                }
                            }
                        }
                        i += 1;
                    }
                }
                "--mine" => {
                    enable_mining = true;
                }
                "--mine-threads" => {
                    if let Some(v) = args.get(i + 1) {
                        match v.parse::<usize>() {
                            Ok(t) => mining_threads = t.clamp(1, 16),
                            Err(_) => eprintln!(
                                "⚠️  Invalid --mine-threads value '{}', using default {}",
                                v, mining_threads
                            ),
                        }
                        i += 1;
                    }
                }
                _ => {
                    // Legacy: bare port number as first arg
                    if i == 1 {
                        if let Ok(p) = args[i].parse::<u16>() {
                            api_port = p;
                        }
                    }
                }
            }
            i += 1;
        }
    }

    // ── MAINNET / TESTNET SAFETY GATE ──────────────────────────────────
    // Prevent accidental mismatches between binary build and runtime flag.
    if mainnet_flag && !los_core::is_mainnet_build() {
        eprintln!(
            "❌ FATAL: --mainnet flag passed but binary was NOT compiled with --features mainnet"
        );
        eprintln!("   Rebuild with: cargo build --release -p los-node --features mainnet");
        std::process::exit(1);
    }
    if los_core::is_mainnet_build() && !mainnet_flag {
        eprintln!("❌ FATAL: Binary was compiled for MAINNET but --mainnet flag is missing");
        eprintln!("   Run with: los-node --mainnet --port <PORT> ...");
        eprintln!("   This safety check prevents accidental mainnet deployment.");
        std::process::exit(1);
    }
    if los_core::is_mainnet_build() {
        println!("═══════════════════════════════════════════════════════");
        println!(
            "  🔒 UNAUTHORITY MAINNET (Chain ID: {})              ",
            los_core::CHAIN_ID
        );
        println!("  All security enforced: consensus, signatures, PoW  ");
        println!("  Faucet: DISABLED | Mint Cap: ENFORCED             ");
        println!("═══════════════════════════════════════════════════════");
    }

    // When launched from Flutter (--json-log), stdout is a pipe (fully buffered).
    // Force line-buffering so JSON events and println! output reach Flutter immediately.
    if json_log {
        use std::io::Write;
        // Flush any pending output, then we rely on explicit flushes in json_event!
        let _ = std::io::stdout().flush();
    }

    // Structured JSON log helper for Flutter process monitoring
    // NOTE: Must flush stdout — when spawned from Flutter, stdout is a pipe
    // (fully buffered), not a TTY (line-buffered). Without flush, JSON events
    // never reach the Flutter process monitor.
    macro_rules! json_event {
        ($event:expr, $($key:expr => $val:expr),*) => {
            if json_log {
                let mut _j = serde_json::json!({"event": $event});
                $(_j[$key] = serde_json::json!($val);)*
                println!("{}", _j);
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
        };
    }

    // --- Initialize database ---
    println!("🗄️  Initializing database...");
    // AUTO-DETECT NODE ID from override, env var, or port
    // TESTNET ONLY: Port-to-name mapping is a development convenience.
    // MAINNET: Validators are identified by their public key/address, not port.
    let node_id = node_id_override.unwrap_or_else(|| {
        std::env::var("LOS_NODE_ID").unwrap_or_else(|_| {
            if los_core::is_testnet_build() {
                match api_port {
                    3030 => "validator-1".to_string(),
                    3031 => "validator-2".to_string(),
                    3032 => "validator-3".to_string(),
                    _ => format!("node-{}", api_port),
                }
            } else {
                format!("node-{}", api_port)
            }
        })
    });

    // Data directory: --data-dir override, or default node_data/<id>/
    let base_data_dir = data_dir_override.unwrap_or_else(|| format!("node_data/{}", node_id));

    println!("🆔 Node ID: {}", node_id);
    println!("📂 Data directory: {}/", base_data_dir);
    json_event!("init", "node_id" => &node_id, "data_dir" => &base_data_dir, "port" => api_port);

    // Create node-specific database path (CRITICAL: Multi-node isolation)
    let db_path = format!("{}/los_database", base_data_dir);
    std::fs::create_dir_all(&base_data_dir)?;

    let database = match LosDatabase::open(&db_path) {
        Ok(db) => {
            let stats = db.stats();
            println!("✅ Database opened: {}", db_path);
            println!(
                "   {} blocks, {} accounts, {:.2} MB on disk",
                stats.blocks_count,
                stats.accounts_count,
                stats.size_on_disk as f64 / 1_048_576.0
            );
            Arc::new(db)
        }
        Err(e) => {
            eprintln!("❌ Failed to open database at {}: {}", db_path, e);
            eprintln!("   Possible causes:");
            eprintln!("   1. Another los-node instance is still running with the same data-dir");
            eprintln!("   2. A previous instance was killed and the OS hasn't released the lock");
            eprintln!("   Fix: kill all los-node processes → pkill -9 -f los-node");
            json_event!("fatal", "error" => "database_lock_failed", "path" => &db_path);
            return Err(e.into());
        }
    };

    // ── PID LOCKFILE ────────────────────────────────────────────────────
    // Write our PID so Flutter (and future starts) can detect stale instances.
    // Cleaned up by the SIGTERM handler on graceful shutdown.
    {
        let pid = std::process::id();
        let pid_path = format!("{}/.los-node.pid", base_data_dir);
        if let Err(e) = std::fs::write(&pid_path, pid.to_string()) {
            eprintln!("⚠️ Could not write PID lockfile: {}", e);
        } else {
            println!("🔒 PID lockfile: {} (PID {})", pid_path, pid);
        }
    }

    // --- Initialize Prometheus metrics ---
    println!("📊 Initializing Prometheus metrics...");
    let metrics = match LosMetrics::new() {
        Ok(m) => {
            println!("✅ Metrics ready: 45+ endpoints registered");
            m
        }
        Err(e) => {
            eprintln!("❌ Failed to initialize metrics: {}", e);
            return Err(e);
        }
    };

    // ── SECURITY: Read secrets from stdin pipe (preferred) or env vars (fallback) ──
    // When launched from Flutter or a secure process manager, secrets are piped via
    // stdin to avoid exposure in /proc/[pid]/environ on Linux.
    // Protocol: line 1 = wallet_password, line 2 = seed_phrase (empty = skip).
    // If stdin is a TTY (interactive), skip and read from env vars as before.
    let (stdin_wallet_pw, stdin_seed_phrase) = {
        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() {
            // stdin is piped — read secrets line by line (blocking, before async runtime)
            let mut line1 = String::new();
            let mut line2 = String::new();
            let _ = std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut line1);
            let _ = std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut line2);
            let pw = line1.trim().to_string();
            let sp = line2.trim().to_string();
            (
                if pw.is_empty() { None } else { Some(pw) },
                if sp.is_empty() { None } else { Some(sp) },
            )
        } else {
            (None, None)
        }
    };

    // Use node-specific wallet file path
    // SECURITY: Wallet keys are encrypted at rest using age encryption.
    // The encryption password is derived from the node ID (for automated startup).
    // MAINNET: operators MUST set LOS_WALLET_PASSWORD — weak auto-key is rejected.
    let wallet_path = format!("{}/wallet.json", &base_data_dir);
    let wallet_password =
        match stdin_wallet_pw.or_else(|| std::env::var("LOS_WALLET_PASSWORD").ok()) {
            Some(pw) if pw.len() >= 12 => pw,
            Some(pw) if !pw.is_empty() => {
                if los_core::is_mainnet_build() {
                    eprintln!(
                        "❌ FATAL: LOS_WALLET_PASSWORD must be at least 12 characters on mainnet."
                    );
                    return Err(Box::<dyn std::error::Error>::from(
                        "LOS_WALLET_PASSWORD too short for mainnet (min 12 chars)",
                    ));
                }
                pw // Testnet: allow shorter passwords
            }
            _ => {
                if los_core::is_mainnet_build() {
                    eprintln!(
                    "❌ FATAL: LOS_WALLET_PASSWORD environment variable is REQUIRED on mainnet."
                );
                    eprintln!("   export LOS_WALLET_PASSWORD='<strong-password-here>'");
                    return Err(Box::<dyn std::error::Error>::from(
                        "LOS_WALLET_PASSWORD required for mainnet build",
                    ));
                }
                // Testnet: auto-generate weak password (acceptable for testing)
                let auto = format!("los-node-{}-autokey", &node_id);
                println!("⚠️  Using auto-generated wallet password (testnet only)");
                auto
            }
        };
    let keys: los_crypto::KeyPair = if let Some(seed_phrase) =
        stdin_seed_phrase.or_else(|| std::env::var("LOS_SEED_PHRASE").ok())
    {
        // DETERMINISTIC KEYPAIR: Derive from BIP39 mnemonic (genesis validator identity)
        // This ensures the node's runtime address matches its genesis address.
        // SECURITY: Prefer stdin pipe over env var to avoid /proc/[pid]/environ exposure.
        let mnemonic = match bip39::Mnemonic::parse_normalized(&seed_phrase) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("FATAL: Seed phrase contains invalid BIP39 mnemonic: {e}");
                eprintln!(
                    "Please check the seed phrase (stdin or LOS_SEED_PHRASE env) and try again."
                );
                std::process::exit(1);
            }
        };
        let bip39_seed = mnemonic.to_seed("");
        let kp = los_crypto::generate_keypair_from_seed(&bip39_seed);
        let derived_addr = los_crypto::public_key_to_address(&kp.public_key);
        println!(
            "🔑 Derived keypair from LOS_SEED_PHRASE → {}",
            get_short_addr(&derived_addr)
        );
        // Save/overwrite wallet.json so subsequent restarts without seed phrase still work
        fs::create_dir_all(&base_data_dir).ok();
        if let Ok(encrypted) = los_crypto::migrate_to_encrypted(&kp, &wallet_password) {
            let _ = fs::write(
                &wallet_path,
                serde_json::to_string(&encrypted).unwrap_or_default(),
            );
        }
        kp
    } else if let Ok(data) = fs::read_to_string(&wallet_path) {
        // Try parsing as encrypted key first, fall back to legacy plaintext
        if let Ok(encrypted) = serde_json::from_str::<los_crypto::EncryptedKey>(&data) {
            let sk =
                los_crypto::decrypt_private_key(&encrypted, &wallet_password).map_err(|e| {
                    Box::<dyn std::error::Error>::from(format!(
                        "Wallet decrypt failed: {}. Set LOS_WALLET_PASSWORD if changed.",
                        e
                    ))
                })?;
            los_crypto::KeyPair {
                public_key: encrypted.public_key,
                secret_key: sk,
            }
        } else if let Ok(plain_key) = serde_json::from_str::<los_crypto::KeyPair>(&data) {
            // Legacy plaintext wallet — auto-migrate to encrypted
            eprintln!("⚠️  Migrating plaintext wallet to encrypted format...");
            let encrypted = los_crypto::migrate_to_encrypted(&plain_key, &wallet_password)
                .map_err(|e| {
                    Box::<dyn std::error::Error>::from(format!("Migration failed: {}", e))
                })?;
            fs::write(&wallet_path, serde_json::to_string(&encrypted)?)?;
            println!("🔒 Wallet migrated to encrypted storage");
            plain_key
        } else {
            return Err(Box::from(
                "Failed to parse wallet file — corrupted or invalid format",
            ));
        }
    } else {
        let new_k = los_crypto::generate_keypair();
        fs::create_dir_all(&base_data_dir)?;
        // Store encrypted from the start
        let encrypted = los_crypto::migrate_to_encrypted(&new_k, &wallet_password)
            .map_err(|e| Box::<dyn std::error::Error>::from(format!("Encryption failed: {}", e)))?;
        fs::write(&wallet_path, serde_json::to_string(&encrypted)?)?;
        println!("🔑 Generated new encrypted keypair for {}", node_id);
        new_k
    };

    let my_address = los_crypto::public_key_to_address(&keys.public_key);
    let my_short = get_short_addr(&my_address);
    // MAINNET SAFETY (W1): Wrap secret key in Zeroizing so it's zeroed on drop
    let secret_key = Zeroizing::new(keys.secret_key.clone());
    json_event!("wallet_ready", "address" => &my_address, "short" => &my_short);

    // ══════════════════════════════════════════════════════════════════════
    // MAINNET SAFETY (M-6): Tor & network security enforcement
    // ══════════════════════════════════════════════════════════════════════
    if los_core::is_mainnet_build() {
        // T-1: Mainnet MUST have Tor SOCKS5 proxy configured
        let has_tor = std::env::var("LOS_SOCKS5_PROXY")
            .or_else(|_| std::env::var("LOS_TOR_SOCKS5"))
            .is_ok();
        if !has_tor {
            eprintln!(
                "❌ FATAL: Mainnet requires Tor. Set LOS_SOCKS5_PROXY=socks5h://127.0.0.1:9050"
            );
            eprintln!("   Unauthority mainnet runs EXCLUSIVELY on Tor Hidden Services.");
            return Err(Box::<dyn std::error::Error>::from(
                "LOS_SOCKS5_PROXY or LOS_TOR_SOCKS5 required for mainnet build",
            ));
        }
        // R-2: Mainnet MUST NOT bind to 0.0.0.0 (IP deanonymization risk)
        if std::env::var("LOS_BIND_ALL").unwrap_or_default() == "1" {
            eprintln!(
                "❌ FATAL: LOS_BIND_ALL=1 is forbidden on mainnet (IP deanonymization risk)."
            );
            eprintln!("   Mainnet validators MUST bind to 127.0.0.1 only (accessed via Tor hidden service).");
            return Err(Box::<dyn std::error::Error>::from(
                "LOS_BIND_ALL=1 forbidden on mainnet — use Tor hidden service instead",
            ));
        }
        println!("🧅 Mainnet Tor enforcement: PASSED");
    }

    // Load ledger and genesis BEFORE wrapping in Arc to prevent race condition
    let mut ledger_state = load_from_disk(&database);

    // Sanitize: remove orphaned blocks from l.blocks that aren't part of any account chain.
    // This cleans up ghost blocks caused by failed process_block() insertions or sync artifacts.
    let orphans_removed = ledger_state.remove_orphaned_blocks();
    if orphans_removed > 0 {
        println!(
            "🧹 Startup: removed {} orphaned block(s) from ledger",
            orphans_removed
        );
    }

    // Collect genesis validator → onion_address mappings during genesis loading.
    // Used to seed validator_endpoints AFTER it's created downstream.
    let mut genesis_onion_map: Vec<(String, String)> = Vec::new();

    // ══════════════════════════════════════════════════════════════════════
    // GENESIS LOADING — Network-aware with validation
    // ══════════════════════════════════════════════════════════════════════
    //
    // Mainnet:  Loads from genesis_config.json (gitignored, contains real keys)
    //           MUST exist and pass full validation. Node refuses to start without it.
    //           Validates: total_supply=21936236, address format, network="mainnet".
    //
    // Testnet:  Loads from testnet-genesis/testnet_wallets.json (git-tracked, test keys)
    //           Falls back gracefully if missing.
    //
    // Both paths use the same insert-if-absent logic to preserve existing state.
    //
    // bootstrap_validators: Populated from genesis — used by /validators and /node-info
    // to avoid hardcoding testnet-specific addresses that would break mainnet.
    let mut bootstrap_validators: Vec<String> = Vec::new();
    let mut genesis_ts_from_config: Option<u64> = None;
    {
        let genesis_path = if los_core::is_mainnet_build() {
            "genesis_config.json"
        } else {
            "testnet-genesis/testnet_wallets.json"
        };

        // MAINNET: genesis_config.json is REQUIRED — refuse to start without it
        if los_core::is_mainnet_build() && !std::path::Path::new(genesis_path).exists() {
            eprintln!("❌ FATAL: genesis_config.json not found!");
            eprintln!("   Mainnet requires genesis_config.json at the working directory root.");
            eprintln!("   Generate with: cargo run -p genesis --bin genesis");
            return Err(Box::<dyn std::error::Error>::from(
                "Missing genesis_config.json for mainnet build",
            ));
        }

        if std::path::Path::new(genesis_path).exists() {
            if let Ok(genesis_json) = std::fs::read_to_string(genesis_path) {
                // Mainnet: use validated GenesisConfig parser
                // Testnet: use the raw JSON wallets parser (legacy format)
                if los_core::is_mainnet_build() {
                    // Validate genesis config BEFORE loading accounts.
                    // Prevents tampered genesis files from silently loading invalid state.
                    {
                        let genesis_config: genesis::GenesisConfig =
                            serde_json::from_str(&genesis_json)
                                .map_err(|e| {
                                    format!("Failed to parse genesis JSON for validation: {}", e)
                                })
                                .unwrap_or_else(|e| {
                                    eprintln!("❌ FATAL: {}", e);
                                    std::process::exit(1);
                                });
                        if let Err(e) = genesis::validate_genesis(&genesis_config) {
                            eprintln!("❌ FATAL: Genesis validation failed: {}", e);
                            return Err(Box::<dyn std::error::Error>::from(format!(
                                "Genesis validation failed: {}",
                                e
                            )));
                        }
                        // Extract bootstrap validator addresses from genesis config
                        if let Some(ref nodes) = genesis_config.bootstrap_nodes {
                            for node in nodes {
                                bootstrap_validators.push(node.address.clone());
                                // Collect host_address (or onion_address fallback) for endpoint discovery
                                if let Some(host) = resolve_genesis_host(node) {
                                    genesis_onion_map.push((node.address.clone(), host));
                                }
                            }
                            println!(
                                "🔍 Loaded {} bootstrap validators from genesis",
                                bootstrap_validators.len()
                            );
                        }
                        // Store genesis_timestamp for reward pool initialization
                        // (avoids re-reading the file and eliminates stale fallback risk)
                        genesis_ts_from_config = genesis_config.genesis_timestamp;
                        println!("✅ Genesis config validated (supply, network, addresses)");
                    }
                    match genesis::load_genesis_from_file(genesis_path) {
                        Ok(accounts) => {
                            let mut loaded_count = 0;
                            let mut genesis_supply_deducted: u128 = 0;
                            for (address, state) in accounts {
                                if state.balance > 0
                                    && !ledger_state.accounts.contains_key(&address)
                                {
                                    genesis_supply_deducted += state.balance;
                                    ledger_state.accounts.insert(address, state);
                                    loaded_count += 1;
                                }
                            }
                            if loaded_count > 0 {
                                // NOTE: remaining_supply starts at PUBLIC_SUPPLY_CAP (21,158,413 LOS)
                                // which already EXCLUDES the dev allocation (~3.5%). Dev wallets are
                                // a separate pre-genesis allocation, NOT minted from the public mining pool.
                                // Do NOT deduct genesis wallets from remaining_supply.
                                save_to_disk_internal(&ledger_state, &database, true);
                                println!(
                                    "🏦 MAINNET genesis: loaded {} accounts ({} CIL pre-allocated)",
                                    loaded_count, genesis_supply_deducted
                                );
                            }
                        }
                        Err(e) => {
                            eprintln!("❌ FATAL: Invalid genesis_config.json: {}", e);
                            return Err(Box::<dyn std::error::Error>::from(format!(
                                "Invalid genesis config: {}",
                                e
                            )));
                        }
                    }
                } else {
                    // Testnet: raw JSON with "wallets" array (legacy format)
                    if let Ok(genesis_data) =
                        serde_json::from_str::<serde_json::Value>(&genesis_json)
                    {
                        if let Some(wallets) = genesis_data["wallets"].as_array() {
                            let mut loaded_count = 0;
                            let mut genesis_supply_deducted: u128 = 0;

                            for wallet in wallets {
                                // Support both "balance_los" and "genesis_balance_los" field names
                                // testnet_wallets.json uses "genesis_balance_los", mainnet uses "balance_los"
                                let balance_str_opt = wallet["balance_los"]
                                    .as_str()
                                    .or_else(|| wallet["genesis_balance_los"].as_str());
                                if let (Some(address), Some(balance_str)) =
                                    (wallet["address"].as_str(), balance_str_opt)
                                {
                                    // Validate testnet genesis wallet entries
                                    if !address.starts_with("LOS") || address.len() < 10 {
                                        eprintln!("⚠️ Testnet genesis: skipping invalid address format: {}", address);
                                        continue;
                                    }
                                    let balance_cil =
                                        genesis::parse_los_to_cil(balance_str).unwrap_or(0);
                                    if balance_cil == 0 {
                                        eprintln!("⚠️ Testnet genesis: skipping zero/invalid balance for {}", address);
                                        continue;
                                    }
                                    // Sanity: no single wallet should exceed total supply
                                    if balance_cil > 21_936_236u128 * CIL_PER_LOS {
                                        eprintln!("⚠️ Testnet genesis: skipping wallet {} (balance exceeds total supply)", address);
                                        continue;
                                    }
                                    // Track validator wallets for /validators endpoint
                                    // Detect by wallet_type field (testnet uses "BootstrapNode(N)")
                                    // or role field (mainnet uses "validator")
                                    let is_validator = wallet["wallet_type"]
                                        .as_str()
                                        .map(|wt| wt.starts_with("BootstrapNode"))
                                        .unwrap_or(false)
                                        || wallet["role"].as_str() == Some("validator");
                                    if is_validator {
                                        bootstrap_validators.push(address.to_string());
                                        // Collect host address for validator endpoint discovery
                                        // Accepts host_address or onion_address from testnet config
                                        let host = wallet["host_address"]
                                            .as_str()
                                            .filter(|s| !s.is_empty())
                                            .or_else(|| {
                                                wallet["onion_address"]
                                                    .as_str()
                                                    .filter(|s| !s.is_empty())
                                            });
                                        if let Some(h) = host {
                                            genesis_onion_map
                                                .push((address.to_string(), h.to_string()));
                                        }
                                    }
                                    if !ledger_state.accounts.contains_key(address) {
                                        ledger_state.accounts.insert(
                                            address.to_string(),
                                            AccountState {
                                                head: "0".to_string(),
                                                balance: balance_cil,
                                                block_count: 0,
                                                is_validator,
                                            },
                                        );
                                        genesis_supply_deducted += balance_cil;
                                        loaded_count += 1;
                                    }
                                }
                            }

                            if loaded_count > 0 {
                                // Validate aggregate balance doesn't exceed total supply
                                let max_supply_cil = 21_936_236u128 * CIL_PER_LOS;
                                if genesis_supply_deducted > max_supply_cil {
                                    eprintln!("❌ FATAL: Testnet genesis aggregate balance ({} CIL) exceeds total supply ({} CIL)",
                                        genesis_supply_deducted, max_supply_cil);
                                    return Err(Box::<dyn std::error::Error>::from(
                                        "Testnet genesis aggregate balance exceeds total supply",
                                    ));
                                }
                                // NOTE: remaining_supply = PUBLIC_SUPPLY_CAP already excludes
                                // dev allocation. Genesis wallets are pre-allocated, not mined from public pool.
                                save_to_disk_internal(&ledger_state, &database, true);
                                println!(
                                    "🎁 Testnet genesis: loaded {} accounts ({} CIL pre-allocated)",
                                    loaded_count, genesis_supply_deducted
                                );
                                if !bootstrap_validators.is_empty() {
                                    println!(
                                        "🔍 Loaded {} bootstrap validators from testnet genesis",
                                        bootstrap_validators.len()
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // VALIDATOR IDENTITY CHECK: Verify if this node's address is a genesis bootstrap validator.
    // With LOS_SEED_PHRASE, the keypair is deterministic and matches genesis config.
    // No state mutation here — genesis loading already set is_validator + balance.
    if testnet_config::get_testnet_config().should_enable_consensus() {
        let is_genesis_validator = bootstrap_validators.contains(&my_address);
        if is_genesis_validator {
            println!(
                "\u{2705} Node address {} matches genesis bootstrap validator (stake: {} LOS)",
                get_short_addr(&my_address),
                ledger_state
                    .accounts
                    .get(&my_address)
                    .map(|a| a.balance / CIL_PER_LOS)
                    .unwrap_or(0)
            );
        } else {
            println!("\u{2139}\u{fe0f}  Node address {} is NOT a genesis validator. Register via API with \u{2265}1 LOS stake.",
                get_short_addr(&my_address));
        }
    }

    // ══════════════════════════════════════════════════════════════════════
    // VALIDATOR REWARD POOL — Initialize and register known validators
    // ══════════════════════════════════════════════════════════════════════
    // Uses genesis_timestamp from validated GenesisConfig (mainnet) or system time (testnet).
    // Bootstrap validators are registered as is_genesis=true (eligible for rewards).
    // Pool is initialized from VALIDATOR_REWARD_POOL_CIL constant.
    let genesis_ts: u64 = if los_core::is_mainnet_build() {
        // Use timestamp stored during genesis validation above (no redundant file I/O)
        genesis_ts_from_config.unwrap_or_else(|| {
            eprintln!(
                "❌ FATAL: genesis_timestamp missing after validation — this should never happen"
            );
            std::process::exit(1);
        })
    } else {
        // Testnet: use current time as genesis to avoid epoch backlog.
        // This means rewards start fresh each time the node is restarted with a clean DB.
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    };
    let mut reward_pool_state = ValidatorRewardPool::new(genesis_ts);

    // Register all bootstrap validators as genesis
    // (tracked for heartbeat/uptime but EXCLUDED from reward distribution —
    //  all rewards go to public validators for fair distribution)
    for addr in &bootstrap_validators {
        let stake = ledger_state
            .accounts
            .get(addr)
            .map(|a| a.balance)
            .unwrap_or(0);
        reward_pool_state.register_validator(addr, true, stake);
    }

    // Register any other validators already in the ledger (non-genesis)
    for (addr, acct) in &ledger_state.accounts {
        if acct.is_validator && !bootstrap_validators.contains(addr) {
            reward_pool_state.register_validator(addr, false, acct.balance);
        }
    }

    // Also register THIS node's own address so heartbeats are tracked
    // (The node's generated keypair may differ from genesis addresses)
    if !reward_pool_state.validators.contains_key(&my_address) {
        let my_stake = ledger_state
            .accounts
            .get(&my_address)
            .map(|a| a.balance)
            .unwrap_or(0);
        let is_bootstrap = bootstrap_validators.contains(&my_address);
        reward_pool_state.register_validator(&my_address, is_bootstrap, my_stake);
        println!(
            "📡 Registered node address {} in reward pool",
            &my_address[..12.min(my_address.len())]
        );
    }

    // Fast-forward through any missed epochs (e.g., after node restart from old genesis)
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let skipped = reward_pool_state.catch_up_epochs(now_secs);
    if skipped > 0 {
        println!(
            "⏩ Skipped {} missed epochs (fast-forward to current time)",
            skipped
        );
    }

    // Set expected heartbeats using the correct interval for testnet/mainnet
    let initial_heartbeat_secs: u64 = if los_core::is_testnet_build() { 10 } else { 60 };
    reward_pool_state.set_expected_heartbeats(initial_heartbeat_secs);

    // ── STARTUP AUTO SELF-REGISTER ──────────────────────────────────
    // If this node's address already has balance >= 1 LOS (from previous
    // mining session) but isn't flagged as a validator, auto-register it.
    // This handles the restart case where the node mined previously.
    let startup_auto_registered = if enable_mining && !bootstrap_validators.contains(&my_address) {
        let should = ledger_state
            .accounts
            .get(&my_address)
            .map(|acc| !acc.is_validator && acc.balance >= MIN_VALIDATOR_REGISTER_CIL)
            .unwrap_or(false);
        if should {
            let balance_los = ledger_state
                .accounts
                .get(&my_address)
                .map(|a| a.balance / CIL_PER_LOS)
                .unwrap_or(0);
            if let Some(acc_mut) = ledger_state.accounts.get_mut(&my_address) {
                acc_mut.is_validator = true;
            }
            println!(
                "✅ Startup auto-register: {} as validator (balance: {} LOS)",
                get_short_addr(&my_address),
                balance_los
            );
            true
        } else {
            false
        }
    } else {
        false
    };

    let reward_pool = Arc::new(Mutex::new(reward_pool_state));
    println!(
        "🏆 Validator reward pool initialized: {} LOS, epoch rate {} LOS/month",
        los_core::VALIDATOR_REWARD_POOL_CIL / CIL_PER_LOS,
        los_core::REWARD_RATE_INITIAL_CIL / CIL_PER_LOS
    );

    // Now wrap in Arc after all initialization is complete
    let ledger = Arc::new(Mutex::new(ledger_state));

    // Load persistent peer storage from database
    let initial_peers = match database.load_peers() {
        Ok(peers) => {
            if !peers.is_empty() {
                println!("📚 Loaded {} known peers from database", peers.len());
            }
            peers
        }
        Err(e) => {
            eprintln!("⚠️ Failed to load peers: {}", e);
            HashMap::new()
        }
    };
    let address_book = Arc::new(Mutex::new(initial_peers));

    // live_peers tracks validators that PROVED liveness via gossipsub.
    // Key = full address, Value = Unix timestamp of last received gossipsub message.
    // Only peers in this map with recent timestamps receive heartbeats for rewards.
    // This prevents dead validators from earning rewards via stale address_book entries.
    let live_peers: Arc<Mutex<HashMap<String, u64>>> = Arc::new(Mutex::new(HashMap::new()));

    // LOCAL REGISTERED VALIDATORS — Tracks wallet addresses registered as validators
    // through THIS node's API. The heartbeat loop records heartbeats for these addresses
    // because this node's liveness proves the registered validator's liveness.
    // Without this, a user who registers a different wallet address than the node's keypair
    // would get 0 heartbeats → 0% uptime → no rewards.
    let local_registered_validators: Arc<Mutex<HashSet<String>>> =
        Arc::new(Mutex::new(HashSet::new()));

    let pending_sends = Arc::new(Mutex::new(HashMap::<String, (Block, u128)>::new()));

    // Mempool: tracks pending transactions with priority ordering and expiration.
    // Runs alongside pending_sends (shadow mode) to provide stats and future block assembly.
    let mempool_pool = Arc::new(Mutex::new(mempool::Mempool::new()));

    // Vote deduplication — track which validators have already voted
    // Prevents a single validator from reaching consensus alone by sending multiple votes
    let send_voters = Arc::new(Mutex::new(HashMap::<String, HashSet<String>>::new()));

    // DESIGN Pending checkpoints accumulating multi-validator signatures.
    // Keyed by checkpoint height → PendingCheckpoint with accumulated signatures.
    // Once 2f+1 sigs collected, finalized via CheckpointManager::store_checkpoint().
    let pending_checkpoints: Arc<Mutex<HashMap<u64, PendingCheckpoint>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // DESIGN Outbox for checkpoint gossip messages.
    // The save task pushes CHECKPOINT_PROPOSE messages here; a gossip consumer drains them.
    let checkpoint_outbox: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    // ══════════════════════════════════════════════════════════════════════
    // VALIDATOR ENDPOINTS — Maps validator_address → host_address
    // ══════════════════════════════════════════════════════════════════════
    // Enables Flutter apps and other nodes to discover validator endpoints
    // beyond the hardcoded bootstrap list. Host can be .onion, IP, or domain.
    // Populated from:
    // 1. This node's own LOS_HOST_ADDRESS or LOS_ONION_ADDRESS
    // 2. Genesis validator host_address/onion_address fields
    // 3. VALIDATOR_REG gossip messages
    // 4. PEER_LIST exchange messages
    let mut initial_endpoints = HashMap::<String, String>::new();
    // Register this node's own host address (with port)
    if let Some(raw_host) = get_node_host_address() {
        let our_host = ensure_host_port(&raw_host, api_port);
        initial_endpoints.insert(my_address.clone(), our_host.clone());
        println!("🌐 Registered own host endpoint: {}", our_host);
    }
    // Seed from genesis validator host addresses (collected during genesis loading)
    for (addr, host) in &genesis_onion_map {
        initial_endpoints
            .entry(addr.clone())
            .or_insert_with(|| host.clone());
    }
    if !genesis_onion_map.is_empty() {
        println!(
            "🌐 Seeded {} genesis validator endpoints for discovery",
            genesis_onion_map.len()
        );
    }
    let validator_endpoints = Arc::new(Mutex::new(initial_endpoints));

    // PoW MINT ENGINE — Fair token distribution via SHA3 proof-of-work
    // miners compute SHA3-256(address || epoch || nonce) and submit proofs.
    // 1 successful mint per address per epoch. Reward halves periodically.
    let mining_state = Arc::new(Mutex::new(MiningState::new(genesis_ts)));
    {
        let mut ms = safe_lock(&mining_state);
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let current_epoch = ms.epoch_from_time(now_secs);

        // Rebuild current_epoch_miners from persisted ledger.
        // Without this, a node restart within the same epoch allows double-mining
        // because the in-memory dedup set starts empty.
        {
            let l = safe_lock(&ledger);
            let epoch_prefix = format!("MINE:{}:", current_epoch);
            for block in l.blocks.values() {
                if block.block_type == BlockType::Mint && block.link.starts_with(&epoch_prefix) {
                    ms.current_epoch_miners.insert(block.account.clone());
                }
            }
            if !ms.current_epoch_miners.is_empty() {
                println!(
                    "⛏️  Rebuilt epoch {} miners from ledger: {} addresses",
                    current_epoch,
                    ms.current_epoch_miners.len()
                );
            }
        }
        // Sync the epoch number in mining state
        ms.current_epoch = current_epoch;

        let epoch_reward = MiningState::epoch_reward_cil(current_epoch) / CIL_PER_LOS;
        println!(
            "⛏️  PoW Mint engine: epoch {}, difficulty {} bits, reward ~{} LOS/epoch",
            current_epoch, ms.difficulty_bits, epoch_reward
        );
        if enable_mining {
            println!(
                "⛏️  Background mining ENABLED ({} thread{})",
                mining_threads,
                if mining_threads > 1 { "s" } else { "" }
            );
        }
    }

    // Slashing Manager (validator accountability)
    let slashing_manager = Arc::new(Mutex::new(SlashingManager::new()));
    // Register existing validators from genesis (only accounts with is_validator flag)
    {
        let l = safe_lock(&ledger);
        let mut sm = safe_lock(&slashing_manager);
        for (addr, acc) in &l.accounts {
            if acc.is_validator {
                sm.register_validator(addr.clone());
            }
        }
        let registered = sm.get_safety_stats().total_validators;
        if registered > 0 {
            println!(
                "🛡️  SlashingManager: {} validators registered from genesis",
                registered
            );
        }
    }

    // Finality Checkpoint Manager (prevents long-range attacks)
    // Use --data-dir path, NOT hardcoded node_data/{node_id}/.
    // The old path was shared across all flutter-validator instances regardless
    // of --data-dir, causing lock conflicts from zombie (UE) processes.
    let checkpoint_db_path = format!("{}/checkpoints", base_data_dir);
    let checkpoint_manager = match CheckpointManager::new(&checkpoint_db_path) {
        Ok(cm) => {
            let latest = cm.get_latest_checkpoint().ok().flatten();
            if let Some(cp) = &latest {
                println!(
                    "🏁 CheckpointManager: resuming from checkpoint at height {}",
                    cp.height
                );
            } else {
                println!(
                    "🏁 CheckpointManager: no checkpoints yet (will create every {} blocks)",
                    CHECKPOINT_INTERVAL
                );
            }
            Arc::new(Mutex::new(cm))
        }
        Err(e) => {
            eprintln!("⚠️ Failed to open checkpoint DB: {} — trying fallback", e);
            // Fallback: temp directory that's guaranteed to have no stale locks
            let fallback_path = format!("{}/checkpoints_fallback", base_data_dir);
            match CheckpointManager::new(&fallback_path) {
                Ok(cm) => Arc::new(Mutex::new(cm)),
                Err(e2) => {
                    eprintln!(
                        "FATAL: Both checkpoint DBs failed: {} — node cannot start safely",
                        e2
                    );
                    eprintln!("   Fix: kill all los-node processes → pkill -9 -f los-node");
                    json_event!("fatal", "error" => "checkpoint_db_lock_failed", "path" => &checkpoint_db_path);
                    return Err(Box::<dyn std::error::Error>::from(e2.to_string()));
                }
            }
        }
    };

    // Init own account in ledger if not exists
    {
        let mut l = safe_lock(&ledger);
        if !l.accounts.contains_key(&my_address) {
            if !testnet_config::get_testnet_config().should_enable_consensus() {
                // Create proper Mint block for testnet initial balance
                // This deducts from distribution.remaining_supply (no free money)
                l.accounts.insert(
                    my_address.clone(),
                    AccountState {
                        head: "0".to_string(),
                        balance: 0,
                        block_count: 0,
                        is_validator: false,
                    },
                );

                let mut init_block = Block {
                    account: my_address.clone(),
                    previous: "0".to_string(),
                    block_type: BlockType::Mint,
                    amount: TESTNET_INITIAL_BALANCE,
                    link: format!(
                        "TESTNET:INITIAL:{}",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs()
                    ),
                    signature: "".to_string(),
                    public_key: hex::encode(&keys.public_key),
                    work: 0,
                    timestamp: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                    fee: 0,
                };

                solve_pow(&mut init_block);
                init_block.signature =
                    match try_sign_hex(init_block.signing_hash().as_bytes(), &secret_key) {
                        Ok(sig) => sig,
                        Err(e) => {
                            eprintln!(
                                "FATAL: Cannot sign init block: {} — node cannot start safely",
                                e
                            );
                            std::process::exit(1);
                        }
                    };

                match l.process_block(&init_block) {
                    Ok(_) => {
                        SAVE_DIRTY.store(true, Ordering::Release);
                        println!("🎁 TESTNET (Functional): Node initialized with 1000 LOS via Mint block (supply deducted)");
                    }
                    Err(e) => {
                        println!(
                            "⚠️ TESTNET initial mint failed: {} — creating empty account",
                            e
                        );
                    }
                }
            } else {
                // Production: Create empty account (balance from PoW mining only)
                l.accounts.insert(
                    my_address.clone(),
                    AccountState {
                        head: "0".to_string(),
                        balance: 0,
                        block_count: 0,
                        is_validator: false,
                    },
                );
            }
        }
    }

    // SAFETY: Set env vars BEFORE any tokio::spawn to avoid data races.
    // Rust 1.83+ marks set_var as unsafe in multi-threaded contexts.
    // Must happen before background tasks are spawned below.
    if std::env::var("LOS_P2P_PORT").is_err() {
        let p2p_port = api_port + 1000;
        unsafe {
            std::env::set_var("LOS_P2P_PORT", p2p_port.to_string());
        }
        println!(
            "📡 P2P port auto-derived: {} (API {} + 1000)",
            p2p_port, api_port
        );
    }

    // Background task for debounced disk saves (prevents race conditions)
    // Clone ledger snapshot THEN release lock BEFORE disk I/O
    let save_ledger = Arc::clone(&ledger);
    let save_database = Arc::clone(&database);
    let save_checkpoint_mgr = Arc::clone(&checkpoint_manager);
    // Clone signing credentials for checkpoint signatures
    let save_secret_key = secret_key.clone();
    let save_my_address = my_address.clone();
    // DESIGN Clone pending checkpoints for multi-validator coordination
    let save_pending_checkpoints = Arc::clone(&pending_checkpoints);
    let save_checkpoint_outbox = Arc::clone(&checkpoint_outbox);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        loop {
            interval.tick().await;

            // Only save if dirty and not currently saving
            if SAVE_DIRTY.load(Ordering::Acquire) && !SAVE_IN_PROGRESS.load(Ordering::Acquire) {
                // Clone ledger under lock, then release lock BEFORE disk I/O
                let (ledger_snapshot, block_count, validator_count) = {
                    let l = safe_lock(&save_ledger);
                    let bc = l.blocks.len() as u64;
                    let vc = l
                        .accounts
                        .iter()
                        .filter(|(_, a)| a.balance >= MIN_VALIDATOR_STAKE_CIL)
                        .count() as u32;
                    (l.clone(), bc, vc)
                }; // Lock released — API requests can proceed during save
                save_to_disk_internal(&ledger_snapshot, &save_database, false);

                // CHECKPOINT: Create finality checkpoint when block_count crosses next interval
                // Use >= instead of == to handle block-lattice where exact multiples may be skipped
                if block_count > 0 {
                    let mut cm = safe_lock(&save_checkpoint_mgr);
                    let latest_height = cm
                        .get_latest_checkpoint()
                        .ok()
                        .flatten()
                        .map(|cp| cp.height)
                        .unwrap_or(0);
                    let next_checkpoint =
                        ((latest_height / CHECKPOINT_INTERVAL) + 1) * CHECKPOINT_INTERVAL;

                    if block_count >= next_checkpoint {
                        // Snap block_count DOWN to aligned interval.
                        // In a block-lattice, block_count rarely lands exactly on a
                        // multiple of CHECKPOINT_INTERVAL. Without snapping, every
                        // checkpoint was silently rejected by is_valid_interval().
                        let checkpoint_height =
                            (block_count / CHECKPOINT_INTERVAL) * CHECKPOINT_INTERVAL;

                        // Calculate simple state root from account balances
                        // DESIGN Use Ledger::compute_state_root() for consistency
                        let state_root = ledger_snapshot.compute_state_root();

                        // Find latest block hash
                        let latest_block_hash = ledger_snapshot
                            .blocks
                            .values()
                            .max_by_key(|b| b.timestamp)
                            .map(|b| b.calculate_hash())
                            .unwrap_or_else(|| "genesis".to_string());

                        // Sign checkpoint data with this node's key.
                        // DESIGN Create checkpoint proposal with our signature,
                        // store in pending_checkpoints map. The gossip broadcast task
                        // will handle broadcasting CHECKPOINT_PROPOSE to peers for
                        // multi-validator signature collection.
                        let checkpoint = FinalityCheckpoint::new(
                            checkpoint_height,
                            latest_block_hash,
                            validator_count.max(1),
                            state_root,
                            vec![], // placeholder — filled below
                        );
                        let signing_data = checkpoint.signing_data();
                        let my_sig = match los_crypto::sign_message(&signing_data, &save_secret_key)
                        {
                            Ok(sig) => sig,
                            Err(e) => {
                                eprintln!("⚠️ Checkpoint signing failed: {} — skipping", e);
                                continue;
                            }
                        };
                        let checkpoint = FinalityCheckpoint::new(
                            checkpoint_height,
                            checkpoint.block_hash,
                            validator_count.max(1),
                            checkpoint.state_root,
                            vec![CheckpointSignature {
                                validator_address: save_my_address.clone(),
                                signature: my_sig,
                            }],
                        );

                        // DESIGN Store as pending checkpoint, awaiting peer signatures.
                        // For single-validator networks, this will immediately pass quorum (1/1).
                        // For multi-validator: gossip task will broadcast CHECKPOINT_PROPOSE.
                        let pending_cp = PendingCheckpoint::new(checkpoint.clone());
                        if pending_cp.has_quorum() {
                            // Single validator — can finalize immediately
                            match cm.store_checkpoint(checkpoint) {
                                Ok(()) => println!("🏁 Checkpoint finalized at height {} (single-validator, sig_count=1/{}, signed=✓)",
                                    checkpoint_height, validator_count),
                                Err(e) => eprintln!("⚠️ Checkpoint storage failed: {}", e),
                            }
                        } else {
                            // Multi-validator — store as pending, await peer signatures
                            let mut pcp = safe_lock(&save_pending_checkpoints);
                            pcp.insert(checkpoint_height, pending_cp);
                            // Queue CHECKPOINT_PROPOSE for gossip broadcast
                            // Format: CHECKPOINT_PROPOSE:<height>:<block_hash>:<state_root>:<proposer>:<sig_hex>
                            let sig_hex = hex::encode(&checkpoint.signatures[0].signature);
                            let propose_msg = format!(
                                "CHECKPOINT_PROPOSE:{}:{}:{}:{}:{}",
                                checkpoint_height,
                                checkpoint.block_hash,
                                checkpoint.state_root,
                                save_my_address,
                                sig_hex,
                            );
                            let mut outbox = safe_lock(&save_checkpoint_outbox);
                            outbox.push(propose_msg);
                            println!("🏁 Checkpoint proposed at height {} (sig_count=1/{}, awaiting peer sigs)",
                                checkpoint_height, validator_count);
                        }
                    }
                }
            }
        }
    });

    // Periodic cleanup of stale pending transactions
    // Pending sends older than 5 minutes are removed to prevent memory leaks
    let cleanup_pending_sends = Arc::clone(&pending_sends);
    let cleanup_send_voters = Arc::clone(&send_voters);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            const PENDING_TTL_SECS: u64 = 300; // 5 minute TTL for pending transactions

            // Clean stale pending sends AND their corresponding vote trackers.
            // Without cleaning send_voters, entries for timed-out txs leak memory forever.
            if let Ok(mut ps) = cleanup_pending_sends.lock() {
                let before = ps.len();
                // Collect hashes of stale entries BEFORE removing them
                let stale_hashes: Vec<String> = ps
                    .iter()
                    .filter(|(_, (block, _))| {
                        now.saturating_sub(block.timestamp) >= PENDING_TTL_SECS
                    })
                    .map(|(hash, _)| hash.clone())
                    .collect();
                ps.retain(|_, (block, _)| now.saturating_sub(block.timestamp) < PENDING_TTL_SECS);
                let removed = before - ps.len();
                if removed > 0 {
                    // Also clean the vote tracker for these stale transactions
                    if let Ok(mut sv) = cleanup_send_voters.lock() {
                        for hash in &stale_hashes {
                            sv.remove(hash);
                        }
                    }
                    println!(
                        "🧹 Cleaned {} stale pending sends + vote trackers (TTL: {}s)",
                        removed, PENDING_TTL_SECS
                    );
                }
            }
        }
    });

    // Periodic cleanup of stale pending checkpoints
    // Prevents unbounded memory growth if checkpoints never reach quorum
    let gc_pending_cp = Arc::clone(&pending_checkpoints);
    let gc_checkpoint_mgr = Arc::clone(&checkpoint_manager);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(120));
        loop {
            interval.tick().await;
            let latest_finalized = {
                let cm = safe_lock(&gc_checkpoint_mgr);
                cm.latest_finalized_height()
            };
            // Remove pending checkpoints that are far behind the latest finalized height.
            // Anything more than 2× CHECKPOINT_INTERVAL behind is certainly stale.
            let cutoff = latest_finalized.saturating_sub(CHECKPOINT_INTERVAL * 2);
            if cutoff > 0 {
                let mut pcp = safe_lock(&gc_pending_cp);
                let before = pcp.len();
                pcp.retain(|height, _| *height > cutoff);
                let removed = before - pcp.len();
                if removed > 0 {
                    println!(
                        "🧹 GC: Removed {} stale pending checkpoints (cutoff height: {})",
                        removed, cutoff
                    );
                }
            }
        }
    });

    // DESIGN Periodic supply invariant audit (every 5 minutes).
    // Verifies that total_supply == sum(balances) + remaining + slashed + fees + reward_pool.
    // Logs a CRITICAL warning if the invariant breaks (indicates a bug).
    let audit_ledger = Arc::clone(&ledger);
    let audit_reward_pool = Arc::clone(&reward_pool);
    tokio::spawn(async move {
        // Wait 30 seconds after startup for genesis to settle
        tokio::time::sleep(Duration::from_secs(30)).await;
        let mut interval = tokio::time::interval(Duration::from_secs(300));
        loop {
            interval.tick().await;
            let (rp_remaining, rp_distributed) = {
                let rp = safe_lock(&audit_reward_pool);
                (rp.remaining_cil, rp.total_distributed_cil)
            };
            let l = safe_lock(&audit_ledger);
            match l.audit_supply(rp_remaining, rp_distributed) {
                Ok(()) => {
                    // Supply invariant holds — only log at debug level
                }
                Err(msg) => {
                    eprintln!("🚨 CRITICAL: {}", msg);
                    json_event!("supply_audit_failed", "detail" => &msg);
                }
            }
        }
    });

    // =========================================================================
    // AUTOMATIC TOR HIDDEN SERVICE GENERATION (OPTIONAL)
    // =========================================================================
    // Tor is OPTIONAL. Users can choose to run with or without Tor.
    // - If LOS_HOST_ADDRESS is set (IP/domain), the node runs without Tor.
    // - If LOS_ONION_ADDRESS is set (manual .onion), Tor is used.
    // - If neither is set, the node tries to auto-generate a .onion via Tor
    //   control port. If Tor is not available, the node runs without it.
    // The 4 mainnet bootstrap nodes always use .onion (configured in genesis).
    if get_node_host_address().is_none() {
        let p2p_port: u16 = std::env::var("LOS_P2P_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(api_port + 1000);

        let tor_config = tor_service::TorServiceConfig::from_env(
            std::path::Path::new(&base_data_dir),
            api_port,
            p2p_port,
        );

        // Check if control port is reachable before attempting
        if tor_service::is_control_port_available(&tor_config.control_addr).await {
            match tor_service::ensure_hidden_service(&tor_config).await {
                Ok(hs) => {
                    // Set env vars so TorConfig::from_env() picks it up in LosNode::start()
                    // SAFETY: Background tasks above do NOT read these env vars.
                    // Only LosNode::start() (called later) reads them. Wrapped in
                    // unsafe per Rust 1.83+ requirement for multi-threaded set_var.
                    unsafe {
                        std::env::set_var("LOS_ONION_ADDRESS", &hs.onion_address);
                        std::env::set_var("LOS_HOST_ADDRESS", &hs.onion_address);
                    }
                    println!("🧅 Auto-generated Tor hidden service: {}", hs.onion_address);

                    // Register in validator_endpoints for peer discovery
                    if let Ok(mut endpoints) = validator_endpoints.lock() {
                        endpoints.insert(my_address.clone(), hs.onion_address.clone());
                    }
                }
                Err(e) => {
                    eprintln!("⚠️ Tor auto-generation failed: {}", e);
                    eprintln!(
                        "   Node will continue without Tor. \
                         Set LOS_HOST_ADDRESS=<ip:port> or LOS_ONION_ADDRESS=<addr.onion> to configure."
                    );
                }
            }
        } else {
            println!(
                "🌐 Tor control port not available at {} — running without Tor",
                tor_config.control_addr
            );
            println!("   Set LOS_HOST_ADDRESS=<ip:port> to announce your node, or configure Tor for .onion");
        }
    } else {
        let host = get_node_host_address().unwrap_or_default();
        if host.contains(".onion") {
            println!("🧅 Using configured onion address: {}", host);
        } else {
            println!("🌐 Using configured host address: {}", host);
        }
    }

    let (tx_out, rx_out) = mpsc::channel(32);
    let (tx_in, mut rx_in) = mpsc::channel(32);

    tokio::spawn(async move {
        match LosNode::start(tx_in, rx_out).await {
            Ok(()) => eprintln!("⚠️ P2P network task exited normally (unexpected)"),
            Err(e) => eprintln!("❌ P2P network task failed: {}", e),
        }
    });

    // DESIGN Checkpoint gossip outbox drainer.
    // Periodically checks for pending CHECKPOINT_PROPOSE messages and sends them via gossip.
    let cp_outbox_tx = tx_out.clone();
    let cp_outbox_drain = Arc::clone(&checkpoint_outbox);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        loop {
            interval.tick().await;
            let msgs: Vec<String> = {
                let mut outbox = safe_lock(&cp_outbox_drain);
                outbox.drain(..).collect()
            };
            for msg in msgs {
                let _ = cp_outbox_tx.send(msg).await;
            }
        }
    });

    // --- Start HTTP API server ---
    let api_ledger = Arc::clone(&ledger);
    let api_tx = tx_out.clone();
    let api_pending_sends = Arc::clone(&pending_sends);
    let api_address_book = Arc::clone(&address_book);
    let api_addr = my_address.clone();
    let api_key = Zeroizing::new(keys.secret_key.clone());
    let api_metrics = Arc::clone(&metrics);
    let api_database = Arc::clone(&database);

    let api_slashing = Arc::clone(&slashing_manager);
    let api_pk = keys.public_key.clone();
    let api_bootstrap = bootstrap_validators.clone();
    let api_reward_pool = Arc::clone(&reward_pool);
    let api_validator_endpoints = Arc::clone(&validator_endpoints);
    let api_mempool = Arc::clone(&mempool_pool);
    let api_local_validators = Arc::clone(&local_registered_validators);

    // Create aBFT Consensus Engine in main() so it's shared with both API server and event loop
    let abft_consensus = {
        let validator_count = {
            let l = safe_lock(&ledger);
            l.accounts
                .iter()
                .filter(|(_, a)| a.balance >= MIN_VALIDATOR_STAKE_CIL)
                .count()
                .max(4)
        };
        Arc::new(Mutex::new(ABFTConsensus::new(
            my_address.clone(),
            validator_count,
        )))
    };
    let api_abft = Arc::clone(&abft_consensus);

    // --- WASM Smart Contract Engine (shared between API + P2P) ---
    let wasm_engine = Arc::new(WasmEngine::new());
    // Restore contract state from DB (if any contracts were previously deployed)
    match database.load_contracts() {
        Ok(Some(vm_data)) => match wasm_engine.deserialize_all(&vm_data) {
            Ok(count) => println!("✅ Restored {} smart contracts from database", count),
            Err(e) => eprintln!("⚠️ Failed to restore contracts: {}", e),
        },
        Ok(None) => { /* No contracts deployed yet */ }
        Err(e) => eprintln!("⚠️ Failed to load contracts from DB: {}", e),
    }
    let api_wasm_engine = Arc::clone(&wasm_engine);
    let api_mining_state = Arc::clone(&mining_state);

    tokio::spawn(async move {
        start_api_server(ApiServerConfig {
            ledger: api_ledger,
            tx_out: api_tx,
            pending_sends: api_pending_sends,
            address_book: api_address_book,
            my_address: api_addr,
            secret_key: api_key,
            api_port,
            metrics: api_metrics,
            database: api_database,
            slashing_manager: api_slashing,
            node_public_key: api_pk,
            bootstrap_validators: api_bootstrap,
            reward_pool: api_reward_pool,
            validator_endpoints: api_validator_endpoints,
            mempool_pool: api_mempool,
            abft_consensus: api_abft,
            local_registered_validators: api_local_validators,
            wasm_engine: api_wasm_engine,
            mining_state: api_mining_state,
            enable_mining,
            mining_threads,
        })
        .await;
    });

    // --- Start gRPC server ---
    let grpc_ledger = Arc::clone(&ledger);
    let grpc_tx = tx_out.clone();
    let grpc_addr = my_address.clone();
    let grpc_port = api_port + 20000; // Dynamic gRPC port (REST+20000)
    let grpc_ab = Arc::clone(&address_book);
    let grpc_bv = bootstrap_validators.clone();
    let grpc_rest_port = api_port;
    let grpc_reward_pool = Arc::clone(&reward_pool);

    tokio::spawn(async move {
        println!("🔧 Starting gRPC server on port {}...", grpc_port);
        // Flush stdout for pipe-buffered environments (Flutter process monitor)
        {
            use std::io::Write;
            let _ = std::io::stdout().flush();
        }
        if let Err(e) = grpc_server::start_grpc_server(
            grpc_ledger,
            grpc_addr,
            grpc_tx,
            grpc_port,
            grpc_ab,
            grpc_bv,
            grpc_rest_port,
            grpc_reward_pool,
        )
        .await
        {
            eprintln!("❌ gRPC Server error: {}", e);
        }
    });

    // (Oracle price broadcaster removed — prices fetched on-demand)

    // ══════════════════════════════════════════════════════════════════════
    // VALIDATOR REWARD SYSTEM — Heartbeat recording + Epoch distribution
    // ══════════════════════════════════════════════════════════════════════
    // Liveness proof via VALIDATOR_HEARTBEAT gossip messages:
    // 1. Each node broadcasts VALIDATOR_HEARTBEAT:<addr>:<sig>:<ts> every tick
    // 2. Receiving nodes verify the Dilithium5 signature + timestamp freshness
    // 3. Verified heartbeats update live_peers with the sender's last-seen time
    // 4. Only peers in live_peers with recent timestamps get heartbeat credit
    //
    // This works identically on testnet AND mainnet — no shortcuts, no bypasses.
    // If gossipsub delivers messages, validators get heartbeats. If not, they don't.
    // ══════════════════════════════════════════════════════════════════════
    let reward_ledger = Arc::clone(&ledger);
    let reward_pool_bg = Arc::clone(&reward_pool);
    let reward_my_addr = my_address.clone();
    let reward_live_peers = Arc::clone(&live_peers);
    let reward_local_validators = Arc::clone(&local_registered_validators);
    let reward_sk = Zeroizing::new(keys.secret_key.clone());
    let reward_pk = keys.public_key.clone();
    let reward_tx = tx_out.clone(); // For gossiping reward/fee Mint blocks + heartbeat broadcasts
    let reward_ve = Arc::clone(&validator_endpoints); // For HTTP heartbeat fallback
    tokio::spawn(async move {
        // Testnet: shorter heartbeat interval (10s) for 2-minute epochs
        // Mainnet: 60s heartbeat for 30-day epochs
        let heartbeat_secs = if los_core::is_testnet_build() { 10 } else { 60 };
        let mut interval = tokio::time::interval(Duration::from_secs(heartbeat_secs));

        // HTTP heartbeat fallback: when gossipsub is down, directly ping
        // validator .onion endpoints via Tor to verify liveness.
        // Runs every 6 ticks (60s on testnet) to avoid overwhelming Tor.
        let http_check_interval: u64 = 6; // every 6 × heartbeat_secs
        let mut tick_counter: u64 = 0;
        let socks_proxy = std::env::var("LOS_SOCKS5_PROXY").ok();

        loop {
            interval.tick().await;
            tick_counter += 1;

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            // ── BROADCAST: Send our VALIDATOR_HEARTBEAT to the network ──
            // This proves OUR liveness to all other nodes. They will verify
            // the signature and update their live_peers map accordingly.
            // Format: VALIDATOR_HEARTBEAT:<address>:<timestamp>:<pk_hex>:<sig_hex>
            {
                let message = format!("VALIDATOR_HEARTBEAT:{}:{}", reward_my_addr, now);
                if let Ok(sig) = los_crypto::sign_message(message.as_bytes(), &reward_sk) {
                    let sig_hex = hex::encode(&sig);
                    let pk_hex = hex::encode(&reward_pk);
                    let hb_msg = format!(
                        "VALIDATOR_HEARTBEAT:{}:{}:{}:{}",
                        reward_my_addr, now, pk_hex, sig_hex
                    );
                    let _ = reward_tx.send(hb_msg).await;
                }
            }

            // Also broadcast heartbeats for locally-registered validator wallets.
            // The node operator registered a wallet address → our node's liveness
            // proves the wallet's participation. We sign with the NODE key but
            // include the wallet address. Other nodes verify this via the
            // VALIDATOR_HEARTBEAT_PROXY variant.
            {
                let local_addrs: Vec<String> = {
                    let lrv = safe_lock(&reward_local_validators);
                    lrv.iter()
                        .filter(|a| **a != reward_my_addr)
                        .cloned()
                        .collect()
                }; // lrv dropped here before .await
                for addr in &local_addrs {
                    let message = format!(
                        "VALIDATOR_HEARTBEAT_PROXY:{}:{}:{}",
                        addr, reward_my_addr, now
                    );
                    if let Ok(sig) = los_crypto::sign_message(message.as_bytes(), &reward_sk) {
                        let sig_hex = hex::encode(&sig);
                        let proxy_msg = format!(
                            "VALIDATOR_HEARTBEAT_PROXY:{}:{}:{}:{}:{}",
                            addr,
                            reward_my_addr,
                            now,
                            hex::encode(&reward_pk),
                            sig_hex
                        );
                        let _ = reward_tx.send(proxy_msg).await;
                    }
                }
            }

            // All pool + ledger work inside scope blocks so MutexGuards drop before .await
            // Phase A: Heartbeat recording (pool lock → release → HTTP fallback → re-lock)
            let mut seen_this_tick = {
                let mut pool = safe_lock(&reward_pool_bg);

                // ── HEARTBEAT RECORDING (idempotent per tick) ──
                // Uses record_heartbeat_once() so each validator gets exactly
                // 1 heartbeat per tick, regardless of how many sources report them.
                let mut seen_this_tick = std::collections::BTreeSet::<String>::new();

                // 1. Record heartbeat for THIS node (we are running = proven liveness)
                pool.record_heartbeat_once(&reward_my_addr, &mut seen_this_tick);

                // 2. Record heartbeats for wallet addresses registered through THIS node's API.
                //    The node's liveness proves the registered wallet's liveness.
                {
                    let lrv = safe_lock(&reward_local_validators);
                    for addr in lrv.iter() {
                        pool.record_heartbeat_once(addr, &mut seen_this_tick);
                    }
                }

                // 3. Record heartbeats for peers that PROVED liveness by sending
                //    gossipsub messages (VALIDATOR_HEARTBEAT, ID:, BLOCK_CONFIRMED:, etc.)
                //    within the last liveness window.
                //    Only recent entries count — stale peers get ZERO heartbeats.
                {
                    let mut lp = safe_lock(&reward_live_peers);
                    // Liveness window: 2× heartbeat interval to allow for Tor network jitter
                    let liveness_window = (heartbeat_secs * 2) as u64;
                    let cutoff = now.saturating_sub(liveness_window);
                    for (peer_addr, last_seen) in lp.iter() {
                        if *last_seen >= cutoff {
                            pool.record_heartbeat_once(peer_addr, &mut seen_this_tick);
                        }
                    }

                    // GC: Evict stale entries older than 10× heartbeat to prevent memory leak.
                    let stale_cutoff = now.saturating_sub(liveness_window * 5);
                    lp.retain(|_, ts| *ts >= stale_cutoff);
                }

                seen_this_tick
            }; // pool dropped here — safe for .await below

            // 4. HTTP HEARTBEAT FALLBACK: If gossipsub failed to deliver heartbeats
            //    from most validators, directly ping their .onion /health endpoints.
            //    This runs every `http_check_interval` ticks to avoid overwhelming Tor.
            //    It compensates for gossipsub mesh collapse (broken Tor circuits).
            if tick_counter.is_multiple_of(http_check_interval) && socks_proxy.is_some() {
                let proxy_url = socks_proxy.as_deref().unwrap_or("socks5h://127.0.0.1:9050");
                // Collect endpoints for validators NOT yet seen this tick
                let endpoints_to_check: Vec<(String, String)> = {
                    let ve = safe_lock(&reward_ve);
                    let pool = safe_lock(&reward_pool_bg);
                    pool.validators
                        .keys()
                        .filter(|addr| !seen_this_tick.contains(*addr) && **addr != reward_my_addr)
                        .filter_map(|addr| ve.get(addr).map(|onion| (addr.clone(), onion.clone())))
                        .collect()
                };

                if !endpoints_to_check.is_empty() {
                    // Concurrent HTTP checks using JoinSet (with timeout per check)
                    let mut check_set = tokio::task::JoinSet::new();
                    for (addr, onion) in endpoints_to_check {
                        let proxy = proxy_url.to_string();
                        check_set.spawn(async move {
                            let url = format!("http://{}:80/health", onion);
                            let ok = tokio::time::timeout(Duration::from_secs(8), async {
                                let proxy_obj = match reqwest::Proxy::all(&proxy) {
                                    Ok(p) => p,
                                    Err(_) => return false,
                                };
                                let client = match reqwest::Client::builder()
                                    .proxy(proxy_obj)
                                    .timeout(Duration::from_secs(6))
                                    .build()
                                {
                                    Ok(c) => c,
                                    Err(_) => return false,
                                };
                                match client.get(&url).send().await {
                                    Ok(resp) => resp.status().is_success(),
                                    Err(_) => false,
                                }
                            })
                            .await
                            .unwrap_or(false);
                            (addr, ok)
                        });
                    }

                    // Collect results first (no locks held across await)
                    let mut http_results: Vec<(String, bool)> = Vec::new();
                    while let Some(result) = check_set.join_next().await {
                        if let Ok(pair) = result {
                            http_results.push(pair);
                        }
                    }

                    // Now lock and apply results
                    let reachable_count = http_results.iter().filter(|(_, ok)| *ok).count();
                    let total_checked = http_results.len();
                    if reachable_count > 0 {
                        let mut pool = safe_lock(&reward_pool_bg);
                        let mut lp = safe_lock(&reward_live_peers);
                        for (addr, is_alive) in &http_results {
                            if *is_alive {
                                pool.record_heartbeat_once(addr, &mut seen_this_tick);
                                lp.insert(addr.clone(), now);
                            }
                        }
                    }
                    if total_checked > 0 {
                        println!(
                            "📡 HTTP heartbeat fallback: {}/{} validators reachable",
                            reachable_count, total_checked
                        );
                    }
                }
            }

            // Re-acquire pool lock for epoch check
            // Split epoch processing into phases to minimize lock hold time.
            // Holding reward_pool for the full ~280 lines including CPU-intensive PoW + signing
            // blocks ALL HTTP routes that touch ledger or reward_pool for seconds.
            // Phase 1 (pool lock) → Phase 2 (no lock, CPU work) → Phase 3 (ledger lock, write)
            let (gossip_queue, fee_gossip_queue) = {
                // ═══════════════════════════════════════════════════════════════════
                // PHASE 1: Epoch check + reward calculation (pool lock only, fast)
                // ═══════════════════════════════════════════════════════════════════
                let (rewards, is_leader, completed_epoch, fee_data) = {
                    let mut pool = safe_lock(&reward_pool_bg);

                    if !pool.is_epoch_complete(now) {
                        // Not epoch boundary — nothing to do
                        (Vec::new(), false, 0u64, None)
                    } else {
                        // DETERMINISTIC LEADER ELECTION
                        // Previously used volatile HTTP heartbeat data (reward_live_peers)
                        // which differs per node on Tor → ALL nodes thought they were leader
                        // → conflicting reward blocks → chain divergence → blacklisting.
                        //
                        // Deterministic round-robin over the sorted registered
                        // validator list. All nodes share the same pool.validators (registered
                        // at genesis), so they ALL agree on who the leader is.
                        // If the elected leader is offline, rewards for that epoch are simply
                        // not distributed — the pool retains the budget for future epochs.
                        let is_leader = {
                            let mut registered: Vec<&String> = pool.validators.keys().collect();
                            registered.sort();
                            if registered.is_empty() {
                                false
                            } else {
                                let leader_idx = (pool.current_epoch as usize) % registered.len();
                                let leader_addr = registered[leader_idx].as_str();
                                let am_leader = leader_addr == reward_my_addr.as_str();
                                if am_leader {
                                    println!(
                                        "👑 Epoch {} leader election: I am the leader (slot {}/{})",
                                        pool.current_epoch,
                                        leader_idx,
                                        registered.len()
                                    );
                                } else {
                                    println!(
                                        "⏳ Epoch {} leader election: leader is {} (slot {}/{})",
                                        pool.current_epoch,
                                        &leader_addr[..leader_addr.len().min(15)],
                                        leader_idx,
                                        registered.len()
                                    );
                                }
                                am_leader
                            }
                        };

                        pool.set_expected_heartbeats(heartbeat_secs);

                        // Only the leader distributes rewards.
                        // Non-leaders just advance the epoch (reset heartbeats, increment counter)
                        // without deducting from the pool. They will receive the leader's
                        // reward blocks via gossip/sync and process them normally through
                        // process_block() which credits the recipient's account.
                        //
                        // Previously ALL nodes called distribute_epoch_rewards() independently,
                        // causing each to deduct different amounts (based on local heartbeat data)
                        // and create conflicting reward blocks → chain divergence → blacklisting.
                        let (rewards, completed_epoch, fee_data) = if is_leader {
                            // Refresh stake weights (brief ledger lock)
                            {
                                let l = safe_lock(&reward_ledger);
                                let addrs: Vec<String> = pool.validators.keys().cloned().collect();
                                for addr in &addrs {
                                    if let Some(acct) = l.accounts.get(addr) {
                                        pool.update_stake(addr, acct.balance);
                                    }
                                }
                            } // ledger released

                            let rewards = pool.distribute_epoch_rewards();
                            pool.set_expected_heartbeats(heartbeat_secs);
                            let completed_epoch = pool.current_epoch.saturating_sub(1);

                            // Collect fee distribution data
                            let fee_data = {
                                let l = safe_lock(&reward_ledger);
                                let fees = l.accumulated_fees_cil;
                                if fees > 0 {
                                    let eligible: Vec<(String, u128)> = l
                                        .accounts
                                        .iter()
                                        .filter(|(_, s)| {
                                            s.is_validator && s.balance >= MIN_VALIDATOR_STAKE_CIL
                                        })
                                        .map(|(addr, s)| {
                                            let weight = calculate_voting_power(s.balance);
                                            (addr.clone(), weight)
                                        })
                                        .collect();
                                    let total_weight: u128 = eligible.iter().map(|(_, w)| *w).sum();
                                    if total_weight > 0 && !eligible.is_empty() {
                                        Some((fees, eligible, total_weight))
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            };

                            if rewards.is_empty() {
                                println!(
                                    "🏆 Epoch {} complete: no eligible validators for rewards",
                                    completed_epoch
                                );
                            }

                            (rewards, completed_epoch, fee_data)
                        } else {
                            // NON-LEADER: Just advance epoch, don't distribute
                            let completed_epoch = pool.current_epoch;
                            pool.advance_epoch_only();
                            pool.set_expected_heartbeats(heartbeat_secs);
                            println!(
                                "🏆 Epoch {} complete: not leader, waiting for reward gossip",
                                completed_epoch
                            );
                            (Vec::new(), completed_epoch, None)
                        };

                        (rewards, is_leader, completed_epoch, fee_data)
                    }
                }; // pool lock RELEASED here — all HTTP routes unblocked

                let mut gossip_queue: Vec<String> = Vec::new();
                let mut fee_gossip_queue: Vec<String> = Vec::new();

                if !rewards.is_empty() && is_leader {
                    println!("👑 This node is the epoch leader — creating reward blocks");

                    // ═══════════════════════════════════════════════════════════
                    // PHASE 2a: Collect account states (brief ledger lock)
                    // ═══════════════════════════════════════════════════════════
                    let now_ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();

                    let mut block_templates: Vec<(String, u128, Block)> = Vec::new();
                    {
                        let l = safe_lock(&reward_ledger);
                        for (addr, reward_cil) in &rewards {
                            if l.distribution.remaining_supply < *reward_cil {
                                eprintln!(
                                    "⚠️ Reward skipped for {}: insufficient remaining supply",
                                    get_short_addr(addr)
                                );
                                continue;
                            }
                            let state = l.accounts.get(addr).cloned().unwrap_or(AccountState {
                                head: "0".to_string(),
                                balance: 0,
                                block_count: 0,
                                is_validator: false,
                            });
                            block_templates.push((
                                addr.clone(),
                                *reward_cil,
                                Block {
                                    block_type: BlockType::Mint,
                                    account: addr.clone(),
                                    previous: state.head.clone(),
                                    link: format!("REWARD:EPOCH:{}", completed_epoch),
                                    amount: *reward_cil,
                                    fee: 0,
                                    timestamp: now_ts,
                                    public_key: hex::encode(&reward_pk),
                                    signature: String::new(),
                                    work: 0,
                                },
                            ));
                        }
                    } // ledger released — HTTP routes unblocked during CPU work

                    // ═══════════════════════════════════════════════════════════
                    // PHASE 2b: PoW + Signing (NO LOCKS HELD — CPU intensive)
                    // ═══════════════════════════════════════════════════════════
                    let mut signed_blocks: Vec<(String, u128, Block)> = Vec::new();
                    for (addr, reward_cil, mut blk) in block_templates {
                        compute_pow_inline(&mut blk, 0);
                        let signing_hash = blk.signing_hash();
                        blk.signature = match try_sign_hex(signing_hash.as_bytes(), &reward_sk) {
                            Ok(sig) => sig,
                            Err(e) => {
                                eprintln!(
                                    "❌ Failed to sign reward block for {}: {}",
                                    get_short_addr(&addr),
                                    e
                                );
                                continue;
                            }
                        };
                        signed_blocks.push((addr, reward_cil, blk));
                    }

                    // ═══════════════════════════════════════════════════════════
                    // PHASE 3a: Process signed reward blocks (ledger lock, fast)
                    // ═══════════════════════════════════════════════════════════
                    {
                        let mut l = safe_lock(&reward_ledger);
                        let mut total_credited: u128 = 0;
                        for (addr, reward_cil, reward_blk) in &signed_blocks {
                            // Re-check previous hash in case ledger changed during signing
                            // (another block may have been processed for this account).
                            // If the previous hash is stale, skip — next epoch will retry.
                            if let Some(acct) = l.accounts.get(addr) {
                                if acct.head != reward_blk.previous {
                                    eprintln!(
                                        "⚠️ Reward block stale for {} (head changed) — will retry next epoch",
                                        get_short_addr(addr)
                                    );
                                    continue;
                                }
                            }
                            match l.process_block(reward_blk) {
                                Ok(result) => {
                                    let hash = result.into_hash();
                                    total_credited += reward_cil;
                                    gossip_queue.push(
                                        serde_json::to_string(reward_blk).unwrap_or_default(),
                                    );
                                    println!(
                                        "💰 Reward Mint: {} → {} LOS (block: {})",
                                        get_short_addr(addr),
                                        reward_cil / CIL_PER_LOS,
                                        &hash[..12]
                                    );
                                }
                                Err(e) => {
                                    eprintln!(
                                        "❌ Reward block failed for {}: {}",
                                        get_short_addr(addr),
                                        e
                                    );
                                }
                            }
                        }
                        if total_credited > 0 {
                            SAVE_DIRTY.store(true, Ordering::Release);
                            println!(
                                "🏆 Epoch {} rewards: {} LOS distributed to {} validators",
                                completed_epoch,
                                total_credited / CIL_PER_LOS,
                                rewards.len()
                            );
                        }
                    } // ledger released
                }

                // ═══════════════════════════════════════════════════════════════════
                // FEE DISTRIBUTION (same phase split: collect → sign → write)
                // ═══════════════════════════════════════════════════════════════════
                if is_leader {
                    if let Some((fees_to_distribute, eligible, total_weight)) = fee_data {
                        let now_ts = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();

                        // Phase A: Collect account states (brief lock)
                        let mut fee_templates: Vec<(String, u128, Block)> = Vec::new();
                        {
                            let l = safe_lock(&reward_ledger);
                            for (addr, weight) in &eligible {
                                let fee_share =
                                    fees_to_distribute.checked_mul(*weight).unwrap_or(0)
                                        / total_weight;
                                if fee_share == 0 {
                                    continue;
                                }
                                let state = l.accounts.get(addr).cloned().unwrap_or(AccountState {
                                    head: "0".to_string(),
                                    balance: 0,
                                    block_count: 0,
                                    is_validator: false,
                                });
                                fee_templates.push((
                                    addr.clone(),
                                    fee_share,
                                    Block {
                                        block_type: BlockType::Mint,
                                        account: addr.clone(),
                                        previous: state.head.clone(),
                                        link: format!("FEE_REWARD:EPOCH:{}", completed_epoch),
                                        amount: fee_share,
                                        fee: 0,
                                        timestamp: now_ts,
                                        public_key: hex::encode(&reward_pk),
                                        signature: String::new(),
                                        work: 0,
                                    },
                                ));
                            }
                        } // ledger released

                        // Phase B: PoW + Sign (NO LOCKS)
                        let mut signed_fee_blocks: Vec<(String, u128, Block)> = Vec::new();
                        for (addr, fee_share, mut blk) in fee_templates {
                            compute_pow_inline(&mut blk, 0);
                            let signing_hash = blk.signing_hash();
                            blk.signature = match try_sign_hex(signing_hash.as_bytes(), &reward_sk)
                            {
                                Ok(sig) => sig,
                                Err(e) => {
                                    eprintln!(
                                        "❌ Fee reward sign failed for {}: {}",
                                        get_short_addr(&addr),
                                        e
                                    );
                                    continue;
                                }
                            };
                            signed_fee_blocks.push((addr, fee_share, blk));
                        }

                        // Phase C: Process signed fee blocks (brief lock)
                        {
                            let mut l = safe_lock(&reward_ledger);
                            let mut total_fee_credited: u128 = 0;
                            for (addr, fee_share, fee_blk) in &signed_fee_blocks {
                                // Re-check previous hash for staleness
                                if let Some(acct) = l.accounts.get(addr) {
                                    if acct.head != fee_blk.previous {
                                        eprintln!(
                                            "⚠️ Fee block stale for {} — will retry next epoch",
                                            get_short_addr(addr)
                                        );
                                        continue;
                                    }
                                }
                                // FEE_REWARD supply handling is now in process_block() itself.
                                // No need for save/restore — process_block skips remaining_supply
                                // deduction for FEE_REWARD: Mint blocks automatically.
                                match l.process_block(fee_blk) {
                                    Ok(result) => {
                                        let hash = result.into_hash();
                                        total_fee_credited += fee_share;
                                        fee_gossip_queue.push(
                                            serde_json::to_string(fee_blk).unwrap_or_default(),
                                        );
                                        println!(
                                            "💸 Fee Reward: {} → {} CIL (block: {})",
                                            get_short_addr(addr),
                                            fee_share,
                                            &hash[..12]
                                        );
                                    }
                                    Err(e) => {
                                        eprintln!(
                                            "❌ Fee reward block failed for {}: {}",
                                            get_short_addr(addr),
                                            e
                                        );
                                    }
                                }
                            }
                            if total_fee_credited > 0 {
                                l.accumulated_fees_cil =
                                    l.accumulated_fees_cil.saturating_sub(total_fee_credited);
                                SAVE_DIRTY.store(true, Ordering::Release);
                                println!(
                                    "💸 Epoch {} fee distribution: {} CIL ({} LOS) to {} validators",
                                    completed_epoch,
                                    total_fee_credited,
                                    total_fee_credited / CIL_PER_LOS,
                                    eligible.len()
                                );
                            }
                        } // ledger released
                    }
                }

                (gossip_queue, fee_gossip_queue)
            };

            // Send all queued gossip messages (reward + fee blocks) after all locks released
            for msg in &gossip_queue {
                let _ = reward_tx.send(msg.clone()).await;
            }
            for msg in &fee_gossip_queue {
                let _ = reward_tx.send(msg.clone()).await;
            }
        }
    });

    // Bootstrapping
    let tx_boot = tx_out.clone();
    let my_addr_boot = my_address.clone();
    let ledger_boot = Arc::clone(&ledger);

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(3)).await; // Wait for P2P to initialize
        let bootstrap_list = get_bootstrap_nodes();
        if bootstrap_list.is_empty() {
            println!(
                "📡 No bootstrap nodes found (checked LOS_BOOTSTRAP_NODES env and genesis config)"
            );
            println!(
                "   To connect manually: export LOS_BOOTSTRAP_NODES=peer1.onion:4030,peer2.onion:4031"
            );
        }
        for addr in &bootstrap_list {
            let _ = tx_boot.send(format!("DIAL:{}", addr)).await;
            tokio::time::sleep(Duration::from_secs(2)).await;
            let s = {
                let l = safe_lock(&ledger_boot);
                l.distribution.remaining_supply
            };
            // Include timestamp nonce to prevent GossipSub message deduplication.
            // GossipSub deduplicates by hashing message.data — identical content
            // gets suppressed. Adding epoch_ms ensures each broadcast is unique.
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let _ = tx_boot
                .send(format!("ID:{}:{}:{}", my_addr_boot, s, ts))
                .await;
        }

        // Wait extra time for GossipSub mesh to form before second broadcast
        tokio::time::sleep(Duration::from_secs(5)).await;
        {
            let s = {
                let l = safe_lock(&ledger_boot);
                l.distribution.remaining_supply
            };
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let _ = tx_boot
                .send(format!("ID:{}:{}:{}", my_addr_boot, s, ts))
                .await;
        }

        // After bootstrapping, request state sync from peers (pull-based)
        if !bootstrap_list.is_empty() {
            tokio::time::sleep(Duration::from_secs(3)).await;
            let block_count = safe_lock(&ledger_boot).blocks.len();
            let _ = tx_boot
                .send(format!("SYNC_REQUEST:{}:{}", my_addr_boot, block_count))
                .await;
            println!(
                "📡 Requesting state sync from peers (local blocks: {})",
                block_count
            );
        }

        // Periodic ID re-announce (every 15s) so late-joining peers discover us.
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        let mut sync_counter: u64 = 0;
        loop {
            interval.tick().await;
            sync_counter += 1;
            let (s, block_count) = {
                let l = safe_lock(&ledger_boot);
                (l.distribution.remaining_supply, l.blocks.len())
            };
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let _ = tx_boot
                .send(format!("ID:{}:{}:{}", my_addr_boot, s, ts))
                .await;

            // Periodic state sync request every 30s (2 × 15s ticks)
            // to catch any gossip messages dropped by GossipSub. Without this,
            // a node that misses a gossip block will have a permanently stale
            // view of that account's chain. The sync mechanism fills in gaps
            // by comparing full state with peers.
            if sync_counter.is_multiple_of(2) {
                let _ = tx_boot
                    .send(format!("SYNC_REQUEST:{}:{}", my_addr_boot, block_count))
                    .await;
            }
        }
    });

    // ══════════════════════════════════════════════════════════════════════
    // PEX: Peer Exchange — Periodically broadcast known validator endpoints
    // ══════════════════════════════════════════════════════════════════════
    // Every 5 minutes, broadcast our known validator endpoints to all
    // connected peers via gossipsub. This enables network-wide discovery of
    // validator endpoints beyond the hardcoded bootstrap list.
    // Endpoints can be .onion, IP, or domain — Tor is optional.
    let pex_tx = tx_out.clone();
    let pex_ve = Arc::clone(&validator_endpoints);
    tokio::spawn(async move {
        // Wait for initial bootstrapping to complete
        tokio::time::sleep(Duration::from_secs(30)).await;
        let pex_interval_secs = if los_core::is_testnet_build() {
            60
        } else {
            300
        };
        let mut interval = tokio::time::interval(Duration::from_secs(pex_interval_secs));
        loop {
            interval.tick().await;
            let endpoints: Vec<serde_json::Value> = {
                let ve = safe_lock(&pex_ve);
                ve.iter()
                    .map(|(addr, host)| {
                        serde_json::json!({
                            "address": addr,
                            "host_address": host,
                            "onion_address": host, // backward compat for older nodes
                        })
                    })
                    .collect()
            };
            if !endpoints.is_empty() {
                let msg = serde_json::json!({
                    "endpoints": endpoints,
                    "timestamp": std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                });
                let _ = pex_tx.send(format!("PEER_LIST:{}", msg)).await;
            }
        }
    });

    // ══════════════════════════════════════════════════════════════════════
    // BACKGROUND REST SYNC — Stale state detector & auto-recovery
    // ══════════════════════════════════════════════════════════════════════
    // Every 2 minutes, checks if block count has been stale (unchanged)
    // for 4+ minutes. If so, iterates known peer endpoints and attempts
    // REST-based state sync via GET /sync/full. This is the ultimate
    // fallback when gossip is broken (Tor circuit collapse, GossipSub
    // mesh disintegration, etc.) and SYNC_GZIP exceeds 8MB.
    //
    // This ensures a node can ALWAYS recover, even if:
    //   - Gossip is completely dead
    //   - State is too large for gossip (>8MB compressed)
    //   - Node was offline for an extended period
    {
        let rest_sync_ledger = Arc::clone(&ledger);
        let rest_sync_ve = Arc::clone(&validator_endpoints);
        let rest_sync_rp = Arc::clone(&reward_pool);
        let rest_sync_sm = Arc::clone(&slashing_manager);
        let rest_sync_db = Arc::clone(&database);
        let rest_sync_my_addr = my_address.clone();
        let rest_sync_api_port = api_port;

        tokio::spawn(async move {
            // Wait for initial bootstrap and gossip sync to settle
            tokio::time::sleep(Duration::from_secs(120)).await;

            let mut interval = tokio::time::interval(Duration::from_secs(120));
            let mut last_block_count: usize = 0;
            let mut stale_ticks: u32 = 0; // Each tick = 2 minutes
            let mut last_rest_sync_secs: u64 = 0;

            loop {
                interval.tick().await;

                let current_blocks = safe_lock(&rest_sync_ledger).blocks.len();

                if current_blocks == last_block_count {
                    stale_ticks += 1;
                } else {
                    stale_ticks = 0;
                    last_block_count = current_blocks;
                }

                // Only trigger REST sync if stale for 4+ minutes (2 ticks) and
                // no REST sync in the last 60 seconds
                if stale_ticks < 2 {
                    continue;
                }

                let now_secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                if now_secs.saturating_sub(last_rest_sync_secs) < 60 {
                    continue; // Rate limit REST sync attempts
                }

                println!("⚠️ Block count stale for {}+ minutes ({} blocks). Attempting REST sync from peers...",
                    stale_ticks * 2, current_blocks);

                // Collect peer endpoints (excluding self)
                let peers: Vec<String> = {
                    let ve = safe_lock(&rest_sync_ve);
                    ve.iter()
                        .filter(|(addr, _)| **addr != rest_sync_my_addr)
                        .map(|(_, host)| {
                            // Ensure host has the REST port
                            ensure_host_port(host, rest_sync_api_port)
                        })
                        .collect()
                };

                if peers.is_empty() {
                    println!("⚠️ No peer endpoints known for REST sync");
                    continue;
                }

                last_rest_sync_secs = now_secs;

                // Try each peer until one succeeds
                let mut synced = false;
                for peer_host in &peers {
                    match rest_sync_from_peer(
                        peer_host,
                        current_blocks,
                        &rest_sync_ledger,
                        &rest_sync_rp,
                        &rest_sync_sm,
                        &rest_sync_db,
                    )
                    .await
                    {
                        Ok(added) if added > 0 => {
                            println!(
                                "✅ REST sync from {} complete: {} new blocks merged",
                                peer_host, added
                            );
                            stale_ticks = 0;
                            last_block_count = safe_lock(&rest_sync_ledger).blocks.len();
                            synced = true;
                            break;
                        }
                        Ok(_) => {
                            // 0 blocks added — peer doesn't have more than us either
                            continue;
                        }
                        Err(e) => {
                            println!("⚠️ REST sync from {} failed: {}", peer_host, e);
                            continue;
                        }
                    }
                }

                if !synced {
                    println!("⚠️ REST sync: no peer had more blocks than us ({}). Will retry in 2 minutes.", current_blocks);
                }
            }
        });
    }

    // ══════════════════════════════════════════════════════════════════════
    // TOR HIDDEN SERVICE HEALTH MONITOR
    // ══════════════════════════════════════════════════════════════════════
    // Periodically self-pings this node's own .onion address via the Tor
    // SOCKS5 proxy to verify the hidden service is reachable from the
    // outside. If unreachable, logs warnings and updates Prometheus metrics.
    //
    // Interval: 2 minutes (testnet: 60s). After 3 consecutive failures,
    // the node logs a CRITICAL warning advising Tor restart.
    //
    // NOTE on testnet (shared Tor daemon, same machine):
    //   Self-ping through SOCKS5 → Tor circuit → own hidden service
    //   still validates the hidden service descriptor is published and
    //   the Tor circuit can reach it. This is NOT a localhost loopback.
    let tor_health_metrics = Arc::clone(&metrics);
    if let Ok(my_onion) = std::env::var("LOS_ONION_ADDRESS") {
        if !my_onion.is_empty() {
            let tor_socks = std::env::var("LOS_SOCKS5_PROXY")
                .or_else(|_| std::env::var("LOS_TOR_SOCKS5"))
                .unwrap_or_else(|_| "socks5h://127.0.0.1:9050".to_string())
                .trim_start_matches("socks5h://")
                .trim_start_matches("socks5://")
                .to_string();
            tokio::spawn(async move {
                // Wait for node to fully start before first self-ping
                tokio::time::sleep(Duration::from_secs(60)).await;

                let check_interval_secs: u64 = if los_core::is_testnet_build() {
                    60
                } else {
                    120
                };
                let mut interval = tokio::time::interval(Duration::from_secs(check_interval_secs));
                let mut consecutive_failures: u32 = 0;

                // Build reqwest client with SOCKS5 proxy for Tor
                let proxy = match reqwest::Proxy::all(format!("socks5h://{}", tor_socks)) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!(
                            "🧅❌ Tor Health Monitor: failed to create SOCKS5 proxy: {}",
                            e
                        );
                        return;
                    }
                };
                let client = match reqwest::Client::builder()
                    .proxy(proxy)
                    .timeout(Duration::from_secs(30))
                    .build()
                {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!(
                            "🧅❌ Tor Health Monitor: failed to build HTTP client: {}",
                            e
                        );
                        return;
                    }
                };

                // Determine the health check URL.
                // Tor hidden services expose the API port (e.g. 3030) — not port 80.
                // The torrc maps HiddenServicePort <api_port> → 127.0.0.1:<api_port>.
                let health_url = format!(
                    "http://{}:{}/health",
                    my_onion.trim_end_matches('/'),
                    api_port
                );

                println!(
                    "🧅🏥 Tor Health Monitor started (checking {} every {}s)",
                    health_url, check_interval_secs
                );

                loop {
                    interval.tick().await;
                    tor_health_metrics.tor_self_ping_total.inc();

                    match client.get(&health_url).send().await {
                        Ok(resp) if resp.status().is_success() => {
                            if consecutive_failures > 0 {
                                println!("🧅✅ Tor Hidden Service RECOVERED after {} consecutive failures",
                                    consecutive_failures);
                            }
                            consecutive_failures = 0;
                            tor_health_metrics.tor_onion_reachable.set(1);
                            tor_health_metrics.tor_consecutive_failures.set(0);
                        }
                        Ok(resp) => {
                            // HTTP response but non-success status — service reachable but unhealthy
                            consecutive_failures += 1;
                            tor_health_metrics.tor_onion_reachable.set(0);
                            tor_health_metrics
                                .tor_consecutive_failures
                                .set(consecutive_failures as i64);
                            tor_health_metrics.tor_self_ping_failures_total.inc();
                            eprintln!(
                                "🧅⚠️ Tor self-ping: HTTP {} (failure #{}) — {}",
                                resp.status(),
                                consecutive_failures,
                                health_url
                            );
                        }
                        Err(e) => {
                            consecutive_failures += 1;
                            tor_health_metrics.tor_onion_reachable.set(0);
                            tor_health_metrics
                                .tor_consecutive_failures
                                .set(consecutive_failures as i64);
                            tor_health_metrics.tor_self_ping_failures_total.inc();

                            if consecutive_failures >= 3 {
                                eprintln!(
                                    "🧅🚨 CRITICAL: Tor Hidden Service UNREACHABLE for {} consecutive checks! \
                                     Error: {}. \
                                     ACTION REQUIRED: Restart Tor daemon or check hidden service config. \
                                     Command: sudo systemctl restart tor",
                                    consecutive_failures, e
                                );
                            } else {
                                eprintln!(
                                    "🧅⚠️ Tor self-ping failed (attempt #{}/3): {} — {}",
                                    consecutive_failures, e, health_url
                                );
                            }
                        }
                    }
                }
            });
        }
    } else {
        // No onion address configured — set metric to -1 (not applicable)
        tor_health_metrics.tor_onion_reachable.set(-1);
        println!("🧅 Tor Health Monitor: skipped (no LOS_ONION_ADDRESS configured)");
    }

    println!("\n==================================================================");
    println!("                 UNAUTHORITY (LOS) ORACLE NODE                   ");
    println!("==================================================================");
    println!("🆔 MY ID        : {}", my_short);
    // Show .onion address if available, otherwise show bind address
    let onion_addr = std::env::var("LOS_ONION_ADDRESS").ok();
    if let Some(ref onion) = onion_addr {
        println!("🧅 REST API     : http://{}", onion);
    } else {
        println!("📡 REST API     : http://127.0.0.1:{}", api_port);
    }
    println!(
        "🔌 gRPC API     : 127.0.0.1:{} (8 services)",
        api_port + 20000
    );
    println!("------------------------------------------------------------------");
    println!("📖 COMMANDS:");
    println!("   bal                   - Check balance");
    println!("   whoami                - Check full address");
    println!("   history               - View transaction history (NEW!)");
    println!("   send <ID> <AMT>       - Send coins");
    println!("   supply                - Check total supply");
    println!("   peers                 - List active nodes");
    println!("   dial <addr>           - Manual connection");
    println!("   exit                  - Exit application");
    println!("------------------------------------------------------------------");

    // Flush banner output before emitting the critical node_ready event
    {
        use std::io::Write;
        let _ = std::io::stdout().flush();
    }

    // Emit structured event for Flutter process monitor
    json_event!("node_ready",
        "address" => &my_address,
        "port" => api_port,
        "onion" => onion_addr.as_deref().unwrap_or("none")
    );

    // ── DELAYED STARTUP VALIDATOR BROADCAST ─────────────────────────────
    // If the node was auto-registered at startup (restart with existing balance),
    // we need to broadcast VALIDATOR_REG to the network AFTER peers connect,
    // AND register in SlashingManager / RewardPool / aBFT / endpoints.
    // The startup auto-register only set is_validator=true in ledger (before
    // Arc wrap), so here we complete the full registration with gossip broadcast.
    // Wait 10 seconds for peer connections to establish.
    // Broadcast VALIDATOR_REG on startup for ANY non-bootstrap mining validator.
    // This covers both freshly auto-registered nodes AND restart scenarios where
    // is_validator was already true from checkpoint but peers don't know our onion.
    let should_startup_broadcast = startup_auto_registered
        || (enable_mining
            && !bootstrap_validators.contains(&my_address)
            && safe_lock(&ledger)
                .accounts
                .get(&my_address)
                .map(|a| a.is_validator)
                .unwrap_or(false));
    if should_startup_broadcast {
        let sr_ledger = Arc::clone(&ledger);
        let sr_sm = Arc::clone(&slashing_manager);
        let sr_rp = Arc::clone(&reward_pool);
        let sr_abft = Arc::clone(&abft_consensus);
        let sr_ve = Arc::clone(&validator_endpoints);
        let sr_lrv = Arc::clone(&local_registered_validators);
        let sr_addr = my_address.clone();
        let sr_sk = Zeroizing::new(keys.secret_key.clone());
        let sr_pk = keys.public_key.clone();
        let sr_tx = tx_out.clone();
        let sr_api_port = api_port;
        tokio::spawn(async move {
            println!("🔧 Delayed startup broadcast: waiting 15s for peer connections...");
            // Wait for peer connections to establish (Tor can be slow)
            tokio::time::sleep(Duration::from_secs(15)).await;
            println!("🔧 Delayed startup broadcast: timer elapsed, starting registration...");

            // 1. Register in SlashingManager
            {
                let mut sm = safe_lock(&sr_sm);
                if sm.get_profile(&sr_addr).is_none() {
                    sm.register_validator(sr_addr.clone());
                    println!("🔧 [1/6] Registered in SlashingManager");
                }
            }
            // 2. Register in RewardPool (non-genesis)
            {
                let balance = safe_lock(&sr_ledger)
                    .accounts
                    .get(&sr_addr)
                    .map(|a| a.balance)
                    .unwrap_or(0);
                let mut rp = safe_lock(&sr_rp);
                rp.register_validator(&sr_addr, false, balance);
                println!(
                    "🔧 [2/6] Registered in RewardPool (balance: {} CIL)",
                    balance
                );
            }
            // 3. Track as locally-registered validator for heartbeats
            {
                let mut lrv = safe_lock(&sr_lrv);
                lrv.insert(sr_addr.clone());
                println!("🔧 [3/6] Tracked in local_registered_validators");
            }
            // 4. Update aBFT validator set
            {
                let l = safe_lock(&sr_ledger);
                let mut validators: Vec<String> = l
                    .accounts
                    .iter()
                    .filter(|(_, a)| a.balance >= MIN_VALIDATOR_REGISTER_CIL && a.is_validator)
                    .map(|(addr, _)| addr.clone())
                    .collect();
                validators.sort();
                let count = validators.len();
                safe_lock(&sr_abft).update_validator_set(validators);
                println!("🔧 [4/6] Updated aBFT validator set ({} validators)", count);
            }
            // 5. Store our onion/host address in validator endpoints
            let host_addr = get_node_host_address().map(|h| ensure_host_port(&h, sr_api_port));
            if let Some(ref host) = host_addr {
                insert_validator_endpoint(&mut safe_lock(&sr_ve), sr_addr.clone(), host.clone());
                println!("🔧 [5/6] Stored validator endpoint: {}", host);
            } else {
                println!("⚠️ [5/6] No host address available for validator endpoint");
            }
            // 6. Broadcast VALIDATOR_REG gossip to peers
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let reg_message = format!("REGISTER_VALIDATOR:{}:{}", sr_addr, ts);
            match los_crypto::sign_message(reg_message.as_bytes(), &sr_sk) {
                Ok(sig) => {
                    let reg_msg = serde_json::json!({
                        "type": "VALIDATOR_REG",
                        "address": sr_addr,
                        "public_key": hex::encode(&sr_pk),
                        "signature": hex::encode(&sig),
                        "timestamp": ts,
                        "host_address": host_addr,
                        "onion_address": host_addr,
                        "rest_port": sr_api_port,
                    });
                    let _ = sr_tx.send(format!("VALIDATOR_REG:{}", reg_msg)).await;
                    println!("🔧 [6/6] VALIDATOR_REG gossip sent");
                }
                Err(e) => {
                    eprintln!("❌ [6/6] Failed to sign VALIDATOR_REG: {:?}", e);
                }
            }
            SAVE_DIRTY.store(true, Ordering::Release);
            println!(
                "📡 Startup validator broadcast complete: {} (host: {})",
                get_short_addr(&sr_addr),
                host_addr.as_deref().unwrap_or("none")
            );
        });
    }

    let mut stdin = BufReader::new(io::stdin()).lines();
    let mut stdin_closed = false; // Track EOF — prevents tokio::select! panic in headless mode

    // ── GRACEFUL SHUTDOWN SIGNAL HANDLER ────────────────────────────────
    // CRITICAL: Without this, SIGTERM triggers sled::Drop during process teardown,
    // which may hang in kernel I/O (flock flush) → process enters macOS UE state
    // (Uninterruptible Exit) → unkillable zombie holding the sled flock forever →
    // next los-node also blocks on flock → cascading UE zombie chain.
    //
    // Solution: Intercept SIGTERM/SIGINT BEFORE the tokio runtime shuts down,
    // explicitly flush the sled DB (non-blocking: just marks pages for write-back),
    // remove the PID lockfile, then exit immediately via std::process::exit(0)
    // which skips Drop destructors entirely — preventing the UE hang.
    {
        let db_for_signal = Arc::clone(&database);
        let data_dir_for_signal = base_data_dir.clone();
        let json_log_signal = json_log;
        tokio::spawn(async move {
            // Helper: perform graceful shutdown
            let do_shutdown = |reason: &str| {
                eprintln!("\n🛑 {} received — shutting down gracefully...", reason);
                if json_log_signal {
                    println!("{{\"event\":\"shutdown\",\"reason\":\"{}\"}}", reason);
                    use std::io::Write;
                    let _ = std::io::stdout().flush();
                }
                // Flush sled DB (schedules write-back, non-blocking)
                if let Err(e) = db_for_signal.flush() {
                    eprintln!("⚠️ DB flush error: {}", e);
                }
                // Remove PID lockfile so next startup knows we exited cleanly
                let pid_path = format!("{}/.los-node.pid", data_dir_for_signal);
                let _ = std::fs::remove_file(&pid_path);
                eprintln!("✅ Clean shutdown complete");
                // exit(0) skips Drop destructors — avoids sled::Drop hanging in flock
                std::process::exit(0);
            };

            #[cfg(unix)]
            {
                use tokio::signal::unix::{signal, SignalKind};
                let mut sigterm = match signal(SignalKind::terminate()) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("❌ Fatal: cannot install SIGTERM handler: {e}. Aborting.");
                        std::process::exit(1);
                    }
                };
                let mut sigint = match signal(SignalKind::interrupt()) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("❌ Fatal: cannot install SIGINT handler: {e}. Aborting.");
                        std::process::exit(1);
                    }
                };

                tokio::select! {
                    _ = sigterm.recv() => do_shutdown("SIGTERM"),
                    _ = sigint.recv() => do_shutdown("SIGINT"),
                }
            }

            #[cfg(not(unix))]
            {
                let _ = tokio::signal::ctrl_c().await;
                do_shutdown("Ctrl+C");
            }
        });
    }

    // Clone database, metrics, and slashing_manager for event loop
    let db_clone = Arc::clone(&database);
    let _metrics_clone = Arc::clone(&metrics);
    let slashing_clone = Arc::clone(&slashing_manager);
    let send_voters_clone = Arc::clone(&send_voters);
    let ve_event = Arc::clone(&validator_endpoints);
    let abft_event = Arc::clone(&abft_consensus);
    let live_peers = Arc::clone(&live_peers); // Shadow for event loop usage
    let rp_sync = Arc::clone(&reward_pool); // For syncing reward pool on incoming REWARD Mint blocks

    loop {
        tokio::select! {
            result = stdin.next_line(), if !stdin_closed => {
                match result {
                    Ok(Some(line)) => {
                let p: Vec<&str> = line.split_whitespace().collect();
                if p.is_empty() { continue; }
                match p[0] {
                    "bal" => {
                        let l = safe_lock(&ledger);
                        let b = l.accounts.get(&my_address).map(|a| a.balance).unwrap_or(0);
                        println!("📊 Balance: {} LOS", format_u128(b / CIL_PER_LOS));
                    },
                    "whoami" => {
                        println!("🆔 My Short ID: {}", my_short);
                        println!("🔑 Full Address: {}", my_address);
                    },
                    "supply" => {
                        let l = safe_lock(&ledger);
                        println!("📉 Remaining Supply: {} LOS", format_u128(l.distribution.remaining_supply / CIL_PER_LOS));
                    },
                    "history" => {
                        let l = safe_lock(&ledger);
                        // 1. Determine target: user input or self if empty
                        let input_addr = if p.len() == 2 { p[1] } else { &my_address };

                        // 2. Find Full Address
                        let target_full = if input_addr.starts_with("los_") {
                            // If user input short ID, search in address book
                            safe_lock(&address_book).get(input_addr).cloned()
                        } else {
                            // If user input full address or this is our own address
                            Some(input_addr.to_string())
                        };

                        if let Some(full_addr) = target_full {
                            if let Some(acct) = l.accounts.get(&full_addr) {
                                let mut history_blocks = Vec::new();
                                let mut curr = acct.head.clone();

                                while curr != "0" {
                                    if let Some(blk) = l.blocks.get(&curr) {
                                        history_blocks.push(blk);
                                        curr = blk.previous.clone();
                                    } else { break; }
                                }

                                if history_blocks.is_empty() {
                                    println!("📭 No transaction history for {}", get_short_addr(&full_addr));
                                } else {
                                    print_history_table(history_blocks);
                                }
                            } else {
                                println!("❌ Account {} has no record in Ledger.", input_addr);
                            }
                        } else {
                            println!("❌ ID {} not found in Address Book.", input_addr);
                        }
                    },
                    "peers" => {
                        let ab = safe_lock(&address_book);
                        println!("👥 Peers: {}", ab.len());
                        for (s, f) in ab.iter() { println!("  - {}: {}", s, f); }
                    },
                    "dial" => {
                        if p.len() == 2 {
                            let tx = tx_out.clone();
                            let ma = my_address.clone();
                            let s = { let l = safe_lock(&ledger); l.distribution.remaining_supply };
                            let target = p[1].to_string();
                            tokio::spawn(async move {
                                let _ = tx.send(format!("DIAL:{}", target)).await;
                                tokio::time::sleep(Duration::from_secs(2)).await;
                                let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
                                let _ = tx.send(format!("ID:{}:{}:{}", ma, s, ts)).await;
                            });
                        }
                    },
                    "send" => {
                        if p.len() == 3 {
                            let target_short = p[1];
                            let amt_raw = match p[2].parse::<u128>() {
                                Ok(v) if v > 0 => v,
                                Ok(_) => {
                                    println!("❌ Send amount must be greater than 0!");
                                    continue;
                                }
                                Err(_) => {
                                    println!("❌ Invalid amount: '{}' — must be a positive integer (LOS)", p[2]);
                                    continue;
                                }
                            };
                            let amt = amt_raw * CIL_PER_LOS;

                            let target_full = safe_lock(&address_book).get(target_short).cloned();

                            if let Some(d) = target_full {
                                // DEADLOCK Never hold L and PS simultaneously.
                                // Step 1: Get state from Ledger (L lock only)
                                let state = {
                                    let l = safe_lock(&ledger);
                                    l.accounts.get(&my_address).cloned().unwrap_or(AccountState {
                                        head: "0".to_string(), balance: 0, block_count: 0, is_validator: false,
                                    })
                                }; // L dropped

                                // Step 2: Check pending total (PS lock only)
                                // Only sum THIS sender's pending txs, not all
                                let pending_total: u128 = safe_lock(&pending_sends).values()
                                    .filter(|(b, _)| b.account == my_address)
                                    .map(|(b, _)| b.amount).sum();

                                if state.balance < (amt + pending_total) {
                                    println!("❌ Insufficient balance! (Balance: {} LOS, In process: {} LOS)",
                                        format_u128(state.balance / CIL_PER_LOS),
                                        format_u128(pending_total / CIL_PER_LOS));
                                    continue;
                                }

                                // Create Send block draft
                                let mut blk = Block {
                                    account: my_address.clone(),
                                    previous: state.head.clone(),
                                    block_type: BlockType::Send,
                                    amount: amt,
                                    link: d.clone(),
                                    signature: "".to_string(),
                                    public_key: hex::encode(&keys.public_key), // Node's public key
                                    work: 0,
                                    timestamp: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs(),
                                    fee: los_core::BASE_FEE_CIL, // Protocol constant from los-core
                                };

                                solve_pow(&mut blk);
                                let signing_hash = blk.signing_hash();
                                blk.signature = match try_sign_hex(signing_hash.as_bytes(), &secret_key) {
                                    Ok(sig) => sig,
                                    Err(e) => { eprintln!("❌ Signing failed: {}", e); continue; }
                                };
                                let hash = blk.calculate_hash();

                                // Save to confirmation queue
                                safe_lock(&pending_sends).insert(hash.clone(), (blk.clone(), 0));

                                // Broadcast confirmation request (REQ) to network
                                let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
                                // Include block data (base64) so peers can validate
                                let block_json = serde_json::to_string(&blk).unwrap_or_default();
                                let block_b64 = base64::engine::general_purpose::STANDARD.encode(block_json.as_bytes());
                                let req_msg = format!("CONFIRM_REQ:{}:{}:{}:{}:{}", hash, my_address, amt, ts, block_b64);
                                let _ = tx_out.send(req_msg).await;

                                println!("⏳ Transaction created. Requesting network confirmation (Anti Double-Spend)...");
                            } else {
                                println!("❌ ID {} not found. Peer must connect first.", target_short);
                            }
                        }
                    },
                    "exit" => break,
                    _ => {}
                }
                    },
                    Ok(None) => {
                        // stdin EOF — running in headless/Flutter mode
                        stdin_closed = true;
                        if json_log {
                            // In Flutter mode, node keeps running without stdin
                            eprintln!("📡 Running in headless mode (stdin closed)");
                        }
                    },
                    Err(e) => {
                        eprintln!("⚠️ stdin error: {}", e);
                        stdin_closed = true;
                    },
                }
            },
            event = rx_in.recv() => {
                let Some(event) = event else {
                    // Network channel closed — P2P task exited/crashed
                    eprintln!("⚠️ Network channel closed, node running in offline mode");
                    // Keep node alive (API server still works) but just sleep
                    loop { tokio::time::sleep(Duration::from_secs(60)).await; }
                };
                if let NetworkEvent::NewBlock(data) = event {
                        if data.starts_with("ID:") {
                            let parts: Vec<&str> = data.split(':').collect();
                            if parts.len() >= 3 {
                                let full = parts[1].to_string();
                                let rem_s = parts[2].parse::<u128>().unwrap_or(0);

                                if full != my_address {
                                    let short = get_short_addr(&full);
                                    // Cap address_book to prevent memory DoS from
                                    // malicious peers flooding fake ID: messages.
                                    const MAX_PEERS: usize = 10_000;
                                    let is_new = {
                                        let mut ab = safe_lock(&address_book);
                                        if ab.len() >= MAX_PEERS && !ab.contains_key(&short) {
                                            None // Address book full; ignore new peer
                                        } else {
                                            let is_new = !ab.contains_key(&short);
                                            ab.insert(short.clone(), full.clone());
                                            Some(is_new)
                                        }
                                    }; // ab dropped — no MutexGuard held past this point

                                    // SECURITY: Mark this peer as LIVE (proven via gossipsub).
                                    // The reward heartbeat system uses this to verify liveness.
                                    {
                                        let ts = std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_secs();
                                        let mut lp = safe_lock(&live_peers);
                                        lp.insert(full.clone(), ts);
                                    }

                                    if let Some(is_new) = is_new {

                                    // Persist peer to database for recovery after restart
                                    if is_new {
                                        if let Err(e) = db_clone.save_peer(&short, &full) {
                                            eprintln!("⚠️ Failed to persist peer {}: {}", short, e);
                                        }
                                    }

                                    // DEADLOCK Never hold L and PS simultaneously.
                                    // Step 1: Ledger operations (L lock only)
                                    let (supply_remaining, full_state_json) = {
                                        let mut l = safe_lock(&ledger);

                                        // Don't blindly trust peer's remaining_supply.
                                        // Instead, verify by recalculating from our own Mint blocks.
                                        // Only sync if peer claims LESS supply remaining AND our calculation confirms it.
                                        if rem_s < l.distribution.remaining_supply && rem_s != 0 {
                                            // Recalculate how much we've minted from our own blocks
                                            let total_minted: u128 = l.blocks.values()
                                                .filter(|b| b.block_type == BlockType::Mint)
                                                .map(|b| b.amount)
                                                .sum();
                                            let calculated_remaining = los_core::distribution::PUBLIC_SUPPLY_CAP.saturating_sub(total_minted);

                                            // Only accept peer's value if it's close to our calculation
                                            // Allow 1% tolerance for network propagation delay
                                            let tolerance = los_core::distribution::PUBLIC_SUPPLY_CAP / 100;
                                            if rem_s >= calculated_remaining.saturating_sub(tolerance)
                                                && rem_s <= calculated_remaining.saturating_add(tolerance) {
                                                l.distribution.remaining_supply = calculated_remaining;
                                                                SAVE_DIRTY.store(true, Ordering::Release);
                                                println!("🔄 Supply Verified & Synced with Peer: {} (calculated: {})", short, calculated_remaining);
                                            } else {
                                                println!("⚠️ Supply sync rejected from {}: peer claims {} but we calculated {}",
                                                    short, rem_s, calculated_remaining);
                                            }
                                        }

                                        println!("🤝 Handshake: {}", short);

                                        let supply = l.distribution.remaining_supply;
                                        let json = if is_new { serde_json::to_string(&*l).ok() } else { None };
                                        (supply, json)
                                    }; // L dropped

                                    // Step 2: Pending transaction resend (PS lock only)
                                    let retry_msgs: Vec<(String, String)> = {
                                        let pending_map = safe_lock(&pending_sends);
                                        pending_map.iter().map(|(hash, (blk, _))| {
                                            let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
                                            // Include block data (base64) in retry too
                                            let block_json = serde_json::to_string(blk).unwrap_or_default();
                                            let block_b64 = base64::engine::general_purpose::STANDARD.encode(block_json.as_bytes());
                                            (format!("CONFIRM_REQ:{}:{}:{}:{}:{}", hash, blk.account, blk.amount, ts, block_b64), hash[..8].to_string())
                                        }).collect()
                                    }; // PS dropped
                                    for (retry_msg, hash_short) in &retry_msgs {
                                        let _ = tx_out.send(retry_msg.clone()).await;
                                        println!("📡 Resending confirmation request to new peer for TX: {}", hash_short);
                                    }

                                    // Step 3: Send identity and state to new peer
                                    if is_new {
                                        let s = supply_remaining;
                                        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
                                        let _ = tx_out.send(format!("ID:{}:{}:{}", my_address, s, ts)).await;

                                        // Only send full state sync for small networks or small ledgers
                                        // This avoids flooding gossipsub with huge payloads in larger networks
                                        if let Some(full_state_json) = full_state_json {
                                            use flate2::write::GzEncoder;
                                            use flate2::Compression;
                                            use std::io::Write;

                                            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
                                            let _ = encoder.write_all(full_state_json.as_bytes());
                                            if let Ok(compressed_bytes) = encoder.finish() {
                                                const MAX_SYNC_PAYLOAD: usize = 8 * 1024 * 1024; // 8 MB max (within gossipsub 10MB limit)
                                                if compressed_bytes.len() <= MAX_SYNC_PAYLOAD {
                                                    let encoded_data = base64::engine::general_purpose::STANDARD.encode(&compressed_bytes);
                                                    let _ = tx_out.send(format!("SYNC_GZIP:{}", encoded_data)).await;
                                                    println!("📦 Sent state sync to new peer ({:.1} KB compressed)",
                                                        compressed_bytes.len() as f64 / 1024.0);
                                                } else {
                                                    println!("⚠️ State too large for gossipsub sync ({:.1} MB > 8 MB limit). Sending SYNC_VIA_REST instead.",
                                                        compressed_bytes.len() as f64 / 1_048_576.0);
                                                    // Tell the new peer to fetch state via REST instead
                                                    // Use | separator to avoid collision with : in host:port
                                                    if let Some(our_host) = get_node_host_address() {
                                                        let rest_host = ensure_host_port(&our_host, api_port);
                                                        let block_count = {
                                                            let l = safe_lock(&ledger);
                                                            l.blocks.len()
                                                        };
                                                        let _ = tx_out.send(format!("SYNC_VIA_REST:{}|{}", rest_host, block_count)).await;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    } // end is_new scope
                                }
                            }
                        } else if let Some(encoded_data) = data.strip_prefix("SYNC_GZIP:") {
                            // Rate limit SYNC_GZIP to prevent DDoS via large payloads
                            static LAST_SYNC: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                            let now_secs = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
                            let last = LAST_SYNC.load(Ordering::Relaxed);
                            if now_secs.saturating_sub(last) < 10 {
                                println!("⚠️ SYNC_GZIP rate limited (min 10s between syncs)");
                                continue;
                            }
                            LAST_SYNC.store(now_secs, Ordering::Relaxed);

                            if let Ok(compressed_bytes) = base64::engine::general_purpose::STANDARD.decode(encoded_data) {
                                use flate2::read::GzDecoder;
                                use std::io::Read;

                                // Limit decompressed size to prevent decompression bomb
                                const MAX_DECOMPRESSED_SIZE: u64 = 50 * 1024 * 1024; // 50 MB max
                                let decoder = GzDecoder::new(&compressed_bytes[..]);
                                let mut limited_decoder = decoder.take(MAX_DECOMPRESSED_SIZE);
                                let mut decompressed_json = String::new();

                                if limited_decoder.read_to_string(&mut decompressed_json).is_ok() {
                                    if let Ok(incoming_ledger) = serde_json::from_str::<Ledger>(&decompressed_json) {
                                        // DESIGN State root comparison — skip sync if states match.
                                        // Prevents redundant O(n) block-by-block processing when two nodes
                                        // already have identical state (common after initial sync).
                                        let incoming_root = incoming_ledger.compute_state_root();
                                        let our_root = {
                                            let l = safe_lock(&ledger);
                                            l.compute_state_root()
                                        };
                                        if incoming_root == our_root {
                                            println!("📦 SYNC_GZIP: state roots match ({}) — skipping sync",
                                                &incoming_root[..16]);
                                            continue;
                                        }

                                        let mut l = safe_lock(&ledger);
                                        let mut added_count = 0;
                                        let mut invalid_count = 0;
                                        let our_block_count = l.blocks.len();
                                        let incoming_block_count = incoming_ledger.blocks.len();

                                        // FAST-PATH: If peer has significantly more blocks (gap > 5),
                                        // use direct state adoption instead of process_block() which
                                        // fails on chain-sequence validation when blocks are missing.
                                        // This fixes the "stuck node" bug where a node that misses
                                        // gossip messages can never catch up because process_block()
                                        // rejects all incoming blocks due to chain head mismatch.
                                        let block_gap = incoming_block_count.saturating_sub(our_block_count);
                                        if block_gap > 5 {
                                            println!("📦 SYNC: Large gap detected ({} blocks behind). Using direct state adoption.", block_gap);

                                            // Validate ALL incoming blocks cryptographically before adopting
                                            let mut crypto_invalid = 0usize;
                                            for blk in incoming_ledger.blocks.values() {
                                                // Verify PoW
                                                if !blk.verify_pow() {
                                                    crypto_invalid += 1;
                                                    continue;
                                                }
                                                // Verify signature
                                                if !blk.verify_signature() {
                                                    crypto_invalid += 1;
                                                    continue;
                                                }
                                            }

                                            // Only adopt if <10% of blocks are invalid (allows for minor
                                            // differences in block validation rules across versions)
                                            let max_invalid = (incoming_block_count / 10).max(3);
                                            if crypto_invalid <= max_invalid {
                                                // Adopt account states from peer for accounts where peer
                                                // has more blocks (more advanced chain)
                                                for (addr, incoming_acct) in &incoming_ledger.accounts {
                                                    let dominated = match l.accounts.get(addr) {
                                                        Some(ours) => incoming_acct.block_count > ours.block_count,
                                                        None => true, // New account we don't have
                                                    };
                                                    if dominated {
                                                        l.accounts.insert(addr.clone(), incoming_acct.clone());
                                                    }
                                                }
                                                // Merge all missing blocks into our ledger
                                                for (hash, blk) in &incoming_ledger.blocks {
                                                    if !l.blocks.contains_key(hash) {
                                                        l.blocks.insert(hash.clone(), blk.clone());
                                                        added_count += 1;
                                                    }
                                                }
                                                // Adopt distribution state (remaining supply, etc.)
                                                // Only if peer has minted more (lower remaining = more distributed)
                                                if incoming_ledger.distribution.remaining_supply < l.distribution.remaining_supply {
                                                    l.distribution = incoming_ledger.distribution.clone();
                                                }
                                                // Merge claimed_sends to prevent double-receive
                                                for claimed in &incoming_ledger.claimed_sends {
                                                    l.claimed_sends.insert(claimed.clone());
                                                }
                                                // Adopt accumulated fees if peer has more
                                                if incoming_ledger.accumulated_fees_cil > l.accumulated_fees_cil {
                                                    l.accumulated_fees_cil = incoming_ledger.accumulated_fees_cil;
                                                }

                                                // Sync reward pool for any incoming reward/fee blocks
                                                for blk in incoming_ledger.blocks.values() {
                                                    if blk.block_type == BlockType::Mint
                                                        && (blk.link.starts_with("REWARD:EPOCH:")
                                                            || blk.link.starts_with("FEE_REWARD:EPOCH:"))
                                                    {
                                                        let mut pool = safe_lock(&rp_sync);
                                                        pool.sync_reward_from_gossip(&blk.account, blk.amount);
                                                    }
                                                }
                                                // Record participation for slashing
                                                {
                                                    let mut sm = safe_lock(&slashing_clone);
                                                    let timestamp = std::time::SystemTime::now()
                                                        .duration_since(std::time::UNIX_EPOCH)
                                                        .unwrap_or_default()
                                                        .as_secs();
                                                    for (addr, acc) in &l.accounts {
                                                        if acc.balance >= MIN_VALIDATOR_STAKE_CIL {
                                                            if sm.get_profile(addr).is_none() {
                                                                sm.register_validator(addr.clone());
                                                            }
                                                            let _ = sm.record_block_participation(addr, l.blocks.len() as u64, timestamp);
                                                        }
                                                    }
                                                }
                                                println!("📚 State Adoption Complete: {} new blocks merged, {} crypto-invalid skipped",
                                                    added_count, crypto_invalid);
                                                // Sanitize: remove orphaned blocks after state adoption
                                                let orphans = l.remove_orphaned_blocks();
                                                if orphans > 0 {
                                                    println!("🧹 Removed {} orphaned block(s) after sync", orphans);
                                                }
                                            } else {
                                                println!("🚫 Rejected state adoption: too many invalid blocks ({}/{}, max {})",
                                                    crypto_invalid, incoming_block_count, max_invalid);
                                            }
                                        } else {
                                        // SLOW-PATH: Small gap — use sequential process_block() validation
                                        // Remove 1000-block cap to allow full state sync.
                                        // Sort blocks by timestamp for O(n log n) sync instead of O(n²)
                                        let mut incoming_blocks: Vec<Block> = incoming_ledger.blocks.values()
                                            .cloned()
                                            .collect();
                                        incoming_blocks.sort_by_key(|b| b.timestamp);

                                        // Two-pass: first pass processes ordered blocks, second catches stragglers
                                        for pass in 0..2 {
                                            for blk in &incoming_blocks {
                                                // Accept Mint/Slash blocks in SYNC if validly signed
                                                // by a staked validator. Blanket-reject caused new nodes to
                                                // permanently miss all minted balances.
                                                if matches!(blk.block_type, BlockType::Mint | BlockType::Slash) {
                                                    let sig_ok = hex::decode(&blk.signature).ok().and_then(|sig| {
                                                        hex::decode(&blk.public_key).ok().map(|pk| {
                                                            let sh = blk.signing_hash();
                                                            los_crypto::verify_signature(sh.as_bytes(), &sig, &pk)
                                                        })
                                                    }).unwrap_or(false);
                                                    if !sig_ok || !blk.verify_pow() {
                                                        invalid_count += 1;
                                                        continue;
                                                    }
                                                }

                                                let hash = blk.calculate_hash();
                                                if l.blocks.contains_key(&hash) { continue; }

                                                if !l.accounts.contains_key(&blk.account) {
                                                    l.accounts.insert(blk.account.clone(), AccountState {
                                                        head: "0".to_string(), balance: 0, block_count: 0, is_validator: false,
                                                    });
                                                }

                                                // FEE_REWARD supply handling is now in process_block() itself.
                                                // No need for save/restore — process_block skips remaining_supply
                                                // deduction for FEE_REWARD: Mint blocks automatically.

                                                match l.process_block(blk) {
                                                    Ok(_) => {
                                                        // Sync reward pool when receiving
                                                        // REWARD:EPOCH or FEE_REWARD:EPOCH Mint blocks from leader.
                                                        // This keeps non-leader pool stats consistent.
                                                        if blk.block_type == BlockType::Mint
                                                            && (blk.link.starts_with("REWARD:EPOCH:")
                                                                || blk.link.starts_with("FEE_REWARD:EPOCH:"))
                                                        {
                                                            let mut pool = safe_lock(&rp_sync);
                                                            pool.sync_reward_from_gossip(&blk.account, blk.amount);
                                                        }
                                                        // SLASHING: Record participation during sync
                                                        {
                                                            let mut sm = safe_lock(&slashing_clone);
                                                            let timestamp = std::time::SystemTime::now()
                                                                .duration_since(std::time::UNIX_EPOCH)
                                                                .unwrap_or_default()
                                                                .as_secs();

                                                            if let Some(acc) = l.accounts.get(&blk.account) {
                                                                if acc.balance >= MIN_VALIDATOR_STAKE_CIL {
                                                                    if sm.get_profile(&blk.account).is_none() {
                                                                        sm.register_validator(blk.account.clone());
                                                                    }
                                                                    let _ = sm.record_block_participation(&blk.account, l.blocks.len() as u64, timestamp);
                                                                }
                                                            }
                                                        }
                                                        added_count += 1;
                                                    },
                                                    Err(_) => {
                                                        if pass == 1 { invalid_count += 1; }
                                                    }
                                                }
                                            }
                                        }

                                        // Log but don't blacklist during sync — chain sequence failures are expected
                                        // when nodes have diverged. Blacklisting honest peers prevented recovery.
                                        if invalid_count > 0 {
                                            println!("⚠️ SYNC: {} blocks failed process_block() validation (likely chain sequence gaps)", invalid_count);
                                        }
                                        } // end slow-path else

                                        if added_count > 0 {
                                            SAVE_DIRTY.store(true, Ordering::Release);
                                            // Sanitize: remove orphaned blocks after slow-path sync
                                            // NOTE: reuse existing `l` — do NOT re-acquire ledger lock (deadlock)
                                            let orphans = l.remove_orphaned_blocks();
                                            if orphans > 0 {
                                                println!("🧹 Sync: removed {} orphaned block(s)", orphans);
                                            }
                                            println!("📚 Sync Complete: {} new blocks validated", added_count);
                                        }
                                    }
                                }
                            }
                        } else if data.starts_with("SYNC_REQUEST:") {
                            // SECURITY P0-4: Rate-limited, per-requester sync response
                            // FORMAT: SYNC_REQUEST:<requester_address>:<their_block_count>
                            static SYNC_RESP_TIMES: std::sync::LazyLock<Mutex<HashMap<String, u64>>> =
                                std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

                            let parts: Vec<&str> = data.split(':').collect();
                            if parts.len() >= 3 {
                                let requester = parts[1].to_string();
                                let their_count: usize = parts[2].parse().unwrap_or(0);

                                // Per-requester rate limit: max 1 sync response per 15 seconds per peer
                                let now_secs = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
                                {
                                    let mut times = safe_lock(&SYNC_RESP_TIMES);
                                    let last = times.get(&requester).copied().unwrap_or(0);
                                    if now_secs.saturating_sub(last) < 15 {
                                        continue; // Rate limited — skip silently
                                    }
                                    times.insert(requester.clone(), now_secs);
                                    // Evict old entries to prevent memory leak
                                    times.retain(|_, ts| now_secs.saturating_sub(*ts) < 300);
                                }

                                // Only respond if we have more blocks than the requester
                                let our_count = safe_lock(&ledger).blocks.len();
                                if our_count > their_count && requester != my_address {
                                    println!("📡 Sync request from {} (they have {} blocks, we have {})",
                                        get_short_addr(&requester), their_count, our_count);

                                    let sync_json = {
                                        let l = safe_lock(&ledger);
                                        serde_json::to_string(&*l).ok()
                                    };

                                    if let Some(json) = sync_json {
                                        use flate2::write::GzEncoder;
                                        use flate2::Compression;
                                        use std::io::Write;

                                        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
                                        let _ = encoder.write_all(json.as_bytes());
                                        if let Ok(compressed) = encoder.finish() {
                                            const MAX_GOSSIP_SYNC: usize = 8 * 1024 * 1024;
                                            if compressed.len() <= MAX_GOSSIP_SYNC {
                                                // Small enough for gossip — send via SYNC_GZIP
                                                let encoded = base64::engine::general_purpose::STANDARD.encode(&compressed);
                                                let _ = tx_out.send(format!("SYNC_GZIP:{}", encoded)).await;
                                                println!("📤 Sent state sync via gossip ({} blocks, {}KB compressed)", our_count, compressed.len() / 1024);
                                            } else {
                                                // State too large for gossip — tell peer to use REST sync
                                                if let Some(our_host) = get_node_host_address() {
                                                    let rest_host = ensure_host_port(&our_host, api_port);
                                                    let _ = tx_out.send(format!("SYNC_VIA_REST:{}|{}", rest_host, our_count)).await;
                                                    println!("📤 State too large for gossip ({:.1} MB). Sent SYNC_VIA_REST redirect to {}",
                                                        compressed.len() as f64 / 1_048_576.0, rest_host);
                                                } else {
                                                    println!("⚠️ State too large for gossip ({:.1} MB) and no host address configured — peer cannot sync",
                                                        compressed.len() as f64 / 1_048_576.0);
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                        } else if let Some(payload) = data.strip_prefix("SYNC_VIA_REST:") {
                            // FORMAT: SYNC_VIA_REST:<host:port>|<their_block_count>
                            // Peer's state is too large for gossip — use HTTP REST to pull full state.
                            // Uses | separator to avoid collision with : in host:port
                            let parts: Vec<&str> = payload.splitn(2, '|').collect();
                            if parts.len() >= 2 {
                                let peer_host = parts[0].to_string();
                                let peer_blocks: usize = parts[1].parse().unwrap_or(0);
                                let our_blocks = safe_lock(&ledger).blocks.len();

                                if peer_blocks > our_blocks {
                                    println!("📡 SYNC_VIA_REST: peer {} has {} blocks (we have {}). Fetching via REST...",
                                        &peer_host, peer_blocks, our_blocks);

                                    // Spawn async HTTP fetch task
                                    let ledger_rest = Arc::clone(&ledger);
                                    let rp_rest = Arc::clone(&rp_sync);
                                    let sm_rest = Arc::clone(&slashing_clone);
                                    let db_rest = Arc::clone(&database);
                                    tokio::spawn(async move {
                                        match rest_sync_from_peer(&peer_host, our_blocks, &ledger_rest, &rp_rest, &sm_rest, &db_rest).await {
                                            Ok(added) => println!("✅ REST sync from {} complete: {} new blocks", peer_host, added),
                                            Err(e) => println!("⚠️ REST sync from {} failed: {}", peer_host, e),
                                        }
                                    });
                                }
                            }
                        } else if data.starts_with("SLASH_REQ:") {
                            // FORMAT: SLASH_REQ:cheater_address:fake_txid:proposer_addr:timestamp:signature:pubkey (7 parts)
                            // Verify Dilithium5 signature on SLASH_REQ (was unsigned).
                            let parts: Vec<&str> = data.split(':').collect();
                            if parts.len() == 7 {
                                let proposer_addr = parts[3].to_string();
                                let slash_sig_hex = parts[5];
                                let slash_pk_hex = parts[6];

                                // Verify cryptographic signature
                                let slash_payload = format!("SLASH:{}:{}:{}:{}", parts[1], parts[2], parts[3], parts[4]);
                                let slash_sig_bytes = hex::decode(slash_sig_hex).unwrap_or_default();
                                let slash_pk_bytes = hex::decode(slash_pk_hex).unwrap_or_default();

                                if !los_crypto::verify_signature(slash_payload.as_bytes(), &slash_sig_bytes, &slash_pk_bytes) {
                                    println!("🚨 Rejected SLASH_REQ: invalid signature from {}", get_short_addr(&proposer_addr));
                                    continue;
                                }
                                // Verify pubkey matches claimed proposer address
                                let derived_proposer = los_crypto::public_key_to_address(&slash_pk_bytes);
                                if derived_proposer != proposer_addr {
                                    println!("🚨 Rejected SLASH_REQ: pubkey mismatch for {}", get_short_addr(&proposer_addr));
                                    continue;
                                }
                            } else if parts.len() == 3 {
                                // Legacy unsigned format — reject on mainnet, warn on testnet
                                if los_core::is_mainnet_build() {
                                    println!("🚨 Rejected unsigned SLASH_REQ (mainnet requires signed messages)");
                                    continue;
                                }
                                println!("⚠️ Accepted unsigned SLASH_REQ (testnet only — will be rejected on mainnet)");
                            } else {
                                continue;
                            }
                            {
                                let cheater_addr = parts[1].to_string();
                                let fake_txid = parts[2].to_string();

                                println!("⚖️  Slash proposal received for: {}", get_short_addr(&cheater_addr));

                                // Step 1: Validate this node is a validator
                                let my_balance = {
                                    let l = safe_lock(&ledger);
                                    l.accounts.get(&my_address).map(|a| a.balance).unwrap_or(0)
                                };
                                if my_balance < MIN_VALIDATOR_STAKE_CIL {
                                    println!("⚠️ Ignoring SLASH_REQ: this node is not a validator");
                                    continue;
                                }

                                // Step 2: Independently verify the evidence
                                // SECURITY P1-1: Check if cheater's TXID was already legitimately minted
                                // Evidence is valid if: cheater exists AND the TXID was NOT found in any
                                // Mint block's link field (i.e., it was never successfully minted)
                                let is_valid_evidence = {
                                    let l = safe_lock(&ledger);
                                    let cheater_exists = l.accounts.contains_key(&cheater_addr);
                                    // Check that no Mint block references this TXID in its link
                                    let txid_was_minted = l.blocks.values().any(|b| {
                                        b.block_type == los_core::BlockType::Mint && b.link.contains(&fake_txid)
                                    });
                                    cheater_exists && !txid_was_minted
                                };

                                if !is_valid_evidence {
                                    println!("⚠️ SLASH_REQ rejected: evidence not confirmed independently");
                                    continue;
                                }

                                // Step 3: Register vote in SlashingManager
                                let should_execute = {
                                    let mut sm = safe_lock(&slashing_manager);
                                    let stats = sm.get_safety_stats();
                                    let total_validators = stats.total_validators.max(1);
                                    let threshold = ((total_validators * 2 / 3) + 1) as usize;

                                    // Use propose_slash to register this vote
                                    let evidence_hash = format!("FAKE_TXID:{}", fake_txid);
                                    let now_ts = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_secs();
                                    let _ = sm.propose_slash(
                                        cheater_addr.clone(),
                                        los_consensus::slashing::ViolationType::FraudulentTransaction,
                                        evidence_hash,
                                        my_address.clone(),
                                        now_ts,
                                    );

                                    // Check if enough validators have proposed slash for this address
                                    let proposal_count = sm.get_pending_proposals()
                                        .iter()
                                        .filter(|p| p.offender == cheater_addr && !p.executed)
                                        .count();

                                    println!("⚖️  Slash votes for {}: {}/{} (need {})",
                                        get_short_addr(&cheater_addr), proposal_count, total_validators, threshold);

                                    proposal_count >= threshold
                                };

                                if should_execute {
                                    // Consensus reached — execute the slash
                                    let slash_gossip: Option<String> = {
                                        let mut l = safe_lock(&ledger);
                                        let mut gossip = None;

                                        if let Some(state) = l.accounts.get(&cheater_addr).cloned() {
                                            if state.balance > 0 {
                                                // Penalty: 10% of total balance
                                                let penalty_amount = state.balance / 10;

                                                let mut slash_blk = Block {
                                                    account: cheater_addr.clone(),
                                                    previous: state.head.clone(),
                                                    block_type: BlockType::Slash,
                                                    amount: penalty_amount,
                                                    link: format!("PENALTY:FAKE_TXID:{}", fake_txid),
                                                    signature: "".to_string(),
                                                    public_key: hex::encode(&keys.public_key),
                                                    work: 0,
                                                    timestamp: std::time::SystemTime::now()
                                                        .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs(),
                                                    fee: 0,
                                                };

                                                solve_pow(&mut slash_blk);
                                                if let Ok(sig) = los_crypto::sign_message(slash_blk.signing_hash().as_bytes(), &secret_key) {
                                                    slash_blk.signature = hex::encode(sig);

                                                    match l.process_block(&slash_blk) {
                                                        Ok(result) => {
                                                            let hash = result.into_hash();
                                                            SAVE_DIRTY.store(true, Ordering::Release);
                                                            gossip = Some(serde_json::to_string(&slash_blk).unwrap_or_default());
                                                            println!("🔨 SLASHED (consensus 2/3+1)! {} penalized {} LOS (block: {})",
                                                                get_short_addr(&cheater_addr),
                                                                penalty_amount / CIL_PER_LOS,
                                                                &hash[..8]
                                                            );
                                                        },
                                                        Err(e) => println!("⚠️ Slash block failed for {}: {}", get_short_addr(&cheater_addr), e),
                                                    }
                                                } else {
                                                    println!("⚠️ Slash signing failed for {}", get_short_addr(&cheater_addr));
                                                }
                                            }
                                        }
                                        gossip
                                    }; // l dropped
                                    if let Some(msg) = slash_gossip {
                                        let _ = tx_out.send(msg).await;
                                    }
                                } else {
                                    println!("⏳ Slash proposal registered, waiting for more validator votes...");
                                }
                            }

                        } else if data.starts_with("CONFIRM_REQ:") {
                            let parts: Vec<&str> = data.split(':').collect();
                            // Support both V1 (5 parts) and V2 (6 parts with block data)
                            if parts.len() >= 5 {
                                let tx_hash = parts[1].to_string();
                                let sender_addr = parts[2].to_string();
                                let amount = parts[3].parse::<u128>().unwrap_or(0);

                                // Decode block from CONFIRM_REQ message (V2 format)
                                // so peers can validate without needing the block in their ledger.
                                let block_from_msg: Option<los_core::Block> = if parts.len() >= 6 {
                                    base64::engine::general_purpose::STANDARD.decode(parts[5]).ok()
                                        .and_then(|bytes| serde_json::from_slice::<los_core::Block>(&bytes).ok())
                                } else {
                                    None
                                };

                                let tx_confirm = tx_out.clone();
                                let ledger_ref = Arc::clone(&ledger);
                                let my_addr_clone = my_address.clone();
                                // Zeroize cloned secret key on async task drop
                                let confirm_sk = secret_key.clone();
                                let confirm_pk = keys.public_key.clone();

                                tokio::spawn(async move {
                                    // SECURITY P0-2: Verify the block exists and matches claims.
                                    // First check ledger (for re-gossipped blocks), then validate
                                    // the embedded block from the CONFIRM_REQ message (consensus fix).
                                    let (sender_balance, block_valid) = {
                                        let l_guard = safe_lock(&ledger_ref);
                                        let bal = l_guard.accounts.get(&sender_addr).map(|a| a.balance).unwrap_or(0);

                                        // Path 1: Block already in ledger (re-gossip or skip_consensus)
                                        let ledger_valid = l_guard.blocks.get(&tx_hash).map(|b| {
                                            b.block_type == los_core::BlockType::Send
                                                && b.account == sender_addr
                                                && b.amount == amount
                                        }).unwrap_or(false);

                                        // Path 2: Validate embedded block from CONFIRM_REQ message
                                        // Full cryptographic validation: hash, signature, PoW, sender binding
                                        let msg_valid = if !ledger_valid {
                                            block_from_msg.as_ref().map(|b| {
                                                // 1. Hash must match claimed tx_hash
                                                let hash_ok = b.calculate_hash() == tx_hash;
                                                // 2. Must be a Send block
                                                let type_ok = b.block_type == los_core::BlockType::Send;
                                                // 3. Sender must match
                                                let sender_ok = b.account == sender_addr;
                                                // 4. Amount must match
                                                let amount_ok = b.amount == amount;
                                                // 5. Dilithium5 signature must be valid
                                                let sig_ok = b.verify_signature();
                                                // 6. PoW must meet difficulty
                                                let pow_ok = b.verify_pow();
                                                // 7. Public key must derive to sender address
                                                let pk_bytes = hex::decode(&b.public_key).unwrap_or_default();
                                                let derived = los_crypto::public_key_to_address(&pk_bytes);
                                                let pk_ok = derived == sender_addr;

                                                if !hash_ok || !type_ok || !sender_ok || !amount_ok || !sig_ok || !pow_ok || !pk_ok {
                                                    println!("⚠️ CONFIRM_REQ block validation failed: hash={} type={} sender={} amount={} sig={} pow={} pk={}",
                                                        hash_ok, type_ok, sender_ok, amount_ok, sig_ok, pow_ok, pk_ok);
                                                }

                                                hash_ok && type_ok && sender_ok && amount_ok && sig_ok && pow_ok && pk_ok
                                            }).unwrap_or(false)
                                        } else { false };

                                        (bal, ledger_valid || msg_valid)
                                    };

                                    if !block_valid {
                                        // P0-2: Block doesn't exist/match and no valid embedded block — don't vote
                                        println!("⚠️ CONFIRM_REQ rejected: block_valid=false for hash={}", &tx_hash[..8.min(tx_hash.len())]);
                                        return;
                                    }

                                    // BALANCE CHECK: Verify sender has sufficient funds before voting YES.
                                    // This is the mainnet-safe path — no shortcuts.
                                    if sender_balance >= amount {
                                        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
                                        // SECURITY P0-1: Sign CONFIRM_RES with Dilithium5
                                        let payload = format!("{}:{}:YES:{}:{}", tx_hash, sender_addr, my_addr_clone, ts);
                                        if let Ok(sig) = los_crypto::sign_message(payload.as_bytes(), &confirm_sk) {
                                            let res = format!("CONFIRM_RES:{}:{}:YES:{}:{}:{}:{}", tx_hash, sender_addr, my_addr_clone, ts, hex::encode(&sig), hex::encode(&confirm_pk));
                                            let _ = tx_confirm.send(res).await;
                                        } else {
                                            eprintln!("\u{26a0}\u{fe0f} Signing failed for CONFIRM_RES \u{2014} skipping");
                                        }
                                    } else {
                                        println!("\u{26a0}\u{fe0f} CONFIRM_REQ rejected: sender {} has insufficient balance ({} CIL < {} CIL)",
                                            get_short_addr(&sender_addr), sender_balance, amount);
                                    }
                                });
                            }
                        } else if data.starts_with("CONFIRM_RES:") {
                            let parts: Vec<&str> = data.split(':').collect();
                            // FORMAT: CONFIRM_RES:tx_hash:sender:YES:voter:timestamp:signature:pubkey (8 parts)
                            if parts.len() == 8 {
                                let tx_hash = parts[1].to_string();
                                let _requester = parts[2].to_string();
                                let voter_addr = parts[4].to_string();
                                let sig_hex = parts[6];
                                let pk_hex = parts[7];

                                // SECURITY P0-1: Verify Dilithium5 signature on confirmation
                                let payload = format!("{}:{}:YES:{}:{}", parts[1], parts[2], parts[4], parts[5]);
                                let sig_bytes = hex::decode(sig_hex).unwrap_or_default();
                                let pk_bytes = hex::decode(pk_hex).unwrap_or_default();

                                if !los_crypto::verify_signature(payload.as_bytes(), &sig_bytes, &pk_bytes) {
                                    println!("🚨 Rejected CONFIRM_RES: invalid signature from {}", get_short_addr(&voter_addr));
                                    continue;
                                }
                                let derived_addr = los_crypto::public_key_to_address(&pk_bytes);
                                if derived_addr != voter_addr {
                                    println!("🚨 Rejected CONFIRM_RES: pubkey mismatch for {}", get_short_addr(&voter_addr));
                                    continue;
                                }

                                // Removed `requester == my_address` guard.
                                // When a user wallet sends through a node, requester = wallet address ≠ node address,
                                // causing ALL votes to be silently dropped. The tx_exists check in pending_sends
                                // already correctly identifies the originating node (only the originator has it).
                                {
                                    // DEADLOCK Never hold PS and L simultaneously.
                                    // Step 1: Check if tx exists in pending (PS lock only)
                                    let tx_exists = {
                                        let pending = safe_lock(&pending_sends);
                                        pending.contains_key(&tx_hash)
                                    }; // PS dropped

                                    if !tx_exists { continue; }

                                    // Step 2: Get voter balance (L lock only)
                                    let (voter_balance, active_vc) = {
                                        let l_guard = safe_lock(&ledger);
                                        // Use in-memory state (authoritative)
                                        // REMOVED: disk re-read that overwrote in-memory state
                                        let bal = l_guard.accounts.get(&voter_addr).map(|a| a.balance).unwrap_or(0);
                                        // Only count accounts with is_validator=true.
                                        // Treasury wallets inflate vc, making quorum impossible.
                                        let vc = l_guard.accounts.values().filter(|a| a.is_validator && a.balance >= MIN_VALIDATOR_STAKE_CIL).count();
                                        (bal, vc)
                                    }; // L dropped

                                    // --- LINEAR VOTING: Power = Stake (Sybil-Neutral) ---
                                    let voter_power_linear = calculate_voting_power(voter_balance);
                                    let voter_power_display = voter_balance / CIL_PER_LOS;

                                    // Step 3: Update votes and check threshold (PS lock only)
                                    let finalize_data = {
                                        // Vote deduplication — prevent single validator from reaching consensus alone
                                        let mut voters = safe_lock(&send_voters_clone);
                                        let voter_set = voters.entry(tx_hash.clone()).or_default();
                                        if voter_set.contains(&voter_addr) {
                                            println!("⚠️ Duplicate send vote from {} — ignored", get_short_addr(&voter_addr));
                                            continue;
                                        }
                                        voter_set.insert(voter_addr.clone());
                                        let distinct_count = voter_set.len();
                                        drop(voters);

                                        let mut pending = safe_lock(&pending_sends);
                                        if let Some((blk, total_power_votes)) = pending.get_mut(&tx_hash) {
                                            if voter_power_linear > 0 {
                                                // Normalize CIL→LOS before * 1000 scaling (matches SEND_CONSENSUS_THRESHOLD units).
                                                let voter_power_los = voter_power_linear / CIL_PER_LOS;
                                                let power_scaled = voter_power_los * 1000;
                                                *total_power_votes += power_scaled;
                                                let min_voters = if !testnet_config::get_testnet_config().should_enable_consensus() { 1 } else { min_distinct_voters(active_vc) };
                                                println!("📩 Konfirmasi Power: {} (Stake: {} LOS, Power: {}) | Total: {}/{} (Voters: {}/{})",
                                                    get_short_addr(&voter_addr), voter_power_display, voter_power_los, total_power_votes, SEND_CONSENSUS_THRESHOLD, distinct_count, min_voters
                                                );
                                            } else {
                                                println!("⚠️ Vote from {} has 0 power (balance: {} CIL, {} LOS) — check genesis sync",
                                                    get_short_addr(&voter_addr), voter_balance, voter_power_display);
                                            }

                                            let min_voters = if !testnet_config::get_testnet_config().should_enable_consensus() { 1 } else { min_distinct_voters(active_vc) };
                                            let threshold: u128 = if !testnet_config::get_testnet_config().should_enable_consensus() { TESTNET_FUNCTIONAL_THRESHOLD } else { SEND_CONSENSUS_THRESHOLD };
                                            if *total_power_votes >= threshold && distinct_count >= min_voters {
                                                Some(blk.clone())
                                            } else { None }
                                        } else { None }
                                    }; // PS dropped

                                    // Step 4: If threshold met, finalize (L lock only, then SM lock only)
                                    if let Some(blk_to_finalize) = finalize_data {
                                        let process_success = {
                                            let mut l = safe_lock(&ledger);
                                            match l.process_block(&blk_to_finalize) {
                                                Ok(_) => {
                                                    // SLASHING: Record finalization participation
                                                    {
                                                        let mut sm = safe_lock(&slashing_clone);
                                                        let timestamp = std::time::SystemTime::now()
                                                            .duration_since(std::time::UNIX_EPOCH)
                                                            .unwrap_or_default()
                                                            .as_secs();

                                                        if let Some(acc) = l.accounts.get(&blk_to_finalize.account) {
                                                            if acc.balance >= MIN_VALIDATOR_STAKE_CIL {
                                                                if sm.get_profile(&blk_to_finalize.account).is_none() {
                                                                    sm.register_validator(blk_to_finalize.account.clone());
                                                                }
                                                                let _ = sm.record_block_participation(&blk_to_finalize.account, l.blocks.len() as u64, timestamp);
                                                            }
                                                        }
                                                    }
                                                    SAVE_DIRTY.store(true, Ordering::Release);
                                                    true
                                                },
                                                Err(e) => {
                                                    println!("❌ Finalization Failed: {:?}", e);
                                                    false
                                                }
                                            }
                                        }; // L dropped

                                        if process_success {
                                            // DESIGN Wire aBFT stats to actual consensus.
                                            // Record this finalization so `/consensus` API reports
                                            // real blocks_finalized count instead of zero.
                                            {
                                                let distinct_count = {
                                                    let voters = safe_lock(&send_voters_clone);
                                                    voters.get(&tx_hash).map(|s| s.len()).unwrap_or(0)
                                                };
                                                let mut abft = safe_lock(&abft_event);
                                                abft.record_external_finalization(distinct_count);
                                            }

                                            println!("✅ Transaction Confirmed (Power Verified) & Added to Ledger");

                                            // AUTO-UNREGISTER: If sender's balance dropped below minimum
                                            // registration stake (1 LOS) after this send, automatically unregister them.
                                            if blk_to_finalize.block_type == BlockType::Send {
                                                let mut l = safe_lock(&ledger);
                                                if let Some(sender_acct) = l.accounts.get_mut(&blk_to_finalize.account) {
                                                    if sender_acct.is_validator && sender_acct.balance < MIN_VALIDATOR_REGISTER_CIL {
                                                        sender_acct.is_validator = false;
                                                        SAVE_DIRTY.store(true, Ordering::Release);
                                                        println!("⚠️ Auto-unregistered validator {}: balance {} < minimum registration stake {} LOS",
                                                            get_short_addr(&blk_to_finalize.account),
                                                            sender_acct.balance / CIL_PER_LOS,
                                                            MIN_VALIDATOR_REGISTER_CIL / CIL_PER_LOS);
                                                    }
                                                }
                                            }

                                            // Auto-create Receive block for ANY recipient.
                                            // The originating node that finalized the consensus must create
                                            // the Receive block — remote wallets (W1, W2, etc.) don't run
                                            // nodes, so no one else will create it.
                                            // For self-sends (link == my_address), use process_block().
                                            // For remote sends, use direct ledger manipulation (like skip_consensus).
                                            if blk_to_finalize.block_type == BlockType::Send {
                                                let target = blk_to_finalize.link.clone();
                                                let send_hash = blk_to_finalize.calculate_hash();

                                                let recv_gossip: Option<String> = {
                                                    let mut l = safe_lock(&ledger);
                                                    if !l.accounts.contains_key(&target) {
                                                        l.accounts.insert(target.clone(), AccountState {
                                                            head: "0".to_string(), balance: 0, block_count: 0, is_validator: false,
                                                        });
                                                    }
                                                    if let Some(recv_state) = l.accounts.get(&target).cloned() {
                                                        let prev_head = recv_state.head.clone();
                                                        let mut recv_blk = Block {
                                                            account: target.clone(),
                                                            previous: recv_state.head,
                                                            block_type: BlockType::Receive,
                                                            amount: blk_to_finalize.amount,
                                                            link: send_hash,
                                                            signature: "".to_string(),
                                                            public_key: hex::encode(&keys.public_key),
                                                            work: 0,
                                                            timestamp: std::time::SystemTime::now()
                                                                .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs(),
                                                            fee: 0,
                                                        };
                                                        solve_pow(&mut recv_blk);
                                                        recv_blk.signature = match try_sign_hex(recv_blk.signing_hash().as_bytes(), &secret_key) {
                                                            Ok(sig) => sig,
                                                            Err(e) => { eprintln!("⚠️ Auto-Receive signing failed: {}", e); String::new() }
                                                        };
                                                        if recv_blk.signature.is_empty() {
                                                            None
                                                        } else {
                                                            // Direct ledger manipulation for Receive block — bypass process_block()
                                                            // because the node's public_key doesn't match the target's account address.
                                                            let recv_hash = recv_blk.calculate_hash();

                                                            // Explicit validations that process_block() would do.
                                                            // 1. Duplicate block hash check
                                                            if l.blocks.contains_key(&recv_hash) {
                                                                eprintln!("⚠️ Auto-Receive duplicate hash — skipping");
                                                                None
                                                            }
                                                            // 2. Timestamp must be >= previous block's timestamp
                                                            else if prev_head != "0" {
                                                                let prev_ts = l.blocks.get(&prev_head).map(|b| b.timestamp).unwrap_or(0);
                                                                if recv_blk.timestamp < prev_ts {
                                                                    eprintln!("⚠️ Auto-Receive timestamp < previous — skipping");
                                                                    None
                                                                } else {
                                                                    // All checks passed — apply
                                                                    if let Some(recv_acct) = l.accounts.get_mut(&target) {
                                                                        recv_acct.balance = recv_acct.balance.saturating_add(blk_to_finalize.amount);
                                                                        recv_acct.head = recv_hash.clone();
                                                                        recv_acct.block_count += 1;
                                                                    }
                                                                    l.blocks.insert(recv_hash.clone(), recv_blk.clone());
                                                                    l.claimed_sends.insert(recv_blk.link.clone());
                                                                    SAVE_DIRTY.store(true, Ordering::Release);
                                                                    println!("📨 Auto-Receive created for {} (+{} CIL)",
                                                                        get_short_addr(&target), blk_to_finalize.amount);
                                                                    let send_b64 = base64::engine::general_purpose::STANDARD.encode(
                                                                        serde_json::to_string(&blk_to_finalize).unwrap_or_default()
                                                                    );
                                                                    let recv_b64 = base64::engine::general_purpose::STANDARD.encode(
                                                                        serde_json::to_string(&recv_blk).unwrap_or_default()
                                                                    );
                                                                    Some(format!("BLOCK_CONFIRMED:{}:{}", send_b64, recv_b64))
                                                                }
                                                            } else {
                                                                // First block on account (head == "0") — no previous to check
                                                                if let Some(recv_acct) = l.accounts.get_mut(&target) {
                                                                    recv_acct.balance = recv_acct.balance.saturating_add(blk_to_finalize.amount);
                                                                    recv_acct.head = recv_hash.clone();
                                                                    recv_acct.block_count += 1;
                                                                }
                                                                l.blocks.insert(recv_hash.clone(), recv_blk.clone());
                                                                l.claimed_sends.insert(recv_blk.link.clone());
                                                                SAVE_DIRTY.store(true, Ordering::Release);
                                                                println!("📨 Auto-Receive created for {} (+{} CIL)",
                                                                    get_short_addr(&target), blk_to_finalize.amount);
                                                                let send_b64 = base64::engine::general_purpose::STANDARD.encode(
                                                                    serde_json::to_string(&blk_to_finalize).unwrap_or_default()
                                                                );
                                                                let recv_b64 = base64::engine::general_purpose::STANDARD.encode(
                                                                    serde_json::to_string(&recv_blk).unwrap_or_default()
                                                                );
                                                                Some(format!("BLOCK_CONFIRMED:{}:{}", send_b64, recv_b64))
                                                            }
                                                        }
                                                    } else { None }
                                                }; // l dropped
                                                if let Some(msg) = recv_gossip {
                                                    let _ = tx_out.send(msg).await;
                                                }
                                            }
                                        }
                                        // Step 5: Remove from pending (PS lock only)
                                        safe_lock(&pending_sends).remove(&tx_hash);
                                        safe_lock(&send_voters_clone).remove(&tx_hash);
                                        // Clean from mempool on confirmation
                                        safe_lock(&mempool_pool).remove_transaction(&tx_hash);
                                    }
                                }
                            }
                        } else if let Some(rest) = data.strip_prefix("VALIDATOR_HEARTBEAT:") {
                            // ── VALIDATOR_HEARTBEAT: Liveness proof from a peer ──
                            // Format: VALIDATOR_HEARTBEAT:<address>:<timestamp>:<pk_hex>:<sig_hex>
                            // The sender signs "VALIDATOR_HEARTBEAT:<address>:<timestamp>" with
                            // their Dilithium5 key and includes their public key for verification.
                            // We verify: pk derives to address, signature valid, timestamp fresh,
                            // address is a registered validator. Only then update live_peers.
                            let parts: Vec<&str> = rest.splitn(4, ':').collect();
                            if parts.len() == 4 {
                                let addr = parts[0];
                                let ts_str = parts[1];
                                let pk_hex = parts[2];
                                let sig_hex = parts[3];

                                // Skip our own heartbeats (we already record self-heartbeat locally)
                                if addr != my_address && los_crypto::validate_address(addr) {
                                    if let Ok(ts) = ts_str.parse::<u64>() {
                                        // Timestamp freshness: within 2× heartbeat interval
                                        let hb_interval = if los_core::is_testnet_build() { 10u64 } else { 60u64 };
                                        let now_ts = std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_secs();
                                        if now_ts.abs_diff(ts) <= hb_interval * 2 {
                                            if let (Ok(pk_bytes), Ok(sig_bytes)) = (hex::decode(pk_hex), hex::decode(sig_hex)) {
                                                // Verify public key derives to claimed address
                                                if los_crypto::public_key_to_address(&pk_bytes) == addr {
                                                    // Verify Dilithium5 signature
                                                    let message = format!("VALIDATOR_HEARTBEAT:{}:{}", addr, ts);
                                                    if los_crypto::verify_signature(message.as_bytes(), &sig_bytes, &pk_bytes) {
                                                        // Signature valid — check if registered validator
                                                        let is_registered = {
                                                            let rp = safe_lock(&reward_pool);
                                                            rp.validators.contains_key(addr)
                                                        };
                                                        if is_registered {
                                                            let mut lp = safe_lock(&live_peers);
                                                            lp.insert(addr.to_string(), now_ts);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        } else if let Some(rest) = data.strip_prefix("VALIDATOR_HEARTBEAT_PROXY:") {
                            // ── VALIDATOR_HEARTBEAT_PROXY: Node vouches for a registered wallet ──
                            // Format: VALIDATOR_HEARTBEAT_PROXY:<wallet_addr>:<node_addr>:<timestamp>:<node_pk_hex>:<signature_hex>
                            // The NODE signs "VALIDATOR_HEARTBEAT_PROXY:<wallet>:<node>:<ts>" with its key.
                            // We verify the node's signature + that the node is a known validator.
                            let parts: Vec<&str> = rest.splitn(5, ':').collect();
                            if parts.len() == 5 {
                                let wallet_addr = parts[0];
                                let node_addr = parts[1];
                                let ts_str = parts[2];
                                let pk_hex = parts[3];
                                let sig_hex = parts[4];

                                if wallet_addr != my_address && los_crypto::validate_address(wallet_addr) && los_crypto::validate_address(node_addr) {
                                    if let Ok(ts) = ts_str.parse::<u64>() {
                                        let hb_interval = if los_core::is_testnet_build() { 10u64 } else { 60u64 };
                                        let now_ts = std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_secs();
                                        if now_ts.abs_diff(ts) <= hb_interval * 2 {
                                            if let (Ok(pk_bytes), Ok(sig_bytes)) = (hex::decode(pk_hex), hex::decode(sig_hex)) {
                                                // Verify the signing node's public key matches its claimed address
                                                if los_crypto::public_key_to_address(&pk_bytes) == node_addr {
                                                    let message = format!("VALIDATOR_HEARTBEAT_PROXY:{}:{}:{}", wallet_addr, node_addr, ts);
                                                    if los_crypto::verify_signature(message.as_bytes(), &sig_bytes, &pk_bytes) {
                                                        // Valid proxy heartbeat — node vouches for wallet.
                                                        // Require BOTH the wallet AND the
                                                        // proxying node to be registered validators with stake.
                                                        // Without this, any anonymous node could spoof uptime
                                                        // for any validator, preventing deserved slashing.
                                                        // Requiring the proxy node to have stake adds economic
                                                        // cost to the attack (min 1000 LOS at risk of slashing).
                                                        let (wallet_registered, node_registered) = {
                                                            let rp = safe_lock(&reward_pool);
                                                            (
                                                                rp.validators.contains_key(wallet_addr),
                                                                rp.validators.contains_key(node_addr),
                                                            )
                                                        };
                                                        if wallet_registered && node_registered {
                                                            let mut lp = safe_lock(&live_peers);
                                                            lp.insert(wallet_addr.to_string(), now_ts);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        } else if let Some(json_str) = data.strip_prefix("VALIDATOR_REG:") {
                            // Handle validator registration broadcast from peers.
                            // Validates the same proof of ownership (Dilithium5 signature)
                            // before accepting the registration — no trust assumptions.
                            match serde_json::from_str::<serde_json::Value>(json_str) {
                                Ok(reg) => {
                                    let addr = reg["address"].as_str().unwrap_or_default().to_string();
                                    let pk_hex = reg["public_key"].as_str().unwrap_or_default().to_string();
                                    let sig_hex = reg["signature"].as_str().unwrap_or_default().to_string();
                                    let ts = reg["timestamp"].as_u64().unwrap_or(0);

                                    // Validate address format
                                    if addr.is_empty() || !los_crypto::validate_address(&addr) {
                                        println!("🚫 VALIDATOR_REG: invalid address from peer");
                                        continue;
                                    }

                                    // Verify public_key derives to claimed address
                                    let pk_bytes = match hex::decode(&pk_hex) {
                                        Ok(b) => b,
                                        Err(_) => { println!("🚫 VALIDATOR_REG: invalid pk hex"); continue; }
                                    };
                                    if los_crypto::public_key_to_address(&pk_bytes) != addr {
                                        println!("🚫 VALIDATOR_REG: pk does not match address");
                                        continue;
                                    }

                                    // Verify Dilithium5 signature
                                    let message = format!("REGISTER_VALIDATOR:{}:{}", addr, ts);
                                    let sig_bytes = match hex::decode(&sig_hex) {
                                        Ok(b) => b,
                                        Err(_) => { println!("🚫 VALIDATOR_REG: invalid sig hex"); continue; }
                                    };
                                    if !los_crypto::verify_signature(message.as_bytes(), &sig_bytes, &pk_bytes) {
                                        println!("🚫 VALIDATOR_REG: signature verification failed for {}", get_short_addr(&addr));
                                        continue;
                                    }

                                    // Timestamp freshness (5 minute window)
                                    let now = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_secs();
                                    if ts == 0 || now.abs_diff(ts) > 300 {
                                        println!("🚫 VALIDATOR_REG: stale timestamp from {}", get_short_addr(&addr));
                                        continue;
                                    }

                                    // Check balance & skip if already registered
                                    let (balance, already) = {
                                        let l = safe_lock(&ledger);
                                        match l.accounts.get(&addr) {
                                            Some(acc) => (acc.balance, acc.is_validator),
                                            None => (0, false),
                                        }
                                    };

                                    if already {
                                        // Already registered — but still update endpoint if missing.
                                        // This handles the case where is_validator was set via
                                        // state sync but no VALIDATOR_REG gossip was received yet.
                                        let raw_host = reg["host_address"]
                                            .as_str()
                                            .filter(|s| !s.is_empty())
                                            .or_else(|| {
                                                reg["onion_address"]
                                                    .as_str()
                                                    .filter(|s| !s.is_empty())
                                            });
                                        if let Some(h) = raw_host {
                                            // Use rest_port from gossip to ensure host has correct port
                                            let port = reg["rest_port"].as_u64().unwrap_or(3030) as u16;
                                            let host_with_port = ensure_host_port(h, port);
                                            let had = safe_lock(&ve_event).contains_key(&addr);
                                            if !had {
                                                insert_validator_endpoint(
                                                    &mut safe_lock(&ve_event),
                                                    addr.clone(),
                                                    host_with_port.clone(),
                                                );
                                                println!(
                                                    "🌐 Updated endpoint for existing validator: {} → {}",
                                                    get_short_addr(&addr),
                                                    host_with_port
                                                );
                                                SAVE_DIRTY.store(true, Ordering::Release);
                                            }
                                        }
                                        continue;
                                    }

                                    if balance < MIN_VALIDATOR_REGISTER_CIL {
                                        println!("🚫 VALIDATOR_REG: {} has insufficient stake ({} LOS)",
                                            get_short_addr(&addr), balance / CIL_PER_LOS);
                                        continue;
                                    }

                                    // All checks passed — register the validator on this node
                                    {
                                        let mut l = safe_lock(&ledger);
                                        if let Some(acc) = l.accounts.get_mut(&addr) {
                                            acc.is_validator = true;
                                        }
                                    }
                                    {
                                        let mut sm = safe_lock(&slashing_clone);
                                        if sm.get_profile(&addr).is_none() {
                                            sm.register_validator(addr.clone());
                                        }
                                    }
                                    {
                                        let mut rp = safe_lock(&reward_pool);
                                        rp.register_validator(&addr, false, balance);
                                    }

                                    SAVE_DIRTY.store(true, Ordering::Release);
                                    println!("✅ Validator registered via P2P: {} (stake: {} LOS)",
                                        get_short_addr(&addr), balance / CIL_PER_LOS);

                                    // Dynamically update aBFT validator set (no restart required)
                                    {
                                        let l = safe_lock(&ledger);
                                        let mut validators: Vec<String> = l
                                            .accounts
                                            .iter()
                                            .filter(|(_, a)| a.balance >= MIN_VALIDATOR_REGISTER_CIL && a.is_validator)
                                            .map(|(addr, _)| addr.clone())
                                            .collect();
                                        validators.sort();
                                        safe_lock(&abft_event).update_validator_set(validators);
                                    }

                                    // Add to address_book so heartbeats are recorded for this validator
                                    {
                                        let short = get_short_addr(&addr);
                                        safe_lock(&address_book).entry(short).or_insert(addr.clone());
                                    }

                                    // Extract and store host address for peer discovery
                                    // Accepts host_address (preferred) or onion_address (backward compat)
                                    let raw_host = reg["host_address"]
                                        .as_str()
                                        .filter(|s| !s.is_empty())
                                        .or_else(|| reg["onion_address"].as_str().filter(|s| !s.is_empty()));
                                    if let Some(h) = raw_host {
                                        // Use rest_port from gossip to ensure host has correct port
                                        let port = reg["rest_port"].as_u64().unwrap_or(3030) as u16;
                                        let host_with_port = ensure_host_port(h, port);
                                        insert_validator_endpoint(&mut safe_lock(&ve_event), addr.clone(), host_with_port.clone());
                                        println!("🌐 Discovered validator endpoint: {} → {}", get_short_addr(&addr), host_with_port);
                                    }
                                },
                                Err(e) => {
                                    println!("⚠️ VALIDATOR_REG: invalid JSON from peer: {}", e);
                                }
                            }
                        } else if let Some(json_str) = data.strip_prefix("VALIDATOR_UNREG:") {
                            // Handle validator unregistration broadcast from peers.
                            // Validates proof of ownership via Dilithium5 signature.
                            match serde_json::from_str::<serde_json::Value>(json_str) {
                                Ok(unreg) => {
                                    let addr = unreg["address"].as_str().unwrap_or_default().to_string();
                                    let pk_hex = unreg["public_key"].as_str().unwrap_or_default().to_string();
                                    let sig_hex = unreg["signature"].as_str().unwrap_or_default().to_string();
                                    let ts = unreg["timestamp"].as_u64().unwrap_or(0);

                                    // Validate address format
                                    if addr.is_empty() || !los_crypto::validate_address(&addr) {
                                        println!("🚫 VALIDATOR_UNREG: invalid address from peer");
                                        continue;
                                    }

                                    // Verify public_key derives to claimed address
                                    let pk_bytes = match hex::decode(&pk_hex) {
                                        Ok(b) => b,
                                        Err(_) => { println!("🚫 VALIDATOR_UNREG: invalid pk hex"); continue; }
                                    };
                                    if los_crypto::public_key_to_address(&pk_bytes) != addr {
                                        println!("🚫 VALIDATOR_UNREG: pk does not match address");
                                        continue;
                                    }

                                    // Verify Dilithium5 signature
                                    let message = format!("UNREGISTER_VALIDATOR:{}:{}", addr, ts);
                                    let sig_bytes = match hex::decode(&sig_hex) {
                                        Ok(b) => b,
                                        Err(_) => { println!("🚫 VALIDATOR_UNREG: invalid sig hex"); continue; }
                                    };
                                    if !los_crypto::verify_signature(message.as_bytes(), &sig_bytes, &pk_bytes) {
                                        println!("🚫 VALIDATOR_UNREG: signature verification failed for {}", get_short_addr(&addr));
                                        continue;
                                    }

                                    // Timestamp freshness (5 minute window)
                                    let now = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_secs();
                                    if ts == 0 || now.abs_diff(ts) > 300 {
                                        println!("🚫 VALIDATOR_UNREG: stale timestamp from {}", get_short_addr(&addr));
                                        continue;
                                    }

                                    // Check if address is currently a validator on this node
                                    let is_validator = {
                                        let l = safe_lock(&ledger);
                                        l.accounts.get(&addr).map(|a| a.is_validator).unwrap_or(false)
                                    };

                                    if !is_validator {
                                        // Already unregistered on this node — silently ignore
                                        continue;
                                    }

                                    // All checks passed — unregister the validator on this node
                                    {
                                        let mut l = safe_lock(&ledger);
                                        if let Some(acc) = l.accounts.get_mut(&addr) {
                                            acc.is_validator = false;
                                        }
                                    }
                                    {
                                        let mut sm = safe_lock(&slashing_clone);
                                        sm.remove_validator(&addr);
                                    }
                                    {
                                        let mut rp = safe_lock(&reward_pool);
                                        rp.unregister_validator(&addr);
                                    }
                                    // Remove from validator_endpoints
                                    {
                                        let mut ve = safe_lock(&ve_event);
                                        ve.remove(&addr);
                                    }

                                    // Update aBFT validator set
                                    {
                                        let l = safe_lock(&ledger);
                                        let mut validators: Vec<String> = l
                                            .accounts
                                            .iter()
                                            .filter(|(_, a)| a.balance >= MIN_VALIDATOR_REGISTER_CIL && a.is_validator)
                                            .map(|(addr, _)| addr.clone())
                                            .collect();
                                        validators.sort();
                                        safe_lock(&abft_event).update_validator_set(validators);
                                    }

                                    SAVE_DIRTY.store(true, Ordering::Release);
                                    println!("🔻 Validator unregistered via P2P: {}", get_short_addr(&addr));
                                },
                                Err(e) => {
                                    println!("⚠️ VALIDATOR_UNREG: invalid JSON from peer: {}", e);
                                }
                            }
                        } else if let Some(json_str) = data.strip_prefix("PEER_LIST:") {
                            // Handle Peer Exchange (PEX) — merge validator endpoints from peers
                            if let Ok(peer_list) = serde_json::from_str::<serde_json::Value>(json_str) {
                                if let Some(endpoints) = peer_list["endpoints"].as_array() {
                                    let mut ve = safe_lock(&ve_event);
                                    let mut added = 0u32;
                                    for ep in endpoints {
                                        let addr = ep["address"].as_str().unwrap_or_default();
                                        // Accept host_address (preferred) or onion_address (backward compat)
                                        let host = ep["host_address"]
                                            .as_str()
                                            .filter(|s| !s.is_empty())
                                            .or_else(|| ep["onion_address"].as_str().filter(|s| !s.is_empty()))
                                            .unwrap_or_default();
                                        // Only accept endpoints for registered
                                        // validators. The map is named `validator_endpoints` and is
                                        // used by the `/validators` API. Accepting non-validators
                                        // would pollute validator listings and enable free uptime
                                        // spoofing via PEX injection.
                                        let is_validator = {
                                            let rp = safe_lock(&reward_pool);
                                            rp.validators.contains_key(addr)
                                        };
                                        if !addr.is_empty() && !host.is_empty()
                                            && los_crypto::validate_address(addr)
                                            && is_validator
                                        {
                                            let is_new = !ve.contains_key(addr);
                                            insert_validator_endpoint(&mut ve, addr.to_string(), host.to_string());
                                            if is_new { added += 1; }
                                        }
                                    }
                                    if added > 0 {
                                        println!("🔄 PEX: merged {} new validator endpoint(s) from peer", added);
                                    }
                                }
                            }
                        } else if data.starts_with("BLOCK_CONFIRMED:") {
                            // CROSS-NODE STATE PROPAGATION: Consensus-confirmed Send+Receive blocks.
                            // The originating node broadcasts this after consensus finalization.
                            // Peers apply via direct ledger manipulation (debit sender, credit recipient)
                            // without process_block() chain-sequence validation — ensures consistency
                            // even if local chains diverge (e.g., independent faucet/mint ops).
                            let parts: Vec<&str> = data.splitn(3, ':').collect();
                            if parts.len() == 3 {
                                let send_block: Option<Block> = base64::engine::general_purpose::STANDARD.decode(parts[1]).ok()
                                    .and_then(|bytes| serde_json::from_slice(&bytes).ok());
                                let recv_block: Option<Block> = base64::engine::general_purpose::STANDARD.decode(parts[2]).ok()
                                    .and_then(|bytes| serde_json::from_slice(&bytes).ok());

                                if let (Some(send_blk), Some(recv_blk)) = (send_block, recv_block) {
                                    let send_hash = send_blk.calculate_hash();

                                    // Validate Send block: signature + PoW + must be Send type
                                    let send_valid = send_blk.block_type == BlockType::Send
                                        && send_blk.verify_signature()
                                        && send_blk.verify_pow();
                                    // Validate Receive block: signature + PoW + must be Receive type + amounts match + link matches Send hash
                                    // Added recv_blk.verify_signature() — without this,
                                    // a malicious node could broadcast forged Receive blocks to credit
                                    // arbitrary accounts.
                                    let recv_valid = recv_blk.block_type == BlockType::Receive
                                        && recv_blk.verify_signature()
                                        && recv_blk.verify_pow()
                                        && recv_blk.amount == send_blk.amount
                                        && recv_blk.link == send_hash
                                        && recv_blk.account == send_blk.link;

                                    if !send_valid || !recv_valid {
                                        println!("🚫 Rejected BLOCK_CONFIRMED: validation failed (send={}, recv={})", send_valid, recv_valid);
                                    } else {
                                        let mut l = safe_lock(&ledger);
                                        // Idempotency: skip if already applied
                                        if l.blocks.contains_key(&send_hash) {
                                            // Already have this block — skip
                                        } else {
                                            let recv_hash = recv_blk.calculate_hash();

                                            // Apply Send: debit sender
                                            if !l.accounts.contains_key(&send_blk.account) {
                                                l.accounts.insert(send_blk.account.clone(), AccountState {
                                                    head: "0".to_string(), balance: 0, block_count: 0, is_validator: false,
                                                });
                                            }
                                            // Chain-sequence + balance validation
                                            // for BLOCK_CONFIRMED. Without these checks, a malicious
                                            // originator could broadcast conflicting BLOCK_CONFIRMED
                                            // messages (double-spend) — receiving nodes would apply
                                            // both via saturating_sub, creating money from nothing.
                                            let send_rejected = if let Some(sender) = l.accounts.get(&send_blk.account) {
                                                let total_debit = send_blk.amount.saturating_add(send_blk.fee);
                                                if sender.head != send_blk.previous {
                                                    // DESIGN Log fork event with diagnostic info.
                                                    // With D-2 (all sends go through consensus), forks should
                                                    // be extremely rare. When they occur, it indicates either:
                                                    // 1. Network partition recovery (benign)
                                                    // 2. Message reordering over Tor (benign)
                                                    // 3. Attempted double-spend (D-2 makes this fail at consensus)
                                                    // We use deterministic hash ordering for logging but don't
                                                    // attempt rollback (too risky in block-lattice architecture).
                                                    let existing_hash = &sender.head;
                                                    let incoming_hash = &send_hash;
                                                    let canonical_winner = if incoming_hash < existing_hash { "incoming" } else { "existing" };
                                                    println!("🚫 FORK DETECTED (BLOCK_CONFIRMED send): \
                                                        account={}, existing_head={}, incoming_prev={}, \
                                                        canonical_winner={} (deterministic hash order)",
                                                        get_short_addr(&send_blk.account),
                                                        get_short_addr(existing_hash),
                                                        get_short_addr(&send_blk.previous),
                                                        canonical_winner);
                                                    json_event!("fork_detected",
                                                        "type" => "send",
                                                        "account" => get_short_addr(&send_blk.account),
                                                        "existing_head" => get_short_addr(existing_hash),
                                                        "incoming_prev" => get_short_addr(&send_blk.previous),
                                                        "canonical_winner" => canonical_winner
                                                    );
                                                    true
                                                } else if sender.balance < total_debit {
                                                    println!("🚫 Rejected BLOCK_CONFIRMED: insufficient sender \
                                                        balance ({} < {}) for {}",
                                                        sender.balance, total_debit,
                                                        get_short_addr(&send_blk.account));
                                                    true
                                                } else {
                                                    false
                                                }
                                            } else {
                                                false // New account — will be created below
                                            };
                                            if send_rejected {
                                                // Skip this BLOCK_CONFIRMED entirely
                                            } else {
                                            if let Some(sender) = l.accounts.get_mut(&send_blk.account) {
                                                let total_debit = send_blk.amount.saturating_add(send_blk.fee);
                                                sender.balance -= total_debit; // Safe: checked above
                                                sender.head = send_hash.clone();
                                                sender.block_count += 1;
                                            }
                                            // Track fees for validator redistribution
                                            l.accumulated_fees_cil = l.accumulated_fees_cil.saturating_add(send_blk.fee);
                                            l.blocks.insert(send_hash.clone(), send_blk.clone());

                                            // Apply Receive: credit recipient
                                            if !l.accounts.contains_key(&recv_blk.account) {
                                                l.accounts.insert(recv_blk.account.clone(), AccountState {
                                                    head: "0".to_string(), balance: 0, block_count: 0, is_validator: false,
                                                });
                                            }
                                            // Validate Receive chain head.
                                            // Without this, a malicious originator could broadcast
                                            // conflicting BLOCK_CONFIRMED messages that overwrite
                                            // the recipient's chain head with stale references.
                                            let recv_rejected = if let Some(recv_acct) = l.accounts.get(&recv_blk.account) {
                                                if recv_acct.head != recv_blk.previous {
                                                    // DESIGN Log recv fork with deterministic ordering info
                                                    let existing_hash = &recv_acct.head;
                                                    let incoming_prev = &recv_blk.previous;
                                                    let canonical_winner = if incoming_prev < existing_hash { "incoming" } else { "existing" };
                                                    println!("🚫 FORK DETECTED (BLOCK_CONFIRMED recv): \
                                                        recipient={}, existing_head={}, incoming_prev={}, \
                                                        canonical_winner={} (deterministic hash order)",
                                                        get_short_addr(&recv_blk.account),
                                                        get_short_addr(existing_hash),
                                                        get_short_addr(incoming_prev),
                                                        canonical_winner);
                                                    json_event!("fork_detected",
                                                        "type" => "recv",
                                                        "account" => get_short_addr(&recv_blk.account),
                                                        "existing_head" => get_short_addr(existing_hash),
                                                        "incoming_prev" => get_short_addr(incoming_prev),
                                                        "canonical_winner" => canonical_winner
                                                    );
                                                    true
                                                } else {
                                                    false
                                                }
                                            } else {
                                                false
                                            };
                                            if !recv_rejected {
                                                if let Some(recipient) = l.accounts.get_mut(&recv_blk.account) {
                                                    recipient.balance = recipient.balance.saturating_add(recv_blk.amount);
                                                    recipient.head = recv_hash.clone();
                                                    recipient.block_count += 1;
                                                }
                                                l.blocks.insert(recv_hash, recv_blk.clone());
                                                // Track claimed Send for double-receive prevention.
                                                // BLOCK_CONFIRMED bypasses process_block(); without this insert,
                                                // a subsequent Receive via process_block() could re-claim the
                                                // same Send (claimed_sends check would return false).
                                                l.claimed_sends.insert(send_hash.clone());

                                                SAVE_DIRTY.store(true, Ordering::Release);
                                                println!("✅ Applied BLOCK_CONFIRMED: {} → {} ({} CIL)",
                                                    get_short_addr(&send_blk.account), get_short_addr(&recv_blk.account), send_blk.amount);
                                            } else {
                                                // Revert the send debit since recv chain validation failed
                                                if let Some(sender) = l.accounts.get_mut(&send_blk.account) {
                                                    let total_debit = send_blk.amount.saturating_add(send_blk.fee);
                                                    sender.balance = sender.balance.saturating_add(total_debit);
                                                    sender.head = send_blk.previous.clone();
                                                    sender.block_count = sender.block_count.saturating_sub(1);
                                                }
                                                l.accumulated_fees_cil = l.accumulated_fees_cil.saturating_sub(send_blk.fee);
                                                l.blocks.remove(&send_hash);
                                            }
                                        } // end if !send_rejected
                                        }
                                    }
                                }
                            }
                        } else if data.starts_with("CONTRACT_DEPLOYED:") {
                            // CROSS-NODE CONTRACT REPLICATION
                            // Format: CONTRACT_DEPLOYED:{block_b64}:{bytecode_b64}:{contract_addr}
                            let parts: Vec<&str> = data.splitn(4, ':').collect();
                            if parts.len() == 4 {
                                let block_opt: Option<Block> = base64::engine::general_purpose::STANDARD
                                    .decode(parts[1]).ok()
                                    .and_then(|bytes| serde_json::from_slice(&bytes).ok());
                                let bytecode_opt = base64::engine::general_purpose::STANDARD.decode(parts[2]).ok();
                                let _contract_addr = parts[3].to_string();

                                if let (Some(deploy_blk), Some(bytecode)) = (block_opt, bytecode_opt) {
                                    // Validate: must be ContractDeploy + valid sig + valid PoW
                                    let valid = deploy_blk.block_type == BlockType::ContractDeploy
                                        && deploy_blk.verify_signature()
                                        && deploy_blk.verify_pow()
                                        && deploy_blk.link.starts_with("DEPLOY:");

                                    if !valid {
                                        println!("🚫 Rejected CONTRACT_DEPLOYED: validation failed");
                                    } else {
                                        let deploy_hash = deploy_blk.calculate_hash();
                                        let mut l = safe_lock(&ledger);
                                        if !l.blocks.contains_key(&deploy_hash) {
                                            // Ensure deployer account exists
                                            if !l.accounts.contains_key(&deploy_blk.account) {
                                                l.accounts.insert(deploy_blk.account.clone(), AccountState {
                                                    head: "0".to_string(), balance: 0, block_count: 0, is_validator: false,
                                                });
                                            }
                                            // Chain-sequence + balance validation
                                            // for CONTRACT_DEPLOYED — same pattern as BLOCK_CONFIRMED fix.
                                            let deploy_rejected = if let Some(deployer) = l.accounts.get(&deploy_blk.account) {
                                                let total_debit = deploy_blk.amount.saturating_add(deploy_blk.fee);
                                                if deployer.head != deploy_blk.previous {
                                                    println!("🚫 Rejected CONTRACT_DEPLOYED: chain fork \
                                                        (deployer={}, head={}, block.previous={})",
                                                        get_short_addr(&deploy_blk.account),
                                                        get_short_addr(&deployer.head),
                                                        get_short_addr(&deploy_blk.previous));
                                                    true
                                                } else if deployer.balance < total_debit {
                                                    println!("🚫 Rejected CONTRACT_DEPLOYED: insufficient \
                                                        deployer balance ({} < {})",
                                                        deployer.balance, total_debit);
                                                    true
                                                } else {
                                                    false
                                                }
                                            } else {
                                                false
                                            };
                                            if !deploy_rejected {
                                                if let Some(deployer) = l.accounts.get_mut(&deploy_blk.account) {
                                                    let total_debit = deploy_blk.amount.saturating_add(deploy_blk.fee);
                                                    deployer.balance -= total_debit; // Safe: checked above
                                                    deployer.head = deploy_hash.clone();
                                                    deployer.block_count += 1;
                                                }
                                                l.accumulated_fees_cil = l.accumulated_fees_cil.saturating_add(deploy_blk.fee);
                                                l.blocks.insert(deploy_hash, deploy_blk.clone());
                                                drop(l); // Release ledger lock before VM operations

                                                // Deploy to local WASM engine
                                                let code_hash = WasmEngine::compute_code_hash(&bytecode);
                                                let expected_hash = &deploy_blk.link[7..]; // After "DEPLOY:"
                                                if code_hash.starts_with(expected_hash) || expected_hash.starts_with(&code_hash[..expected_hash.len().min(code_hash.len())]) {
                                                    let now_ts = std::time::SystemTime::now()
                                                        .duration_since(std::time::UNIX_EPOCH)
                                                        .unwrap_or_default()
                                                        .as_secs();
                                                    match wasm_engine.deploy_contract(
                                                        deploy_blk.account.clone(),
                                                        bytecode,
                                                        BTreeMap::new(),
                                                        now_ts,
                                                    ) {
                                                        Ok(addr) => {
                                                            // Fund contract if amount > 0
                                                            if deploy_blk.amount > 0 {
                                                                let _ = wasm_engine.send_to_contract(&addr, deploy_blk.amount);
                                                            }
                                                            // Persist VM state
                                                            if let Ok(vm_data) = wasm_engine.serialize_all() {
                                                                let _ = database.save_contracts(&vm_data);
                                                            }
                                                            println!("✅ Replicated CONTRACT_DEPLOYED: {} (owner: {})",
                                                                addr, get_short_addr(&deploy_blk.account));
                                                        }
                                                        Err(e) => eprintln!("⚠️ Failed to replicate contract deploy: {}", e),
                                                    }
                                                } else {
                                                    eprintln!("🚫 CONTRACT_DEPLOYED: code hash mismatch");
                                                }

                                                SAVE_DIRTY.store(true, Ordering::Release);
                                            } // end if !deploy_rejected
                                        }
                                    }
                                }
                            }
                        } else if data.starts_with("CONTRACT_CALLED:") {
                            // CROSS-NODE CONTRACT CALL REPLICATION
                            // Format: CONTRACT_CALLED:{block_b64}
                            let parts: Vec<&str> = data.splitn(2, ':').collect();
                            if parts.len() == 2 {
                                let block_opt: Option<Block> = base64::engine::general_purpose::STANDARD
                                    .decode(parts[1]).ok()
                                    .and_then(|bytes| serde_json::from_slice(&bytes).ok());

                                if let Some(call_blk) = block_opt {
                                    let valid = call_blk.block_type == BlockType::ContractCall
                                        && call_blk.verify_signature()
                                        && call_blk.verify_pow()
                                        && call_blk.link.starts_with("CALL:");

                                    if !valid {
                                        println!("🚫 Rejected CONTRACT_CALLED: validation failed");
                                    } else {
                                        let call_hash = call_blk.calculate_hash();
                                        let mut l = safe_lock(&ledger);
                                        if !l.blocks.contains_key(&call_hash) {
                                            // Ensure caller account exists
                                            if !l.accounts.contains_key(&call_blk.account) {
                                                l.accounts.insert(call_blk.account.clone(), AccountState {
                                                    head: "0".to_string(), balance: 0, block_count: 0, is_validator: false,
                                                });
                                            }
                                            // Chain-sequence + balance validation
                                            // for CONTRACT_CALLED — same pattern as BLOCK_CONFIRMED fix.
                                            let call_rejected = if let Some(caller) = l.accounts.get(&call_blk.account) {
                                                let total_debit = call_blk.amount.saturating_add(call_blk.fee);
                                                if caller.head != call_blk.previous {
                                                    println!("🚫 Rejected CONTRACT_CALLED: chain fork \
                                                        (caller={}, head={}, block.previous={})",
                                                        get_short_addr(&call_blk.account),
                                                        get_short_addr(&caller.head),
                                                        get_short_addr(&call_blk.previous));
                                                    true
                                                } else if caller.balance < total_debit {
                                                    println!("🚫 Rejected CONTRACT_CALLED: insufficient \
                                                        caller balance ({} < {})",
                                                        caller.balance, total_debit);
                                                    true
                                                } else {
                                                    false
                                                }
                                            } else {
                                                false
                                            };
                                            if !call_rejected {
                                                if let Some(caller_acct) = l.accounts.get_mut(&call_blk.account) {
                                                    let total_debit = call_blk.amount.saturating_add(call_blk.fee);
                                                    caller_acct.balance -= total_debit; // Safe: checked above
                                                    caller_acct.head = call_hash.clone();
                                                    caller_acct.block_count += 1;
                                                }
                                                l.accumulated_fees_cil = l.accumulated_fees_cil.saturating_add(call_blk.fee);
                                                l.blocks.insert(call_hash, call_blk.clone());
                                                drop(l);

                                                // Parse call data from link: "CALL:{addr}:{func}:{args_b64}"
                                                let call_data = &call_blk.link[5..];
                                                let call_parts: Vec<&str> = call_data.splitn(3, ':').collect();
                                                if call_parts.len() >= 2 {
                                                    let contract_addr = call_parts[0];
                                                    let function = call_parts[1];
                                                    let args: Vec<String> = if call_parts.len() == 3 {
                                                        base64::engine::general_purpose::STANDARD
                                                            .decode(call_parts[2]).ok()
                                                            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                                                            .unwrap_or_default()
                                                    } else {
                                                        Vec::new()
                                                    };
                                                    let gas_limit = call_blk.fee / los_core::GAS_PRICE_CIL.max(1);

                                                    // Value transfer to contract
                                                    if call_blk.amount > 0 {
                                                        let _ = wasm_engine.send_to_contract(contract_addr, call_blk.amount);
                                                    }

                                                    // Execute deterministically (same result on all nodes)
                                                    let call = ContractCall {
                                                        contract: contract_addr.to_string(),
                                                        function: function.to_string(),
                                                        args,
                                                        gas_limit: gas_limit as u64,
                                                        caller: call_blk.account.clone(),
                                                        block_timestamp: call_blk.timestamp,
                                                    };
                                                    match wasm_engine.call_contract(call) {
                                                        Ok(result) => {
                                                            if let Ok(vm_data) = wasm_engine.serialize_all() {
                                                                let _ = database.save_contracts(&vm_data);
                                                            }
                                                            println!("✅ Replicated CONTRACT_CALLED: {}::{} → {}",
                                                                contract_addr, function,
                                                                if result.success { "OK" } else { "FAIL" });
                                                        }
                                                        Err(e) => eprintln!("⚠️ Failed to replicate contract call: {}", e),
                                                    }
                                                }

                                                SAVE_DIRTY.store(true, Ordering::Release);
                                            } // end if !call_rejected
                                        }
                                    }
                                }
                            }                        } else if let Some(rest) = data.strip_prefix("CHECKPOINT_PROPOSE:") {
                            // ── DESIGN Multi-validator checkpoint coordination ──
                            // Format: CHECKPOINT_PROPOSE:<height>:<block_hash>:<state_root>:<proposer>:<sig_hex>
                            // When we receive a checkpoint proposal, verify our state matches,
                            // sign the checkpoint data, and broadcast CHECKPOINT_SIGN back.
                            let parts: Vec<&str> = rest.splitn(5, ':').collect();
                            if parts.len() == 5 {
                                if let Ok(height) = parts[0].parse::<u64>() {
                                    let block_hash = parts[1];
                                    let state_root = parts[2];
                                    let proposer = parts[3];
                                    let sig_hex = parts[4];

                                    // Skip our own proposals
                                    if proposer == my_address {
                                        continue;
                                    }

                                    // Verify our state root matches the proposal
                                    let our_state_root = {
                                        let l = safe_lock(&ledger);
                                        l.compute_state_root()
                                    };

                                    if our_state_root == state_root {
                                        // State matches — sign the checkpoint
                                        let cp = FinalityCheckpoint::new(
                                            height,
                                            block_hash.to_string(),
                                            1, // validator_count filled later
                                            state_root.to_string(),
                                            vec![],
                                        );
                                        let signing_data = cp.signing_data();
                                        if let Ok(my_sig) = los_crypto::sign_message(&signing_data, &secret_key) {
                                            let my_sig_hex = hex::encode(&my_sig);
                                            let sign_msg = format!(
                                                "CHECKPOINT_SIGN:{}:{}:{}:{}:{}",
                                                height, block_hash, state_root, my_address, my_sig_hex
                                            );
                                            let _ = tx_out.send(sign_msg).await;
                                            println!("✍️ Signed checkpoint proposal at height {} from {}",
                                                height, &proposer[..proposer.len().min(16)]);
                                        }

                                        // Also add the proposer's signature to our pending map
                                        // SECURITY: Verify proposer's Dilithium5 signature first
                                        if let Ok(proposer_sig) = hex::decode(sig_hex) {
                                            let proposer_pk: Option<Vec<u8>> = {
                                                let l = safe_lock(&ledger);
                                                l.accounts.get(proposer).and_then(|acc| {
                                                    l.blocks.get(&acc.head).and_then(|blk| {
                                                        hex::decode(&blk.public_key).ok()
                                                    })
                                                })
                                            };

                                            let proposer_verified = if let Some(pk_bytes) = proposer_pk {
                                                let cp_verify = FinalityCheckpoint::new(
                                                    height,
                                                    block_hash.to_string(),
                                                    1,
                                                    state_root.to_string(),
                                                    vec![],
                                                );
                                                los_crypto::verify_signature(&cp_verify.signing_data(), &proposer_sig, &pk_bytes)
                                            } else {
                                                false
                                            };

                                            if !proposer_verified {
                                                println!("🚫 Rejected CHECKPOINT_PROPOSE: unverified proposer sig from {}", &proposer[..proposer.len().min(16)]);
                                            } else {
                                                let mut pcp = safe_lock(&pending_checkpoints);
                                            let pending = pcp.entry(height).or_insert_with(|| {
                                                let vc = {
                                                    let l = safe_lock(&ledger);
                                                    l.accounts.iter()
                                                        .filter(|(_, a)| a.balance >= MIN_VALIDATOR_STAKE_CIL)
                                                        .count() as u32
                                                };
                                                PendingCheckpoint::new(FinalityCheckpoint::new(
                                                    height,
                                                    block_hash.to_string(),
                                                    vc.max(1),
                                                    state_root.to_string(),
                                                    vec![],
                                                ))
                                            });
                                            pending.add_signature(CheckpointSignature {
                                                validator_address: proposer.to_string(),
                                                signature: proposer_sig,
                                            });
                                            } // end proposer_verified
                                        }
                                    } else {
                                        println!("⚠️ Checkpoint proposal state mismatch at height {} (ours={}, theirs={})",
                                            height, &our_state_root[..16], &state_root[..state_root.len().min(16)]);
                                    }
                                }
                            }
                        } else if let Some(rest) = data.strip_prefix("CHECKPOINT_SIGN:") {
                            // ── DESIGN Collect checkpoint signatures from peers ──
                            // Format: CHECKPOINT_SIGN:<height>:<block_hash>:<state_root>:<signer>:<sig_hex>
                            let parts: Vec<&str> = rest.splitn(5, ':').collect();
                            if parts.len() == 5 {
                                if let Ok(height) = parts[0].parse::<u64>() {
                                    let block_hash_cp = parts[1];
                                    let state_root_cp = parts[2];
                                    let signer = parts[3];
                                    let sig_hex = parts[4];

                                    if signer == my_address {
                                        continue; // Skip our own signatures
                                    }

                                    if let Ok(sig_bytes) = hex::decode(sig_hex) {
                                        // SECURITY: Verify the Dilithium5 signature before accepting.
                                        // Without this, an attacker can forge signatures for any signer
                                        // and reach quorum trivially — enabling checkpoint manipulation.
                                        //
                                        // Step 1: Look up signer's public key from their head block
                                        let signer_pk: Option<Vec<u8>> = {
                                            let l = safe_lock(&ledger);
                                            l.accounts.get(signer).and_then(|acc| {
                                                l.blocks.get(&acc.head).and_then(|blk| {
                                                    hex::decode(&blk.public_key).ok()
                                                })
                                            })
                                        };

                                        let pk_bytes = match signer_pk {
                                            Some(pk) => pk,
                                            None => {
                                                // Unknown signer (no blocks) — reject
                                                println!("🚫 Rejected CHECKPOINT_SIGN: unknown signer {} (no blocks found)", &signer[..signer.len().min(16)]);
                                                continue;
                                            }
                                        };

                                        // Step 2: Verify the signature over the checkpoint signing data
                                        let cp = FinalityCheckpoint::new(
                                            height,
                                            block_hash_cp.to_string(),
                                            1,
                                            state_root_cp.to_string(),
                                            vec![],
                                        );
                                        let signing_data = cp.signing_data();
                                        if !los_crypto::verify_signature(&signing_data, &sig_bytes, &pk_bytes) {
                                            println!("🚫 Rejected CHECKPOINT_SIGN: invalid signature from {}", &signer[..signer.len().min(16)]);
                                            continue;
                                        }

                                        let mut pcp = safe_lock(&pending_checkpoints);
                                        if let Some(pending) = pcp.get_mut(&height) {
                                            let was_new = pending.add_signature(CheckpointSignature {
                                                validator_address: signer.to_string(),
                                                signature: sig_bytes,
                                            });
                                            if was_new && pending.has_quorum() {
                                                // Quorum reached — finalize checkpoint!
                                                let finalized = pending.checkpoint.clone();
                                                let sig_count = finalized.signature_count;
                                                let vc = finalized.validator_count;
                                                drop(pcp); // Release lock before acquiring checkpoint_manager
                                                let mut cm = safe_lock(&checkpoint_manager);
                                                match cm.store_checkpoint(finalized) {
                                                    Ok(()) => {
                                                        println!("🏁 Checkpoint FINALIZED at height {} (sig_count={}/{}, quorum reached!)",
                                                            height, sig_count, vc);
                                                        // Remove from pending
                                                        let mut pcp = safe_lock(&pending_checkpoints);
                                                        pcp.remove(&height);
                                                    }
                                                    Err(e) => eprintln!("⚠️ Checkpoint finalization failed at {}: {}", height, e),
                                                }
                                            } else if was_new {
                                                let sc = pending.checkpoint.signature_count;
                                                let vc = pending.checkpoint.validator_count;
                                                println!("✍️ Collected checkpoint signature at height {} ({}/{})",
                                                    height, sc, vc);
                                            }
                                        }
                                    }
                                }
                            }
                        } else if let Some(rest) = data.strip_prefix("MINE_BLOCK:") {
                            // ── PoW MINT: Replicate a mined Mint block from another node ──
                            // Full verification of mining PoW proof hash,
                            // epoch validity, double-mining, and reward amount bounds.
                            // Previously only checked signature + anti-spam PoW — a malicious
                            // node could craft a MINE_BLOCK with a fake nonce/amount.
                            if let Ok(mint_blk) = serde_json::from_str::<Block>(rest) {
                                // Must be a Mint block with MINE: link
                                if mint_blk.block_type != BlockType::Mint || !mint_blk.link.starts_with("MINE:") {
                                    println!("🚫 Rejected MINE_BLOCK: not a MINE: Mint block");
                                    continue;
                                }
                                // Genesis bootstrap validators cannot mine.
                                // All mining rewards are reserved for public miners.
                                if bootstrap_validators.contains(&mint_blk.account) {
                                    println!("🚫 Rejected MINE_BLOCK: genesis bootstrap validator {} cannot mine",
                                        get_short_addr(&mint_blk.account));
                                    continue;
                                }
                                // Verify signature
                                if mint_blk.signature.is_empty() || mint_blk.public_key.is_empty() {
                                    println!("🚫 Rejected unsigned MINE_BLOCK from P2P");
                                    continue;
                                }
                                let sig_ok = hex::decode(&mint_blk.signature).ok().and_then(|sig| {
                                    hex::decode(&mint_blk.public_key).ok().map(|pk| {
                                        let signing_hash = mint_blk.signing_hash();
                                        los_crypto::verify_signature(signing_hash.as_bytes(), &sig, &pk)
                                    })
                                }).unwrap_or(false);
                                if !sig_ok {
                                    println!("🚫 Rejected MINE_BLOCK: invalid signature");
                                    continue;
                                }
                                if !mint_blk.verify_pow() {
                                    println!("🚫 Rejected MINE_BLOCK: invalid PoW");
                                    continue;
                                }

                                // ── SEC-MINE-01: Parse and verify mining PoW proof ──
                                // Link format: "MINE:{epoch}:{nonce}"
                                let link_parts: Vec<&str> = mint_blk.link.splitn(3, ':').collect();
                                if link_parts.len() != 3 {
                                    println!("🚫 Rejected MINE_BLOCK: malformed link '{}'", &mint_blk.link);
                                    continue;
                                }
                                let proof_epoch: u64 = match link_parts[1].parse() {
                                    Ok(e) => e,
                                    Err(_) => {
                                        println!("🚫 Rejected MINE_BLOCK: invalid epoch in link");
                                        continue;
                                    }
                                };
                                let proof_nonce: u64 = match link_parts[2].parse() {
                                    Ok(n) => n,
                                    Err(_) => {
                                        println!("🚫 Rejected MINE_BLOCK: invalid nonce in link");
                                        continue;
                                    }
                                };

                                // Verify the PoW hash meets difficulty
                                let difficulty_bits = {
                                    let ms = safe_lock(&mining_state);
                                    ms.difficulty_bits
                                };
                                if !verify_mining_hash(&mint_blk.account, proof_epoch, proof_nonce, difficulty_bits) {
                                    println!("🚫 Rejected MINE_BLOCK: PoW hash does not meet difficulty ({} bits) for {}",
                                        difficulty_bits, get_short_addr(&mint_blk.account));
                                    continue;
                                }

                                // Verify epoch is current or very recent (within ±1 epoch tolerance
                                // for network propagation delay)
                                let now_secs = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs();
                                let current_epoch = {
                                    let ms = safe_lock(&mining_state);
                                    ms.epoch_from_time(now_secs)
                                };
                                if proof_epoch > current_epoch + 1 {
                                    println!("🚫 Rejected MINE_BLOCK: future epoch {} (current: {})",
                                        proof_epoch, current_epoch);
                                    continue;
                                }
                                // Allow blocks from recent past (up to 2 epochs back) for propagation delay
                                if current_epoch > 2 && proof_epoch < current_epoch - 2 {
                                    println!("🚫 Rejected MINE_BLOCK: stale epoch {} (current: {})",
                                        proof_epoch, current_epoch);
                                    continue;
                                }

                                // Verify reward amount is within bounds (max 1,000 LOS per block,
                                // and no more than the epoch reward)
                                let max_mint_cil = 1_000 * CIL_PER_LOS;
                                let epoch_reward = MiningState::epoch_reward_cil(proof_epoch);
                                if mint_blk.amount > max_mint_cil || mint_blk.amount > epoch_reward {
                                    println!("🚫 Rejected MINE_BLOCK: reward {} exceeds max (cap: {}, epoch: {})",
                                        mint_blk.amount, max_mint_cil, epoch_reward);
                                    continue;
                                }
                                if mint_blk.amount == 0 {
                                    println!("🚫 Rejected MINE_BLOCK: zero reward amount");
                                    continue;
                                }

                                // Idempotency: skip if already processed
                                let hash = mint_blk.calculate_hash();
                                {
                                    let l = safe_lock(&ledger);
                                    if l.blocks.contains_key(&hash) {
                                        continue;
                                    }
                                }

                                // Register miner in mining state (double-mining check)
                                {
                                    let mut ms = safe_lock(&mining_state);
                                    // Advance epoch if needed
                                    ms.maybe_advance_epoch(now_secs);
                                    if ms.current_epoch_miners.contains(&mint_blk.account) {
                                        println!("🚫 Rejected MINE_BLOCK: {} already mined epoch {}",
                                            get_short_addr(&mint_blk.account), proof_epoch);
                                        continue;
                                    }
                                    ms.current_epoch_miners.insert(mint_blk.account.clone());
                                }

                                // Process the Mint block
                                {
                                    let mut l = safe_lock(&ledger);
                                    if !l.accounts.contains_key(&mint_blk.account) {
                                        l.accounts.insert(mint_blk.account.clone(), AccountState {
                                            head: "0".to_string(), balance: 0, block_count: 0, is_validator: false
                                        });
                                    }
                                    match l.process_block(&mint_blk) {
                                        Ok(_) => {
                                            SAVE_DIRTY.store(true, Ordering::Release);
                                            let reward_los = mint_blk.amount / CIL_PER_LOS;
                                            println!("⛏️  Replicated MINE_BLOCK: {} → {} LOS (epoch {})",
                                                get_short_addr(&mint_blk.account), reward_los, proof_epoch);
                                            if let Err(e) = database.save_block(&hash, &mint_blk) {
                                                eprintln!("⚠️ DB save error for replicated mine block: {}", e);
                                            }
                                        }
                                        Err(e) => {
                                            // Chain sequence error — don't force-insert (creates orphaned ghost blocks).
                                            // The block will be recovered via periodic SYNC_REQUEST or REST sync
                                            // which delivers the complete ordered state including missed blocks.
                                            let mut ms = safe_lock(&mining_state);
                                            ms.current_epoch_miners.remove(&mint_blk.account);
                                            if e.contains("Chain Error") {
                                                println!("⚠️ MINE_BLOCK chain gap for {} epoch {} — will recover via sync",
                                                    get_short_addr(&mint_blk.account), proof_epoch);
                                            } else {
                                                println!("❌ Failed to replicate MINE_BLOCK: {}", e);
                                            }
                                        }
                                    }
                                }
                            }
                        } else if let Ok(inc) = serde_json::from_str::<Block>(&data) {
                            // Mint/Slash blocks from P2P are accepted ONLY if they
                            // carry a valid validator signature + valid PoW. Previously blanket-
                            // rejected, which caused minted tokens to exist only on the originating
                            // node — splitting the ledger permanently across the network.
                            if matches!(inc.block_type, BlockType::Mint | BlockType::Slash) {
                                // Verify: non-empty signature, valid signature, valid PoW
                                if inc.signature.is_empty() || inc.public_key.is_empty() {
                                    println!("🚫 Rejected unsigned {:?} block from P2P", inc.block_type);
                                    continue;
                                }
                                let sig_ok = hex::decode(&inc.signature).ok().and_then(|sig| {
                                    hex::decode(&inc.public_key).ok().map(|pk| {
                                        let signing_hash = inc.signing_hash();
                                        los_crypto::verify_signature(signing_hash.as_bytes(), &sig, &pk)
                                    })
                                }).unwrap_or(false);
                                if !sig_ok {
                                    println!("🚫 Rejected {:?} block from P2P: invalid signature", inc.block_type);
                                    continue;
                                }
                                if !inc.verify_pow() {
                                    println!("🚫 Rejected {:?} block from P2P: invalid PoW", inc.block_type);
                                    continue;
                                }
                                // Also verify the signing key maps to a known staked validator.
                                // In dev/testnet mode, skip this check for Mint blocks to avoid
                                // the chicken-and-egg problem: faucet Mints can't propagate because
                                // the signer isn't funded on peers, but funding requires Mints.
                                // Mainnet has no faucet — genesis provides initial validator balances.
                                let is_dev_mode = testnet_config::get_testnet_config().enable_faucet;
                                if !is_dev_mode || inc.block_type == BlockType::Slash {
                                    let signer_addr = hex::decode(&inc.public_key)
                                        .map(|pk| los_crypto::public_key_to_address(&pk))
                                        .unwrap_or_default();
                                    let is_validator = {
                                        let l = safe_lock(&ledger);
                                        l.accounts.get(&signer_addr)
                                            .map(|a| a.balance >= MIN_VALIDATOR_REGISTER_CIL)
                                            .unwrap_or(false)
                                    };
                                    if !is_validator {
                                        println!("🚫 Rejected {:?} block from P2P: signer {} is not a staked validator", inc.block_type, get_short_addr(&signer_addr));
                                        continue;
                                    }
                                }
                                // Signature + PoW [+ staked validator check in mainnet] → accept into ledger
                            }

                            // IDEMPOTENCY: Skip if block already in ledger (received via state sync
                            // or prior gossip). Must check BEFORE double-signing detection, otherwise
                            // the same block arriving via both state sync and gossip triggers a false
                            // positive double-sign slash.
                            let block_hash = inc.calculate_hash();
                            {
                                let l = safe_lock(&ledger);
                                if l.blocks.contains_key(&block_hash) {
                                    continue; // Already applied
                                }
                            }

                            // SLASHING: Check for double-signing before processing
                            let timestamp = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();

                            // Phase 1: Account init + double-sign detection + optional slash (all synchronous)
                            let (double_sign_detected, ds_gossip) = {
                                let mut l = safe_lock(&ledger);
                                if !l.accounts.contains_key(&inc.account) {
                                    l.accounts.insert(inc.account.clone(), AccountState { head: "0".to_string(), balance: 0, block_count: 0, is_validator: false });
                                }

                                // Skip double-sign detection for SYSTEM-CREATED blocks (Mint, Slash).
                                // These blocks are created by the epoch leader or validators themselves,
                                // NOT by the account owner. When reward + fee_reward blocks arrive
                                // for the same validator at the same block_count via gossip, the
                                // second triggers a false "different hash at same height" detection.
                                // True double-signing only applies to user-initiated blocks (Send, Change)
                                // where the account owner signs two conflicting blocks at the same height.
                                let is_system_block = matches!(inc.block_type, BlockType::Mint | BlockType::Slash);

                                let double_sign_detected = if is_system_block {
                                    false // System blocks cannot be "double-signed" by the account
                                } else {
                                    let mut sm = safe_lock(&slashing_clone);
                                    // Register validator if not exists (only if flagged as validator)
                                    if sm.get_profile(&inc.account).is_none() {
                                        if let Some(acc) = l.accounts.get(&inc.account) {
                                            if acc.is_validator {
                                                sm.register_validator(inc.account.clone());
                                            }
                                        }
                                    }

                                    // Only check double-signing for registered validators.
                                    // Non-validators (wallets, faucet recipients) can't double-sign.
                                    // record_signature returns Err("not registered") for non-validators
                                    // which was incorrectly treated as double-signing via is_err().
                                    if sm.get_profile(&inc.account).is_some() {
                                        let block_height = l.accounts.get(&inc.account)
                                            .map(|a| a.block_count)
                                            .unwrap_or(0);
                                        sm.record_signature(&inc.account, block_height, block_hash.clone(), timestamp).is_err()
                                    } else {
                                        false // Non-validator — skip double-sign detection
                                    }
                                };

                                let mut gossip = None;
                                if double_sign_detected {
                                    println!("🚨 DOUBLE-SIGNING DETECTED from {}! Slashing...", get_short_addr(&inc.account));

                                    // Slash validator for double-signing (100%) via proper Slash block
                                    let staked_amount = l.accounts.get(&inc.account).map(|a| a.balance).unwrap_or(0);
                                    let mut sm = safe_lock(&slashing_clone);
                                    if let Ok(slashed) = sm.slash_double_signing(&inc.account, l.blocks.len() as u64, staked_amount, timestamp) {
                                        println!("⚖️ Validator {} slashed {} CIL (100%) for double-signing",
                                            get_short_addr(&inc.account), slashed);
                                        drop(sm);

                                        // Create proper Slash block instead of direct balance mutation
                                        // This ensures all nodes see the slash in the blockchain
                                        let cheater_state = l.accounts.get(&inc.account).cloned().unwrap_or(AccountState {
                                            head: "0".to_string(), balance: 0, block_count: 0, is_validator: false,
                                        });
                                        let mut slash_blk = Block {
                                            account: inc.account.clone(),
                                            previous: cheater_state.head.clone(),
                                            block_type: BlockType::Slash,
                                            amount: slashed,
                                            link: format!("PENALTY:DOUBLE_SIGN:{}", block_hash),
                                            signature: "".to_string(),
                                            public_key: hex::encode(&keys.public_key),
                                            work: 0,
                                            timestamp,
                                            fee: 0,
                                        };
                                        solve_pow(&mut slash_blk);
                                        slash_blk.signature = match try_sign_hex(slash_blk.signing_hash().as_bytes(), &secret_key) {
                                            Ok(sig) => sig,
                                            Err(e) => { eprintln!("⚠️ Slash signing failed: {}", e); String::new() }
                                        };
                                        if !slash_blk.signature.is_empty() {
                                        match l.process_block(&slash_blk) {
                                            Ok(_) => {
                                                gossip = Some(serde_json::to_string(&slash_blk).unwrap_or_default());
                                                println!("⚖️ Slash block created and broadcast for {}", get_short_addr(&inc.account));
                                            },
                                            Err(e) => eprintln!("⚠️ Slash block failed: {}", e),
                                        }
                                        }
                                        SAVE_DIRTY.store(true, Ordering::Release);
                                    }
                                }
                                (double_sign_detected, gossip)
                            }; // l dropped — Phase 1 complete

                            if let Some(msg) = ds_gossip {
                                let _ = tx_out.send(msg).await;
                            }
                            if double_sign_detected {
                                continue; // Don't process the original block
                            }

                            // Phase 2: Process incoming block + tracking + auto-receive (all synchronous)
                            let phase2_gossip: Vec<String> = {
                                let mut l = safe_lock(&ledger);
                                let mut msgs = Vec::new();

                                match l.process_block(&inc) {
                                    Ok(result) => {
                                        let block_hash = result.into_hash();
                                        // SLASHING: Record block participation for uptime tracking
                                        {
                                            let mut sm = safe_lock(&slashing_clone);
                                            let global_height = l.blocks.len() as u64;
                                            let _ = sm.record_block_participation(&inc.account, global_height, timestamp);

                                            // Check for downtime and slash if needed
                                            if let Some(acc) = l.accounts.get(&inc.account) {
                                                if let Ok(Some(slashed)) = sm.check_and_slash_downtime(
                                                    &inc.account,
                                                    global_height,
                                                    acc.balance,
                                                    timestamp
                                                ) {
                                                    println!("⚖️ Validator {} downtime penalty: {} CIL (1%)",
                                                        get_short_addr(&inc.account), slashed);

                                                    // Create proper Slash block for downtime penalty
                                                    let dt_state = l.accounts.get(&inc.account).cloned().unwrap_or(AccountState {
                                                        head: "0".to_string(), balance: 0, block_count: 0, is_validator: false,
                                                    });
                                                    let mut dt_slash = Block {
                                                        account: inc.account.clone(),
                                                        previous: dt_state.head.clone(),
                                                        block_type: BlockType::Slash,
                                                        amount: slashed,
                                                        link: format!("PENALTY:DOWNTIME:{}", global_height),
                                                        signature: "".to_string(),
                                                        public_key: hex::encode(&keys.public_key),
                                                        work: 0,
                                                        timestamp,
                                                        fee: 0,
                                                    };
                                                    solve_pow(&mut dt_slash);
                                                    dt_slash.signature = match try_sign_hex(dt_slash.signing_hash().as_bytes(), &secret_key) {
                                                        Ok(sig) => sig,
                                                        Err(e) => { eprintln!("⚠️ Downtime slash signing failed: {}", e); String::new() }
                                                    };
                                                    if !dt_slash.signature.is_empty() && l.process_block(&dt_slash).is_ok() {
                                                        msgs.push(serde_json::to_string(&dt_slash).unwrap_or_default());
                                                    }
                                                }
                                            }
                                        }

                                        if inc.block_type == BlockType::Mint {
                                            let mint_val = inc.amount / CIL_PER_LOS;
                                            println!("✅ Network Mint Verified: +{} LOS", format_u128(mint_val));
                                        }
                                        SAVE_DIRTY.store(true, Ordering::Release);
                                        println!("✅ Block Verified: {:?} from {}", inc.block_type, get_short_addr(&inc.account));

                                        // AUTO-UNREGISTER: If a Send block caused sender's balance
                                        // to drop below minimum registration stake (1 LOS), unregister them.
                                        if inc.block_type == BlockType::Send {
                                            if let Some(sender_acct) = l.accounts.get_mut(&inc.account) {
                                                if sender_acct.is_validator && sender_acct.balance < MIN_VALIDATOR_REGISTER_CIL {
                                                    sender_acct.is_validator = false;
                                                    println!("⚠️ Auto-unregistered validator {}: balance {} < minimum registration stake {} LOS",
                                                        get_short_addr(&inc.account),
                                                        sender_acct.balance / CIL_PER_LOS,
                                                        MIN_VALIDATOR_REGISTER_CIL / CIL_PER_LOS);
                                                }
                                            }
                                        }

                                        if inc.block_type == BlockType::Send && inc.link == my_address {
                                            if !l.accounts.contains_key(&my_address) {
                                                l.accounts.insert(my_address.clone(), AccountState { head: "0".to_string(), balance: 0, block_count: 0, is_validator: false });
                                            }
                                            if let Some(state) = l.accounts.get(&my_address).cloned() {
                                                let mut rb = Block {
                                                    account: my_address.clone(), previous: state.head, block_type: BlockType::Receive,
                                                    amount: inc.amount, link: block_hash, signature: "".to_string(),
                                                    public_key: hex::encode(&keys.public_key), // Node's public key
                                                    work: 0,
                                                    timestamp: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs(),
                                                    fee: 0,
                                                };
                                                solve_pow(&mut rb);
                                                rb.signature = match try_sign_hex(rb.signing_hash().as_bytes(), &secret_key) {
                                                    Ok(sig) => sig,
                                                    Err(e) => { eprintln!("⚠️ Auto-Receive signing failed: {}", e); String::new() }
                                                };
                                                if !rb.signature.is_empty() && l.process_block(&rb).is_ok() {
                                                    SAVE_DIRTY.store(true, Ordering::Release);
                                                    msgs.push(serde_json::to_string(&rb).unwrap_or_default());
                                                    println!("📥 Incoming Transfer Received Automatically!");
                                                }
                                            }
                                        }
                                    },
                                    Err(e) => {
                                        println!("❌ Block Rejected: {:?} (Sender: {})", e, get_short_addr(&inc.account));
                                    }
                                }
                                msgs
                            }; // l dropped — Phase 2 complete
                            for msg in phase2_gossip {
                                let _ = tx_out.send(msg).await;
                            }
                        }
                    }
            }
            else => {
                // Both stdin (closed/EOF) and network channel (dropped) are inactive.
                // This happens in headless mode (nohup) when the P2P network task
                // hasn't sent any events yet. Sleep briefly and retry — the network
                // task may still produce events.
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
    Ok(())
}
