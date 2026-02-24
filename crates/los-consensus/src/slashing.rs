// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// UNAUTHORITY (LOS) - VALIDATOR SLASHING & SAFETY
//
// Task #4: Anti-Misbehavior Mechanism
// - Double-signing detection (100% slash + permanent ban)
// - Uptime tracking with 1% slash for extended downtime
// - Validator state machine (active → slashed → banned)
// - Automatic punishment enforcement
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

/// Slashing constants — all percentages expressed as basis points (1/100 of a percent)
/// for deterministic cross-platform consensus. 10000 bps = 100%.
pub const DOUBLE_SIGNING_SLASH_BPS: u32 = 10_000; // 100% of stake
pub const DOWNTIME_SLASH_BPS: u32 = 100; // 1% of stake
pub const DOWNTIME_THRESHOLD_BLOCKS: u64 = 10000; // ~1 hour at 0.36s blocks
pub const DOWNTIME_WINDOW_BLOCKS: u64 = 50000; // ~5 hours observation window
pub const MIN_UPTIME_BPS: u32 = 9500; // Need 95%+ uptime (9500 bps)

/// Violation types that trigger slashing
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ViolationType {
    DoubleSigning,
    ExtendedDowntime,
    FraudulentTransaction,
}

/// Validator slash record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlashEvent {
    pub block_height: u64,
    pub validator_address: String,
    pub violation_type: ViolationType,
    pub slash_amount_cil: u128,
    /// Slash percentage in basis points (10000 = 100%)
    pub slash_bps: u32,
    pub timestamp: u64,
}

/// Validator safety state machine
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ValidatorStatus {
    Active,
    Slashed,   // Caught misbehaving but not yet banned
    Banned,    // Permanently removed from consensus
    Unstaking, // Voluntary exit in progress
}

/// Per-validator signature tracking for double-signing detection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureRecord {
    pub block_height: u64,
    pub signature_hash: String,
    pub timestamp: u64,
}

/// Validator safety profile
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorSafetyProfile {
    pub validator_address: String,

    /// Current status in state machine
    pub status: ValidatorStatus,

    /// Total stake slashed (CIL)
    pub total_slashed_cil: u128,

    /// Recent signatures for double-signing detection
    pub recent_signatures: VecDeque<SignatureRecord>,

    /// Blocks participated in (for uptime calculation)
    pub blocks_participated: u64,

    /// Total blocks in observation window
    pub total_blocks_observed: u64,

    /// Last block participation timestamp
    pub last_participation_timestamp: u64,

    /// Slash history (for audit trail)
    pub slash_history: Vec<SlashEvent>,

    /// Number of times slashed
    pub violation_count: u32,
}

impl ValidatorSafetyProfile {
    pub fn new(validator_address: String) -> Self {
        Self {
            validator_address,
            status: ValidatorStatus::Active,
            total_slashed_cil: 0,
            recent_signatures: VecDeque::new(),
            blocks_participated: 0,
            total_blocks_observed: 0,
            last_participation_timestamp: 0,
            slash_history: Vec::new(),
            violation_count: 0,
        }
    }

    /// Calculate uptime in basis points (10000 = 100%) — deterministic integer math
    pub fn get_uptime_bps(&self) -> u32 {
        if self.total_blocks_observed == 0 {
            return 10_000; // 100%
        }
        // Integer: (participated * 10000) / observed
        ((self.blocks_participated as u128 * 10_000) / self.total_blocks_observed as u128) as u32
    }

    /// Legacy f64 helper — for display/logging only, NOT for consensus decisions
    /// MAINNET SAFETY: Excluded from mainnet builds (uses f64)
    #[cfg(not(feature = "mainnet"))]
    pub fn get_uptime_percent(&self) -> f64 {
        self.get_uptime_bps() as f64 / 100.0
    }

    /// Check if validator meets minimum uptime requirement (deterministic)
    pub fn meets_uptime_requirement(&self) -> bool {
        self.get_uptime_bps() >= MIN_UPTIME_BPS
    }
}

/// Slashing proposal - requires multiple validator confirmations before execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlashProposal {
    pub proposal_id: String,
    pub offender: String,
    pub offense_type: ViolationType,
    pub evidence_hash: String,
    pub proposed_at: u64,
    pub proposer: String,
    pub confirmations: Vec<String>, // Validator addresses that confirmed
    pub executed: bool,
    pub staked_amount_cil: Option<u128>, // Actual staked amount from ledger (not hardcoded)
}

/// Slashing Manager - core safety enforcement with multi-validator confirmation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlashingManager {
    /// Per-validator safety profiles
    /// MAINNET: BTreeMap for deterministic serialization
    validators: BTreeMap<String, ValidatorSafetyProfile>,

    /// Global slash events log
    slash_events: Vec<SlashEvent>,

    /// Current block height
    current_block_height: u64,

    /// Pending slash proposals requiring confirmation
    /// MAINNET: BTreeMap for deterministic serialization
    pending_proposals: BTreeMap<String, SlashProposal>,
}

impl Default for SlashingManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SlashingManager {
    /// Create new slashing manager
    pub fn new() -> Self {
        Self {
            validators: BTreeMap::new(),
            slash_events: Vec::new(),
            current_block_height: 0,
            pending_proposals: BTreeMap::new(),
        }
    }

    /// Register a new validator for safety tracking
    pub fn register_validator(&mut self, validator_address: String) {
        let addr_clone = validator_address.clone();
        self.validators
            .entry(validator_address)
            .or_insert_with(|| ValidatorSafetyProfile::new(addr_clone));
    }

    /// Record a block signature for double-signing detection
    pub fn record_signature(
        &mut self,
        validator_address: &str,
        block_height: u64,
        signature_hash: String,
        timestamp: u64,
    ) -> Result<(), String> {
        let profile = self
            .validators
            .get_mut(validator_address)
            .ok_or_else(|| format!("Validator {} not registered", validator_address))?;

        // Check if already signed a different block at same height
        for sig in &profile.recent_signatures {
            if sig.block_height == block_height && sig.signature_hash != signature_hash {
                // DOUBLE SIGNING DETECTED!
                return Err(format!(
                    "Double-signing detected for {} at height {}",
                    validator_address, block_height
                ));
            }
        }

        // Record signature
        profile.recent_signatures.push_back(SignatureRecord {
            block_height,
            signature_hash,
            timestamp,
        });

        // Keep only recent signatures (last 1000 blocks)
        if profile.recent_signatures.len() > 1000 {
            profile.recent_signatures.pop_front();
        }

        Ok(())
    }

    /// Slash validator for double-signing (100% slash + ban)
    pub fn slash_double_signing(
        &mut self,
        validator_address: &str,
        block_height: u64,
        staked_amount_cil: u128,
        timestamp: u64,
    ) -> Result<u128, String> {
        let profile = self
            .validators
            .get_mut(validator_address)
            .ok_or_else(|| format!("Validator {} not registered", validator_address))?;

        if profile.status == ValidatorStatus::Banned {
            return Err(format!("Validator {} already banned", validator_address));
        }

        let slash_amount = staked_amount_cil; // 100% slash
        profile.total_slashed_cil += slash_amount;
        profile.status = ValidatorStatus::Banned; // Permanent ban
        profile.violation_count += 1;

        let event = SlashEvent {
            block_height,
            validator_address: validator_address.to_string(),
            violation_type: ViolationType::DoubleSigning,
            slash_amount_cil: slash_amount,
            slash_bps: DOUBLE_SIGNING_SLASH_BPS,
            timestamp,
        };

        profile.slash_history.push(event.clone());
        self.slash_events.push(event);

        Ok(slash_amount)
    }

    /// Record block participation (for uptime tracking)
    pub fn record_block_participation(
        &mut self,
        validator_address: &str,
        _block_height: u64,
        timestamp: u64,
    ) -> Result<(), String> {
        let profile = self
            .validators
            .get_mut(validator_address)
            .ok_or_else(|| format!("Validator {} not registered", validator_address))?;

        profile.blocks_participated += 1;
        profile.total_blocks_observed += 1;
        profile.last_participation_timestamp = timestamp;

        Ok(())
    }

    /// Record block observation (whether validator participated or not)
    pub fn record_block_observation(&mut self, validator_address: &str) -> Result<(), String> {
        let profile = self
            .validators
            .get_mut(validator_address)
            .ok_or_else(|| format!("Validator {} not registered", validator_address))?;

        profile.total_blocks_observed += 1;

        Ok(())
    }

    /// Check and slash for extended downtime
    pub fn check_and_slash_downtime(
        &mut self,
        validator_address: &str,
        block_height: u64,
        staked_amount_cil: u128,
        timestamp: u64,
    ) -> Result<Option<u128>, String> {
        let profile = self
            .validators
            .get_mut(validator_address)
            .ok_or_else(|| format!("Validator {} not registered", validator_address))?;

        if profile.status == ValidatorStatus::Banned {
            return Err(format!("Validator {} is banned", validator_address));
        }

        // Check if uptime falls below threshold in observation window
        if profile.total_blocks_observed >= DOWNTIME_WINDOW_BLOCKS
            && !profile.meets_uptime_requirement()
        {
            // Use integer math for slash calculation
            // DOWNTIME: 1% of stake (100 bps). Double-signing: 100% (10000 bps).
            let slash_amount = if DOWNTIME_SLASH_BPS >= 10_000 {
                staked_amount_cil
            } else {
                // Use DOWNTIME_SLASH_BPS constant properly
                // slash = stake * bps / 10_000, rounds up via ceiling division
                (staked_amount_cil * DOWNTIME_SLASH_BPS as u128).div_ceil(10_000)
            };

            profile.total_slashed_cil += slash_amount;
            profile.status = ValidatorStatus::Slashed;
            profile.violation_count += 1;

            let event = SlashEvent {
                block_height,
                validator_address: validator_address.to_string(),
                violation_type: ViolationType::ExtendedDowntime,
                slash_amount_cil: slash_amount,
                slash_bps: DOWNTIME_SLASH_BPS,
                timestamp,
            };

            profile.slash_history.push(event.clone());
            self.slash_events.push(event);

            // Reset observation window
            profile.blocks_participated = 0;
            profile.total_blocks_observed = 0;

            Ok(Some(slash_amount))
        } else {
            Ok(None)
        }
    }

    /// Get validator safety profile
    pub fn get_profile(&self, validator_address: &str) -> Option<&ValidatorSafetyProfile> {
        self.validators.get(validator_address)
    }

    /// Get validator status
    pub fn get_status(&self, validator_address: &str) -> Option<ValidatorStatus> {
        self.validators.get(validator_address).map(|p| p.status)
    }

    /// Ban a validator (emergency mechanism)
    pub fn emergency_ban(&mut self, validator_address: &str, _reason: &str) -> Result<(), String> {
        let profile = self
            .validators
            .get_mut(validator_address)
            .ok_or_else(|| format!("Validator {} not registered", validator_address))?;

        profile.status = ValidatorStatus::Banned;
        Ok(())
    }

    /// Get all banned validators
    pub fn get_banned_validators(&self) -> Vec<String> {
        self.validators
            .iter()
            .filter(|(_, profile)| profile.status == ValidatorStatus::Banned)
            .map(|(addr, _)| addr.clone())
            .collect()
    }

    /// Get all slashed validators
    pub fn get_slashed_validators(&self) -> Vec<String> {
        self.validators
            .iter()
            .filter(|(_, profile)| profile.status == ValidatorStatus::Slashed)
            .map(|(addr, _)| addr.clone())
            .collect()
    }

    /// Get slash history for validator
    pub fn get_slash_history(&self, validator_address: &str) -> Option<Vec<SlashEvent>> {
        self.validators
            .get(validator_address)
            .map(|p| p.slash_history.clone())
    }

    /// Get all slash events
    pub fn get_all_slash_events(&self) -> &[SlashEvent] {
        &self.slash_events
    }

    /// Set validator status to Unstaking (voluntary exit).
    /// Returns Err if validator is not found or already banned/unstaking.
    pub fn set_unstaking(&mut self, validator_address: &str) -> Result<(), String> {
        let profile = self
            .validators
            .get_mut(validator_address)
            .ok_or_else(|| format!("Validator {} not registered", validator_address))?;

        match profile.status {
            ValidatorStatus::Banned => Err(format!(
                "Validator {} is permanently banned",
                validator_address
            )),
            ValidatorStatus::Unstaking => Err(format!(
                "Validator {} is already unstaking",
                validator_address
            )),
            ValidatorStatus::Active | ValidatorStatus::Slashed => {
                profile.status = ValidatorStatus::Unstaking;
                Ok(())
            }
        }
    }

    /// Get total slashed amount for validator
    pub fn get_total_slashed(&self, validator_address: &str) -> Option<u128> {
        self.validators
            .get(validator_address)
            .map(|p| p.total_slashed_cil)
    }

    /// Update block height
    pub fn update_block_height(&mut self, height: u64) {
        self.current_block_height = height;
    }

    /// Fully remove a validator from the SlashingManager (on unregister).
    /// Unlike set_unstaking which preserves the record, this removes all traces.
    pub fn remove_validator(&mut self, validator_address: &str) -> bool {
        self.validators.remove(validator_address).is_some()
    }

    /// Get all registered validator addresses (genesis + dynamically registered)
    pub fn get_all_validator_addresses(&self) -> Vec<String> {
        self.validators.keys().cloned().collect()
    }

    /// Get network safety statistics
    pub fn get_safety_stats(&self) -> SafetyStats {
        let total_validators = self.validators.len() as u32;
        let banned_count = self
            .validators
            .values()
            .filter(|p| p.status == ValidatorStatus::Banned)
            .count() as u32;
        let slashed_count = self
            .validators
            .values()
            .filter(|p| p.status == ValidatorStatus::Slashed)
            .count() as u32;

        let total_slashed: u128 = self.validators.values().map(|p| p.total_slashed_cil).sum();

        SafetyStats {
            total_validators,
            banned_count,
            slashed_count,
            total_slashed_cil: total_slashed,
            total_slash_events: self.slash_events.len() as u32,
            active_validators: total_validators - banned_count - slashed_count,
        }
    }

    /// Clear all data (for testing)
    pub fn clear(&mut self) {
        self.validators.clear();
        self.slash_events.clear();
        self.current_block_height = 0;
        self.pending_proposals.clear();
    }

    /// Propose a slash - requires 2/3+1 validator confirmations before execution
    pub fn propose_slash(
        &mut self,
        offender: String,
        offense: ViolationType,
        evidence_hash: String,
        proposer: String,
        timestamp: u64,
    ) -> Result<String, String> {
        // Check if offender is registered
        if !self.validators.contains_key(&offender) {
            return Err(format!("Offender {} not registered", offender));
        }

        // SECURITY P1-5: Include evidence hash in proposal ID to prevent collision
        // Old: format!("slash_{}_{}", offender, timestamp) — collides when same offender
        // is slashed at the same second for different offenses
        let proposal_id = format!("slash_{}_{}_{}", offender, evidence_hash, timestamp);

        // Check if proposal already exists
        if self.pending_proposals.contains_key(&proposal_id) {
            return Err("Proposal already exists".to_string());
        }

        let proposal = SlashProposal {
            proposal_id: proposal_id.clone(),
            offender,
            offense_type: offense,
            evidence_hash,
            proposed_at: timestamp,
            proposer: proposer.clone(),
            confirmations: vec![proposer], // Proposer auto-confirms
            executed: false,
            staked_amount_cil: None, // Set later via confirm_slash with actual ledger balance
        };

        self.pending_proposals.insert(proposal_id.clone(), proposal);
        Ok(proposal_id)
    }

    /// Confirm a slash proposal - returns true if threshold met and slash executed
    /// staked_amount_cil: actual balance of offender from ledger (used if threshold met)
    pub fn confirm_slash(
        &mut self,
        proposal_id: &str,
        confirmer: String,
        total_validators: usize,
        timestamp: u64,
        staked_amount_cil: Option<u128>,
    ) -> Result<bool, String> {
        let proposal = self
            .pending_proposals
            .get_mut(proposal_id)
            .ok_or("Proposal not found")?;

        // Update staked amount if provided (from ledger at confirmation time)
        if let Some(amount) = staked_amount_cil {
            proposal.staked_amount_cil = Some(amount);
        }

        if proposal.executed {
            return Err("Already executed".to_string());
        }

        // Add confirmation if not already confirmed by this validator
        if !proposal.confirmations.contains(&confirmer) {
            proposal.confirmations.push(confirmer);
        }

        // Require 2/3 + 1 confirmations (Byzantine fault tolerance)
        let threshold = (total_validators * 2 / 3) + 1;

        if proposal.confirmations.len() >= threshold {
            // Execute slash
            let offender = proposal.offender.clone();
            let offense_type = proposal.offense_type;

            // Use actual staked_amount from the proposal instead of hardcoded 100k
            // The staked_amount should be provided by the caller or read from ledger
            let staked_amount = proposal.staked_amount_cil.unwrap_or(0);

            // Execute appropriate slash
            match offense_type {
                ViolationType::DoubleSigning => {
                    self.slash_double_signing(
                        &offender,
                        self.current_block_height,
                        staked_amount,
                        timestamp,
                    )?;
                }
                ViolationType::ExtendedDowntime => {
                    if let Some(slash_amt) = self.check_and_slash_downtime(
                        &offender,
                        self.current_block_height,
                        staked_amount,
                        timestamp,
                    )? {
                        // Slash executed
                        let _ = slash_amt;
                    }
                }
                ViolationType::FraudulentTransaction => {
                    // Fraudulent transactions — 100% slash like double signing
                    self.slash_double_signing(
                        &offender,
                        self.current_block_height,
                        staked_amount,
                        timestamp,
                    )?;
                }
            }

            // Mark as executed
            if let Some(proposal) = self.pending_proposals.get_mut(proposal_id) {
                proposal.executed = true;
            }

            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Get pending slash proposals
    pub fn get_pending_proposals(&self) -> Vec<SlashProposal> {
        self.pending_proposals.values().cloned().collect()
    }
}

/// Network-wide safety statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyStats {
    pub total_validators: u32,
    pub banned_count: u32,
    pub slashed_count: u32,
    pub active_validators: u32,
    pub total_slashed_cil: u128,
    pub total_slash_events: u32,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TESTS
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validator_registration() {
        let mut manager = SlashingManager::new();
        manager.register_validator("validator1".to_string());

        assert!(manager.get_profile("validator1").is_some());
        assert_eq!(
            manager.get_status("validator1"),
            Some(ValidatorStatus::Active)
        );
    }

    #[test]
    fn test_double_signing_detection() {
        let mut manager = SlashingManager::new();
        manager.register_validator("validator1".to_string());

        // First signature
        manager
            .record_signature("validator1", 100, "sig_hash_1".to_string(), 1000)
            .unwrap();

        // Different signature for same block height = double signing
        let result = manager.record_signature("validator1", 100, "sig_hash_2".to_string(), 1001);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Double-signing detected"));
    }

    #[test]
    fn test_double_signing_slash() {
        let mut manager = SlashingManager::new();
        manager.register_validator("validator1".to_string());

        let staked = 100_000_000_000u128; // 1 LOS
        let slash_amount = manager
            .slash_double_signing("validator1", 100, staked, 1000)
            .unwrap();

        // 100% slash
        assert_eq!(slash_amount, staked);

        // Validator now banned
        assert_eq!(
            manager.get_status("validator1"),
            Some(ValidatorStatus::Banned)
        );

        // Cannot slash again
        let result = manager.slash_double_signing("validator1", 101, staked, 1001);
        assert!(result.is_err());
    }

    #[test]
    fn test_uptime_tracking() {
        let mut manager = SlashingManager::new();
        manager.register_validator("validator1".to_string());

        // Record 95 participations out of 100 blocks
        for _ in 0..95 {
            manager
                .record_block_participation("validator1", 1, 1000)
                .unwrap();
        }
        for _ in 0..5 {
            manager.record_block_observation("validator1").unwrap();
        }

        let profile = manager.get_profile("validator1").unwrap();
        assert_eq!(profile.blocks_participated, 95);
        assert_eq!(profile.total_blocks_observed, 100);
        assert_eq!(profile.get_uptime_bps(), 9500); // 95.00% in basis points
        assert!(profile.meets_uptime_requirement());
    }

    #[test]
    fn test_downtime_slash() {
        let mut manager = SlashingManager::new();
        manager.register_validator("validator1".to_string());

        let staked = 100_000_000_000u128; // 1 LOS

        // Record low uptime: 90% (below 95% threshold)
        for _ in 0..45000 {
            manager
                .record_block_participation("validator1", 1, 1000)
                .unwrap();
        }
        for _ in 0..5000 {
            manager.record_block_observation("validator1").unwrap();
        }

        // Now check downtime (observation window reached)
        let slash_result = manager
            .check_and_slash_downtime("validator1", 50000, staked, 1000)
            .unwrap();

        assert!(slash_result.is_some());
        let slash_amount = slash_result.unwrap();
        assert_eq!(slash_amount, 1_000_000_000); // 1% of 100B = 1B

        // Validator now slashed
        assert_eq!(
            manager.get_status("validator1"),
            Some(ValidatorStatus::Slashed)
        );
    }

    #[test]
    fn test_no_slash_if_uptime_sufficient() {
        let mut manager = SlashingManager::new();
        manager.register_validator("validator1".to_string());

        let staked = 100_000_000_000u128;

        // Record high uptime: 99%
        for _ in 0..49500 {
            manager
                .record_block_participation("validator1", 1, 1000)
                .unwrap();
        }
        for _ in 0..500 {
            manager.record_block_observation("validator1").unwrap();
        }

        let slash_result = manager
            .check_and_slash_downtime("validator1", 50000, staked, 1000)
            .unwrap();

        assert!(slash_result.is_none()); // No slash
        assert_eq!(
            manager.get_status("validator1"),
            Some(ValidatorStatus::Active)
        );
    }

    #[test]
    fn test_slash_history() {
        let mut manager = SlashingManager::new();
        manager.register_validator("validator1".to_string());

        let staked = 100_000_000_000u128;

        manager
            .slash_double_signing("validator1", 100, staked, 1000)
            .unwrap();

        let history = manager.get_slash_history("validator1").unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].violation_type, ViolationType::DoubleSigning);
    }

    #[test]
    fn test_banned_validators_list() {
        let mut manager = SlashingManager::new();
        manager.register_validator("validator1".to_string());
        manager.register_validator("validator2".to_string());

        let staked = 100_000_000_000u128;

        manager
            .slash_double_signing("validator1", 100, staked, 1000)
            .unwrap();

        let banned = manager.get_banned_validators();
        assert_eq!(banned.len(), 1);
        assert_eq!(banned[0], "validator1");
    }

    #[test]
    fn test_safety_stats() {
        let mut manager = SlashingManager::new();
        manager.register_validator("validator1".to_string());
        manager.register_validator("validator2".to_string());
        manager.register_validator("validator3".to_string());

        let staked = 100_000_000_000u128;

        manager
            .slash_double_signing("validator1", 100, staked, 1000)
            .unwrap();

        let stats = manager.get_safety_stats();
        assert_eq!(stats.total_validators, 3);
        assert_eq!(stats.banned_count, 1);
        assert_eq!(stats.active_validators, 2);
        assert_eq!(stats.total_slash_events, 1);
    }

    #[test]
    fn test_emergency_ban() {
        let mut manager = SlashingManager::new();
        manager.register_validator("validator1".to_string());

        manager
            .emergency_ban("validator1", "DoS attack detected")
            .unwrap();

        assert_eq!(
            manager.get_status("validator1"),
            Some(ValidatorStatus::Banned)
        );
    }

    #[test]
    fn test_multiple_validators_independent() {
        let mut manager = SlashingManager::new();
        manager.register_validator("validator1".to_string());
        manager.register_validator("validator2".to_string());

        let staked = 100_000_000_000u128;

        // Slash validator1
        manager
            .slash_double_signing("validator1", 100, staked, 1000)
            .unwrap();

        // Validator2 should still be active
        assert_eq!(
            manager.get_status("validator1"),
            Some(ValidatorStatus::Banned)
        );
        assert_eq!(
            manager.get_status("validator2"),
            Some(ValidatorStatus::Active)
        );
    }

    #[test]
    fn test_total_slashed_calculation() {
        let mut manager = SlashingManager::new();
        manager.register_validator("validator1".to_string());

        let staked1 = 100_000_000_000u128; // 1000 LOS

        manager
            .slash_double_signing("validator1", 100, staked1, 1000)
            .unwrap();

        let total = manager.get_total_slashed("validator1").unwrap();
        assert_eq!(total, staked1);
    }

    #[test]
    fn test_consecutive_slash_events() {
        let mut manager = SlashingManager::new();
        manager.register_validator("validator1".to_string());

        let staked = 100_000_000_000u128;

        // Record signature first
        manager
            .record_signature("validator1", 100, "sig_1".to_string(), 1000)
            .unwrap();

        // Try to slash for double-signing
        manager
            .slash_double_signing("validator1", 100, staked, 1000)
            .unwrap();

        let events = manager.get_all_slash_events();
        assert_eq!(events.len(), 1);
    }
}
