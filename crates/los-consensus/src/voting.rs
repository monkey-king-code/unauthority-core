// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// UNAUTHORITY (LOS) - LINEAR STAKE VOTING
//
// Voting power = stake (1 CIL = 1 vote)
// Pure linear staking — Sybil-resistant by definition:
// splitting stake into N identities yields the same total power.
//
// Previous implementation used √stake (quadratic voting) which was
// VULNERABLE to Sybil attacks: splitting 10,000 into 10×1,000 gives
// 10×√1000 ≈ 316 vs √10000 = 100, rewarding dishonest splitting.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Voting power calculation precision (decimal places)
pub const VOTING_POWER_PRECISION: u32 = 6;

/// Minimum stake required to participate in consensus (1 LOS minimum).
/// Permissionless: any validator with ≥1 LOS gets voting power.
/// Reward eligibility requires ≥1,000 LOS (enforced in validator_rewards.rs).
/// 1 LOS = 100_000_000_000 CIL (10^11 precision)
pub const MIN_STAKE_CIL: u128 = 100_000_000_000; // 1 LOS × 10^11

/// Maximum stake for voting power calculation (prevents overflow)
/// Total supply = 21,936,236 LOS × 10^11 CIL_PER_LOS
pub const MAX_STAKE_FOR_VOTING_CIL: u128 = 2_193_623_600_000_000_000_000; // Total supply in CIL

/// Validator voting information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidatorVote {
    /// Validator address
    pub validator_address: String,

    /// Current staked amount (in CIL)
    pub staked_amount_cil: u128,

    /// Calculated voting power (linear: 1 CIL = 1 vote)
    /// Changed from √stake to linear to prevent Sybil attacks.
    /// Uses u128 for cross-platform determinism.
    pub voting_power: u128,

    /// Vote preference (proposition ID or "abstain")
    pub vote_preference: String,

    /// Is validator currently active
    pub is_active: bool,
}

impl ValidatorVote {
    pub fn new(
        validator_address: String,
        staked_amount_cil: u128,
        vote_preference: String,
        is_active: bool,
    ) -> Self {
        // Linear voting power (1 CIL = 1 vote)
        let voting_power = calculate_voting_power(staked_amount_cil);

        Self {
            validator_address,
            staked_amount_cil,
            voting_power,
            vote_preference,
            is_active,
        }
    }
}

/// Calculate voting power using LINEAR formula: power = stake (1 CIL = 1 vote)
///
/// Changed from √stake to linear to prevent Sybil attacks.
/// Under √stake, splitting 10,000 into 10×1,000 yields MORE total power
/// (10×√1000 > √10000), incentivizing dishonest identity splitting.
/// Linear staking is Sybil-neutral: total power is conserved regardless
/// of how stake is distributed across identities.
///
/// # Returns
/// Voting power as u128 (equal to staked CIL), or 0 if below minimum stake.
pub fn calculate_voting_power(staked_amount_cil: u128) -> u128 {
    if staked_amount_cil < MIN_STAKE_CIL {
        return 0;
    }

    // Linear: 1 CIL = 1 unit of voting power
    staked_amount_cil.min(MAX_STAKE_FOR_VOTING_CIL)
}

/// Deterministic integer square root using Newton's method.
/// Returns floor(√n) for any u128 value.
///
/// NOTE: No longer used for voting power.
/// Kept for potential future use (e.g., AMM LP token calculation).
#[allow(dead_code)]
fn isqrt(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// Voting power summary for a network
/// All fields use deterministic integer math.
/// Concentration ratio uses basis points (0-10000 = 0%-100%).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VotingPowerSummary {
    /// Total validators participating
    pub total_validators: u32,

    /// Total network stake (CIL)
    pub total_stake_cil: u128,

    /// Total voting power (linear: sum of staked CIL) — deterministic u128
    pub total_voting_power: u128,

    /// Validators with voting power
    pub votes: Vec<ValidatorVote>,

    /// Average voting power per validator (integer division, floor)
    pub average_voting_power: u128,

    /// Maximum voting power (richest validator)
    pub max_voting_power: u128,

    /// Minimum voting power (poorest active validator)
    pub min_voting_power: u128,

    /// Power concentration in basis points (max_power * 10000 / total_power)
    /// Lower = more decentralized. 10000 = one validator controls 100%.
    pub concentration_ratio_bps: u32,
}

/// Voting system to calculate and track voting power
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VotingSystem {
    /// MAINNET: BTreeMap for deterministic validator iteration order
    validators: BTreeMap<String, ValidatorVote>,
}

impl Default for VotingSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl VotingSystem {
    /// Create new voting system
    pub fn new() -> Self {
        Self {
            validators: BTreeMap::new(),
        }
    }

    /// Register or update a validator
    /// Returns u128 voting power (deterministic)
    pub fn register_validator(
        &mut self,
        validator_address: String,
        staked_amount_cil: u128,
        vote_preference: String,
        is_active: bool,
    ) -> Result<u128, String> {
        if staked_amount_cil > MAX_STAKE_FOR_VOTING_CIL {
            return Err(format!(
                "Stake {} exceeds maximum {}",
                staked_amount_cil, MAX_STAKE_FOR_VOTING_CIL
            ));
        }

        let vote = ValidatorVote::new(
            validator_address.clone(),
            staked_amount_cil,
            vote_preference,
            is_active,
        );

        let voting_power = vote.voting_power;
        self.validators.insert(validator_address, vote);

        Ok(voting_power)
    }

    /// Update validator stake (happens during epochs)
    /// Returns u128 voting power (deterministic)
    pub fn update_stake(
        &mut self,
        validator_address: &str,
        new_stake_cil: u128,
    ) -> Result<u128, String> {
        let validator = self
            .validators
            .get_mut(validator_address)
            .ok_or_else(|| format!("Validator {} not found", validator_address))?;

        validator.staked_amount_cil = new_stake_cil;
        validator.voting_power = calculate_voting_power(new_stake_cil);

        Ok(validator.voting_power)
    }

    /// Update validator vote preference
    pub fn update_vote_preference(
        &mut self,
        validator_address: &str,
        preference: String,
    ) -> Result<(), String> {
        let validator = self
            .validators
            .get_mut(validator_address)
            .ok_or_else(|| format!("Validator {} not found", validator_address))?;

        validator.vote_preference = preference;
        Ok(())
    }

    /// Get individual validator voting power (deterministic u128)
    pub fn get_validator_power(&self, validator_address: &str) -> Option<u128> {
        self.validators
            .get(validator_address)
            .map(|v| v.voting_power)
    }

    /// Get normalized voting power in basis points (0-10000)
    /// Uses integer math for determinism.
    pub fn get_normalized_power(&self, validator_address: &str) -> Option<u32> {
        let total_power: u128 = self.validators.values().map(|v| v.voting_power).sum();
        if total_power == 0 {
            return Some(0);
        }
        self.validators
            .get(validator_address)
            .map(|v| ((v.voting_power * 10_000) / total_power) as u32)
    }

    /// Calculate voting power summary — all deterministic integer math
    /// Eliminates f64 from governance summary.
    pub fn get_summary(&self) -> VotingPowerSummary {
        let votes: Vec<ValidatorVote> = self
            .validators
            .values()
            .filter(|v| v.is_active)
            .cloned()
            .collect();

        let total_validators = votes.len() as u32;
        let total_stake_cil: u128 = votes.iter().map(|v| v.staked_amount_cil).sum();
        let total_voting_power: u128 = votes.iter().map(|v| v.voting_power).sum();

        let (max_voting_power, min_voting_power) = if votes.is_empty() {
            (0u128, 0u128)
        } else {
            let max = votes.iter().map(|v| v.voting_power).max().unwrap_or(0);
            let min = votes.iter().map(|v| v.voting_power).min().unwrap_or(0);
            (max, min)
        };

        let average_voting_power = if total_validators > 0 {
            total_voting_power / total_validators as u128
        } else {
            0
        };

        let concentration_ratio_bps = if total_voting_power > 0 {
            ((max_voting_power * 10_000) / total_voting_power) as u32
        } else {
            0
        };

        VotingPowerSummary {
            total_validators,
            total_stake_cil,
            total_voting_power,
            votes,
            average_voting_power,
            max_voting_power,
            min_voting_power,
            concentration_ratio_bps,
        }
    }

    /// Reach consensus on a proposal (>50% voting power needed)
    /// Returns (votes_for_u128, percentage_bps_u32, consensus_bool)
    /// percentage_bps: 0-10000 basis points (5000 = 50%, 10000 = 100%)
    /// Consensus requires >5000 bps (strictly greater than 50%)
    pub fn calculate_proposal_consensus(&self, proposal_id: &str) -> (u128, u32, bool) {
        let votes_for: u128 = self
            .validators
            .values()
            .filter(|v| v.is_active && v.vote_preference == proposal_id)
            .map(|v| v.voting_power)
            .sum();

        let total_voting_power: u128 = self
            .validators
            .values()
            .filter(|v| v.is_active)
            .map(|v| v.voting_power)
            .sum();

        let percentage_bps: u32 = if total_voting_power > 0 {
            ((votes_for * 10_000) / total_voting_power) as u32
        } else {
            0
        };

        let consensus_reached = percentage_bps > 5_000; // Strictly > 50%

        (votes_for, percentage_bps, consensus_reached)
    }

    /// Compare voting power concentration between two stake distributions.
    /// Returns basis points (u32) instead of f64 ratios.
    /// Returns (concentrated_bps, distributed_bps, improvement_bps)
    pub fn compare_scenarios(
        whale_scenario: &[(String, u128)],
        distributed_scenario: &[(String, u128)],
    ) -> (u32, u32, u32) {
        // Whale scenario
        let whale_total_power: u128 = whale_scenario
            .iter()
            .map(|(_, stake)| calculate_voting_power(*stake))
            .sum();

        // Distributed scenario
        let distributed_total_power: u128 = distributed_scenario
            .iter()
            .map(|(_, stake)| calculate_voting_power(*stake))
            .sum();

        let max_whale: u128 = whale_scenario
            .iter()
            .map(|(_, stake)| calculate_voting_power(*stake))
            .max()
            .unwrap_or(0);

        let max_distributed: u128 = distributed_scenario
            .iter()
            .map(|(_, stake)| calculate_voting_power(*stake))
            .max()
            .unwrap_or(0);

        let whale_concentration_bps = if whale_total_power > 0 {
            ((max_whale * 10_000) / whale_total_power) as u32
        } else {
            0
        };

        let distributed_concentration_bps = if distributed_total_power > 0 {
            ((max_distributed * 10_000) / distributed_total_power) as u32
        } else {
            0
        };

        let improvement_bps = if whale_concentration_bps > 0 {
            ((whale_concentration_bps as u64).saturating_sub(distributed_concentration_bps as u64)
                * 10_000
                / whale_concentration_bps as u64) as u32
        } else {
            0
        };

        (
            whale_concentration_bps,
            distributed_concentration_bps,
            improvement_bps,
        )
    }

    /// Clear all validators
    pub fn clear(&mut self) {
        self.validators.clear();
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TESTS
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    // 1 LOS = 100_000_000_000 CIL (10^11)
    // MIN_STAKE_CIL = 1 LOS = 100_000_000_000 CIL (10^11)
    const LOS: u128 = 100_000_000_000; // 10^11 CIL per LOS

    #[test]
    fn test_voting_power_calculation() {
        // 1 LOS = MIN_STAKE = 100_000_000_000 CIL
        let power = calculate_voting_power(1 * LOS);
        // Linear: power = stake in CIL
        assert_eq!(power, 1 * LOS);

        // 1000 LOS
        let power = calculate_voting_power(1000 * LOS);
        assert_eq!(power, 1000 * LOS);

        // 10000 LOS = 1_000_000_000_000_000 CIL
        let power = calculate_voting_power(10_000 * LOS);
        assert_eq!(power, 10_000 * LOS);
    }

    #[test]
    fn test_voting_power_below_minimum() {
        // 0 LOS = below MIN_STAKE (1 LOS)
        let power = calculate_voting_power(0);
        assert_eq!(power, 0); // No voting power

        // Half a LOS = below MIN_STAKE
        let power = calculate_voting_power(LOS / 2);
        assert_eq!(power, 0); // No voting power
    }

    #[test]
    fn test_sybil_resistance_linear() {
        // Linear voting is Sybil-neutral.
        // 1 whale with 10000 LOS should have EXACTLY EQUAL power
        // to 10 nodes with 1000 LOS each.
        let whale_stake = 10_000 * LOS;
        let whale_power = calculate_voting_power(whale_stake);

        let node_stake = 1_000 * LOS;
        let nodes_power = calculate_voting_power(node_stake) * 10;

        // Linear: whale_power = 10_000 LOS, nodes_power = 1_000 * 10 = 10_000 LOS
        // Equal power = Sybil-neutral (no advantage to splitting)
        assert_eq!(whale_power, nodes_power);
    }

    #[test]
    fn test_voting_system_registration() {
        let mut system = VotingSystem::new();

        let power = system
            .register_validator(
                "validator1".to_string(),
                1_000 * LOS, // 1000 LOS = minimum stake
                "proposal_1".to_string(),
                true,
            )
            .unwrap();

        assert!(power > 0);
        assert_eq!(system.get_validator_power("validator1"), Some(power));
    }

    #[test]
    fn test_voting_system_summary() {
        let mut system = VotingSystem::new();

        // Add 3 validators with valid stakes (>= 1000 LOS)
        system
            .register_validator("val1".to_string(), 1_000 * LOS, "prop_1".to_string(), true)
            .unwrap();
        system
            .register_validator("val2".to_string(), 1_000 * LOS, "prop_1".to_string(), true)
            .unwrap();
        system
            .register_validator("val3".to_string(), 10_000 * LOS, "prop_1".to_string(), true)
            .unwrap();

        let summary = system.get_summary();

        assert_eq!(summary.total_validators, 3);
        assert!(summary.total_voting_power > 0);
        assert!(summary.average_voting_power > 0);
        assert!(summary.max_voting_power > summary.average_voting_power);
    }

    #[test]
    fn test_consensus_calculation() {
        let mut system = VotingSystem::new();

        // Add validators voting for proposal (all >= 1000 LOS)
        system
            .register_validator(
                "val1".to_string(),
                1_000 * LOS,
                "proposal_1".to_string(),
                true,
            )
            .unwrap();
        system
            .register_validator(
                "val2".to_string(),
                1_000 * LOS,
                "proposal_1".to_string(),
                true,
            )
            .unwrap();
        system
            .register_validator(
                "val3".to_string(),
                1_000 * LOS,
                "proposal_2".to_string(),
                true,
            )
            .unwrap();

        let (votes_for, percentage_bps, consensus) =
            system.calculate_proposal_consensus("proposal_1");

        // Linear: each 1000 LOS validator has power = 1000 * LOS CIL
        assert_eq!(votes_for, calculate_voting_power(1_000 * LOS) * 2);
        assert!(percentage_bps > 5_000); // 2/3 validators (≈6666 bps = 66.7%)
        assert!(consensus); // Passed
    }

    #[test]
    fn test_no_consensus_with_split_votes() {
        let mut system = VotingSystem::new();

        // Equal vote split (both >= 1000 LOS)
        system
            .register_validator(
                "val1".to_string(),
                1_000 * LOS,
                "proposal_1".to_string(),
                true,
            )
            .unwrap();
        system
            .register_validator(
                "val2".to_string(),
                1_000 * LOS,
                "proposal_2".to_string(),
                true,
            )
            .unwrap();

        let (_, percentage_bps, consensus) = system.calculate_proposal_consensus("proposal_1");

        assert_eq!(percentage_bps, 5_000); // 50% = 5000 bps
        assert!(!consensus); // Needs > 50%, not ≥ 50%
    }

    #[test]
    fn test_update_stake() {
        let mut system = VotingSystem::new();

        system
            .register_validator("val1".to_string(), 1_000 * LOS, "prop_1".to_string(), true)
            .unwrap();

        let old_power = system.get_validator_power("val1").unwrap();

        // Increase stake (10x)
        system.update_stake("val1", 10_000 * LOS).unwrap();

        let new_power = system.get_validator_power("val1").unwrap();
        assert!(new_power > old_power);
    }

    #[test]
    fn test_concentration_ratio() {
        let mut system = VotingSystem::new();

        // Whale has 10x more stake
        system
            .register_validator(
                "whale".to_string(),
                10_000 * LOS,
                "prop_1".to_string(),
                true,
            )
            .unwrap();
        system
            .register_validator(
                "small1".to_string(),
                1_000 * LOS,
                "prop_1".to_string(),
                true,
            )
            .unwrap();

        let summary = system.get_summary();
        // Linear: Whale = 10000 LOS, Small = 1000 LOS, Total = 11000 LOS
        // Concentration = 10000 / 11000 ≈ 9090 bps (90.9%)
        assert!(summary.concentration_ratio_bps > 9_000);
    }
}
