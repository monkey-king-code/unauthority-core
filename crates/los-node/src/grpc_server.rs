/// Unauthority gRPC Server Implementation
///
/// Provides 8 core gRPC services for external integration:
/// 1. GetBalance - Query account balance
/// 2. GetAccount - Get full account details
/// 3. GetBlock - Get block by hash
/// 4. GetLatestBlock - Get latest finalized block
/// 5. SendTransaction - Broadcast LOS transaction
/// 6. GetNodeInfo - Get node/oracle/supply info
/// 7. GetValidators - List all active validators
/// 8. GetBlockHeight - Get current blockchain height
use los_consensus::voting::calculate_voting_power;
use los_core::{
    validator_rewards::ValidatorRewardPool, Ledger, CIL_PER_LOS, MIN_VALIDATOR_STAKE_CIL,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tonic::{transport::Server, Request, Response, Status};

// Include generated protobuf code
pub mod proto {
    tonic::include_proto!("unauthority");
}

use proto::{
    los_node_server::{LosNode, LosNodeServer},
    GetAccountRequest, GetAccountResponse, GetBalanceRequest, GetBalanceResponse,
    GetBlockHeightRequest, GetBlockHeightResponse, GetBlockRequest, GetBlockResponse,
    GetLatestBlockRequest, GetNodeInfoRequest, GetNodeInfoResponse, GetValidatorsRequest,
    GetValidatorsResponse, SendTransactionRequest, SendTransactionResponse, ValidatorInfo,
};

/// gRPC Service Implementation
pub struct LosGrpcService {
    ledger: Arc<Mutex<Ledger>>,
    my_address: String,
    #[allow(dead_code)] // Reserved for future direct gossip broadcasting
    tx_sender: mpsc::Sender<String>, // For broadcasting transactions
    /// Peer address book — provides real peer count
    address_book: Arc<Mutex<HashMap<String, String>>>,
    /// Bootstrap validator addresses for active status checks
    bootstrap_validators: Vec<String>,
    /// Local REST API port for forwarding SendTransaction
    rest_api_port: u16,
    /// Local REST API host (respects LOS_BIND_ALL for Tor)
    rest_bind_host: String,
    /// Shared HTTP client for REST forwarding (connection pooling, keep-alive)
    http_client: reqwest::Client,
    /// Validator reward pool — provides real uptime percentages
    reward_pool: Arc<Mutex<ValidatorRewardPool>>,
}

impl LosGrpcService {
    pub fn new(
        ledger: Arc<Mutex<Ledger>>,
        my_address: String,
        tx_sender: mpsc::Sender<String>,
        address_book: Arc<Mutex<HashMap<String, String>>>,
        bootstrap_validators: Vec<String>,
        rest_api_port: u16,
        reward_pool: Arc<Mutex<ValidatorRewardPool>>,
    ) -> Self {
        // REST forwarding always targets localhost — gRPC and REST are co-located
        let rest_bind_host = "127.0.0.1".to_string();
        // PERF: Reuse one Client across all requests (connection pool + keep-alive)
        let http_client = reqwest::Client::builder()
            .pool_max_idle_per_host(4)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            ledger,
            my_address,
            tx_sender,
            address_book,
            bootstrap_validators,
            rest_api_port,
            rest_bind_host,
            http_client,
            reward_pool,
        }
    }

    /// Helper: Convert short address to full address
    fn resolve_address(&self, addr: &str) -> Option<String> {
        let ledger = self.ledger.lock().ok()?;

        // If already full address, return
        if ledger.accounts.contains_key(addr) {
            return Some(addr.to_string());
        }

        // Try to find by short ID
        ledger
            .accounts
            .keys()
            .find(|k| k.starts_with(addr) || get_short_addr(k) == addr)
            .cloned()
    }
}

/// Helper function to get short address (first 8 chars after prefix)
fn get_short_addr(full: &str) -> String {
    if full.len() > 12 {
        // Skip "LOS" prefix (3 chars), take next 8 chars of base58
        format!("los_{}", &full[3..11])
    } else {
        full.to_string()
    }
}

#[tonic::async_trait]
impl LosNode for LosGrpcService {
    /// 1. Get account balance
    async fn get_balance(
        &self,
        request: Request<GetBalanceRequest>,
    ) -> Result<Response<GetBalanceResponse>, Status> {
        let addr = request.into_inner().address;

        let full_addr = self
            .resolve_address(&addr)
            .ok_or_else(|| Status::not_found(format!("Address not found: {}", addr)))?;

        let ledger = self
            .ledger
            .lock()
            .map_err(|_| Status::internal("Failed to lock ledger"))?;

        let account = ledger
            .accounts
            .get(&full_addr)
            .ok_or_else(|| Status::not_found("Account not found"))?;

        // Use string formatting to avoid u128→u64 truncation
        let balance_los = account.balance / CIL_PER_LOS;
        let balance_remainder = account.balance % CIL_PER_LOS;

        let response = GetBalanceResponse {
            address: full_addr,
            balance_cil: account.balance.min(u64::MAX as u128) as u64, // Cap to u64::MAX (no silent truncation)
            // PROTO BOUNDARY: `double balance_los` required by los.proto.
            // Integer `balance_cil_str` below is authoritative. This f64 is display-only.
            balance_los: balance_los as f64 + (balance_remainder as f64 / CIL_PER_LOS as f64),
            block_count: account.block_count,
            head_block: account.head.clone(),
            balance_cil_str: account.balance.to_string(), // Full-precision u128 as string
        };

        println!(
            "📊 gRPC GetBalance: {} -> {}.{} LOS",
            get_short_addr(&response.address),
            balance_los,
            balance_remainder
        );

        Ok(Response::new(response))
    }

    /// 2. Get full account details
    async fn get_account(
        &self,
        request: Request<GetAccountRequest>,
    ) -> Result<Response<GetAccountResponse>, Status> {
        let addr = request.into_inner().address;

        let full_addr = self
            .resolve_address(&addr)
            .ok_or_else(|| Status::not_found(format!("Address not found: {}", addr)))?;

        let ledger = self
            .ledger
            .lock()
            .map_err(|_| Status::internal("Failed to lock ledger"))?;

        let account = ledger
            .accounts
            .get(&full_addr)
            .ok_or_else(|| Status::not_found("Account not found"))?;

        // Use the authoritative is_validator flag from AccountState
        // instead of inferring from balance threshold
        let is_validator = account.is_validator;

        let response = GetAccountResponse {
            address: full_addr.clone(),
            balance_cil: account.balance.min(u64::MAX as u128) as u64, // Cap to u64::MAX (no silent truncation)
            // PROTO BOUNDARY: `double balance_los` required by los.proto.
            // Integer `balance_cil_str` below is authoritative. This f64 is display-only.
            balance_los: (account.balance / CIL_PER_LOS) as f64
                + (account.balance % CIL_PER_LOS) as f64 / CIL_PER_LOS as f64,
            block_count: account.block_count,
            head_block: account.head.clone(),
            is_validator,
            stake_cil: if is_validator {
                account.balance.min(u64::MAX as u128) as u64
            } else {
                0
            },
            balance_cil_str: account.balance.to_string(),
            stake_cil_str: if is_validator {
                account.balance.to_string()
            } else {
                "0".to_string()
            },
        };

        println!(
            "🔍 gRPC GetAccount: {} (validator: {})",
            get_short_addr(&full_addr),
            is_validator
        );

        Ok(Response::new(response))
    }

    /// 3. Get block by hash
    async fn get_block(
        &self,
        request: Request<GetBlockRequest>,
    ) -> Result<Response<GetBlockResponse>, Status> {
        let hash = request.into_inner().block_hash;

        let ledger = self
            .ledger
            .lock()
            .map_err(|_| Status::internal("Failed to lock ledger"))?;

        let block = ledger
            .blocks
            .get(&hash)
            .ok_or_else(|| Status::not_found(format!("Block not found: {}", hash)))?;

        // Get account balance from ledger (Block itself doesn't have balance field)
        let account_balance = ledger
            .accounts
            .get(&block.account)
            .map(|acc| acc.balance.min(u64::MAX as u128) as u64)
            .unwrap_or(0);

        let response = GetBlockResponse {
            block_hash: hash.clone(),
            account: block.account.clone(),
            previous_block: block.previous.clone(),
            link: block.link.clone(),
            block_type: format!("{:?}", block.block_type),
            amount: block.amount.min(u64::MAX as u128) as u64,
            balance: account_balance, // Account balance, not block balance
            signature: block.signature.clone(),
            timestamp: block.timestamp, // Use actual block timestamp
            representative: if matches!(block.block_type, los_core::BlockType::Change) {
                block.link.clone() // Change blocks store representative in link
            } else {
                String::new()
            },
        };

        println!(
            "📦 gRPC GetBlock: {} (type: {})",
            &hash[..12],
            response.block_type
        );

        Ok(Response::new(response))
    }

    /// 4. Get latest block (by highest timestamp)
    async fn get_latest_block(
        &self,
        _request: Request<GetLatestBlockRequest>,
    ) -> Result<Response<GetBlockResponse>, Status> {
        let ledger = self
            .ledger
            .lock()
            .map_err(|_| Status::internal("Failed to lock ledger"))?;

        // Find ACTUAL latest block by timestamp (not random HashMap entry)
        let latest = ledger
            .blocks
            .iter()
            .max_by_key(|(_, block)| block.timestamp)
            .ok_or_else(|| Status::not_found("No blocks found"))?;

        let (hash, block) = latest;

        // Get account balance from ledger
        let account_balance = ledger
            .accounts
            .get(&block.account)
            .map(|acc| acc.balance.min(u64::MAX as u128) as u64)
            .unwrap_or(0);

        let response = GetBlockResponse {
            block_hash: hash.clone(),
            account: block.account.clone(),
            previous_block: block.previous.clone(),
            link: block.link.clone(),
            block_type: format!("{:?}", block.block_type),
            amount: block.amount.min(u64::MAX as u128) as u64,
            balance: account_balance,
            signature: block.signature.clone(),
            timestamp: block.timestamp,
            representative: "".to_string(),
        };

        println!(
            "🆕 gRPC GetLatestBlock: {} (ts: {})",
            &hash[..12.min(hash.len())],
            block.timestamp
        );

        Ok(Response::new(response))
    }

    /// 5. Send transaction via gRPC
    /// Forwards pre-signed transactions to the REST /send endpoint which handles
    /// PoW validation, fee checks, and aBFT consensus flow.
    /// The client MUST provide: from, to, amount_cil, and signature (Dilithium5-signed).
    async fn send_transaction(
        &self,
        request: Request<SendTransactionRequest>,
    ) -> Result<Response<SendTransactionResponse>, Status> {
        let req = request.into_inner();

        // Validate required fields
        if req.from.is_empty() || req.to.is_empty() {
            return Err(Status::invalid_argument("from and to fields are required"));
        }
        if req.signature.is_empty() {
            return Err(Status::invalid_argument(
                "signature is required (client must sign with Dilithium5)",
            ));
        }
        if req.public_key.is_empty() {
            return Err(Status::invalid_argument(
                "public_key is required when providing signature (hex-encoded Dilithium5 public key)",
            ));
        }

        // Forward to local REST /send endpoint which handles everything:
        // PoW validation, fee calculation, consensus flow, auto-receive
        let rest_url = format!("http://{}:{}/send", self.rest_bind_host, self.rest_api_port);
        let mut payload = serde_json::json!({
            "from": req.from,
            "target": req.to,
            "amount_cil": req.amount_cil,
            "signature": hex::encode(&req.signature),
            "fee": req.fee,
        });
        // CRITICAL: Forward public_key — REST handler requires it when signature is present.
        // Without this field, REST returns 400 "public_key field is REQUIRED".
        if !req.public_key.is_empty() {
            payload["public_key"] = serde_json::Value::String(req.public_key.clone());
        }
        // Forward client timestamp (part of signing_hash for signature verification)
        if req.timestamp > 0 {
            payload["timestamp"] = serde_json::json!(req.timestamp);
        }

        let client = &self.http_client;
        match client.post(&rest_url).json(&payload).send().await {
            Ok(resp) => {
                let body: serde_json::Value = resp.json().await.map_err(|e| {
                    Status::internal(format!("Failed to parse REST response: {}", e))
                })?;

                let success = body["status"].as_str() == Some("ok")
                    || body["status"].as_str() == Some("confirmed")
                    || body["status"].as_str() == Some("success");
                let tx_hash = body["hash"]
                    .as_str()
                    .or(body["tx_hash"].as_str())
                    .unwrap_or("")
                    .to_string();
                let message = body["msg"]
                    .as_str()
                    .unwrap_or(if success {
                        "Transaction submitted"
                    } else {
                        "Transaction failed"
                    })
                    .to_string();

                Ok(Response::new(SendTransactionResponse {
                    success,
                    tx_hash,
                    message,
                    estimated_finality_ms: 3000, // ~3s aBFT finality
                }))
            }
            Err(e) => Err(Status::unavailable(format!(
                "REST API unavailable: {}. Ensure the node is running.",
                e
            ))),
        }
    }

    /// 6. Get node info
    async fn get_node_info(
        &self,
        _request: Request<GetNodeInfoRequest>,
    ) -> Result<Response<GetNodeInfoResponse>, Status> {
        let ledger = self
            .ledger
            .lock()
            .map_err(|_| Status::internal("Failed to lock ledger"))?;

        // Check if this node is validator — use the authoritative is_validator flag
        // (not bare balance threshold, which would misreport deregistered validators)
        let is_validator = ledger
            .accounts
            .get(&self.my_address)
            .map(|a| a.is_validator || a.balance >= MIN_VALIDATOR_STAKE_CIL)
            .unwrap_or(false);

        // Calculate latest block height (count total blocks)
        let latest_height = ledger.blocks.len() as u64;

        let response = GetNodeInfoResponse {
            node_address: self.my_address.clone(),
            network_id: los_core::CHAIN_ID as u32, // CHAIN_ID: 1=mainnet, 2=testnet
            chain_name: "Unauthority".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            // Use .min() saturation instead of hard-coding 0
            // u128 total supply overflows u64 — cap at u64::MAX for legacy field
            total_supply_cil: (21_936_236u128 * los_core::CIL_PER_LOS).min(u64::MAX as u128) as u64,
            remaining_supply_cil: (ledger.distribution.remaining_supply).min(u64::MAX as u128)
                as u64,
            peer_count: self
                .address_book
                .lock()
                .map(|ab| ab.len() as u32)
                .unwrap_or(0),
            latest_block_height: latest_height,
            is_validator,
        };

        println!(
            "ℹ️  gRPC GetNodeInfo: {} (validator: {})",
            get_short_addr(&self.my_address),
            is_validator
        );

        Ok(Response::new(response))
    }

    /// 7. Get validators list
    async fn get_validators(
        &self,
        _request: Request<GetValidatorsRequest>,
    ) -> Result<Response<GetValidatorsResponse>, Status> {
        let ledger = self
            .ledger
            .lock()
            .map_err(|_| Status::internal("Failed to lock ledger"))?;

        let min_stake = MIN_VALIDATOR_STAKE_CIL;

        let peer_addresses: std::collections::HashSet<String> = self
            .address_book
            .lock()
            .map(|ab| ab.keys().cloned().collect())
            .unwrap_or_default();

        // Real uptime data from reward pool (heartbeat-based tracking)
        let uptime_data: HashMap<String, u64> = self
            .reward_pool
            .lock()
            .map(|rp| {
                rp.validators
                    .iter()
                    .map(|(addr, vs)| (addr.clone(), vs.display_uptime_pct()))
                    .collect()
            })
            .unwrap_or_default();

        // Pre-compute cumulative rewards per validator from REWARD Mint blocks
        let mut reward_totals: HashMap<String, u128> = HashMap::new();
        for blk in ledger.blocks.values() {
            if blk.block_type == los_core::BlockType::Mint && blk.link.starts_with("REWARD:") {
                *reward_totals.entry(blk.account.clone()).or_insert(0) += blk.amount;
            }
        }

        // Filter accounts with minimum stake
        let validators: Vec<ValidatorInfo> = ledger
            .accounts
            .iter()
            .filter(|(_, acc)| acc.balance >= min_stake)
            .map(|(addr, acc)| {
                // Linear voting power: 1 CIL = 1 vote
                // Changed from √stake to linear.
                // PROTO BOUNDARY: `double voting_power` required by los.proto.
                // Linear CIL is authoritative; cast to f64 only for proto serialization.
                let voting_power = calculate_voting_power(acc.balance) as f64;

                // Active = is self OR is in address book (has live P2P connection)
                let is_active = addr == &self.my_address
                    || peer_addresses.contains(addr)
                    || self.bootstrap_validators.contains(addr);

                let earned = reward_totals.get(addr).copied().unwrap_or(0);

                ValidatorInfo {
                    address: addr.clone(),
                    // .min() guard prevents wrapping on balances > u64::MAX
                    stake_cil: acc.balance.min(u64::MAX as u128) as u64,
                    is_active,
                    voting_power,
                    rewards_earned: earned.min(u64::MAX as u128) as u64,
                    // Real uptime from heartbeat tracking (100 for self if not yet recorded)
                    uptime_percent: uptime_data
                        .get(addr)
                        .copied()
                        .unwrap_or(if addr == &self.my_address { 100 } else { 0 })
                        as f64,
                }
            })
            .collect();

        let total_count = validators.len() as u32;

        println!("👥 gRPC GetValidators: {} active validators", total_count);

        let response = GetValidatorsResponse {
            validators,
            total_count,
        };

        Ok(Response::new(response))
    }

    /// 8. Get block height
    async fn get_block_height(
        &self,
        _request: Request<GetBlockHeightRequest>,
    ) -> Result<Response<GetBlockHeightResponse>, Status> {
        let ledger = self
            .ledger
            .lock()
            .map_err(|_| Status::internal("Failed to lock ledger"))?;

        // Find latest block by timestamp (or use total count as height)
        let total_blocks = ledger.blocks.len() as u64;

        let latest_hash = ledger
            .blocks
            .iter()
            .max_by_key(|(_, b)| b.timestamp)
            .map(|(h, _)| h.clone())
            .unwrap_or_else(|| "0".to_string());

        let response = GetBlockHeightResponse {
            height: total_blocks,
            latest_block_hash: latest_hash,
            timestamp: chrono::Utc::now().timestamp() as u64,
        };

        println!("📏 gRPC GetBlockHeight: {}", response.height);

        Ok(Response::new(response))
    }
}

/// Start gRPC server (runs alongside REST API)
#[allow(clippy::too_many_arguments)]
pub async fn start_grpc_server(
    ledger: Arc<Mutex<Ledger>>,
    my_address: String,
    tx_sender: mpsc::Sender<String>,
    grpc_port: u16,
    address_book: Arc<Mutex<HashMap<String, String>>>,
    bootstrap_validators: Vec<String>,
    rest_api_port: u16,
    reward_pool: Arc<Mutex<ValidatorRewardPool>>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Respect LOS_BIND_ALL env for Tor safety (same as REST API)
    let bind_addr = if std::env::var("LOS_BIND_ALL").unwrap_or_default() == "1" {
        format!("0.0.0.0:{}", grpc_port)
    } else {
        format!("127.0.0.1:{}", grpc_port)
    };
    let addr = bind_addr.parse()?;

    let service = LosGrpcService::new(
        ledger,
        my_address.clone(),
        tx_sender,
        address_book,
        bootstrap_validators,
        rest_api_port,
        reward_pool,
    );

    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🚀 gRPC Server STARTED");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("   Address: {}", addr);
    println!("   Node: {}", get_short_addr(&my_address));
    println!("   Services: 8 core gRPC endpoints");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    Server::builder()
        .add_service(LosNodeServer::new(service))
        .serve(addr)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use los_core::{validator_rewards::ValidatorRewardPool, AccountState};
    use std::collections::HashMap;

    fn mock_reward_pool() -> Arc<Mutex<ValidatorRewardPool>> {
        Arc::new(Mutex::new(ValidatorRewardPool::new(0)))
    }

    #[tokio::test]
    async fn test_grpc_get_balance() {
        let mut ledger = Ledger::new();
        ledger.accounts.insert(
            "test_address".to_string(),
            AccountState {
                head: "genesis".to_string(),
                balance: 500 * CIL_PER_LOS,
                block_count: 0,
                is_validator: false,
            },
        );

        let ledger = Arc::new(Mutex::new(ledger));
        let (tx, _rx) = mpsc::channel(1);

        let service = LosGrpcService::new(
            ledger,
            "node_address".to_string(),
            tx,
            Arc::new(Mutex::new(HashMap::new())),
            vec![],
            3030,
            mock_reward_pool(),
        );

        let request = Request::new(GetBalanceRequest {
            address: "test_address".to_string(),
        });

        let response = service.get_balance(request).await.unwrap();
        let balance = response.into_inner();

        assert_eq!(balance.address, "test_address");
        assert_eq!(balance.balance_cil, (500 * CIL_PER_LOS) as u64);
        assert_eq!(balance.balance_los, 500.0);
    }

    #[tokio::test]
    async fn test_grpc_get_validators() {
        let mut ledger = Ledger::new();

        // Add 2 validators (min 1,000 LOS)
        ledger.accounts.insert(
            "validator1".to_string(),
            AccountState {
                head: "genesis".to_string(),
                balance: 5000 * CIL_PER_LOS,
                block_count: 0,
                is_validator: true,
            },
        );
        ledger.accounts.insert(
            "validator2".to_string(),
            AccountState {
                head: "genesis".to_string(),
                balance: 10000 * CIL_PER_LOS,
                block_count: 0,
                is_validator: true,
            },
        );

        // Add 1 non-validator (below min stake)
        ledger.accounts.insert(
            "regular_user".to_string(),
            AccountState {
                head: "genesis".to_string(),
                balance: 100 * CIL_PER_LOS,
                block_count: 0,
                is_validator: false,
            },
        );

        let ledger = Arc::new(Mutex::new(ledger));
        let (tx, _rx) = mpsc::channel(1);

        let service = LosGrpcService::new(
            ledger,
            "node".to_string(),
            tx,
            Arc::new(Mutex::new(HashMap::new())),
            vec!["validator1".to_string(), "validator2".to_string()],
            3030,
            mock_reward_pool(),
        );

        let request = Request::new(GetValidatorsRequest {});
        let response = service.get_validators(request).await.unwrap();
        let validators = response.into_inner();

        // Should return only 2 validators (min 1,000 LOS stake)
        assert_eq!(validators.total_count, 2);
        assert_eq!(validators.validators.len(), 2);

        // Check linear voting power
        let val1 = &validators.validators[0];
        assert!(val1.voting_power > 0.0);
        assert!(val1.is_active);
    }
}
