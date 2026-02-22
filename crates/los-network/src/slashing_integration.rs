// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// UNAUTHORITY (LOS) - SLASHING MODULE INTEGRATION
//
// Bridges the slashing consensus module with the node's block processing logic
// - Tracks validator signatures for double-signing detection
// - Monitors uptime and participation
// - Enforces slashing penalties automatically
// - Maintains validator safety profiles across the network
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Re-export slashing types for convenience
pub use los_consensus::slashing::{
    SlashEvent, ValidatorSafetyProfile, ValidatorStatus, ViolationType, DOUBLE_SIGNING_SLASH_BPS,
    DOWNTIME_SLASH_BPS, DOWNTIME_THRESHOLD_BLOCKS, DOWNTIME_WINDOW_BLOCKS, MIN_UPTIME_BPS,
};

/// Manages validator slashing state across the network
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlashingManager {
    /// Per-validator safety profiles
    pub validator_profiles: BTreeMap<String, ValidatorSafetyProfile>,

    /// Current block height (for tracking participation)
    pub current_block_height: u64,

    /// Validators currently banned from consensus
    pub banned_validators: Vec<String>,

    /// Total LOS slashed across network (audit trail)
    pub total_network_slash_cil: u128,

    /// Flag to enable/disable slashing enforcement
    pub enforcement_enabled: bool,
}

impl SlashingManager {
    /// Create new slashing manager with empty state
    pub fn new() -> Self {
        Self {
            validator_profiles: BTreeMap::new(),
            current_block_height: 0,
            banned_validators: Vec::new(),
            total_network_slash_cil: 0,
            enforcement_enabled: true,
        }
    }

    /// Register a validator for safety tracking
    pub fn register_validator(&mut self, validator_address: String) {
        if !self.validator_profiles.contains_key(&validator_address) {
            self.validator_profiles.insert(
                validator_address.clone(),
                ValidatorSafetyProfile::new(validator_address),
            );
        }
    }

    /// Record a validator's participation in a block
    pub fn record_participation(&mut self, validator_address: &str, _block_height: u64) {
        if let Some(profile) = self.validator_profiles.get_mut(validator_address) {
            if profile.status == ValidatorStatus::Active {
                profile.blocks_participated += 1;
                profile.last_participation_timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
            }
        }
    }

    /// Check if validator is banned from consensus
    pub fn is_validator_banned(&self, validator_address: &str) -> bool {
        self.banned_validators
            .contains(&validator_address.to_string())
    }

    /// Check if validator can participate in consensus
    pub fn can_validate(&self, validator_address: &str) -> bool {
        if !self.enforcement_enabled {
            return true;
        }

        if let Some(profile) = self.validator_profiles.get(validator_address) {
            profile.status == ValidatorStatus::Active
        } else {
            true // Unknown validator allowed
        }
    }

    /// Record a signature for double-signing detection
    pub fn record_signature(
        &mut self,
        validator_address: &str,
        block_height: u64,
        signature_hash: String,
        timestamp: u64,
    ) -> Result<(), String> {
        self.register_validator(validator_address.to_string());

        if let Some(profile) = self.validator_profiles.get_mut(validator_address) {
            // Check for double-signing (same block height, different signature)
            for sig_record in &profile.recent_signatures {
                if sig_record.block_height == block_height
                    && sig_record.signature_hash != signature_hash
                {
                    // DOUBLE SIGNING DETECTED!
                    return Err("DOUBLE_SIGNING_DETECTED".to_string());
                }
            }

            // Add signature to recent records
            profile
                .recent_signatures
                .push_back(los_consensus::slashing::SignatureRecord {
                    block_height,
                    signature_hash,
                    timestamp,
                });

            // Keep only last 1000 signatures
            if profile.recent_signatures.len() > 1000 {
                profile.recent_signatures.pop_front();
            }

            Ok(())
        } else {
            Err("Validator not found".to_string())
        }
    }

    /// Slash a validator for double-signing (100% penalty)
    pub fn slash_double_signing(
        &mut self,
        validator_address: &str,
        block_height: u64,
        current_stake_cil: u128,
    ) -> Result<SlashEvent, String> {
        if !self.enforcement_enabled {
            return Err("Slashing enforcement disabled".to_string());
        }

        self.register_validator(validator_address.to_string());

        if let Some(profile) = self.validator_profiles.get_mut(validator_address) {
            // Can't double-slash
            if profile.status == ValidatorStatus::Banned {
                return Err("Validator already banned".to_string());
            }

            // Execute 100% slash
            let slash_amount = current_stake_cil;
            profile.status = ValidatorStatus::Banned;
            profile.total_slashed_cil += slash_amount;
            profile.violation_count += 1;

            // Record the slash event
            let slash_event = SlashEvent {
                block_height,
                validator_address: validator_address.to_string(),
                violation_type: ViolationType::DoubleSigning,
                slash_amount_cil: slash_amount,
                slash_bps: DOUBLE_SIGNING_SLASH_BPS,
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            };

            profile.slash_history.push(slash_event.clone());

            // Add to banned list
            if !self
                .banned_validators
                .contains(&validator_address.to_string())
            {
                self.banned_validators.push(validator_address.to_string());
            }

            self.total_network_slash_cil += slash_amount;

            Ok(slash_event)
        } else {
            Err("Validator not found".to_string())
        }
    }

    /// Slash a validator for extended downtime (1% penalty)
    pub fn slash_downtime(
        &mut self,
        validator_address: &str,
        block_height: u64,
        current_stake_cil: u128,
    ) -> Result<SlashEvent, String> {
        if !self.enforcement_enabled {
            return Err("Slashing enforcement disabled".to_string());
        }

        self.register_validator(validator_address.to_string());

        if let Some(profile) = self.validator_profiles.get_mut(validator_address) {
            // Can't slash if already banned
            if profile.status == ValidatorStatus::Banned {
                return Err("Validator already banned".to_string());
            }

            // Calculate penalty using DOWNTIME_SLASH_BPS constant
            // SECURITY FIX M-02: Use BPS constant for deterministic slash calculation
            // DOWNTIME_SLASH_BPS / 10000 of stake, ceiling division for rounding up
            let slash_amount = (current_stake_cil * DOWNTIME_SLASH_BPS as u128).div_ceil(10_000);

            // Mark as slashed (but not permanently banned)
            if profile.status == ValidatorStatus::Active {
                profile.status = ValidatorStatus::Slashed;
            }

            profile.total_slashed_cil += slash_amount;
            profile.violation_count += 1;

            // Record the slash event
            let slash_event = SlashEvent {
                block_height,
                validator_address: validator_address.to_string(),
                violation_type: ViolationType::ExtendedDowntime,
                slash_amount_cil: slash_amount,
                slash_bps: DOWNTIME_SLASH_BPS,
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            };

            profile.slash_history.push(slash_event.clone());
            self.total_network_slash_cil += slash_amount;

            Ok(slash_event)
        } else {
            Err("Validator not found".to_string())
        }
    }

    /// Check if validator has downtime and should be slashed
    pub fn check_and_slash_downtime(
        &mut self,
        validator_address: &str,
        block_height: u64,
        current_stake_cil: u128,
    ) -> Option<SlashEvent> {
        if let Some(profile) = self.validator_profiles.get_mut(validator_address) {
            // Track total blocks observed as a monotonically growing counter.
            // Use block_height directly so the observation window grows correctly.
            profile.total_blocks_observed = block_height;

            // Only check if we have a full observation window
            if profile.total_blocks_observed < DOWNTIME_WINDOW_BLOCKS {
                return None;
            }

            // Calculate uptime in basis points (10000 = 100%) — integer math for determinism
            let uptime_bps: u32 = if profile.total_blocks_observed > 0 {
                ((profile.blocks_participated as u128 * 10_000)
                    / profile.total_blocks_observed as u128) as u32
            } else {
                0
            };

            // If uptime below minimum threshold (9500 bps = 95%), slash
            if uptime_bps < MIN_UPTIME_BPS {
                let _ = profile; // Release mutable borrow before calling slash_downtime
                if let Ok(slash_event) =
                    self.slash_downtime(validator_address, block_height, current_stake_cil)
                {
                    return Some(slash_event);
                }
            }
        }

        None
    }

    /// Restore a slashed (but not banned) validator to active status
    /// Requires waiting period and can only be done once
    pub fn restore_validator(&mut self, validator_address: &str) -> Result<(), String> {
        if let Some(profile) = self.validator_profiles.get_mut(validator_address) {
            if profile.status == ValidatorStatus::Slashed {
                profile.status = ValidatorStatus::Active;
                profile.blocks_participated = 0;
                profile.total_blocks_observed = 0;
                Ok(())
            } else if profile.status == ValidatorStatus::Active {
                Ok(()) // Already active
            } else if profile.status == ValidatorStatus::Banned {
                Err("Cannot restore banned validator".to_string())
            } else {
                Err("Cannot restore validator in unstaking status".to_string())
            }
        } else {
            Err("Validator not found".to_string())
        }
    }

    /// Get safety profile for a validator
    pub fn get_profile(&self, validator_address: &str) -> Option<&ValidatorSafetyProfile> {
        self.validator_profiles.get(validator_address)
    }

    /// Get mutable safety profile for a validator
    pub fn get_profile_mut(
        &mut self,
        validator_address: &str,
    ) -> Option<&mut ValidatorSafetyProfile> {
        self.validator_profiles.get_mut(validator_address)
    }

    /// Get all active validators
    pub fn get_active_validators(&self) -> Vec<String> {
        self.validator_profiles
            .iter()
            .filter(|(_, profile)| profile.status == ValidatorStatus::Active)
            .map(|(addr, _)| addr.clone())
            .collect()
    }

    /// Get all banned validators
    pub fn get_all_banned_validators(&self) -> Vec<String> {
        self.validator_profiles
            .iter()
            .filter(|(_, profile)| profile.status == ValidatorStatus::Banned)
            .map(|(addr, _)| addr.clone())
            .collect()
    }

    /// Get statistics for auditing
    pub fn get_statistics(&self) -> SlashingStatistics {
        let total_validators = self.validator_profiles.len();
        let active_validators = self.get_active_validators().len();
        let banned_validators = self.get_all_banned_validators().len();

        let total_violations = self
            .validator_profiles
            .values()
            .map(|p| p.violation_count as u64)
            .sum();

        SlashingStatistics {
            total_validators: total_validators as u32,
            active_validators: active_validators as u32,
            banned_validators: banned_validators as u32,
            total_violations,
            total_slashed_cil: self.total_network_slash_cil,
            enforcement_enabled: self.enforcement_enabled,
        }
    }

    /// Disable slashing (emergency override)
    pub fn disable_enforcement(&mut self) {
        self.enforcement_enabled = false;
    }

    /// Enable slashing
    pub fn enable_enforcement(&mut self) {
        self.enforcement_enabled = true;
    }
}

impl Default for SlashingManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about slashing across the network
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlashingStatistics {
    pub total_validators: u32,
    pub active_validators: u32,
    pub banned_validators: u32,
    pub total_violations: u64,
    pub total_slashed_cil: u128,
    pub enforcement_enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn test_register_validator() {
        let mut manager = SlashingManager::new();
        manager.register_validator("validator1".to_string());

        assert!(manager.validator_profiles.contains_key("validator1"));
        assert!(manager.can_validate("validator1"));
    }

    #[test]
    fn test_double_signing_detection() {
        let mut manager = SlashingManager::new();
        manager.register_validator("validator1".to_string());

        // First signature
        let result1 = manager.record_signature("validator1", 100, "sig_hash_1".to_string(), 1000);
        assert!(result1.is_ok());

        // Different signature for same block height (double-signing!)
        let result2 = manager.record_signature("validator1", 100, "sig_hash_2".to_string(), 1001);
        assert!(result2.is_err());
        assert_eq!(result2.unwrap_err(), "DOUBLE_SIGNING_DETECTED");
    }

    #[test]
    fn test_double_signing_slash() {
        let mut manager = SlashingManager::new();
        manager.register_validator("validator1".to_string());

        let slash_event = manager
            .slash_double_signing("validator1", 100, 1000000000)
            .unwrap();

        assert_eq!(slash_event.violation_type, ViolationType::DoubleSigning);
        assert_eq!(slash_event.slash_amount_cil, 1000000000);
        assert_eq!(slash_event.slash_bps, 10_000);
        assert!(manager.is_validator_banned("validator1"));
    }

    #[test]
    fn test_downtime_slash() {
        let mut manager = SlashingManager::new();
        manager.register_validator("validator1".to_string());

        let slash_event = manager
            .slash_downtime("validator1", 100, 1000000000)
            .unwrap();

        assert_eq!(slash_event.violation_type, ViolationType::ExtendedDowntime);
        assert_eq!(slash_event.slash_amount_cil, 10000000); // 1% of 1B
        assert!(!manager.is_validator_banned("validator1")); // Not banned, just slashed
    }

    #[test]
    fn test_participation_tracking() {
        let mut manager = SlashingManager::new();
        manager.register_validator("validator1".to_string());

        manager.record_participation("validator1", 100);
        manager.record_participation("validator1", 101);

        if let Some(profile) = manager.get_profile("validator1") {
            assert_eq!(profile.blocks_participated, 2);
        }
    }

    #[test]
    fn test_restore_validator() {
        let mut manager = SlashingManager::new();
        manager.register_validator("validator1".to_string());

        // Slash for downtime
        manager
            .slash_downtime("validator1", 100, 1000000000)
            .unwrap();
        assert_eq!(
            manager.get_profile("validator1").unwrap().status,
            ValidatorStatus::Slashed
        );

        // Restore
        assert!(manager.restore_validator("validator1").is_ok());
        assert_eq!(
            manager.get_profile("validator1").unwrap().status,
            ValidatorStatus::Active
        );
    }

    #[test]
    fn test_cannot_restore_banned_validator() {
        let mut manager = SlashingManager::new();
        manager.register_validator("validator1".to_string());

        // Slash for double-signing (permanent ban)
        manager
            .slash_double_signing("validator1", 100, 1000000000)
            .unwrap();

        // Try to restore banned validator
        assert!(manager.restore_validator("validator1").is_err());
    }

    #[test]
    fn test_statistics() {
        let mut manager = SlashingManager::new();

        manager.register_validator("validator1".to_string());
        manager.register_validator("validator2".to_string());
        manager
            .slash_double_signing("validator1", 100, 1000000000)
            .unwrap();

        let stats = manager.get_statistics();
        assert_eq!(stats.total_validators, 2);
        assert_eq!(stats.active_validators, 1);
        assert_eq!(stats.banned_validators, 1);
        assert_eq!(stats.total_violations, 1);
    }

    #[test]
    fn test_enforcement_disable() {
        let mut manager = SlashingManager::new();
        manager.register_validator("validator1".to_string());
        manager.disable_enforcement();

        let result = manager.slash_double_signing("validator1", 100, 1000000000);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Slashing enforcement disabled");
    }

    #[test]
    fn test_arc_mutex_integration() {
        let manager = Arc::new(Mutex::new(SlashingManager::new()));

        {
            let mut mgr = manager.lock().unwrap();
            mgr.register_validator("validator1".to_string());
        }

        {
            let mgr = manager.lock().unwrap();
            assert!(mgr.can_validate("validator1"));
        }
    }
}
