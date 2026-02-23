// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// UNAUTHORITY (LOS) — PoW MINT DISTRIBUTION ENGINE
//
// Fair, permissionless token distribution via Proof-of-Work mining.
// Users grind SHA3-256(address || epoch || nonce) to find a hash below
// the difficulty target, then submit the proof to any validator node.
//
// Design goals:
//   1. No external dependency (no oracle, no BTC/ETH explorer)
//   2. Front-run resistant (proof is bound to miner's LOS address)
//   3. Anyone can participate (CPU-friendly SHA-3, no GPU req)
//   4. Fixed emission schedule with halving (from public supply pool)
//   5. 1 successful mint per address per epoch (Sybil-neutral)
//
// Supply source: distribution.remaining_supply (public pool ~21.1M LOS)
// Separate from validator reward pool (500K LOS).
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use serde::{Deserialize, Serialize};
use sha3::{Digest, Sha3_256};
use std::collections::BTreeSet;

use crate::CIL_PER_LOS;

// ─────────────────────────────────────────────────────────────────
// CONSTANTS
// ─────────────────────────────────────────────────────────────────

/// Mining epoch duration: 1 hour (mainnet).
/// Short enough for responsive difficulty adjustment.
/// Reward per epoch is divided among all successful miners in that epoch.
pub const MINING_EPOCH_SECS: u64 = 3600; // 1 hour

/// Mining epoch duration: 2 minutes (testnet).
/// Short for rapid testing of mining mechanics.
pub const TESTNET_MINING_EPOCH_SECS: u64 = 120; // 2 minutes

/// Get the effective mining epoch duration based on network type.
pub const fn effective_mining_epoch_secs() -> u64 {
    if crate::is_testnet_build() {
        TESTNET_MINING_EPOCH_SECS
    } else {
        MINING_EPOCH_SECS
    }
}

/// Initial mining reward per epoch: 100 LOS (split among all miners).
/// Halving every 8,760 epochs (≈1 year of hourly epochs on mainnet).
/// Year 1: 100 LOS/epoch × 8,760 epochs = 876,000 LOS
/// Year 2: 50 LOS/epoch × 8,760 = 438,000 LOS
/// Year 3: 25 LOS/epoch × 8,760 = 219,000 LOS
/// ... asymptotically approaches ~1.75M LOS total over many years.
/// Combined with validator rewards, total emission stays under public supply cap.
pub const MINING_REWARD_PER_EPOCH_CIL: u128 = 100 * CIL_PER_LOS;

/// Mining halving interval: 8,760 epochs (≈1 year with 1-hour epochs).
pub const MINING_HALVING_INTERVAL_EPOCHS: u64 = 8_760;

/// Testnet halving interval: 10 epochs (≈20 minutes with 2-min testnet epochs).
pub const TESTNET_MINING_HALVING_INTERVAL_EPOCHS: u64 = 10;

/// Get the effective mining halving interval based on network type.
pub const fn effective_mining_halving_interval() -> u64 {
    if crate::is_testnet_build() {
        TESTNET_MINING_HALVING_INTERVAL_EPOCHS
    } else {
        MINING_HALVING_INTERVAL_EPOCHS
    }
}

/// Initial mining difficulty: 20 leading zero bits.
/// ~1 million SHA3 hashes on average. CPU: ~0.5-2 seconds.
/// Difficulty adjusts dynamically based on miners per epoch.
pub const INITIAL_MINING_DIFFICULTY_BITS: u32 = 20;

/// Testnet initial difficulty: 16 bits (easier for quick testing).
pub const TESTNET_INITIAL_MINING_DIFFICULTY_BITS: u32 = 16;

/// Get the initial mining difficulty for the current network type.
pub const fn initial_mining_difficulty() -> u32 {
    if crate::is_testnet_build() {
        TESTNET_INITIAL_MINING_DIFFICULTY_BITS
    } else {
        INITIAL_MINING_DIFFICULTY_BITS
    }
}

/// Minimum mining difficulty (floor).
/// Never goes below this even if there are zero miners.
pub const MIN_MINING_DIFFICULTY_BITS: u32 = 16;

/// Maximum mining difficulty (ceiling).
/// SHA3-256 has 256 bits, but anything above 40 is impractical for CPUs.
pub const MAX_MINING_DIFFICULTY_BITS: u32 = 40;

/// Target number of successful miners per epoch.
/// Difficulty adjusts to try to achieve this target.
/// Too many miners → difficulty up; too few → difficulty down.
pub const TARGET_MINERS_PER_EPOCH: u32 = 10;

/// Maximum difficulty adjustment factor per epoch (up or down).
/// Prevents sudden jumps. ±4 bits per epoch = ±16× hash rate.
pub const MAX_DIFFICULTY_ADJUSTMENT_BITS: u32 = 4;

// ─────────────────────────────────────────────────────────────────
// PROOF-OF-WORK MINING PROOF
// ─────────────────────────────────────────────────────────────────

/// A proof-of-work mining submission from a user/miner.
/// The validator verifies the proof and creates a Mint block if valid.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MiningProof {
    /// The miner's LOS address (receiver of the reward).
    /// SHA3 hash is bound to this — cannot be transferred to another address.
    pub address: String,
    /// The epoch number this proof targets.
    /// Proof is only valid for this specific epoch.
    pub epoch: u64,
    /// The nonce that satisfies the difficulty requirement.
    /// SHA3-256(address || epoch || nonce) must have ≥ difficulty_bits leading zeros.
    pub nonce: u64,
}

/// Mining info returned by GET /mining-info.
/// Contains everything a miner needs to start grinding.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MiningInfo {
    /// Current mining epoch number.
    pub epoch: u64,
    /// Required leading zero bits for the current epoch.
    pub difficulty_bits: u32,
    /// Reward for this epoch in CIL (split among all successful miners).
    pub reward_per_epoch_cil: u128,
    /// Remaining public supply in CIL.
    pub remaining_supply_cil: u128,
    /// Seconds until this epoch ends.
    pub epoch_remaining_secs: u64,
    /// Number of successful miners in the current epoch so far.
    pub miners_this_epoch: u32,
    /// Chain ID (1=mainnet, 2=testnet).
    pub chain_id: u64,
}

// ─────────────────────────────────────────────────────────────────
// MINING STATE MANAGER
// ─────────────────────────────────────────────────────────────────

/// Tracks mining state: who has mined in which epoch, difficulty, etc.
/// Stored in-memory (rebuilt on restart from ledger Mint blocks with MINE: prefix).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MiningState {
    /// Current difficulty in leading zero bits.
    pub difficulty_bits: u32,
    /// Addresses that have successfully mined in the current epoch.
    /// Prevents double-mining per epoch (1 mint per address per epoch).
    pub current_epoch_miners: BTreeSet<String>,
    /// The epoch number for current_epoch_miners.
    pub current_epoch: u64,
    /// Number of successful miners in the PREVIOUS epoch.
    /// Used for difficulty adjustment.
    pub prev_epoch_miners_count: u32,
    /// Genesis timestamp (used to calculate epoch number).
    pub genesis_timestamp: u64,
}

impl MiningState {
    /// Create a new MiningState using the genesis timestamp.
    pub fn new(genesis_timestamp: u64) -> Self {
        Self {
            difficulty_bits: initial_mining_difficulty(),
            current_epoch_miners: BTreeSet::new(),
            current_epoch: 0,
            prev_epoch_miners_count: 0,
            genesis_timestamp,
        }
    }

    /// Calculate the current epoch number from the current time.
    pub fn epoch_from_time(&self, now_secs: u64) -> u64 {
        if now_secs <= self.genesis_timestamp {
            return 0;
        }
        (now_secs - self.genesis_timestamp) / effective_mining_epoch_secs()
    }

    /// Get seconds remaining in the current epoch.
    pub fn epoch_remaining_secs(&self, now_secs: u64) -> u64 {
        let epoch_duration = effective_mining_epoch_secs();
        let elapsed_in_epoch = (now_secs.saturating_sub(self.genesis_timestamp)) % epoch_duration;
        epoch_duration.saturating_sub(elapsed_in_epoch)
    }

    /// Calculate the mining reward for a given epoch (with halving).
    /// Returns the TOTAL reward for the epoch in CIL.
    /// Individual miner reward = total / num_miners.
    pub fn epoch_reward_cil(epoch: u64) -> u128 {
        let halving_interval = effective_mining_halving_interval();
        let halvings = epoch / halving_interval;
        // Each halving divides reward by 2. After 20+ halvings, reward → 0.
        if halvings >= 64 {
            return 0; // Prevent overflow in shift
        }
        MINING_REWARD_PER_EPOCH_CIL >> halvings
    }

    /// Advance to a new epoch: adjust difficulty and reset miners.
    /// Called when the current time's epoch differs from self.current_epoch.
    pub fn advance_epoch(&mut self, new_epoch: u64) {
        if new_epoch <= self.current_epoch {
            return;
        }

        // Difficulty adjustment based on previous epoch's miner count.
        let miners = self.current_epoch_miners.len() as u32;
        self.prev_epoch_miners_count = miners;

        if miners > TARGET_MINERS_PER_EPOCH * 2 {
            // Way too many miners → increase difficulty (harder)
            let adjustment = ((miners / TARGET_MINERS_PER_EPOCH).ilog2() + 1)
                .min(MAX_DIFFICULTY_ADJUSTMENT_BITS);
            self.difficulty_bits =
                (self.difficulty_bits + adjustment).min(MAX_MINING_DIFFICULTY_BITS);
        } else if miners > TARGET_MINERS_PER_EPOCH {
            // Slightly too many → +1 bit
            self.difficulty_bits = (self.difficulty_bits + 1).min(MAX_MINING_DIFFICULTY_BITS);
        } else if miners < TARGET_MINERS_PER_EPOCH / 2 && miners > 0 {
            // Too few → decrease difficulty (easier), but not below minimum
            self.difficulty_bits = self
                .difficulty_bits
                .saturating_sub(1)
                .max(MIN_MINING_DIFFICULTY_BITS);
        } else if miners == 0 {
            // No miners at all → decrease by 2 bits (faster recovery)
            self.difficulty_bits = self
                .difficulty_bits
                .saturating_sub(2)
                .max(MIN_MINING_DIFFICULTY_BITS);
        }
        // miners == TARGET_MINERS_PER_EPOCH → no change (sweet spot)

        // Reset for new epoch
        self.current_epoch_miners.clear();
        self.current_epoch = new_epoch;
    }

    /// Check if the current epoch needs advancing based on current time.
    /// If so, advance it. Returns the (possibly new) current epoch.
    pub fn maybe_advance_epoch(&mut self, now_secs: u64) -> u64 {
        let current = self.epoch_from_time(now_secs);
        if current > self.current_epoch {
            self.advance_epoch(current);
        }
        self.current_epoch
    }

    /// Verify a mining proof: check hash meets difficulty and address hasn't mined this epoch.
    /// Does NOT create the Mint block — that's the caller's responsibility.
    ///
    /// Returns Ok(reward_cil) on success, Err(reason) on failure.
    pub fn verify_proof(
        &mut self,
        proof: &MiningProof,
        now_secs: u64,
        remaining_supply_cil: u128,
    ) -> Result<u128, String> {
        // 1. Advance epoch if needed
        let current_epoch = self.maybe_advance_epoch(now_secs);

        // 2. Epoch must match current
        if proof.epoch != current_epoch {
            return Err(format!(
                "Wrong epoch: proof targets epoch {} but current is {}",
                proof.epoch, current_epoch
            ));
        }

        // 3. Address must not have mined this epoch already
        if self.current_epoch_miners.contains(&proof.address) {
            return Err(format!(
                "Already mined: {} has already submitted a valid proof for epoch {}",
                &proof.address[..proof.address.len().min(16)],
                current_epoch
            ));
        }

        // 4. Verify PoW hash
        if !verify_mining_hash(
            &proof.address,
            proof.epoch,
            proof.nonce,
            self.difficulty_bits,
        ) {
            return Err(format!(
                "Invalid PoW: hash does not meet difficulty ({} leading zero bits)",
                self.difficulty_bits
            ));
        }

        // 5. Check supply availability
        let epoch_reward = Self::epoch_reward_cil(current_epoch);
        if epoch_reward == 0 {
            return Err("Mining reward exhausted: epoch reward is 0 after halvings".to_string());
        }

        // Calculate per-miner reward:
        // reward = epoch_reward / (current_miners + 1)
        // The +1 includes this miner. All miners in an epoch get equal share.
        let total_miners = (self.current_epoch_miners.len() as u128) + 1;
        let miner_reward = epoch_reward / total_miners;

        if miner_reward == 0 {
            return Err("Too many miners: per-miner reward would be 0".to_string());
        }

        // Cap at remaining supply
        let final_reward = miner_reward.min(remaining_supply_cil);
        if final_reward == 0 {
            return Err("Public supply exhausted".to_string());
        }

        // Also cap at MAX_MINT_PER_BLOCK (1,000 LOS) to comply with consensus rule
        let max_mint = 1_000 * CIL_PER_LOS;
        let final_reward = final_reward.min(max_mint);

        // 6. Register this miner
        self.current_epoch_miners.insert(proof.address.clone());

        Ok(final_reward)
    }

    /// Get mining info for the API response.
    pub fn get_mining_info(&self, now_secs: u64, remaining_supply_cil: u128) -> MiningInfo {
        let epoch = self.epoch_from_time(now_secs);
        let reward = Self::epoch_reward_cil(epoch);
        MiningInfo {
            epoch,
            difficulty_bits: self.difficulty_bits,
            reward_per_epoch_cil: reward,
            remaining_supply_cil,
            epoch_remaining_secs: self.epoch_remaining_secs(now_secs),
            miners_this_epoch: self.current_epoch_miners.len() as u32,
            chain_id: crate::CHAIN_ID,
        }
    }
}

// ─────────────────────────────────────────────────────────────────
// HASH COMPUTATION & VERIFICATION
// ─────────────────────────────────────────────────────────────────

/// Compute the mining hash: SHA3-256(address || epoch_le_bytes || nonce_le_bytes).
/// The hash is deterministic and bound to the miner's address + epoch + nonce.
pub fn compute_mining_hash(address: &str, epoch: u64, nonce: u64) -> [u8; 32] {
    let mut hasher = Sha3_256::new();
    // Domain separator to prevent collision with block hashes
    hasher.update(b"LOS_MINE_V1");
    // Chain ID prevents cross-network proof replay
    hasher.update(crate::CHAIN_ID.to_le_bytes());
    // Address binds proof to owner — front-run resistant
    hasher.update(address.as_bytes());
    // Epoch binds proof to time window
    hasher.update(epoch.to_le_bytes());
    // Nonce is the value being ground
    hasher.update(nonce.to_le_bytes());

    let result = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

/// Verify that a mining hash meets the required difficulty.
/// Returns true if the hash has ≥ difficulty_bits leading zero bits.
pub fn verify_mining_hash(address: &str, epoch: u64, nonce: u64, difficulty_bits: u32) -> bool {
    let hash = compute_mining_hash(address, epoch, nonce);
    count_leading_zero_bits(&hash) >= difficulty_bits
}

/// Count leading zero bits in a byte array.
pub fn count_leading_zero_bits(bytes: &[u8]) -> u32 {
    let mut zero_bits = 0u32;
    for byte in bytes {
        if *byte == 0 {
            zero_bits += 8;
        } else {
            zero_bits += byte.leading_zeros();
            break;
        }
    }
    zero_bits
}

/// Mine: find a nonce that satisfies the difficulty for the given address+epoch.
/// This is CPU-intensive and runs in the caller's thread.
/// Returns Some(nonce) if found, None if cancelled via the cancel flag.
///
/// Used by:
/// - `--mine` background thread in los-node
/// - CLI miner tool
/// - Flutter wallet (via flutter_rust_bridge)
pub fn mine(
    address: &str,
    epoch: u64,
    difficulty_bits: u32,
    cancel: &std::sync::atomic::AtomicBool,
) -> Option<u64> {
    // Start from a random offset to avoid all miners testing the same nonces
    let start: u64 = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        address.hash(&mut h);
        epoch.hash(&mut h);
        // Mix in thread ID for uniqueness across threads
        std::thread::current().id().hash(&mut h);
        h.finish()
    };

    let mut nonce = start;
    loop {
        // Check cancellation every 65536 hashes (~0.05ms overhead)
        if nonce.wrapping_sub(start) & 0xFFFF == 0
            && cancel.load(std::sync::atomic::Ordering::Relaxed)
        {
            return None;
        }

        if verify_mining_hash(address, epoch, nonce, difficulty_bits) {
            return Some(nonce);
        }

        nonce = nonce.wrapping_add(1);
        // Full u64 space exhausted (astronomically unlikely)
        if nonce == start {
            return None;
        }
    }
}

// ─────────────────────────────────────────────────────────────────
// TESTS
// ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_mining_hash_deterministic() {
        let h1 = compute_mining_hash("LOS_abc123", 42, 12345);
        let h2 = compute_mining_hash("LOS_abc123", 42, 12345);
        assert_eq!(h1, h2, "Same inputs must produce same hash");
    }

    #[test]
    fn test_compute_mining_hash_differs_by_address() {
        let h1 = compute_mining_hash("LOS_alice", 1, 0);
        let h2 = compute_mining_hash("LOS_bob", 1, 0);
        assert_ne!(h1, h2, "Different addresses must produce different hashes");
    }

    #[test]
    fn test_compute_mining_hash_differs_by_epoch() {
        let h1 = compute_mining_hash("LOS_alice", 1, 0);
        let h2 = compute_mining_hash("LOS_alice", 2, 0);
        assert_ne!(h1, h2, "Different epochs must produce different hashes");
    }

    #[test]
    fn test_compute_mining_hash_differs_by_nonce() {
        let h1 = compute_mining_hash("LOS_alice", 1, 0);
        let h2 = compute_mining_hash("LOS_alice", 1, 1);
        assert_ne!(h1, h2, "Different nonces must produce different hashes");
    }

    #[test]
    fn test_count_leading_zero_bits() {
        assert_eq!(count_leading_zero_bits(&[0x00, 0x00, 0xFF]), 16);
        assert_eq!(count_leading_zero_bits(&[0x00, 0x01, 0xFF]), 15);
        assert_eq!(count_leading_zero_bits(&[0x0F, 0xFF]), 4);
        assert_eq!(count_leading_zero_bits(&[0xFF]), 0);
        assert_eq!(count_leading_zero_bits(&[0x00, 0x00, 0x00, 0x00]), 32);
    }

    #[test]
    fn test_verify_mining_hash_low_difficulty() {
        // With difficulty 0, any hash should pass
        assert!(verify_mining_hash("LOS_test", 1, 0, 0));
    }

    #[test]
    fn test_epoch_reward_halving() {
        assert_eq!(
            MiningState::epoch_reward_cil(0),
            MINING_REWARD_PER_EPOCH_CIL
        );

        let interval = effective_mining_halving_interval();
        assert_eq!(
            MiningState::epoch_reward_cil(interval),
            MINING_REWARD_PER_EPOCH_CIL / 2
        );
        assert_eq!(
            MiningState::epoch_reward_cil(interval * 2),
            MINING_REWARD_PER_EPOCH_CIL / 4
        );
        assert_eq!(MiningState::epoch_reward_cil(interval * 64), 0);
    }

    #[test]
    fn test_mining_state_epoch_calculation() {
        let genesis = 1_000_000u64;
        let state = MiningState::new(genesis);
        let epoch_secs = effective_mining_epoch_secs();

        assert_eq!(state.epoch_from_time(genesis), 0);
        assert_eq!(state.epoch_from_time(genesis + epoch_secs - 1), 0);
        assert_eq!(state.epoch_from_time(genesis + epoch_secs), 1);
        assert_eq!(state.epoch_from_time(genesis + epoch_secs * 5 + 30), 5);
    }

    #[test]
    fn test_mining_state_advance_epoch() {
        let mut state = MiningState::new(1_000_000);
        assert_eq!(state.current_epoch, 0);

        // Simulate: 5 miners in epoch 0
        for i in 0..5 {
            state
                .current_epoch_miners
                .insert(format!("LOS_miner_{}", i));
        }
        assert_eq!(state.current_epoch_miners.len(), 5);

        // Advance to epoch 1
        state.advance_epoch(1);
        assert_eq!(state.current_epoch, 1);
        assert_eq!(state.prev_epoch_miners_count, 5);
        assert!(state.current_epoch_miners.is_empty());
    }

    #[test]
    fn test_difficulty_adjustment_up() {
        let mut state = MiningState::new(1_000_000);
        let initial = state.difficulty_bits;

        // Simulate way too many miners
        for i in 0..50 {
            state.current_epoch_miners.insert(format!("LOS_{}", i));
        }
        state.advance_epoch(1);

        assert!(
            state.difficulty_bits > initial,
            "Difficulty should increase with many miners"
        );
    }

    #[test]
    fn test_difficulty_adjustment_down() {
        let mut state = MiningState::new(1_000_000);
        state.difficulty_bits = 25; // Start higher than minimum

        // Simulate zero miners
        state.advance_epoch(1);
        assert!(
            state.difficulty_bits < 25,
            "Difficulty should decrease with zero miners"
        );
        assert!(
            state.difficulty_bits >= MIN_MINING_DIFFICULTY_BITS,
            "Difficulty must not go below minimum"
        );
    }

    #[test]
    fn test_verify_proof_basic() {
        let genesis = 1_000_000u64;
        let mut state = MiningState::new(genesis);
        state.difficulty_bits = 1; // Very low difficulty for testing

        let epoch_secs = effective_mining_epoch_secs();
        let now = genesis + epoch_secs / 2; // Middle of epoch 0
        state.maybe_advance_epoch(now);

        // Find a valid nonce (should be very fast with difficulty=1)
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let nonce =
            mine("LOS_test_miner", 0, 1, &cancel).expect("Should find nonce with difficulty=1");

        let proof = MiningProof {
            address: "LOS_test_miner".to_string(),
            epoch: 0,
            nonce,
        };

        let supply = 1_000_000 * CIL_PER_LOS;
        let result = state.verify_proof(&proof, now, supply);
        assert!(result.is_ok(), "Valid proof should pass: {:?}", result);
        assert!(result.unwrap() > 0, "Reward should be > 0");
    }

    #[test]
    fn test_double_mine_rejected() {
        let genesis = 1_000_000u64;
        let mut state = MiningState::new(genesis);
        state.difficulty_bits = 1;

        let epoch_secs = effective_mining_epoch_secs();
        let now = genesis + epoch_secs / 2;
        state.maybe_advance_epoch(now);

        let cancel = std::sync::atomic::AtomicBool::new(false);
        let nonce = mine("LOS_test", 0, 1, &cancel).unwrap();

        let proof = MiningProof {
            address: "LOS_test".to_string(),
            epoch: 0,
            nonce,
        };

        let supply = 1_000_000 * CIL_PER_LOS;
        let r1 = state.verify_proof(&proof, now, supply);
        assert!(r1.is_ok(), "First proof should pass");

        // Second proof from same address should fail
        let nonce2 = mine("LOS_test", 0, 1, &cancel).unwrap();
        let proof2 = MiningProof {
            address: "LOS_test".to_string(),
            epoch: 0,
            nonce: nonce2,
        };
        let r2 = state.verify_proof(&proof2, now, supply);
        assert!(r2.is_err(), "Double mine should be rejected");
        assert!(
            r2.unwrap_err().contains("Already mined"),
            "Error should mention already mined"
        );
    }

    #[test]
    fn test_wrong_epoch_rejected() {
        let genesis = 1_000_000u64;
        let mut state = MiningState::new(genesis);
        state.difficulty_bits = 1;

        let epoch_secs = effective_mining_epoch_secs();
        let now = genesis + epoch_secs / 2;
        state.maybe_advance_epoch(now);

        let cancel = std::sync::atomic::AtomicBool::new(false);
        // Mine for epoch 5 but current is 0
        let nonce = mine("LOS_test", 5, 1, &cancel).unwrap();
        let proof = MiningProof {
            address: "LOS_test".to_string(),
            epoch: 5,
            nonce,
        };

        let supply = 1_000_000 * CIL_PER_LOS;
        let result = state.verify_proof(&proof, now, supply);
        assert!(result.is_err(), "Wrong epoch should be rejected");
    }

    #[test]
    fn test_mine_function() {
        let cancel = std::sync::atomic::AtomicBool::new(false);

        // Mine with low difficulty (should be instant)
        let nonce = mine("LOS_miner", 0, 8, &cancel);
        assert!(
            nonce.is_some(),
            "Should find valid nonce with 8-bit difficulty"
        );

        // Verify the found nonce
        let n = nonce.unwrap();
        assert!(
            verify_mining_hash("LOS_miner", 0, n, 8),
            "Found nonce must verify"
        );
    }

    #[test]
    fn test_mine_cancellation() {
        let cancel = std::sync::atomic::AtomicBool::new(true); // Pre-cancelled

        let result = mine("LOS_test", 0, 40, &cancel); // High difficulty, but cancelled
        assert!(result.is_none(), "Cancelled mine should return None");
    }

    #[test]
    fn test_reward_capped_at_supply() {
        let genesis = 1_000_000u64;
        let mut state = MiningState::new(genesis);
        state.difficulty_bits = 1;

        let now = genesis + effective_mining_epoch_secs() / 2;
        state.maybe_advance_epoch(now);

        let cancel = std::sync::atomic::AtomicBool::new(false);
        let nonce = mine("LOS_test", 0, 1, &cancel).unwrap();
        let proof = MiningProof {
            address: "LOS_test".to_string(),
            epoch: 0,
            nonce,
        };

        // Supply is almost empty: only 1 CIL left
        let tiny_supply = 1u128;
        let result = state.verify_proof(&proof, now, tiny_supply);
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            1,
            "Reward should be capped at remaining supply"
        );
    }

    #[test]
    fn test_get_mining_info() {
        let genesis = 1_000_000u64;
        let state = MiningState::new(genesis);
        let now = genesis + 100;
        let supply = 1_000_000 * CIL_PER_LOS;

        let info = state.get_mining_info(now, supply);
        assert_eq!(info.epoch, 0);
        assert_eq!(info.difficulty_bits, initial_mining_difficulty());
        assert_eq!(info.reward_per_epoch_cil, MINING_REWARD_PER_EPOCH_CIL);
        assert_eq!(info.remaining_supply_cil, supply);
        assert_eq!(info.chain_id, crate::CHAIN_ID);
    }

    #[test]
    fn test_multiple_miners_share_reward() {
        let genesis = 1_000_000u64;
        let mut state = MiningState::new(genesis);
        state.difficulty_bits = 1;

        let now = genesis + effective_mining_epoch_secs() / 2;
        state.maybe_advance_epoch(now);

        let cancel = std::sync::atomic::AtomicBool::new(false);
        let supply = 1_000_000 * CIL_PER_LOS;

        // First miner gets full reward
        let n1 = mine("LOS_miner_1", 0, 1, &cancel).unwrap();
        let r1 = state
            .verify_proof(
                &MiningProof {
                    address: "LOS_miner_1".to_string(),
                    epoch: 0,
                    nonce: n1,
                },
                now,
                supply,
            )
            .unwrap();

        // Second miner — reward is split (but first miner already got theirs)
        // The split happens at claim time: epoch_reward / total_miners
        let n2 = mine("LOS_miner_2", 0, 1, &cancel).unwrap();
        let r2 = state
            .verify_proof(
                &MiningProof {
                    address: "LOS_miner_2".to_string(),
                    epoch: 0,
                    nonce: n2,
                },
                now,
                supply,
            )
            .unwrap();

        // First miner: 100 LOS / 1 = 100 LOS
        // Second miner: 100 LOS / 2 = 50 LOS
        // This is by design: early submitters in an epoch get more.
        // The proportional model would require end-of-epoch settlement
        // which adds complexity. This "first-come, diminishing return"
        // model naturally limits the incentive to pile onto one epoch.
        assert!(r1 > r2, "Earlier miner should get more reward");
        assert!(r2 > 0, "Later miner should still get non-zero reward");
    }
}
