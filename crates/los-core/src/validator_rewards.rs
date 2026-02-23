// ─────────────────────────────────────────────────────────────────
// Validator Reward System — Epoch-Based Proportional Distribution
// ─────────────────────────────────────────────────────────────────
// Pool:        500,000 LOS (from public allocation)
// Rate:        5,000 LOS/epoch (30 days), halving every 48 epochs (4 yrs)
// Weight:      Linear stake (1 CIL = 1 unit of reward weight)
// Eligibility: 1000 LOS min stake, 95% uptime, 30-day probation passed
// Lifespan:    Pool lasts ~16-20 years (asymptotic halving)
//
// Changed from √stake to linear weight.
// √stake incentivizes Sybil attacks (splitting stake into multiple
// identities yields more total reward weight). Linear is Sybil-neutral.
// ─────────────────────────────────────────────────────────────────

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::{
    effective_reward_epoch_secs, MIN_VALIDATOR_STAKE_CIL, REWARD_HALVING_INTERVAL_EPOCHS,
    REWARD_MIN_UPTIME_PCT, REWARD_PROBATION_EPOCHS, REWARD_RATE_INITIAL_CIL,
    VALIDATOR_REWARD_POOL_CIL,
};

/// Per-validator reward tracking state.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ValidatorRewardState {
    /// Epoch when this validator first registered (0-indexed)
    pub join_epoch: u64,
    /// Total heartbeats sent during the current epoch
    pub heartbeats_current_epoch: u64,
    /// Expected heartbeats for the current epoch (based on epoch duration / heartbeat interval)
    pub expected_heartbeats: u64,
    /// Cumulative rewards received (CIL units)
    pub cumulative_rewards_cil: u128,
    /// Whether this is a genesis bootstrap validator.
    /// Genesis validators are eligible for rewards like all other validators —
    /// they secure the network from day one.
    pub is_genesis: bool,
    /// Current stake snapshot (CIL) — updated each epoch from ledger
    pub stake_cil: u128,
    /// Last completed epoch's uptime percentage (0–100)
    /// Used for API display so uptime doesn't show 0% at epoch start.
    #[serde(default)]
    pub last_epoch_uptime_pct: u64,
}

impl ValidatorRewardState {
    pub fn new(join_epoch: u64, is_genesis: bool, stake_cil: u128) -> Self {
        Self {
            join_epoch,
            heartbeats_current_epoch: 0,
            expected_heartbeats: 0,
            cumulative_rewards_cil: 0,
            is_genesis,
            stake_cil,
            last_epoch_uptime_pct: 0,
        }
    }

    /// Uptime percentage for the current epoch (0–100)
    /// Uses pure integer math — no floating point.
    pub fn uptime_pct(&self) -> u64 {
        if self.expected_heartbeats == 0 {
            // If expected is 0 but we have heartbeats, the validator registered
            // mid-epoch and expected_heartbeats hasn't been set yet (happens at
            // epoch boundary via set_expected_heartbeats). The validator IS alive
            // and sending heartbeats, so report 100% to avoid false 0%.
            return if self.heartbeats_current_epoch > 0 {
                100
            } else {
                0
            };
        }
        // Integer: (heartbeats * 100) / expected, capped at 100
        let pct = (self.heartbeats_current_epoch * 100) / self.expected_heartbeats;
        pct.min(100)
    }

    /// Best-effort uptime for API display.
    /// Returns the HIGHER of current epoch progress and last completed epoch uptime.
    /// This prevents misleading 0% at epoch start when the validator had 100% last epoch.
    pub fn display_uptime_pct(&self) -> u64 {
        let current = self.uptime_pct();
        current.max(self.last_epoch_uptime_pct)
    }

    /// Returns true if this validator is eligible for rewards this epoch.
    /// Requirements (identical for ALL validators — genesis and non-genesis):
    /// 1. Past probation period: must have completed at least 1 full epoch
    /// 2. Meets minimum uptime (95%)
    /// 3. Meets minimum stake (1000 LOS)
    ///
    /// Genesis bootstrap validators ARE eligible for rewards — they secure
    /// the network from day one and deserve the same compensation as any validator.
    pub fn is_eligible(&self, current_epoch: u64) -> bool {
        // Probation: must complete at least 1 full epoch before earning rewards.
        // A validator joining at epoch N is eligible starting at epoch N + PROBATION_EPOCHS.
        // This applies to ALL validators equally — genesis and non-genesis.
        // Epoch 0 is the bootstrap epoch; no validator earns rewards in their join epoch.
        if current_epoch < self.join_epoch + REWARD_PROBATION_EPOCHS {
            return false;
        }
        // Use display_uptime_pct() which returns max(current_epoch, last_epoch).
        // This prevents validators from appearing ineligible at epoch start
        // when current heartbeats are still accumulating but last epoch was 100%.
        // At epoch boundary (when rewards are actually distributed), current uptime
        // naturally reflects the full epoch's heartbeat count.
        if self.display_uptime_pct() < REWARD_MIN_UPTIME_PCT {
            return false;
        }
        if self.stake_cil < MIN_VALIDATOR_STAKE_CIL {
            return false;
        }
        true
    }

    /// Linear stake weight: returns stake_cil directly (1 CIL = 1 reward weight unit).
    /// Changed from √stake to linear to prevent Sybil attacks.
    pub fn linear_stake_weight(&self) -> u128 {
        self.stake_cil
    }
}

/// Global reward pool and epoch tracking state.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ValidatorRewardPool {
    /// Remaining CIL in the reward pool
    pub remaining_cil: u128,
    /// Current epoch number (starts at 0)
    pub current_epoch: u64,
    /// Timestamp when the current epoch started (Unix seconds)
    pub epoch_start_timestamp: u64,
    /// Number of halvings that have occurred
    pub halvings_occurred: u64,
    /// Total CIL distributed across all epochs
    pub total_distributed_cil: u128,
    /// Per-validator reward state (keyed by address)
    /// MAINNET: BTreeMap for deterministic iteration and serialization
    pub validators: BTreeMap<String, ValidatorRewardState>,
    /// Epoch duration in seconds (testnet=120, mainnet=2592000)
    /// Defaults to effective_reward_epoch_secs() if not present (backwards-compatible).
    #[serde(default = "default_epoch_duration")]
    pub epoch_duration_secs: u64,
}

fn default_epoch_duration() -> u64 {
    effective_reward_epoch_secs()
}

impl ValidatorRewardPool {
    /// Create a new reward pool with full funding.
    /// `genesis_timestamp` = network genesis time (Unix seconds).
    pub fn new(genesis_timestamp: u64) -> Self {
        Self {
            remaining_cil: VALIDATOR_REWARD_POOL_CIL,
            current_epoch: 0,
            epoch_start_timestamp: genesis_timestamp,
            halvings_occurred: 0,
            total_distributed_cil: 0,
            validators: BTreeMap::new(),
            epoch_duration_secs: effective_reward_epoch_secs(),
        }
    }

    /// Create from a custom initial balance (for testing or partial funding).
    pub fn with_balance(genesis_timestamp: u64, balance_cil: u128) -> Self {
        Self {
            remaining_cil: balance_cil,
            current_epoch: 0,
            epoch_start_timestamp: genesis_timestamp,
            halvings_occurred: 0,
            total_distributed_cil: 0,
            validators: BTreeMap::new(),
            epoch_duration_secs: effective_reward_epoch_secs(),
        }
    }

    /// Register a validator for reward tracking.
    /// If already registered, updates stake and genesis status.
    pub fn register_validator(&mut self, address: &str, is_genesis: bool, stake_cil: u128) {
        self.validators
            .entry(address.to_string())
            .and_modify(|v| {
                v.stake_cil = stake_cil;
                v.is_genesis = is_genesis;
            })
            .or_insert_with(|| {
                ValidatorRewardState::new(self.current_epoch, is_genesis, stake_cil)
            });
    }

    /// Record a heartbeat from a validator (proving liveness).
    /// Can be called multiple times per tick from different sources;
    /// each call increments the counter. Use `record_heartbeat_once()` with
    /// a per-tick dedup set to ensure exactly 1 heartbeat per validator per tick.
    pub fn record_heartbeat(&mut self, address: &str) {
        if let Some(state) = self.validators.get_mut(address) {
            state.heartbeats_current_epoch += 1;
        }
    }

    /// Record exactly ONE heartbeat per validator per tick.
    /// Uses the caller-provided `seen` set for deduplication.
    /// Returns true if a heartbeat was recorded (first call for this address this tick).
    pub fn record_heartbeat_once(
        &mut self,
        address: &str,
        seen_this_tick: &mut std::collections::BTreeSet<String>,
    ) -> bool {
        if seen_this_tick.contains(address) {
            return false; // Already counted this tick
        }
        if let Some(state) = self.validators.get_mut(address) {
            state.heartbeats_current_epoch += 1;
            seen_this_tick.insert(address.to_string());
            return true;
        }
        false
    }

    /// Calculate the reward rate for the current epoch (with halving).
    /// Rate halves every `REWARD_HALVING_INTERVAL_EPOCHS` epochs.
    /// After n halvings: rate = initial_rate >> n
    pub fn epoch_reward_rate(&self) -> u128 {
        let halvings = self.current_epoch / REWARD_HALVING_INTERVAL_EPOCHS;
        if halvings >= 128 {
            return 0; // Effectively zero after 128 halvings
        }
        REWARD_RATE_INITIAL_CIL >> halvings
    }

    /// Check if the current epoch has ended (based on timestamp).
    ///
    /// DESIGN Adds a small grace period (5 minutes) to allow for
    /// clock skew between validators. Without this, nodes with slightly
    /// different clocks could process epoch transitions at different times,
    /// causing one node to distribute rewards before others have accumulated
    /// enough heartbeats. The 5-minute grace period ensures all validators
    /// with clocks within ±5min of each other advance epochs together.
    ///
    /// For mainnet (30-day epochs), 5 minutes is negligible (0.01% of epoch).
    /// For testnet (2-minute epochs), we use a shorter 5-second grace period.
    pub fn is_epoch_complete(&self, now_secs: u64) -> bool {
        let grace_secs = if self.epoch_duration_secs <= 300 {
            5
        } else {
            300
        };
        now_secs >= self.epoch_start_timestamp + self.epoch_duration_secs + grace_secs
    }

    /// How many seconds remain in the current epoch.
    pub fn epoch_remaining_secs(&self, now_secs: u64) -> u64 {
        let end = self.epoch_start_timestamp + self.epoch_duration_secs;
        end.saturating_sub(now_secs)
    }

    /// Fast-forward through missed epochs (e.g., after node restart).
    /// Skips all fully-elapsed epochs without distributing rewards for them,
    /// since nobody was online to earn them. Returns number of epochs skipped.
    pub fn catch_up_epochs(&mut self, now_secs: u64) -> u64 {
        if self.epoch_duration_secs == 0 {
            return 0;
        }
        let elapsed = now_secs.saturating_sub(self.epoch_start_timestamp);
        let epochs_behind = elapsed / self.epoch_duration_secs;
        if epochs_behind <= 1 {
            return 0; // Current epoch or just one behind — normal processing
        }
        // Skip all but the current epoch (no rewards for missed epochs)
        let skip = epochs_behind - 1;
        self.current_epoch += skip;
        self.epoch_start_timestamp += skip * self.epoch_duration_secs;
        self.halvings_occurred = self.current_epoch / REWARD_HALVING_INTERVAL_EPOCHS;
        // Reset heartbeats since nobody was online
        for state in self.validators.values_mut() {
            state.heartbeats_current_epoch = 0;
            state.expected_heartbeats = 0;
        }
        skip
    }

    /// Set expected heartbeats for all validators at the start of an epoch.
    /// `heartbeat_interval_secs` = time between heartbeats (e.g., 60s).
    ///
    /// **PRORATING FIX:** When called at epoch END (before distribution),
    /// validators that already have heartbeats get `expected` prorated
    /// to match their actual participation window, preventing unfair
    /// penalization of mid-epoch joins or restarts.
    ///
    /// When called at epoch START (after advance_epoch reset counters),
    /// all validators have heartbeats=0 so they get the full expected count.
    pub fn set_expected_heartbeats(&mut self, heartbeat_interval_secs: u64) {
        let full_expected = if heartbeat_interval_secs > 0 {
            self.epoch_duration_secs / heartbeat_interval_secs
        } else {
            0
        };
        for state in self.validators.values_mut() {
            if state.heartbeats_current_epoch == 0 {
                // Epoch start or no heartbeats yet: set full expected
                state.expected_heartbeats = full_expected;
            } else if state.expected_heartbeats == 0 && full_expected > 0 {
                // Epoch end, validator joined mid-epoch (expected was never set):
                // Prorate expected to match what they could have actually sent.
                // Cap at their actual heartbeats to give benefit of the doubt
                // (they sent as many as they could since joining).
                // This prevents uptime_pct from being unfairly low.
                let prorated = state.heartbeats_current_epoch.min(full_expected);
                state.expected_heartbeats = prorated;
            } else {
                // Normal epoch-end call: expected was already set at epoch start.
                // Just ensure it's the full epoch value.
                state.expected_heartbeats = full_expected;
            }
        }
    }

    /// Distribute rewards for the completed epoch.
    ///
    /// Returns a Vec of (address, reward_cil) for each validator that received rewards.
    /// The caller is responsible for crediting these amounts to the ledger.
    ///
    /// After distribution, advances to the next epoch and resets heartbeat counters.
    pub fn distribute_epoch_rewards(&mut self) -> Vec<(String, u128)> {
        let epoch_rate = self.epoch_reward_rate();
        if epoch_rate == 0 || self.remaining_cil == 0 {
            self.advance_epoch();
            return vec![];
        }

        // Cap at remaining pool balance
        let budget = epoch_rate.min(self.remaining_cil);

        // Collect eligible validators and their linear stake weights
        let eligible: Vec<(String, u128)> = self
            .validators
            .iter()
            .filter(|(_, v)| v.is_eligible(self.current_epoch))
            .map(|(addr, v)| (addr.clone(), v.linear_stake_weight()))
            .filter(|(_, w)| *w > 0)
            .collect();

        if eligible.is_empty() {
            // No eligible validators this epoch — budget stays in pool
            self.advance_epoch();
            return vec![];
        }

        let total_weight: u128 = eligible.iter().map(|(_, w)| w).sum();
        if total_weight == 0 {
            self.advance_epoch();
            return vec![];
        }

        // Proportional distribution: reward_i = budget × (weight_i / total_weight)
        let mut rewards: Vec<(String, u128)> = Vec::new();
        let mut actually_distributed: u128 = 0;

        for (addr, weight) in &eligible {
            // Use u128 multiplication then divide to avoid overflow:
            // reward = (budget * weight) / total_weight
            // On overflow, use divide-before-multiply fallback
            // instead of returning 0 (which would silently lose validator rewards).
            let reward = match budget.checked_mul(*weight) {
                Some(prod) => prod / total_weight,
                None => {
                    // Overflow: divide first (less precise, but never zero for non-zero inputs)
                    (budget / total_weight) * (*weight)
                        + (budget % total_weight) * (*weight) / total_weight
                }
            };

            if reward > 0 {
                rewards.push((addr.clone(), reward));
                actually_distributed += reward;
            }
        }

        // Deduct from pool
        self.remaining_cil = self.remaining_cil.saturating_sub(actually_distributed);
        self.total_distributed_cil += actually_distributed;

        // Update per-validator cumulative totals
        for (addr, reward) in &rewards {
            if let Some(state) = self.validators.get_mut(addr) {
                state.cumulative_rewards_cil += reward;
            }
        }

        self.advance_epoch();
        rewards
    }

    /// Advance to the next epoch: increment counter, reset heartbeats, update halvings.
    fn advance_epoch(&mut self) {
        self.current_epoch += 1;
        self.epoch_start_timestamp += self.epoch_duration_secs;
        self.halvings_occurred = self.current_epoch / REWARD_HALVING_INTERVAL_EPOCHS;

        // Save last epoch uptime before resetting counters
        for state in self.validators.values_mut() {
            state.last_epoch_uptime_pct = state.uptime_pct();
            state.heartbeats_current_epoch = 0;
            state.expected_heartbeats = 0;
        }
    }

    /// Public version of advance_epoch for non-leader nodes.
    /// Advances to next epoch WITHOUT distributing rewards.
    /// Non-leaders use this to stay in sync with the epoch counter
    /// while waiting for the leader's reward blocks via gossip.
    pub fn advance_epoch_only(&mut self) {
        self.advance_epoch();
    }

    /// Sync pool accounting when a REWARD:EPOCH:N mint block is received from
    /// the leader via gossip/sync. This keeps the non-leader's pool stats
    /// consistent with the actual ledger state.
    pub fn sync_reward_from_gossip(&mut self, recipient: &str, amount_cil: u128) {
        self.remaining_cil = self.remaining_cil.saturating_sub(amount_cil);
        self.total_distributed_cil += amount_cil;
        if let Some(state) = self.validators.get_mut(recipient) {
            state.cumulative_rewards_cil += amount_cil;
        }
    }

    /// Unregister a validator from reward tracking (voluntary exit or auto-unregister).
    /// Returns true if the validator was found and removed.
    pub fn unregister_validator(&mut self, address: &str) -> bool {
        self.validators.remove(address).is_some()
    }

    /// Update stake weight for a validator (e.g., after receiving rewards or additional funds).
    /// This ensures reward distribution reflects the current balance.
    pub fn update_stake(&mut self, address: &str, new_stake_cil: u128) {
        if let Some(state) = self.validators.get_mut(address) {
            state.stake_cil = new_stake_cil;
        }
    }

    /// Get reward info for a specific validator.
    pub fn validator_info(&self, address: &str) -> Option<&ValidatorRewardState> {
        self.validators.get(address)
    }

    /// Summary stats for the reward pool.
    pub fn pool_summary(&self) -> RewardPoolSummary {
        let eligible_count = self
            .validators
            .values()
            .filter(|v| v.is_eligible(self.current_epoch))
            .count() as u64;
        let total_validators = self.validators.len() as u64;

        RewardPoolSummary {
            remaining_cil: self.remaining_cil,
            total_distributed_cil: self.total_distributed_cil,
            current_epoch: self.current_epoch,
            epoch_reward_rate_cil: self.epoch_reward_rate(),
            halvings_occurred: self.halvings_occurred,
            total_validators,
            eligible_validators: eligible_count,
            pool_exhaustion_bps: if VALIDATOR_REWARD_POOL_CIL > 0 {
                // Basis points (10000 = 100%) — pure integer math
                ((VALIDATOR_REWARD_POOL_CIL - self.remaining_cil) * 10_000
                    / VALIDATOR_REWARD_POOL_CIL) as u64
            } else {
                0
            },
        }
    }
}

/// Serializable summary of reward pool state (for /reward-info endpoint).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RewardPoolSummary {
    pub remaining_cil: u128,
    pub total_distributed_cil: u128,
    pub current_epoch: u64,
    pub epoch_reward_rate_cil: u128,
    pub halvings_occurred: u64,
    pub total_validators: u64,
    pub eligible_validators: u64,
    /// Pool exhaustion in basis points (10000 = 100%), pure integer
    pub pool_exhaustion_bps: u64,
}

// ─────────────────────────────────────────────────────────────────
// Integer square root (Newton's method) — deterministic across platforms
// NOTE: No longer used for reward weights.
// Kept for AMM/DEX math (LP token calculation). NOT for voting or rewards.
// Scoped to crate to prevent external misuse for voting power.
// ─────────────────────────────────────────────────────────────────
#[allow(dead_code)]
pub(crate) fn isqrt(n: u128) -> u128 {
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

// ─────────────────────────────────────────────────────────────────
// Unit Tests
// ─────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use crate::CIL_PER_LOS;

    const GENESIS_TS: u64 = 1_770_580_908; // Same as genesis_config.json

    #[test]
    fn test_isqrt() {
        assert_eq!(isqrt(0), 0);
        assert_eq!(isqrt(1), 1);
        assert_eq!(isqrt(4), 2);
        assert_eq!(isqrt(9), 3);
        assert_eq!(isqrt(100), 10);
        assert_eq!(isqrt(1000), 31); // √1000 ≈ 31.6
        assert_eq!(isqrt(10000), 100);
        assert_eq!(isqrt(1_000_000), 1000);
    }

    #[test]
    fn test_pool_creation() {
        let pool = ValidatorRewardPool::new(GENESIS_TS);
        assert_eq!(pool.remaining_cil, 500_000 * CIL_PER_LOS);
        assert_eq!(pool.current_epoch, 0);
        assert_eq!(pool.total_distributed_cil, 0);
    }

    #[test]
    fn test_epoch_reward_rate_halving() {
        let mut pool = ValidatorRewardPool::new(GENESIS_TS);
        // Epoch 0: full rate
        assert_eq!(pool.epoch_reward_rate(), 5_000 * CIL_PER_LOS);

        // Advance to epoch 48 (first halving)
        pool.current_epoch = 48;
        assert_eq!(pool.epoch_reward_rate(), 2_500 * CIL_PER_LOS);

        // Epoch 96 (second halving)
        pool.current_epoch = 96;
        assert_eq!(pool.epoch_reward_rate(), 1_250 * CIL_PER_LOS);

        // Epoch 144 (third halving)
        pool.current_epoch = 144;
        assert_eq!(pool.epoch_reward_rate(), 625 * CIL_PER_LOS);
    }

    #[test]
    fn test_genesis_validators_excluded_mainnet() {
        // Genesis validators are now eligible for rewards on BOTH testnet and mainnet.
        // They secure the network from genesis and deserve the same compensation.
        let mut pool = ValidatorRewardPool::new(GENESIS_TS);
        let genesis_addr = "LOSgenesis1";
        let normal_addr = "LOSnormal1";

        pool.register_validator(genesis_addr, true, 1000 * CIL_PER_LOS);
        pool.register_validator(normal_addr, false, 1000 * CIL_PER_LOS);

        // Advance past probation
        pool.current_epoch = 2;

        // Set heartbeats to 100% uptime
        pool.set_expected_heartbeats(60);
        for v in pool.validators.values_mut() {
            v.heartbeats_current_epoch = v.expected_heartbeats;
        }

        let genesis_state = pool.validators.get(genesis_addr).unwrap();
        // Genesis validators are eligible (both testnet and mainnet)
        assert!(genesis_state.is_eligible(pool.current_epoch));

        let normal_state = pool.validators.get(normal_addr).unwrap();
        assert!(normal_state.is_eligible(pool.current_epoch));
    }

    #[test]
    fn test_genesis_validators_obey_probation() {
        // Genesis validators must ALSO pass probation — no shortcuts.
        // Epoch 0 = join epoch. They should NOT be eligible at epoch 0.
        let mut pool = ValidatorRewardPool::new(GENESIS_TS);
        let genesis_addr = "LOSgenesis_prob";
        pool.register_validator(genesis_addr, true, 1000 * CIL_PER_LOS);
        pool.set_expected_heartbeats(60);
        for v in pool.validators.values_mut() {
            v.heartbeats_current_epoch = v.expected_heartbeats;
        }

        // Epoch 0: in probation, NOT eligible
        assert!(!pool.validators.get(genesis_addr).unwrap().is_eligible(0));

        // Epoch 1: past probation, eligible
        pool.current_epoch = 1;
        assert!(pool.validators.get(genesis_addr).unwrap().is_eligible(1));
    }

    #[test]
    fn test_heartbeat_once_dedup() {
        use std::collections::BTreeSet;
        let mut pool = ValidatorRewardPool::new(GENESIS_TS);
        pool.register_validator("LOSval1", false, 1000 * CIL_PER_LOS);

        let mut seen = BTreeSet::new();
        // First call: recorded
        assert!(pool.record_heartbeat_once("LOSval1", &mut seen));
        // Second call same tick: deduplicated
        assert!(!pool.record_heartbeat_once("LOSval1", &mut seen));
        // Third call same tick: still deduplicated
        assert!(!pool.record_heartbeat_once("LOSval1", &mut seen));

        // Only 1 heartbeat recorded
        assert_eq!(
            pool.validators
                .get("LOSval1")
                .unwrap()
                .heartbeats_current_epoch,
            1
        );

        // New tick (new seen set) — can record again
        let mut seen2 = BTreeSet::new();
        assert!(pool.record_heartbeat_once("LOSval1", &mut seen2));
        assert_eq!(
            pool.validators
                .get("LOSval1")
                .unwrap()
                .heartbeats_current_epoch,
            2
        );
    }

    #[test]
    fn test_probation_period() {
        let mut pool = ValidatorRewardPool::new(GENESIS_TS);
        let addr = "LOSvalidator1";

        pool.register_validator(addr, false, 2000 * CIL_PER_LOS);
        pool.set_expected_heartbeats(60);

        // During epoch 0 (join epoch) — still in probation
        {
            let v = pool.validators.get_mut(addr).unwrap();
            v.heartbeats_current_epoch = v.expected_heartbeats; // 100% uptime
        }
        assert!(!pool.validators.get(addr).unwrap().is_eligible(0));

        // Epoch 1 — past probation → eligible
        pool.current_epoch = 1;
        {
            let v = pool.validators.get_mut(addr).unwrap();
            v.heartbeats_current_epoch = v.expected_heartbeats;
        }
        assert!(pool.validators.get(addr).unwrap().is_eligible(1));
    }

    #[test]
    fn test_uptime_requirement() {
        let mut pool = ValidatorRewardPool::new(GENESIS_TS);
        let addr = "LOSvalidator2";

        pool.register_validator(addr, false, 1000 * CIL_PER_LOS);
        pool.current_epoch = 2;
        // Use heartbeat interval of 1s so we get enough heartbeats
        // for meaningful uptime calculation (epoch_duration / 1 = epoch_duration)
        pool.set_expected_heartbeats(1);

        // 90% uptime — below 95% threshold
        {
            let v = pool.validators.get_mut(addr).unwrap();
            let expected = v.expected_heartbeats;
            v.heartbeats_current_epoch = expected * 90 / 100;
        }
        assert!(!pool.validators.get(addr).unwrap().is_eligible(2));

        // 95% uptime — meets threshold
        {
            let v = pool.validators.get_mut(addr).unwrap();
            let expected = v.expected_heartbeats;
            v.heartbeats_current_epoch = expected * 95 / 100;
        }
        assert!(pool.validators.get(addr).unwrap().is_eligible(2));
    }

    #[test]
    fn test_linear_stake_weight() {
        let v1 = ValidatorRewardState::new(0, false, 1_000 * CIL_PER_LOS);
        let v2 = ValidatorRewardState::new(0, false, 10_000 * CIL_PER_LOS);

        // Linear weight = stake_cil
        // 10× the stake gives exactly 10× the weight (Sybil-neutral)
        assert_eq!(v1.linear_stake_weight(), 1_000 * CIL_PER_LOS);
        assert_eq!(v2.linear_stake_weight(), 10_000 * CIL_PER_LOS);
        assert_eq!(v2.linear_stake_weight() / v1.linear_stake_weight(), 10);
    }

    #[test]
    fn test_distribute_epoch_rewards() {
        let mut pool = ValidatorRewardPool::new(GENESIS_TS);

        // Register 3 validators: 1 genesis, 2 normal
        pool.register_validator("LOSgenesis_v1", true, 1000 * CIL_PER_LOS);
        pool.register_validator("LOSnormal_v1", false, 1000 * CIL_PER_LOS);
        pool.register_validator("LOSnormal_v2", false, 4000 * CIL_PER_LOS);

        // Advance past probation (epoch 2)
        pool.current_epoch = 2;
        pool.set_expected_heartbeats(60);
        for v in pool.validators.values_mut() {
            v.heartbeats_current_epoch = v.expected_heartbeats; // 100% uptime
        }

        let initial_remaining = pool.remaining_cil;
        let rewards = pool.distribute_epoch_rewards();

        // All 3 validators eligible (including genesis, past probation epoch)
        assert_eq!(rewards.len(), 3);

        let total_rewarded: u128 = rewards.iter().map(|(_, r)| r).sum();
        assert!(total_rewarded > 0);
        assert!(total_rewarded <= 5_000 * CIL_PER_LOS);

        // Pool should be reduced
        assert_eq!(pool.remaining_cil, initial_remaining - total_rewarded);
        assert_eq!(pool.total_distributed_cil, total_rewarded);

        // Epoch should have advanced
        assert_eq!(pool.current_epoch, 3);
    }

    #[test]
    fn test_no_eligible_validators_preserves_pool() {
        let mut pool = ValidatorRewardPool::new(GENESIS_TS);

        // Only genesis validators — now eligible like everyone else
        pool.register_validator("LOSgenesis_v1", true, 1000 * CIL_PER_LOS);
        pool.current_epoch = 5;
        pool.set_expected_heartbeats(60);
        for v in pool.validators.values_mut() {
            v.heartbeats_current_epoch = v.expected_heartbeats;
        }

        let rewards = pool.distribute_epoch_rewards();

        // Genesis validators now earn rewards (eligible after probation with sufficient uptime)
        assert_eq!(rewards.len(), 1);
        assert_eq!(pool.current_epoch, 6); // Epoch still advances
    }

    #[test]
    fn test_pool_exhaustion_cap() {
        // Create a pool with only 1000 LOS remaining
        let mut pool = ValidatorRewardPool::with_balance(GENESIS_TS, 1_000 * CIL_PER_LOS);

        pool.register_validator("LOSval1", false, 2000 * CIL_PER_LOS);
        pool.current_epoch = 2;
        pool.set_expected_heartbeats(60);
        for v in pool.validators.values_mut() {
            v.heartbeats_current_epoch = v.expected_heartbeats;
        }

        // Rate is 5000 LOS but only 1000 available — should cap at 1000
        let rewards = pool.distribute_epoch_rewards();
        let total: u128 = rewards.iter().map(|(_, r)| r).sum();
        assert!(total <= 1_000 * CIL_PER_LOS);
    }

    #[test]
    fn test_epoch_timing() {
        let pool = ValidatorRewardPool::new(GENESIS_TS);
        let epoch_dur = pool.epoch_duration_secs;
        // Grace period: 5s for testnet (epoch_dur <= 300), 300s for mainnet
        let grace = if epoch_dur <= 300 { 5u64 } else { 300u64 };

        // Not complete at start
        assert!(!pool.is_epoch_complete(GENESIS_TS));
        assert!(!pool.is_epoch_complete(GENESIS_TS + epoch_dur - 1));

        // NOT complete at exact epoch boundary (grace period not elapsed)
        assert!(!pool.is_epoch_complete(GENESIS_TS + epoch_dur));
        assert!(!pool.is_epoch_complete(GENESIS_TS + epoch_dur + grace - 1));

        // Complete after epoch boundary + grace period
        assert!(pool.is_epoch_complete(GENESIS_TS + epoch_dur + grace));

        // Remaining seconds (measures to epoch boundary, not grace)
        assert_eq!(pool.epoch_remaining_secs(GENESIS_TS), epoch_dur);
        assert_eq!(pool.epoch_remaining_secs(GENESIS_TS + 10), epoch_dur - 10);
        assert_eq!(pool.epoch_remaining_secs(GENESIS_TS + epoch_dur), 0);
    }

    #[test]
    fn test_heartbeat_recording() {
        let mut pool = ValidatorRewardPool::new(GENESIS_TS);
        pool.register_validator("LOSval1", false, 1000 * CIL_PER_LOS);

        pool.record_heartbeat("LOSval1");
        pool.record_heartbeat("LOSval1");
        pool.record_heartbeat("LOSval1");

        assert_eq!(
            pool.validators
                .get("LOSval1")
                .unwrap()
                .heartbeats_current_epoch,
            3
        );

        // Recording heartbeat for unknown validator is a no-op
        pool.record_heartbeat("LOSunknown");
    }

    #[test]
    fn test_pool_summary() {
        let mut pool = ValidatorRewardPool::new(GENESIS_TS);
        pool.register_validator("LOSgenesis", true, 1000 * CIL_PER_LOS);
        pool.register_validator("LOSval1", false, 2000 * CIL_PER_LOS);
        pool.current_epoch = 2;
        pool.set_expected_heartbeats(60);
        for v in pool.validators.values_mut() {
            v.heartbeats_current_epoch = v.expected_heartbeats;
        }

        let summary = pool.pool_summary();
        assert_eq!(summary.total_validators, 2);
        // Both eligible (including genesis, past probation epoch)
        assert_eq!(summary.eligible_validators, 2);
        assert_eq!(summary.current_epoch, 2);
        assert_eq!(summary.epoch_reward_rate_cil, 5_000 * CIL_PER_LOS);
    }

    #[test]
    fn test_minimum_stake_requirement() {
        let mut pool = ValidatorRewardPool::new(GENESIS_TS);

        // Register with less than 1000 LOS stake
        pool.register_validator("LOSpoor", false, 500 * CIL_PER_LOS);
        pool.current_epoch = 2;
        pool.set_expected_heartbeats(60);
        for v in pool.validators.values_mut() {
            v.heartbeats_current_epoch = v.expected_heartbeats;
        }

        assert!(!pool.validators.get("LOSpoor").unwrap().is_eligible(2));
    }
}
