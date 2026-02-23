// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// UNAUTHORITY (LOS) - DYNAMIC FEE SCALING
//
// Task #3a: Anti-Spam Mechanism
// - Track transaction rate per address
// - Apply exponential fee multiplier (x2, x4, x8...)
// - Prevent spam attacks and network abuse
// - Non-persistent penalty system
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Spam detection threshold (transactions per second)
pub const SPAM_THRESHOLD_TX_PER_SEC: u32 = 10;

/// Base multiplier for fee scaling
pub const SPAM_SCALING_FACTOR: u32 = 2;

/// Time window for rate limiting (seconds)
pub const RATE_LIMIT_WINDOW_SECS: u64 = 1;

/// Per-address spam detection state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddressSpamState {
    /// Recent transaction timestamps (sliding window)
    pub recent_tx_timestamps: Vec<u64>,

    /// Current fee multiplier for this address
    pub fee_multiplier: u128,

    /// Last multiplier reset time
    pub last_reset_timestamp: u64,

    /// Total spam events detected
    pub spam_violations: u32,
}

impl Default for AddressSpamState {
    fn default() -> Self {
        Self {
            recent_tx_timestamps: Vec::new(),
            fee_multiplier: 1,
            last_reset_timestamp: 0,
            spam_violations: 0,
        }
    }
}

/// Spam Detection Manager
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpamDetector {
    /// Per-address spam tracking
    address_states: BTreeMap<String, AddressSpamState>,

    /// Spam threshold (tx/sec)
    spam_threshold: u32,

    /// Fee scaling factor (multiplier)
    scaling_factor: u32,
}

impl SpamDetector {
    /// Create new spam detector with custom thresholds
    pub fn new(threshold: u32, factor: u32) -> Self {
        Self {
            address_states: BTreeMap::new(),
            spam_threshold: threshold,
            scaling_factor: factor,
        }
    }

    /// Create with default thresholds
    pub fn default_config() -> Self {
        Self::new(SPAM_THRESHOLD_TX_PER_SEC, SPAM_SCALING_FACTOR)
    }

    /// Check transaction rate and calculate fee multiplier
    ///
    /// # Arguments
    /// * `sender_address` - Address sending the transaction
    /// * `current_timestamp` - Current block timestamp (seconds)
    ///
    /// # Returns
    /// Fee multiplier to apply (1 = normal, 2 = 2x, 4 = 4x, etc.)
    ///
    /// # Example
    /// ```ignore
    /// let mut detector = SpamDetector::new(10, 2);
    ///
    /// // First 10 tx in same second = normal rate
    /// for i in 0..10 {
    ///     let multiplier = detector.check_and_update("user1", 1000)?;
    ///     assert_eq!(multiplier, 1);
    /// }
    ///
    /// // 11th tx = above threshold
    /// let multiplier = detector.check_and_update("user1", 1000)?;
    /// assert_eq!(multiplier, 2); // 2x multiplier
    /// ```
    pub fn check_and_update(
        &mut self,
        sender_address: &str,
        current_timestamp: u64,
    ) -> Result<u128, String> {
        let state = self
            .address_states
            .entry(sender_address.to_string())
            .or_default();

        // Clean old timestamps outside the rate limit window
        state
            .recent_tx_timestamps
            .retain(|&ts| current_timestamp.saturating_sub(ts) < RATE_LIMIT_WINDOW_SECS);

        let tx_count = state.recent_tx_timestamps.len() as u32;

        // Check if address is spamming
        if tx_count >= self.spam_threshold {
            // Calculate excess transactions
            let excess = tx_count - self.spam_threshold + 1;

            // Exponential scaling: multiplier^excess
            let multiplier = (self.scaling_factor as u128).pow(excess);

            // Update state
            state.fee_multiplier = multiplier;
            state.spam_violations += 1;

            // Record this transaction
            state.recent_tx_timestamps.push(current_timestamp);

            Ok(multiplier)
        } else {
            // Normal rate - no multiplier
            state.fee_multiplier = 1;

            // Record this transaction
            state.recent_tx_timestamps.push(current_timestamp);

            Ok(1)
        }
    }

    /// Get current fee multiplier for address (without updating)
    pub fn get_multiplier(&self, sender_address: &str) -> u128 {
        self.address_states
            .get(sender_address)
            .map(|state| state.fee_multiplier)
            .unwrap_or(1)
    }

    /// Reset multiplier for an address (called after time window passes)
    pub fn reset_multiplier(&mut self, sender_address: &str, current_timestamp: u64) {
        if let Some(state) = self.address_states.get_mut(sender_address) {
            // Only reset if enough time has passed
            if current_timestamp.saturating_sub(state.last_reset_timestamp) > RATE_LIMIT_WINDOW_SECS
            {
                state.fee_multiplier = 1;
                state.recent_tx_timestamps.clear();
                state.last_reset_timestamp = current_timestamp;
            }
        }
    }

    /// Get spam state for address
    pub fn get_spam_state(&self, sender_address: &str) -> Option<AddressSpamState> {
        self.address_states.get(sender_address).cloned()
    }

    /// Get all spam violators (for monitoring/statistics)
    pub fn get_violators(&self) -> Vec<(String, u32)> {
        self.address_states
            .iter()
            .filter(|(_, state)| state.spam_violations > 0)
            .map(|(addr, state)| (addr.clone(), state.spam_violations))
            .collect()
    }

    /// Clear all spam detection state (for testing/reset)
    pub fn clear_all(&mut self) {
        self.address_states.clear();
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// FEE SCALING LOGIC
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Apply dynamic fee scaling based on multiplier
///
/// # Arguments
/// * `base_fee` - Base transaction fee (from validator_rewards module)
/// * `multiplier` - Fee multiplier (1x, 2x, 4x, etc.)
///
/// # Returns
/// Final fee after scaling, respecting MAX_GAS_PER_TX limit
pub fn apply_fee_multiplier(base_fee: u128, multiplier: u128) -> Result<u128, String> {
    const MAX_GAS_PER_TX: u128 = 10_000_000; // 0.1 LOS

    let final_fee = base_fee.saturating_mul(multiplier);

    if final_fee > MAX_GAS_PER_TX {
        return Err(format!(
            "Scaled fee {} exceeds maximum {} (base: {}, multiplier: {})",
            final_fee, MAX_GAS_PER_TX, base_fee, multiplier
        ));
    }

    Ok(final_fee)
}

/// Calculate next escalation multiplier
///
/// Given current multiplier and violations, determine if escalation needed
pub fn calculate_escalation_multiplier(current_violations: u32, base_factor: u32) -> u128 {
    if current_violations == 0 {
        return 1;
    }
    (base_factor as u128).pow(current_violations)
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BURN LIMIT PER BLOCK
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Maximum CIL that can be burned in a single block (1000 LOS as per whitepaper)
/// 1 LOS = 100_000_000_000 CIL (10^11)
pub const BURN_LIMIT_PER_BLOCK_CIL: u128 = 1_000 * 100_000_000_000; // 1000 LOS max per block

/// Track burn activity per block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockBurnState {
    pub block_height: u64,
    pub total_burn_cil: u128,
    pub burn_count: u32,
    pub remaining_capacity: u128,
}

impl BlockBurnState {
    pub fn new(block_height: u64) -> Self {
        Self {
            block_height,
            total_burn_cil: 0,
            burn_count: 0,
            remaining_capacity: BURN_LIMIT_PER_BLOCK_CIL,
        }
    }

    /// Try to add burn to block - returns true if allowed
    pub fn try_add_burn(&mut self, burn_amount: u128) -> Result<bool, String> {
        if burn_amount > self.remaining_capacity {
            return Err(format!(
                "Burn amount {} exceeds block capacity {} (total this block: {})",
                burn_amount, self.remaining_capacity, self.total_burn_cil
            ));
        }

        self.total_burn_cil += burn_amount;
        self.remaining_capacity = self.remaining_capacity.saturating_sub(burn_amount);
        self.burn_count += 1;

        Ok(self.remaining_capacity > 0)
    }

    /// Check if block capacity is exhausted
    pub fn is_capacity_exhausted(&self) -> bool {
        self.remaining_capacity == 0
    }

    /// Get fill percentage in basis points (0-10000 = 0%-100%)
    /// MAINNET SAFETY: Integer-only. No f64 in production code.
    pub fn get_capacity_percentage_bps(&self) -> u32 {
        let used = BURN_LIMIT_PER_BLOCK_CIL - self.remaining_capacity;
        // (used * 10000) / total gives basis points (0.01% precision)
        ((used * 10_000) / BURN_LIMIT_PER_BLOCK_CIL) as u32
    }

    /// Get fill percentage (0-100) — DEPRECATED: use get_capacity_percentage_bps() instead
    /// Kept for backward compatibility during migration. Will be removed before mainnet.
    /// MAINNET SAFETY: Excluded from mainnet builds (uses f64)
    #[cfg(not(feature = "mainnet"))]
    #[deprecated(note = "Uses f64. Migrate to get_capacity_percentage_bps() for mainnet.")]
    pub fn get_capacity_percentage(&self) -> f64 {
        let used = (BURN_LIMIT_PER_BLOCK_CIL - self.remaining_capacity) as f64;
        let total = BURN_LIMIT_PER_BLOCK_CIL as f64;
        (used / total) * 100.0
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TESTS
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spam_detection_normal_rate() {
        let mut detector = SpamDetector::new(10, 2);

        // First 10 transactions at normal rate (under threshold)
        for _i in 0..10 {
            let multiplier = detector.check_and_update("user1", 1000).unwrap();
            assert_eq!(multiplier, 1); // No multiplier
        }
    }

    #[test]
    fn test_spam_detection_exceeding_threshold() {
        let mut detector = SpamDetector::new(10, 2);

        // Send 11 transactions (exceeds 10 tx/sec threshold)
        for i in 0..11 {
            let multiplier = detector.check_and_update("user1", 1000).unwrap();

            if i < 10 {
                assert_eq!(multiplier, 1); // Normal rate
            } else {
                assert_eq!(multiplier, 2); // 11th transaction triggers 2x multiplier
            }
        }
    }

    #[test]
    fn test_spam_escalation() {
        let mut detector = SpamDetector::new(10, 2);
        let timestamp = 1000u64;

        // Send transactions to trigger escalation
        let mut multipliers = Vec::new();
        for _i in 0..30 {
            let mult = detector.check_and_update("whale", timestamp).unwrap();
            multipliers.push(mult);
        }

        // Check multiplier escalation pattern
        // Multiplier is recalculated for each tx based on cumulative count
        // First 10: multiplier = 1
        // 11-20: each tx adds to count, multiplier = 2^(count-10)
        // Pattern: [1,1,1,1,1,1,1,1,1,1, 2,4,8,16,32,64,128,256,512,1024, ...]
        assert_eq!(multipliers[9], 1); // 10th tx
        assert_eq!(multipliers[10], 2); // 11th tx: 2^(11-10) = 2
        assert_eq!(multipliers[11], 4); // 12th tx: 2^(12-10) = 4
        assert_eq!(multipliers[12], 8); // 13th tx: 2^(13-10) = 8
                                        // Verify exponential escalation
        assert!(multipliers[20] > multipliers[10]); // Escalation continues
    }

    #[test]
    fn test_fee_multiplier_application() {
        let base_fee = 3_560u128;
        let multiplier = 2u128;

        let final_fee = apply_fee_multiplier(base_fee, multiplier).unwrap();
        assert_eq!(final_fee, 7_120); // 3,560 × 2
    }

    #[test]
    fn test_fee_multiplier_exceeds_max() {
        let base_fee = 6_000_000u128; // Already high
        let multiplier = 2u128;

        let result = apply_fee_multiplier(base_fee, multiplier);
        assert!(result.is_err()); // Should exceed MAX_GAS_PER_TX
    }

    #[test]
    fn test_burn_limit_per_block() {
        let mut burn_state = BlockBurnState::new(1);

        // Add burn transactions up to limit (1000 LOS = 100_000_000_000_000 CIL)
        let burn_amount = BURN_LIMIT_PER_BLOCK_CIL / 2; // 500 LOS
        burn_state.try_add_burn(burn_amount).unwrap();
        burn_state.try_add_burn(burn_amount).unwrap();

        // Should have capacity exhausted
        assert!(burn_state.is_capacity_exhausted());
        assert_eq!(burn_state.total_burn_cil, BURN_LIMIT_PER_BLOCK_CIL);
        assert_eq!(burn_state.burn_count, 2);
    }

    #[test]
    fn test_burn_limit_exceeded() {
        let mut burn_state = BlockBurnState::new(1);

        // Try to add more than block capacity
        let result = burn_state.try_add_burn(BURN_LIMIT_PER_BLOCK_CIL + 1);
        assert!(result.is_err());
    }

    #[test]
    fn test_capacity_percentage() {
        let mut burn_state = BlockBurnState::new(1);

        // Empty: 0%
        assert_eq!(burn_state.get_capacity_percentage_bps(), 0);

        // Half full
        burn_state
            .try_add_burn(BURN_LIMIT_PER_BLOCK_CIL / 2)
            .unwrap();
        assert_eq!(burn_state.get_capacity_percentage_bps(), 5000); // 50.00%

        // Full
        burn_state
            .try_add_burn(BURN_LIMIT_PER_BLOCK_CIL / 2)
            .unwrap();
        assert_eq!(burn_state.get_capacity_percentage_bps(), 10000); // 100.00%
    }

    #[test]
    fn test_get_multiplier_without_update() {
        let mut detector = SpamDetector::new(10, 2);

        // Address not yet tracked
        assert_eq!(detector.get_multiplier("unknown"), 1);

        // After transaction
        detector.check_and_update("user1", 1000).unwrap();
        assert_eq!(detector.get_multiplier("user1"), 1);
    }

    #[test]
    fn test_multiple_addresses_independent() {
        let mut detector = SpamDetector::new(5, 2);
        let timestamp = 1000u64;

        // User1 spams
        for _ in 0..10 {
            detector.check_and_update("user1", timestamp).unwrap();
        }

        // User2 is normal
        detector.check_and_update("user2", timestamp).unwrap();

        // Check multipliers are independent
        assert!(detector.get_multiplier("user1") > 1); // Spammer
        assert_eq!(detector.get_multiplier("user2"), 1); // Normal user
    }
}
