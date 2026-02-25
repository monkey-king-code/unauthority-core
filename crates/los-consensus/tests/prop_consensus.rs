// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// PROPERTY-BASED TESTS — los-consensus
//
// Verifies consensus invariants hold for ALL possible validator sets and votes.
//
// ZERO production code changes — integration test file only.
// Run: cargo test --release -p los-consensus --test prop_consensus
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use los_consensus::voting::{
    calculate_voting_power, MIN_STAKE_CIL, MAX_STAKE_FOR_VOTING_CIL,
};
use los_consensus::abft::ABFTConsensus;
use proptest::prelude::*;

// ─────────────────────────────────────────────────────────────────
// VOTING POWER PROPERTIES
// ─────────────────────────────────────────────────────────────────

proptest! {
    /// PROPERTY: Linear voting power — splitting stake conserves total power (Sybil-neutral)
    /// This is THE critical security property. If this fails, the network is Sybil-vulnerable.
    #[test]
    fn prop_sybil_neutral(
        total_stake in MIN_STAKE_CIL..=1_000_000 * 100_000_000_000u128,
        split_count in 2u128..=100,
    ) {
        let single_power = calculate_voting_power(total_stake);

        // Split into N equal parts
        let per_split = total_stake / split_count;
        let remainder = total_stake % split_count;

        let mut split_total = 0u128;
        for _ in 0..split_count {
            split_total += calculate_voting_power(per_split);
        }
        // Add remainder to last split
        if remainder > 0 {
            split_total += calculate_voting_power(remainder);
        }

        // Linear: split_total should be ≤ single_power
        // (Equal when all splits meet minimum, less if some fall below MIN_STAKE)
        prop_assert!(
            split_total <= single_power,
            "Splitting must NOT increase total power (Sybil attack): {} > {}",
            split_total, single_power
        );
    }

    /// PROPERTY: Voting power is monotonically non-decreasing with stake
    #[test]
    fn prop_voting_power_monotonic(
        stake1 in 0u128..=MAX_STAKE_FOR_VOTING_CIL / 2,
        delta in 1u128..=1_000_000u128,
    ) {
        let stake2 = stake1.saturating_add(delta).min(MAX_STAKE_FOR_VOTING_CIL);
        let vp1 = calculate_voting_power(stake1);
        let vp2 = calculate_voting_power(stake2);
        prop_assert!(vp2 >= vp1, "More stake must give >= voting power: {} < {}", vp2, vp1);
    }

    /// PROPERTY: Below-minimum stake always yields zero voting power
    #[test]
    fn prop_below_minimum_zero(stake in 0u128..MIN_STAKE_CIL) {
        prop_assert_eq!(calculate_voting_power(stake), 0,
            "Stake below minimum must yield 0 voting power");
    }

    /// PROPERTY: At-or-above minimum stake yields non-zero voting power
    #[test]
    fn prop_above_minimum_nonzero(stake in MIN_STAKE_CIL..=MAX_STAKE_FOR_VOTING_CIL) {
        prop_assert!(calculate_voting_power(stake) > 0,
            "Stake at/above minimum must yield non-zero voting power");
    }

    /// PROPERTY: Voting power equals stake (linear identity) for valid stakes
    #[test]
    fn prop_voting_power_is_linear(stake in MIN_STAKE_CIL..=MAX_STAKE_FOR_VOTING_CIL) {
        let vp = calculate_voting_power(stake);
        prop_assert_eq!(vp, stake, "Linear voting: power must equal stake");
    }

    /// PROPERTY: Voting power capped at MAX_STAKE_FOR_VOTING_CIL
    #[test]
    fn prop_voting_power_capped(stake in MAX_STAKE_FOR_VOTING_CIL..=u128::MAX / 2) {
        let vp = calculate_voting_power(stake);
        prop_assert!(vp <= MAX_STAKE_FOR_VOTING_CIL,
            "Voting power must be capped at MAX_STAKE");
    }
}

// ─────────────────────────────────────────────────────────────────
// BFT QUORUM PROPERTIES
// ─────────────────────────────────────────────────────────────────

/// Calculate minimum distinct voters needed for BFT quorum.
/// Reproduces the formula from los-node/src/main.rs.
fn min_distinct_voters(n: usize) -> usize {
    if n <= 1 {
        return 1;
    }
    let f = (n - 1) / 3; // max Byzantine faults tolerated
    2 * f + 1             // quorum = 2f+1
}

proptest! {
    /// PROPERTY: BFT quorum never exceeds total validators
    #[test]
    fn prop_quorum_bounded(n in 1usize..=1000) {
        let q = min_distinct_voters(n);
        prop_assert!(q <= n, "Quorum cannot exceed total validators: {} > {}", q, n);
    }

    /// PROPERTY: Single validator always has quorum of 1
    #[test]
    fn prop_single_validator_quorum(_dummy in 0u8..=255) {
        prop_assert_eq!(min_distinct_voters(1), 1);
    }

    /// PROPERTY: BFT quorum > n/3 (can't be too small)
    #[test]
    fn prop_quorum_at_least_third(n in 4usize..=1000) {
        let q = min_distinct_voters(n);
        let min_threshold = n / 3;
        prop_assert!(q > min_threshold,
            "Quorum {} must be > n/3 = {} for n={}", q, min_threshold, n);
    }

    /// PROPERTY: 4 validators → quorum = 3 (standard BFT)
    #[test]
    fn prop_quorum_standard_4(_dummy in 0u8..=255) {
        prop_assert_eq!(min_distinct_voters(4), 3);
    }

    /// PROPERTY: 7 validators → quorum = 5
    #[test]
    fn prop_quorum_standard_7(_dummy in 0u8..=255) {
        prop_assert_eq!(min_distinct_voters(7), 5);
    }
}

// ─────────────────────────────────────────────────────────────────
// aBFT CONSENSUS PROPERTIES
// ─────────────────────────────────────────────────────────────────

proptest! {
    /// PROPERTY: New consensus instance starts at view 0 with empty state
    #[test]
    fn prop_consensus_fresh_state(
        node_id in "validator[0-9]{1,4}",
        num_validators in 1usize..=20,
    ) {
        let consensus = ABFTConsensus::new(node_id.clone(), num_validators);

        prop_assert_eq!(consensus.view, 0, "Fresh consensus must start at view 0");
        prop_assert_eq!(consensus.total_validators, num_validators.max(1),
            "Validator count must match");
    }

    /// PROPERTY: aBFT block hash is deterministic
    #[test]
    fn prop_abft_block_hash_deterministic(
        height in 0u64..=1_000_000,
        timestamp in 1_700_000_000u64..=2_000_000_000u64,
        data in proptest::collection::vec(any::<u8>(), 0..1024),
        proposer in "val_[0-9]{1,4}",
        parent_hash in "[0-9a-f]{64}",
    ) {
        let block = los_consensus::abft::Block {
            height,
            timestamp,
            data: data.clone(),
            proposer: proposer.clone(),
            parent_hash: parent_hash.clone(),
        };
        let h1 = block.calculate_hash();
        let h2 = block.calculate_hash();
        prop_assert_eq!(h1, h2, "aBFT block hash must be deterministic");
    }
}
