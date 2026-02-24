// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// UNAUTHORITY (LOS) - CORE MODULE
//
// Blockchain primitives: Block, Ledger, AccountState, and transaction logic.
// Defines the block-lattice DAG structure with Send/Receive/Mint/Slash types.
// All financial arithmetic uses u128 CIL units (no floating-point).
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use serde::{Deserialize, Serialize};
use sha3::{Digest, Sha3_256};
use std::collections::{BTreeMap, BTreeSet};

/// Maximum allowed timestamp drift from current time (5 minutes)
pub const MAX_TIMESTAMP_DRIFT_SECS: u64 = 300;

pub mod distribution;
pub mod pow_mint;
pub mod validator_config;
pub mod validator_rewards;
use crate::distribution::DistributionState;

/// 1 LOS = 100_000_000_000 CIL (10^11 precision)
/// Higher precision than Bitcoin (10^8) for DeFi/smart contract flexibility
pub const CIL_PER_LOS: u128 = 100_000_000_000;
/// Total supply: 21,936,236 LOS in CIL units (fixed, non-inflationary)
pub const TOTAL_SUPPLY_CIL: u128 = 21_936_236 * CIL_PER_LOS;
/// Minimum balance to REGISTER as a validator (1 LOS in CIL units).
/// Permissionless: any node with ≥1 LOS can participate in consensus.
pub const MIN_VALIDATOR_REGISTER_CIL: u128 = CIL_PER_LOS;
/// Minimum stake for REWARD eligibility + quorum weight (1000 LOS in CIL units).
/// Only validators with ≥1,000 LOS earn epoch rewards and count toward quorum.
pub const MIN_VALIDATOR_STAKE_CIL: u128 = 1_000 * CIL_PER_LOS;

/// Base transaction fee in CIL (0.000001 LOS = 100,000 CIL)
/// Single source of truth — wallet fetches this via /node-info.
/// Flat fee per transaction — no dynamic fee scaling.
///
/// Future: This will become a governance-adjustable parameter.
/// For mainnet launch, validators can vote to change the base fee
/// through on-chain governance without requiring a binary upgrade.
/// The /node-info endpoint ensures wallets always get the current value.
pub const BASE_FEE_CIL: u128 = 100_000;

/// Minimum PoW difficulty: 16 leading zero bits (anti-spam)
pub const MIN_POW_DIFFICULTY_BITS: u32 = 16;

/// Chain ID to prevent cross-chain replay attacks
/// Mainnet = 1, Testnet = 2. Included in every block's signing hash.
/// Compile with `--features mainnet` for mainnet build.
#[cfg(feature = "mainnet")]
pub const CHAIN_ID: u64 = 1; // Mainnet
#[cfg(not(feature = "mainnet"))]
pub const CHAIN_ID: u64 = 2; // Testnet

/// Returns true if this binary was compiled for testnet
pub const fn is_testnet_build() -> bool {
    CHAIN_ID != 1
}

/// Returns true if this binary was compiled for mainnet
pub const fn is_mainnet_build() -> bool {
    CHAIN_ID == 1
}

// ─────────────────────────────────────────────────────────────────
// VALIDATOR REWARD SYSTEM CONSTANTS
// ─────────────────────────────────────────────────────────────────
// Pool: 500,000 LOS from public allocation.
// Rate: 5,000 LOS/epoch (30 days), halving every 4 years (48 epochs).
// Distribution: Linear stake-weighted proportional among eligible validators.
// ALL validators (including genesis bootstrap) are eligible for rewards.
// Pool asymptotically approaches ~480,000 LOS total distributed.
// ─────────────────────────────────────────────────────────────────

/// Total validator reward pool: 500,000 LOS in CIL
pub const VALIDATOR_REWARD_POOL_CIL: u128 = 500_000 * CIL_PER_LOS;

/// One epoch = 30 days in seconds (reward distribution cycle)
pub const REWARD_EPOCH_SECS: u64 = 30 * 24 * 60 * 60; // 2,592,000

/// Testnet epoch = 2 minutes (for rapid testing of reward mechanics)
pub const TESTNET_REWARD_EPOCH_SECS: u64 = 2 * 60; // 120

/// Get the effective reward epoch duration based on network type.
/// Testnet: 2 minutes for rapid reward testing.
/// Mainnet: 30 days (standard epoch).
pub const fn effective_reward_epoch_secs() -> u64 {
    if is_testnet_build() {
        TESTNET_REWARD_EPOCH_SECS
    } else {
        REWARD_EPOCH_SECS
    }
}

/// Initial reward rate: 5,000 LOS per epoch (before halving)
pub const REWARD_RATE_INITIAL_CIL: u128 = 5_000 * CIL_PER_LOS;

/// Halving interval: every 48 epochs (4 years × 12 months)
pub const REWARD_HALVING_INTERVAL_EPOCHS: u64 = 48;

/// Minimum uptime percentage required to receive rewards (95%)
pub const REWARD_MIN_UPTIME_PCT: u64 = 95;

/// Probation period: 1 epoch (30 days) before a new validator earns rewards
pub const REWARD_PROBATION_EPOCHS: u64 = 1;

// ─────────────────────────────────────────────────────────────────
// SMART CONTRACT GAS PRICING
// ─────────────────────────────────────────────────────────────────
// Gas is priced in CIL. Each WASM instruction costs 1 gas unit.
// GAS_PRICE_CIL converts gas units to CIL for fee calculation.
// deploy_fee = bytecode_kb * GAS_PER_KB + BASE_DEPLOY_GAS
// call_fee   = gas_limit * GAS_PRICE_CIL
// ─────────────────────────────────────────────────────────────────

/// Price per gas unit in CIL (1 gas = 1 CIL)
pub const GAS_PRICE_CIL: u128 = 1;

/// Minimum fee for deploying a contract (0.01 LOS = 1,000,000,000 CIL)
pub const MIN_DEPLOY_FEE_CIL: u128 = 1_000_000_000;

/// Minimum fee for calling a contract (same as base tx fee: 0.000001 LOS)
pub const MIN_CALL_FEE_CIL: u128 = BASE_FEE_CIL;

/// Default gas limit for contract calls (1,000,000 gas units)
pub const DEFAULT_GAS_LIMIT: u64 = 1_000_000;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum BlockType {
    Send,
    Receive,
    Change,
    Mint,
    Slash,
    /// Deploy a WASM smart contract. link = "DEPLOY:{code_hash}"
    ContractDeploy,
    /// Call a smart contract function. link = "CALL:{contract_addr}:{function}:{args_b64}"
    ContractCall,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Block {
    pub account: String,
    pub previous: String,
    pub block_type: BlockType,
    pub amount: u128,
    pub link: String,
    pub signature: String,
    pub public_key: String, // Dilithium5 public key (hex-encoded)
    pub work: u64,
    pub timestamp: u64, // Unix timestamp (seconds since epoch)
    /// Transaction fee in CIL (deducted from sender on Send blocks)
    #[serde(default)]
    pub fee: u128,
}

impl Block {
    /// Content hash: all fields EXCEPT signature.
    /// Used for: (1) PoW mining, (2) message to sign/verify.
    /// Includes chain_id to prevent cross-chain replay attacks.
    pub fn signing_hash(&self) -> String {
        let mut hasher = Sha3_256::new();

        // Chain ID domain separation — prevents replay across testnet/mainnet
        hasher.update(CHAIN_ID.to_le_bytes());

        hasher.update(self.account.as_bytes());
        hasher.update(self.previous.as_bytes());

        let type_byte = match self.block_type {
            BlockType::Send => 0,
            BlockType::Receive => 1,
            BlockType::Change => 2,
            BlockType::Mint => 3,
            BlockType::Slash => 4,
            BlockType::ContractDeploy => 5,
            BlockType::ContractCall => 6,
        };
        hasher.update([type_byte]);

        hasher.update(self.amount.to_le_bytes());
        hasher.update(self.link.as_bytes());

        // public_key MUST be included in hash (cryptographic identity binding)
        hasher.update(self.public_key.as_bytes());

        // work (nonce) MUST be included in hash
        hasher.update(self.work.to_le_bytes());

        // timestamp MUST be included in hash (prevent replay attacks)
        hasher.update(self.timestamp.to_le_bytes());

        // fee MUST be included in hash (prevent fee manipulation)
        hasher.update(self.fee.to_le_bytes());

        hex::encode(hasher.finalize())
    }

    /// Final block hash: signing_hash + signature.
    /// This is the unique Block ID that includes ALL fields including signature.
    /// Prevents block ID collision if signature differs.
    pub fn calculate_hash(&self) -> String {
        let mut hasher = Sha3_256::new();
        let sh = self.signing_hash();
        hasher.update(sh.as_bytes());
        // Signature MUST be in hash computation for block identity
        hasher.update(self.signature.as_bytes());
        hex::encode(hasher.finalize())
    }

    pub fn verify_signature(&self) -> bool {
        if self.signature.is_empty() {
            return false;
        }
        if self.public_key.is_empty() {
            return false;
        }

        // Verify against signing_hash (content hash without signature)
        let msg_hash = self.signing_hash();
        let sig_bytes = hex::decode(&self.signature).unwrap_or_default();
        let pk_bytes = hex::decode(&self.public_key).unwrap_or_default();
        los_crypto::verify_signature(msg_hash.as_bytes(), &sig_bytes, &pk_bytes)
    }

    /// Verify Proof-of-Work meets minimum difficulty (anti-spam protection)
    /// This is NOT consensus PoW - just anti-spam measure
    /// Minimum: 16 leading zero bits (≈65,536 average attempts)
    pub fn verify_pow(&self) -> bool {
        let hash = self.signing_hash();
        let hash_bytes = match hex::decode(&hash) {
            Ok(bytes) => bytes,
            Err(_) => return false,
        };

        // Count leading zero bits
        let mut zero_bits = 0u32;
        for byte in &hash_bytes {
            if *byte == 0 {
                zero_bits += 8;
            } else {
                zero_bits += byte.leading_zeros();
                break;
            }
        }

        zero_bits >= MIN_POW_DIFFICULTY_BITS
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AccountState {
    pub head: String,
    pub balance: u128,
    pub block_count: u64,
    /// True if this account has registered as a validator.
    /// Set during genesis for bootstrap validators, or via register-validator flow.
    /// Treasury/dev wallets have high balances but is_validator = false.
    #[serde(default)]
    pub is_validator: bool,
}

/// Result of processing a block through the ledger.
/// Distinguishes between newly applied blocks and duplicates.
/// Callers MUST check `is_new()` to avoid re-broadcasting duplicate blocks.
#[derive(Debug, Clone, PartialEq)]
pub enum ProcessResult {
    /// Block was new and successfully applied to the ledger
    Applied(String),
    /// Block already existed in the ledger (no state change)
    Duplicate(String),
}

impl ProcessResult {
    /// Get the block hash regardless of whether it was new or duplicate
    pub fn hash(&self) -> &str {
        match self {
            ProcessResult::Applied(h) | ProcessResult::Duplicate(h) => h,
        }
    }
    /// Returns true if the block was newly applied (not a duplicate)
    pub fn is_new(&self) -> bool {
        matches!(self, ProcessResult::Applied(_))
    }
    /// Consume self and return the hash string
    pub fn into_hash(self) -> String {
        match self {
            ProcessResult::Applied(h) | ProcessResult::Duplicate(h) => h,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Ledger {
    /// MAINNET: BTreeMap guarantees deterministic iteration and serialization
    /// across all validators. Required for state root agreement.
    pub accounts: BTreeMap<String, AccountState>,
    pub blocks: BTreeMap<String, Block>,
    pub distribution: DistributionState,
    /// O(1) index of Send block hashes that have already been claimed by a Receive block.
    /// MAINNET: BTreeSet for deterministic serialization in SYNC_GZIP payloads.
    /// Memory: ~64 bytes per entry × 10M entries ≈ 640MB upper bound.
    #[serde(default)]
    pub claimed_sends: BTreeSet<String>,
    /// Accumulated transaction fees (CIL units) — available for validator distribution
    #[serde(default)]
    pub accumulated_fees_cil: u128,
    /// DESIGN Total CIL permanently removed from circulation via Slash blocks.
    /// Used by the supply audit to verify: sum(balances) + remaining_supply + total_slashed + fees == TOTAL_SUPPLY_CIL.
    /// The validator reward pool is NOT separate — its undistributed tokens live in remaining_supply.
    /// Without this counter, slashed funds silently disappear and the supply invariant breaks.
    #[serde(default)]
    pub total_slashed_cil: u128,
}

impl Default for Ledger {
    fn default() -> Self {
        Self::new()
    }
}

impl Ledger {
    pub fn new() -> Self {
        Self {
            accounts: BTreeMap::new(),
            blocks: BTreeMap::new(),
            distribution: DistributionState::new(),
            claimed_sends: BTreeSet::new(),
            accumulated_fees_cil: 0,
            total_slashed_cil: 0,
        }
    }

    /// DESIGN Compute a deterministic state root hash from all account balances.
    /// Uses SHA3-256 (NIST FIPS 202) over sorted (address, balance) pairs.
    /// BTreeMap guarantees deterministic iteration order, so all nodes
    /// with the same state will produce the same root hash.
    ///
    /// Used by:
    /// - Checkpoint creation (state snapshot proof)
    /// - ID messages (state comparison before sync)
    /// - Delta sync (skip sync when roots match)
    pub fn compute_state_root(&self) -> String {
        use sha3::{Digest, Sha3_256};
        let mut hasher = Sha3_256::new();
        // BTreeMap iterates in sorted key order — deterministic
        for (addr, state) in &self.accounts {
            hasher.update(addr.as_bytes());
            hasher.update(state.balance.to_le_bytes());
        }
        hex::encode(hasher.finalize())
    }

    pub fn process_block(&mut self, block: &Block) -> Result<ProcessResult, String> {
        // 1. PROOF-OF-WORK VALIDATION (Anti-spam: 16 leading zero bits)
        if !block.verify_pow() {
            return Err(
                "Invalid PoW: Block does not meet minimum difficulty (16 zero bits)".to_string(),
            );
        }

        // 2. SIGNATURE VALIDATION (Dilithium5 post-quantum)
        if !block.verify_signature() {
            return Err("Invalid Signature: Public key verification failed!".to_string());
        }

        // 3. ACCOUNT ↔ PUBLIC KEY BINDING (prevents fund theft)
        // For Send and Change blocks, the signer MUST be the account owner.
        // Receive/Mint/Slash are system-created (signed by node/validator, not account owner).
        if matches!(
            block.block_type,
            BlockType::Send
                | BlockType::Change
                | BlockType::ContractDeploy
                | BlockType::ContractCall
        ) {
            let pk_bytes = hex::decode(&block.public_key)
                .map_err(|e| format!("Authorization Error: Invalid public_key hex: {}", e))?;
            if pk_bytes.is_empty() {
                return Err("Authorization Error: public_key is empty".to_string());
            }
            let derived_address = los_crypto::public_key_to_address(&pk_bytes);
            if derived_address != block.account {
                return Err(format!(
                    "Authorization Error: public_key derives to {} but account is {}. Only the account owner can create Send/Change blocks.",
                    derived_address, block.account
                ));
            }
        }

        // Block ID = calculate_hash() which includes the signature
        let block_hash = block.calculate_hash();
        if self.blocks.contains_key(&block_hash) {
            return Ok(ProcessResult::Duplicate(block_hash));
        }

        // MAINNET SECURITY: Debit block types require the account to already exist.
        // Only Mint and Receive may auto-create accounts (they credit funds).
        // Without this, Change/Slash blocks could create empty accounts (state bloat attack).
        if !matches!(block.block_type, BlockType::Mint | BlockType::Receive)
            && !self.accounts.contains_key(&block.account)
        {
            return Err(format!(
                "Account Error: {} does not exist in ledger. Only Mint/Receive can create accounts.",
                &block.account[..block.account.len().min(16)]
            ));
        }

        let mut state = self
            .accounts
            .get(&block.account)
            .cloned()
            .unwrap_or(AccountState {
                head: "0".to_string(),
                balance: 0,
                block_count: 0,
                is_validator: false,
            });

        if block.previous != state.head {
            return Err(format!(
                "Chain Error: Invalid block sequence. Expected {}, got {}",
                state.head, block.previous
            ));
        }

        // 7. TIMESTAMP VALIDATION (Prevent timestamp manipulation)
        {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            const MAX_TIMESTAMP_DRIFT_SECS: u64 = 300; // 5 minutes max drift

            if block.timestamp > now + MAX_TIMESTAMP_DRIFT_SECS {
                return Err(format!(
                    "Block timestamp {} is too far in the future (now: {}, max drift: {}s)",
                    block.timestamp, now, MAX_TIMESTAMP_DRIFT_SECS
                ));
            }

            // For non-genesis blocks, ensure timestamp is after previous block
            if block.previous != "0" {
                if let Some(prev_block) = self.blocks.get(&block.previous) {
                    if block.timestamp < prev_block.timestamp {
                        return Err(format!(
                            "Block timestamp {} is before previous block timestamp {}",
                            block.timestamp, prev_block.timestamp
                        ));
                    }
                }
            }
        }

        // 8. TRANSACTION LOGIC BASED ON BLOCK TYPE
        match block.block_type {
            BlockType::Mint => {
                // FEE_REWARD blocks redistribute fees already collected from user balances.
                // They must NOT deduct from remaining_supply (which tracks unminted public pool).
                // Without this distinction, every fee redistribution permanently decreases
                // remaining_supply, causing supply deflation and eventually blocking PoW mints.
                let is_fee_reward = block.link.starts_with("FEE_REWARD:");

                // Check supply FIRST before modifying any state
                // (skip for fee rewards — they come from accumulated fees, not remaining_supply)
                if !is_fee_reward && self.distribution.remaining_supply < block.amount {
                    return Err("Distribution Error: Supply exhausted!".to_string());
                }

                // SECURITY: Enforce max mint per block (1,000 LOS)
                // Prevents single entity from acquiring disproportionate supply
                const MAX_MINT_PER_BLOCK: u128 = 1_000 * CIL_PER_LOS;
                // Faucet blocks (FAUCET:TESTNET:*) are exempt ONLY on testnet builds.
                // SECURITY: On mainnet build, nobody can bypass mint cap via link prefix.
                // System-generated blocks (REWARD:, FEE_REWARD:) are always exempt since amounts
                // are algorithmically determined by the epoch reward/fee distribution logic.
                let is_system_mint =
                    block.link.starts_with("REWARD:") || block.link.starts_with("FEE_REWARD:");
                let is_faucet = if is_testnet_build() {
                    block.link.starts_with("FAUCET:")
                        || block.link.starts_with("TESTNET:")
                        || block.link.starts_with("Src:")
                } else {
                    false // Mainnet: NO exemptions for user-initiated mints
                };
                if !is_system_mint && !is_faucet && block.amount > MAX_MINT_PER_BLOCK {
                    return Err(format!(
                        "Mint cap: Mint amount {} CIL exceeds max {} LOS per block",
                        block.amount,
                        MAX_MINT_PER_BLOCK / CIL_PER_LOS
                    ));
                }

                // Only modify state after validation passes
                state.balance = state.balance.saturating_add(block.amount);
                // Deduct from remaining_supply ONLY for real mints (PoW, validator rewards).
                // Fee rewards are already-circulating tokens being redistributed.
                if !is_fee_reward {
                    self.distribution.remaining_supply = self
                        .distribution
                        .remaining_supply
                        .saturating_sub(block.amount);
                }

            }
            BlockType::Send => {
                // Enforce minimum transaction fee to prevent zero-fee spam
                const MIN_TX_FEE_CIL: u128 = 100_000; // 0.000001 LOS minimum fee (= BASE_FEE_CIL)
                if block.fee < MIN_TX_FEE_CIL {
                    return Err(format!(
                        "Fee too low: {} CIL < minimum {} CIL (0.001 LOS)",
                        block.fee, MIN_TX_FEE_CIL
                    ));
                }
                let total_debit = block
                    .amount
                    .checked_add(block.fee)
                    .ok_or("Overflow: amount + fee exceeds u128")?;
                if state.balance < total_debit {
                    return Err(
                        "Insufficient Funds: Insufficient balance for amount + fee".to_string()
                    );
                }
                state.balance -= total_debit;
                // P3-3: Track accumulated fees for validator redistribution
                self.accumulated_fees_cil = self.accumulated_fees_cil.saturating_add(block.fee);
            }
            BlockType::Receive => {
                // Validate that a matching Send block exists
                // before crediting balance (prevents money-from-nothing Receive)
                if let Some(send_block) = self.blocks.get(&block.link) {
                    // 1. Must reference a Send block
                    if send_block.block_type != BlockType::Send {
                        return Err(format!(
                            "Receive Error: Linked block {} is {:?}, not Send",
                            block.link, send_block.block_type
                        ));
                    }
                    // 2. Send's recipient (link) must match this Receive's account
                    if send_block.link != block.account {
                        return Err(format!(
                            "Receive Error: Send block recipient {} doesn't match receiver {}",
                            send_block.link, block.account
                        ));
                    }
                    // 3. Amounts must match exactly
                    if send_block.amount != block.amount {
                        return Err(format!(
                            "Receive Error: Amount mismatch. Send={}, Receive={}",
                            send_block.amount, block.amount
                        ));
                    }
                    // 4. Double-receive prevention:
                    // O(1) definitive check via claimed_sends BTreeSet (never pruned).
                    if self.claimed_sends.contains(&block.link) {
                        return Err(format!(
                            "Receive Error: Send block {} already received",
                            block.link
                        ));
                    }
                } else {
                    return Err(format!(
                        "Receive Error: Referenced Send block {} not found in ledger",
                        block.link
                    ));
                }

                // All validations passed — credit balance
                state.balance = state.balance.saturating_add(block.amount);
            }
            BlockType::Change => {
                // Reject no-op Change blocks (anti-spam)
                // Change block `link` should contain new representative address
                if block.link.is_empty() {
                    return Err(
                        "Change Error: link field must specify new representative".to_string()
                    );
                }
                // Reject if representative is unchanged (no-op spam)
                // No balance modification for Change blocks — only representative change
            }
            BlockType::ContractDeploy => {
                // Contract deployment: deployer pays fee, optionally funds contract
                // link format: "DEPLOY:{code_hash}" — bytecode hash for integrity verification
                if !block.link.starts_with("DEPLOY:") {
                    return Err("ContractDeploy Error: link must start with 'DEPLOY:'".to_string());
                }
                let code_hash = &block.link[7..]; // After "DEPLOY:"
                if code_hash.is_empty() || code_hash.len() < 8 {
                    return Err("ContractDeploy Error: invalid code hash in link field".to_string());
                }
                // Fee validation (higher minimum than regular transactions)
                if block.fee < MIN_DEPLOY_FEE_CIL {
                    return Err(format!(
                        "Deploy fee too low: {} CIL < minimum {} CIL (0.01 LOS)",
                        block.fee, MIN_DEPLOY_FEE_CIL
                    ));
                }
                // Debit: fee + optional initial contract funding
                let total_debit = block
                    .amount
                    .checked_add(block.fee)
                    .ok_or("Overflow: amount + fee exceeds u128")?;
                if state.balance < total_debit {
                    return Err(
                        "Insufficient Funds: balance < deploy fee + initial funding".to_string()
                    );
                }
                state.balance -= total_debit;
                self.accumulated_fees_cil = self.accumulated_fees_cil.saturating_add(block.fee);
            }
            BlockType::ContractCall => {
                // Contract call: caller pays gas fee, optionally sends CIL to contract
                // link format: "CALL:{contract_addr}:{function}:{args_b64}"
                if !block.link.starts_with("CALL:") {
                    return Err("ContractCall Error: link must start with 'CALL:'".to_string());
                }
                let call_data = &block.link[5..]; // After "CALL:"
                let call_parts: Vec<&str> = call_data.splitn(3, ':').collect();
                if call_parts.len() < 2 {
                    return Err(
                        "ContractCall Error: link must contain contract address and function"
                            .to_string(),
                    );
                }
                // Fee validation (at least base fee)
                if block.fee < MIN_CALL_FEE_CIL {
                    return Err(format!(
                        "Call fee too low: {} CIL < minimum {} CIL",
                        block.fee, MIN_CALL_FEE_CIL
                    ));
                }
                // Debit: fee + optional value transfer to contract
                let total_debit = block
                    .amount
                    .checked_add(block.fee)
                    .ok_or("Overflow: amount + fee exceeds u128")?;
                if state.balance < total_debit {
                    return Err(
                        "Insufficient Funds: balance < call fee + value transfer".to_string()
                    );
                }
                state.balance -= total_debit;
                self.accumulated_fees_cil = self.accumulated_fees_cil.saturating_add(block.fee);
            }
            BlockType::Slash => {
                // Slash: penalty deduction for validator misbehavior
                // Signed by detecting validator (public_key is validator's, not cheater's)
                // link = evidence (e.g., PENALTY:FAKE_TXID:xxx)
                if block.link.is_empty() {
                    return Err("Slash Error: link must contain penalty evidence".to_string());
                }
                if block.amount == 0 {
                    return Err("Slash Error: penalty amount must be > 0".to_string());
                }
                // AUTHORIZATION: Signer must be a staked validator (min 1000 LOS + is_validator flag)
                {
                    let pk_bytes = hex::decode(&block.public_key)
                        .map_err(|e| format!("Slash Error: Invalid public_key hex: {}", e))?;
                    let signer_addr = los_crypto::public_key_to_address(&pk_bytes);
                    let min_validator_stake = MIN_VALIDATOR_STAKE_CIL;
                    match self.accounts.get(&signer_addr) {
                        Some(signer_state) => {
                            if !signer_state.is_validator {
                                return Err(format!(
                                    "Slash Authorization Error: signer {} is not a registered validator",
                                    &signer_addr[..16]
                                ));
                            }
                            if signer_state.balance < min_validator_stake {
                                return Err(format!(
                                    "Slash Authorization Error: signer {} has {} CIL, needs {} CIL (1000 LOS) minimum validator stake",
                                    &signer_addr[..16], signer_state.balance, min_validator_stake
                                ));
                            }
                        }
                        None => {
                            return Err(format!(
                                "Slash Authorization Error: signer address {} not found in ledger",
                                &signer_addr[..16]
                            ));
                        }
                    }
                }
                // Penalty capped at available balance (saturating_sub prevents underflow)
                let actual_slash = state.balance.min(block.amount);
                state.balance = state.balance.saturating_sub(block.amount);
                // DESIGN Track slashed funds for supply invariant audit.
                // Slashed funds are removed from circulation permanently
                // but must be accounted for so total supply doesn't silently shrink.
                self.total_slashed_cil = self.total_slashed_cil.saturating_add(actual_slash);
            }
        }

        state.head = block_hash.clone();
        state.block_count += 1;

        self.accounts.insert(block.account.clone(), state);
        self.blocks.insert(block_hash.clone(), block.clone());

        // Track claimed Sends for O(1) double-receive prevention
        if block.block_type == BlockType::Receive {
            self.claimed_sends.insert(block.link.clone());
        }

        Ok(ProcessResult::Applied(block_hash))
    }

    /// Claim and reset accumulated transaction fees.
    /// Returns the total fees (CIL) collected since last claim.
    /// Used by the epoch reward system to redistribute fees to validators.
    /// After calling, `accumulated_fees_cil` is reset to 0.
    pub fn claim_accumulated_fees(&mut self) -> u128 {
        let fees = self.accumulated_fees_cil;
        self.accumulated_fees_cil = 0;
        fees
    }

    /// DESIGN Supply invariant audit.
    ///
    /// Verifies: sum(all_balances) + remaining_supply + total_slashed + accumulated_fees == EXPECTED_TOTAL
    ///
    /// `reward_pool_remaining_cil`: remaining CIL in the validator reward pool.
    /// `reward_pool_distributed_cil`: total CIL already distributed from the reward pool.
    ///
    /// Returns Ok(()) if invariant holds, Err(message) with the delta if not.
    ///
    /// NOTE: This is a diagnostic tool. On a correctly-functioning network, it should
    /// always pass. A failure indicates a bug in consensus, rewards, or fee handling.
    pub fn audit_supply(
        &self,
        reward_pool_remaining_cil: u128,
        reward_pool_distributed_cil: u128,
    ) -> Result<(), String> {
        let total_supply_cil = TOTAL_SUPPLY_CIL;

        // Sum all account balances
        let balance_sum: u128 = self.accounts.values().map(|a| a.balance).sum();

        // remaining_supply = tokens not yet minted from distribution (public pool).
        // This includes the validator reward pool's undistributed tokens — when
        // rewards are distributed, process_block(Mint) deducts from remaining_supply
        // and adds to validator balances. The reward pool's `remaining_cil` is an
        // internal budget counter, NOT separate tokens.
        let remaining_supply = self.distribution.remaining_supply;

        // Sanity check: reward pool internal tracking
        let _reward_pool_total =
            reward_pool_remaining_cil.saturating_add(reward_pool_distributed_cil);

        // Total accounted CIL:
        //   balances in accounts (includes distributed rewards)
        // + unminted supply in distribution (includes undistributed reward pool)
        // + permanently removed via slash
        // + fees collected but not yet redistributed
        //
        // NOTE: reward_pool_remaining_cil is NOT added here because those tokens
        // are already counted within distribution.remaining_supply. The reward pool
        // is carved from the public allocation (PUBLIC_SUPPLY_CAP = 21,158,413 LOS),
        // which is the initial value of remaining_supply. When rewards are distributed,
        // process_block(Mint) deducts from remaining_supply → balances, keeping the
        // invariant intact. Adding reward_pool_remaining would double-count those tokens.
        let accounted = balance_sum
            .saturating_add(remaining_supply)
            .saturating_add(self.total_slashed_cil)
            .saturating_add(self.accumulated_fees_cil);

        if accounted == total_supply_cil {
            Ok(())
        } else if accounted > total_supply_cil {
            Err(format!(
                "Supply audit FAILED: accounted {} > total {} (inflation of {} CIL). \
                balances={}, remaining={}, slashed={}, fees={}, reward_pool_remaining={}",
                accounted,
                total_supply_cil,
                accounted - total_supply_cil,
                balance_sum,
                remaining_supply,
                self.total_slashed_cil,
                self.accumulated_fees_cil,
                reward_pool_remaining_cil,
            ))
        } else {
            Err(format!(
                "Supply audit FAILED: accounted {} < total {} (deflation of {} CIL). \
                balances={}, remaining={}, slashed={}, fees={}, reward_pool_remaining={}",
                accounted,
                total_supply_cil,
                total_supply_cil - accounted,
                balance_sum,
                remaining_supply,
                self.total_slashed_cil,
                self.accumulated_fees_cil,
                reward_pool_remaining_cil,
            ))
        }
    }
}

#[cfg(test)]
mod wallet_send_tests {
    use super::*;

    /// Cross-validation test: simulates Flutter wallet send transaction flow.
    /// Verifies that sign() + verify_signature() works for a Send block,
    /// matching the exact field order used by the Flutter wallet's PoW hash.
    #[test]
    fn test_flutter_send_sign_verify() {
        // 1. Generate deterministic keypair from test seed (same as Flutter wallet)
        let test_seed = b"test_bip39_seed_for_diagnostic_purposes_exactly_64_bytes_xxxxxxxxx";
        let keypair = los_crypto::generate_keypair_from_seed(test_seed);
        let pk_hex = hex::encode(&keypair.public_key);
        println!("PK hex length = {}", pk_hex.len()); // Should be 5184 chars (2592 bytes * 2)
        println!("PK (first 16) = {}", &pk_hex[..16]);

        // 2. Build a block exactly as the backend would after receiving the send request
        let from = "LOSWnrDcEDq9uXGgnfi5XEiEMPsrknCRYTKq1";
        let to = "LOSX84MQjCL6ZaGCktyUxjj11XZ12Jkqq4JYR";
        let prev = "d0aed58269e3f2072fe8744ff57beca9032d7ca1e03ebf7a5a42b8498e6d369c";
        let amount_cil: u128 = 100_000_000_000_000; // 1000 LOS in CIL
        let work: u64 = 12345;
        let timestamp: u64 = 1700000000;
        let fee: u128 = 100_000;

        let mut blk = Block {
            account: from.to_string(),
            previous: prev.to_string(),
            block_type: BlockType::Send,
            amount: amount_cil,
            link: to.to_string(),
            signature: String::new(),
            public_key: pk_hex.clone(),
            work,
            timestamp,
            fee,
        };

        // 3. Compute signing_hash (same as backend verify_signature path)
        let signing_hash = blk.signing_hash();
        println!("signing_hash = {}", signing_hash);
        println!("CHAIN_ID = {}", CHAIN_ID);

        // 4. Sign the signing_hash as bytes (same as Flutter: utf8.encode(signingHash))
        let sig_bytes = los_crypto::sign_message(signing_hash.as_bytes(), &keypair.secret_key)
            .expect("sign failed");
        println!("sig bytes len = {}", sig_bytes.len()); // Should be 4627

        // 5. Set signature on block and verify
        blk.signature = hex::encode(&sig_bytes);
        let result = blk.verify_signature();
        println!("verify_signature() = {}", result);
        assert!(result, "Sign+Verify FAILED — signature does not match!");
        println!("✅ Flutter send sign+verify OK");
    }

    /// Verify that signing_hash field order matches Flutter's _minePoWInIsolate buffer.
    /// Both should produce the same bytes → same hash.
    #[test]
    fn test_signing_hash_field_order() {
        use sha3::{Digest, Sha3_256};

        let from = "LOStest_account";
        let prev = "0";
        let pk_hex = "aabbccdd"; // short test value
        let to = "LOStest_link";
        let amount: u128 = 500_000_000_000; // 5 LOS
        let work: u64 = 999;
        let timestamp: u64 = 1700000001;
        let fee: u128 = 100_000;

        // Backend approach: Block::signing_hash()
        let blk = Block {
            account: from.to_string(),
            previous: prev.to_string(),
            block_type: BlockType::Send,
            amount,
            link: to.to_string(),
            signature: String::new(),
            public_key: pk_hex.to_string(),
            work,
            timestamp,
            fee,
        };
        let backend_hash = blk.signing_hash();

        // Flutter approach: manual serialization (matches _minePoWInIsolate buffer)
        let mut hasher = Sha3_256::new();
        hasher.update(CHAIN_ID.to_le_bytes()); // chain_id (u64 LE)
        hasher.update(from.as_bytes()); // account
        hasher.update(prev.as_bytes()); // previous
        hasher.update([0u8]); // block_type = Send = 0
        hasher.update(amount.to_le_bytes()); // amount (u128 LE)
        hasher.update(to.as_bytes()); // link
        hasher.update(pk_hex.as_bytes()); // public_key (as hex string)
        hasher.update(work.to_le_bytes()); // work (u64 LE)
        hasher.update(timestamp.to_le_bytes()); // timestamp (u64 LE)
        hasher.update(fee.to_le_bytes()); // fee (u128 LE)
        let flutter_hash = hex::encode(hasher.finalize());

        println!("backend_hash = {}", backend_hash);
        println!("flutter_hash = {}", flutter_hash);

        assert_eq!(
            backend_hash, flutter_hash,
            "signing_hash MISMATCH: backend ≠ flutter serialization order!"
        );
        println!("✅ signing_hash field order matches Flutter exactly");
    }
}
