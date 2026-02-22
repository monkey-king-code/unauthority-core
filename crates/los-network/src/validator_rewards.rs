// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// UNAUTHORITY (LOS) - VALIDATOR REWARD DISTRIBUTION
//
// Task #2: Non-Inflationary Reward Model
// - 100% Transaction Fees → Validator Account
// - Dynamic Gas Calculation
// - Priority Tipping Mechanism
// - Automatic Fee Distribution
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Base gas price in CIL (smallest unit)
pub const BASE_GAS_PRICE_CIL: u128 = 1_000;

/// Gas cost per byte of transaction data
pub const GAS_PER_BYTE: u64 = 10;

/// Maximum gas per transaction (10M CIL)
pub const MAX_GAS_PER_TX: u128 = 10_000_000;

/// Transaction Fee Structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionFee {
    /// Base fee calculated from gas
    pub base_fee_cil: u128,

    /// Optional priority tip (higher = faster inclusion)
    pub priority_tip_cil: u128,

    /// Total fee (base + priority)
    pub total_fee_cil: u128,

    /// Fee multiplier (for spam detection)
    pub multiplier: u128,

    /// Timestamp when fee was calculated
    pub timestamp: u64,
}

/// Validator Reward Distribution Record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorReward {
    /// Validator address (unique per validator)
    pub validator_address: String,

    /// Total fees collected in this block
    pub collected_fees_cil: u128,

    /// Number of transactions processed
    pub tx_count: u32,

    /// Block height
    pub block_height: u64,

    /// Timestamp
    pub timestamp: u64,
}

/// Reward Distribution State (per validator)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RewardAccount {
    /// Total accumulated rewards (CIL)
    pub total_rewards_cil: u128,

    /// Pending rewards (not yet claimed)
    pub pending_rewards_cil: u128,

    /// Last claim timestamp
    pub last_claim_timestamp: u64,

    /// Total blocks produced
    pub blocks_produced: u64,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GAS CALCULATION FUNCTIONS
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Calculate base gas fee for a transaction
///
/// Formula: base_fee = BASE_GAS_PRICE + (tx_size_bytes * GAS_PER_BYTE)
///
/// # Arguments
/// * `tx_size_bytes` - Size of transaction in bytes
/// * `base_price_cil` - Base gas price (default: 1,000 CIL)
/// * `gas_per_byte` - Gas cost per byte (default: 10)
/// * `fee_multiplier` - Multiplier for spam/network congestion
///
/// # Returns
/// Base gas fee in CIL, or error if exceeds MAX_GAS_PER_TX
///
/// # Example
/// ```ignore
/// let tx_size = 256; // bytes
/// let fee = calculate_gas_fee(tx_size, 1_000, 10, 1)?;
/// assert_eq!(fee, 1_000 + (256 * 10)); // 3,560 CIL
/// ```
pub fn calculate_gas_fee(
    tx_size_bytes: u64,
    base_price_cil: u128,
    gas_per_byte: u64,
    fee_multiplier: u128,
) -> Result<u128, String> {
    // Calculate base fee (size-dependent)
    let size_fee = tx_size_bytes as u128 * gas_per_byte as u128;
    let base_fee = base_price_cil + size_fee;

    // Apply fee multiplier (for spam detection)
    let total_fee = base_fee.saturating_mul(fee_multiplier);

    // Enforce maximum gas per transaction
    if total_fee > MAX_GAS_PER_TX {
        return Err(format!(
            "Transaction fee {} CIL exceeds maximum {} CIL",
            total_fee, MAX_GAS_PER_TX
        ));
    }

    Ok(total_fee)
}

/// Calculate transaction fee with priority tipping
///
/// # Arguments
/// * `base_fee` - Base gas fee calculated by calculate_gas_fee()
/// * `priority_tip` - Optional priority tip in CIL (0 for normal priority)
///
/// # Returns
/// Total transaction fee (base + priority)
///
/// # Example
/// ```ignore
/// let base_fee = 3_560;
/// let priority_tip = 10_000; // 0.0001 LOS extra for faster inclusion
/// let total_fee = calculate_transaction_fee(base_fee, priority_tip)?;
/// assert_eq!(total_fee, 13_560);
/// ```
pub fn calculate_transaction_fee(base_fee: u128, priority_tip: u128) -> Result<u128, String> {
    let total_fee = base_fee.saturating_add(priority_tip);

    if total_fee > MAX_GAS_PER_TX {
        return Err(format!(
            "Total fee {} CIL exceeds maximum {} CIL",
            total_fee, MAX_GAS_PER_TX
        ));
    }

    Ok(total_fee)
}

/// Create comprehensive transaction fee structure
pub fn build_transaction_fee(
    tx_size_bytes: u64,
    priority_tip_cil: u128,
    fee_multiplier: u128,
    timestamp: u64,
) -> Result<TransactionFee, String> {
    // Calculate base fee
    let base_fee = calculate_gas_fee(
        tx_size_bytes,
        BASE_GAS_PRICE_CIL,
        GAS_PER_BYTE,
        fee_multiplier,
    )?;

    // Calculate total with priority tip
    let total_fee = calculate_transaction_fee(base_fee, priority_tip_cil)?;

    Ok(TransactionFee {
        base_fee_cil: base_fee,
        priority_tip_cil,
        total_fee_cil: total_fee,
        multiplier: fee_multiplier,
        timestamp,
    })
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// REWARD DISTRIBUTION FUNCTIONS
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Distribute 100% of transaction fees to validator account
///
/// This implements the non-inflationary reward model:
/// - All transaction fees go directly to block producer
/// - No new coins are minted
/// - Reward is immediate upon block finality
///
/// # Arguments
/// * `validator_address` - Address of validator producing the block
/// * `total_fees_cil` - Sum of all transaction fees in block
/// * `reward_account` - Mutable reference to validator's reward account
///
/// # Returns
/// Updated reward account state
pub fn distribute_transaction_fees(
    validator_address: &str,
    total_fees_cil: u128,
    reward_account: &mut RewardAccount,
) -> ValidatorReward {
    // Add to pending rewards
    reward_account.pending_rewards_cil += total_fees_cil;

    // Track total accumulated
    reward_account.total_rewards_cil += total_fees_cil;

    // Create reward record for this block
    ValidatorReward {
        validator_address: validator_address.to_string(),
        collected_fees_cil: total_fees_cil,
        tx_count: 0,     // Will be filled by caller
        block_height: 0, // Will be filled by caller
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    }
}

/// Claim pending rewards (if needed, for accounting/tax purposes)
///
/// In LOS model, rewards are automatically available after block finality.
/// This function is for explicit claims/transfers if needed.
pub fn claim_rewards(reward_account: &mut RewardAccount, timestamp: u64) -> Result<u128, String> {
    if reward_account.pending_rewards_cil == 0 {
        return Err("No pending rewards to claim".to_string());
    }

    let claimed_amount = reward_account.pending_rewards_cil;
    reward_account.pending_rewards_cil = 0;
    reward_account.last_claim_timestamp = timestamp;

    Ok(claimed_amount)
}

/// Accumulate block rewards over time
///
/// Tracks validator's total fee collection per block
pub fn accumulate_block_rewards(
    validator_address: &str,
    rewards: &mut BTreeMap<String, RewardAccount>,
    total_fees_cil: u128,
) -> ValidatorReward {
    let account = rewards.entry(validator_address.to_string()).or_default();

    account.blocks_produced += 1;

    distribute_transaction_fees(validator_address, total_fees_cil, account)
}

/// Get validator's total pending rewards
pub fn get_pending_rewards(
    validator_address: &str,
    rewards: &BTreeMap<String, RewardAccount>,
) -> Result<u128, String> {
    rewards
        .get(validator_address)
        .map(|acc| acc.pending_rewards_cil)
        .ok_or_else(|| "Validator not found in rewards".to_string())
}

/// Get validator's total accumulated rewards
pub fn get_total_rewards(
    validator_address: &str,
    rewards: &BTreeMap<String, RewardAccount>,
) -> Result<u128, String> {
    rewards
        .get(validator_address)
        .map(|acc| acc.total_rewards_cil)
        .ok_or_else(|| "Validator not found in rewards".to_string())
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BLOCK REWARD FINALIZATION
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Process all transaction fees in a block and distribute to validator
///
/// This is called when block is finalized (aBFT finalizes after 1 block)
///
/// # Arguments
/// * `validator_address` - Address of validator who produced the block
/// * `transaction_fees` - List of TransactionFee structs from block
/// * `rewards` - Mutable reference to reward accounts
/// * `block_height` - Current block height
///
/// # Returns
/// ValidatorReward record with details
pub fn finalize_block_rewards(
    validator_address: &str,
    transaction_fees: &[TransactionFee],
    rewards: &mut BTreeMap<String, RewardAccount>,
    block_height: u64,
) -> ValidatorReward {
    // Calculate total fees in block
    let total_fees: u128 = transaction_fees.iter().map(|tf| tf.total_fee_cil).sum();

    // Get or create reward account
    let account = rewards.entry(validator_address.to_string()).or_default();

    // Update account
    account.pending_rewards_cil += total_fees;
    account.total_rewards_cil += total_fees;
    account.blocks_produced += 1;

    // Create reward record
    ValidatorReward {
        validator_address: validator_address.to_string(),
        collected_fees_cil: total_fees,
        tx_count: transaction_fees.len() as u32,
        block_height,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// STATISTICS & REPORTING
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Validator Reward Statistics
/// MAINNET SAFETY: All fields are integer-only. Display formatting (LOS) done at API boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorRewardStats {
    pub validator_address: String,
    pub total_rewards_cil: u128,
    /// Integer LOS (truncated). For precise display: total_rewards_cil / CIL_PER_LOS
    pub total_rewards_los: u128,
    pub pending_rewards_cil: u128,
    pub blocks_produced: u64,
    /// Average fee per block in CIL (integer division)
    pub average_fee_per_block_cil: u128,
}

/// Get comprehensive statistics for a validator
pub fn get_validator_stats(
    validator_address: &str,
    rewards: &BTreeMap<String, RewardAccount>,
) -> Result<ValidatorRewardStats, String> {
    let account = rewards
        .get(validator_address)
        .ok_or_else(|| "Validator not found".to_string())?;

    // MAINNET SAFETY: Integer-only math. No f64 in financial calculations.
    let total_los = account.total_rewards_cil / 100_000_000_000; // CIL_PER_LOS = 10^11
    let avg_fee_cil = if account.blocks_produced > 0 {
        account.total_rewards_cil / account.blocks_produced as u128
    } else {
        0
    };

    Ok(ValidatorRewardStats {
        validator_address: validator_address.to_string(),
        total_rewards_cil: account.total_rewards_cil,
        total_rewards_los: total_los,
        pending_rewards_cil: account.pending_rewards_cil,
        blocks_produced: account.blocks_produced,
        average_fee_per_block_cil: avg_fee_cil,
    })
}

/// Get statistics for all validators
pub fn get_all_validator_stats(
    rewards: &BTreeMap<String, RewardAccount>,
) -> Vec<ValidatorRewardStats> {
    rewards
        .iter()
        .map(|(address, account)| {
            // MAINNET SAFETY: Integer-only math. No f64 in financial calculations.
            let total_los = account.total_rewards_cil / 100_000_000_000; // CIL_PER_LOS = 10^11
            let avg_fee_cil = if account.blocks_produced > 0 {
                account.total_rewards_cil / account.blocks_produced as u128
            } else {
                0
            };

            ValidatorRewardStats {
                validator_address: address.clone(),
                total_rewards_cil: account.total_rewards_cil,
                total_rewards_los: total_los,
                pending_rewards_cil: account.pending_rewards_cil,
                blocks_produced: account.blocks_produced,
                average_fee_per_block_cil: avg_fee_cil,
            }
        })
        .collect()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TESTS
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_gas_fee() {
        // Transaction size: 256 bytes, multiplier: 1x
        let fee = calculate_gas_fee(256, 1_000, 10, 1).unwrap();
        assert_eq!(fee, 1_000 + (256 * 10)); // 3,560 CIL
    }

    #[test]
    fn test_calculate_gas_fee_with_multiplier() {
        // Same transaction, but with 2x multiplier (spam detected)
        let fee = calculate_gas_fee(256, 1_000, 10, 2).unwrap();
        assert_eq!(fee, (1_000 + (256 * 10)) * 2); // 7,120 CIL
    }

    #[test]
    fn test_priority_tipping() {
        let base_fee = 3_560;
        let priority_tip = 10_000;
        let total = calculate_transaction_fee(base_fee, priority_tip).unwrap();
        assert_eq!(total, 13_560);
    }

    #[test]
    fn test_reward_distribution() {
        let mut account = RewardAccount::default();

        let reward = distribute_transaction_fees(
            "LOS_VALIDATOR_1",
            100_000_000_000, // 1 LOS = 10^11 CIL
            &mut account,
        );

        assert_eq!(account.pending_rewards_cil, 100_000_000_000);
        assert_eq!(account.total_rewards_cil, 100_000_000_000);
        assert_eq!(reward.collected_fees_cil, 100_000_000_000);
    }

    #[test]
    fn test_block_finalization() {
        let mut rewards = BTreeMap::new();

        // Create sample fees
        let fees = vec![
            TransactionFee {
                base_fee_cil: 3_560,
                priority_tip_cil: 0,
                total_fee_cil: 3_560,
                multiplier: 1,
                timestamp: 1000,
            },
            TransactionFee {
                base_fee_cil: 5_000,
                priority_tip_cil: 10_000,
                total_fee_cil: 15_000,
                multiplier: 1,
                timestamp: 1001,
            },
        ];

        let result = finalize_block_rewards("LOS_VALIDATOR_1", &fees, &mut rewards, 1);

        assert_eq!(result.collected_fees_cil, 18_560);
        assert_eq!(result.tx_count, 2);
        assert_eq!(result.block_height, 1);
    }
}
