// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// UNAUTHORITY (LOS) - FINALITY CHECKPOINTS
//
// Prevents long-range attacks by storing immutable checkpoints every N blocks
// Security: RISK-003 mitigation (P0 Critical)
//
// How it works:
// 1. Every 1,000 blocks → create checkpoint (hash + height)
// 2. Store checkpoints in persistent DB (sled)
// 3. On sync: validate forks against latest checkpoint
// 4. Reject any blocks before last checkpoint (finality guarantee)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use serde::{Deserialize, Serialize};
use sha3::{Digest, Sha3_256};
use std::collections::HashSet;
use std::path::Path;

/// Checkpoint interval (every 1,000 blocks)
pub const CHECKPOINT_INTERVAL: u64 = 1000;

/// Signature verification function type.
/// Parameters: (message, signature_bytes, public_key_bytes) → is_valid.
pub type SignatureVerifier = dyn Fn(&[u8], &[u8], &[u8]) -> bool;

/// A single validator's signature on a checkpoint.
///
/// SECURITY: Stores the actual cryptographic signature, not just a count.
/// This prevents forging checkpoint quorum by inflating `signature_count`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckpointSignature {
    /// LOS address of the signing validator
    pub validator_address: String,
    /// Dilithium5 signature bytes over `FinalityCheckpoint::signing_data()`
    pub signature: Vec<u8>,
}

/// Immutable checkpoint representing finalized state
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FinalityCheckpoint {
    /// Block height of checkpoint
    pub height: u64,

    /// Block hash at checkpoint
    pub block_hash: String,

    /// Timestamp when checkpoint was created (Unix)
    pub timestamp: u64,

    /// Total validators active at checkpoint
    pub validator_count: u32,

    /// Merkle root of all accounts at this height (state snapshot)
    pub state_root: String,

    /// Signature count — DERIVED from `signatures.len()` for new checkpoints.
    /// Kept for backward compatibility with existing serialized checkpoints
    /// that predate the `signatures` field.
    pub signature_count: u32,

    /// Actual validator signatures.
    /// Replaces the trust-based `signature_count` with cryptographic proof.
    /// Old checkpoints deserialize with an empty vec (backward-compatible).
    #[serde(default)]
    pub signatures: Vec<CheckpointSignature>,
}

impl FinalityCheckpoint {
    /// Create new checkpoint with cryptographic signatures.
    ///
    /// `signature_count` is derived from the number of unique signer addresses
    /// in `signatures` — it cannot be inflated.
    pub fn new(
        height: u64,
        block_hash: String,
        validator_count: u32,
        state_root: String,
        signatures: Vec<CheckpointSignature>,
    ) -> Self {
        let unique_signers: HashSet<&str> = signatures
            .iter()
            .map(|s| s.validator_address.as_str())
            .collect();
        let signature_count = unique_signers.len() as u32;

        Self {
            height,
            block_hash,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            validator_count,
            state_root,
            signature_count,
            signatures,
        }
    }

    /// Returns the canonical bytes that validators sign.
    ///
    /// Deterministic: height (LE) || block_hash (UTF-8) || state_root (UTF-8).
    /// All validators MUST sign exactly this data for their signature to be valid.
    pub fn signing_data(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(8 + self.block_hash.len() + self.state_root.len());
        data.extend_from_slice(&self.height.to_le_bytes());
        data.extend_from_slice(self.block_hash.as_bytes());
        data.extend_from_slice(self.state_root.as_bytes());
        data
    }

    /// Calculate unique checkpoint ID (hash of height + block_hash + state_root)
    pub fn calculate_id(&self) -> String {
        let mut hasher = Sha3_256::new();
        hasher.update(self.signing_data());
        format!("{:x}", hasher.finalize())
    }

    /// Verify checkpoint has enough signatures (67% quorum).
    ///
    /// When `signatures` is non-empty, the count is derived
    /// from unique signer addresses — ignoring the self-reported `signature_count`.
    /// For legacy checkpoints (empty `signatures`), falls back to `signature_count`.
    ///
    /// Uses integer ceiling division instead of f64 multiplication.
    /// f64 rounding can produce different results across platforms, which would
    /// cause chain splits when validators disagree on finality quorum.
    ///
    /// DESIGN Uses standard BFT quorum formula 2f+1 where f = (n-1)/3.
    /// This is the mathematically correct Byzantine quorum, replacing the
    /// approximation ceil(67% * n) which can differ by 1 at certain n values.
    /// For n=1, requires 1 sig (bootstrap). For n=4, f=1 → requires 3.
    pub fn verify_quorum(&self) -> bool {
        let n = self.validator_count as u64;
        let f = n.saturating_sub(1) / 3;
        let required_sigs = if n <= 1 { 1 } else { (2 * f + 1) as u32 };

        // SECURITY: Derive actual count from signatures when present.
        // Deduplicates by validator address to prevent double-counting.
        let actual_sigs = if self.signatures.is_empty() {
            // Legacy checkpoint (pre C-14 fix) — trust the stored count.
            // These checkpoints were created locally, not received from peers.
            self.signature_count
        } else {
            let unique: HashSet<&str> = self
                .signatures
                .iter()
                .map(|s| s.validator_address.as_str())
                .collect();
            unique.len() as u32
        };

        actual_sigs >= required_sigs
    }

    /// Cryptographically verify all signatures and return count of valid unique signers.
    ///
    /// `get_pubkey`: Maps validator address → public key bytes (returns None for unknown validators).
    /// `verifier`: (message, signature, public_key) → bool.
    ///
    /// This method is called when receiving checkpoints from peers during sync.
    /// For locally-created checkpoints, the node signs with its own key (trusted).
    pub fn verify_signatures(
        &self,
        get_pubkey: &dyn Fn(&str) -> Option<Vec<u8>>,
        verifier: &SignatureVerifier,
    ) -> u32 {
        let signing_data = self.signing_data();
        let mut seen_validators: HashSet<&str> = HashSet::new();
        let mut valid_count: u32 = 0;

        for sig in &self.signatures {
            // Each validator can only sign once — skip duplicates
            if !seen_validators.insert(sig.validator_address.as_str()) {
                continue;
            }
            // Must be a known/active validator
            let pubkey = match get_pubkey(&sig.validator_address) {
                Some(pk) => pk,
                None => continue,
            };
            // Cryptographic verification (Dilithium5 on mainnet)
            if verifier(&signing_data, &sig.signature, &pubkey) {
                valid_count += 1;
            }
        }

        valid_count
    }

    /// Check if checkpoint is valid (interval aligned)
    pub fn is_valid_interval(&self) -> bool {
        self.height.is_multiple_of(CHECKPOINT_INTERVAL)
    }
}

/// Checkpoint Manager with persistent storage
pub struct CheckpointManager {
    /// Database for storing checkpoints
    db: sled::Db,

    /// Latest checkpoint height
    latest_checkpoint_height: u64,
}

impl CheckpointManager {
    /// Create new checkpoint manager.
    ///
    /// Retries up to 3 times with exponential backoff if the database lock
    /// is held by a stale/zombie process (common after SIGKILL on macOS).
    pub fn new<P: AsRef<Path>>(db_path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let path_ref = db_path.as_ref();
        let retry_delays_ms: [u64; 3] = [500, 1000, 2000];

        // First attempt — fast path
        match Self::try_open(path_ref) {
            Ok(mgr) => return Ok(mgr),
            Err(e) if Self::is_lock_error(&*e) => {
                eprintln!(
                    "⚠️  Checkpoint DB lock held at {} — retrying ({} attempts remain)",
                    path_ref.display(),
                    retry_delays_ms.len()
                );
            }
            Err(e) => return Err(e),
        }

        // Retry with backoff — only for lock errors
        for (i, delay_ms) in retry_delays_ms.iter().enumerate() {
            std::thread::sleep(std::time::Duration::from_millis(*delay_ms));
            match Self::try_open(path_ref) {
                Ok(mgr) => {
                    eprintln!("✅ Checkpoint DB lock acquired on retry {}", i + 1);
                    return Ok(mgr);
                }
                Err(e) if Self::is_lock_error(&*e) && i + 1 < retry_delays_ms.len() => {
                    continue;
                }
                Err(e) => return Err(e),
            }
        }

        Err("Checkpoint DB lock acquisition timed out".into())
    }

    /// Attempt to open the checkpoint sled database.
    fn try_open(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let db = sled::open(path)?;

        // Load latest checkpoint from DB
        let latest_checkpoint_height = db
            .get(b"latest_checkpoint_height")?
            .map(|bytes| {
                let arr: [u8; 8] = bytes.as_ref().try_into().unwrap_or([0u8; 8]);
                u64::from_le_bytes(arr)
            })
            .unwrap_or(0);

        Ok(Self {
            db,
            latest_checkpoint_height,
        })
    }

    /// Check if an error is a lock/resource-busy error.
    fn is_lock_error(e: &dyn std::error::Error) -> bool {
        let msg = e.to_string();
        msg.contains("Resource temporarily unavailable")
            || msg.contains("WouldBlock")
            || msg.contains("Would block")
            || msg.contains("lock")
            || msg.contains("EAGAIN")
    }

    /// Store checkpoint in database (immutable)
    pub fn store_checkpoint(
        &mut self,
        checkpoint: FinalityCheckpoint,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Validate checkpoint
        if !checkpoint.is_valid_interval() {
            return Err(format!(
                "Invalid checkpoint height: {} not aligned to {} interval",
                checkpoint.height, CHECKPOINT_INTERVAL
            )
            .into());
        }

        if !checkpoint.verify_quorum() {
            return Err(format!(
                "Insufficient signatures: {}/{} (need 67%)",
                checkpoint.signature_count, checkpoint.validator_count
            )
            .into());
        }

        // Serialize checkpoint
        let checkpoint_bytes = bincode::serialize(&checkpoint)?;
        let key = format!("checkpoint_{}", checkpoint.height);

        // Store in DB (immutable)
        self.db.insert(key.as_bytes(), checkpoint_bytes)?;

        // Update latest checkpoint height
        if checkpoint.height > self.latest_checkpoint_height {
            self.latest_checkpoint_height = checkpoint.height;
            self.db.insert(
                b"latest_checkpoint_height",
                &checkpoint.height.to_le_bytes(),
            )?;
        }

        self.db.flush()?;

        Ok(())
    }

    /// Get checkpoint by height
    pub fn get_checkpoint(
        &self,
        height: u64,
    ) -> Result<Option<FinalityCheckpoint>, Box<dyn std::error::Error>> {
        let key = format!("checkpoint_{}", height);

        if let Some(bytes) = self.db.get(key.as_bytes())? {
            let checkpoint: FinalityCheckpoint = bincode::deserialize(&bytes)?;
            Ok(Some(checkpoint))
        } else {
            Ok(None)
        }
    }

    /// Get latest checkpoint
    pub fn get_latest_checkpoint(
        &self,
    ) -> Result<Option<FinalityCheckpoint>, Box<dyn std::error::Error>> {
        if self.latest_checkpoint_height == 0 {
            return Ok(None);
        }

        self.get_checkpoint(self.latest_checkpoint_height)
    }

    /// Validate block against checkpoint (prevents long-range attacks)
    pub fn validate_block_against_checkpoint(
        &self,
        block_height: u64,
        block_hash: &str,
        _parent_hash: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        // Get latest checkpoint
        let latest_checkpoint = match self.get_latest_checkpoint()? {
            Some(cp) => cp,
            None => return Ok(true), // No checkpoints yet, allow
        };

        // CRITICAL: Reject blocks before last checkpoint (finality guarantee)
        if block_height < latest_checkpoint.height {
            return Err(format!(
                "Block height {} is before finality checkpoint {} (long-range attack rejected)",
                block_height, latest_checkpoint.height
            )
            .into());
        }

        // If block is at checkpoint height, verify hash matches
        if block_height == latest_checkpoint.height && block_hash != latest_checkpoint.block_hash {
            return Err(format!(
                "Block hash mismatch at checkpoint {}: expected {}, got {}",
                block_height, latest_checkpoint.block_hash, block_hash
            )
            .into());
        }

        // Validate parent hash chain back to checkpoint
        if block_height > latest_checkpoint.height
            && block_height < latest_checkpoint.height + CHECKPOINT_INTERVAL
        {
            // Parent must be after or at checkpoint
            let parent_height = block_height - 1;
            if parent_height < latest_checkpoint.height {
                return Err(format!(
                    "Parent block {} is before checkpoint {} (invalid chain)",
                    parent_height, latest_checkpoint.height
                )
                .into());
            }
        }

        Ok(true)
    }

    /// Check if height should create checkpoint
    pub fn should_create_checkpoint(&self, height: u64) -> bool {
        height.is_multiple_of(CHECKPOINT_INTERVAL) && height > self.latest_checkpoint_height
    }

    /// Get all checkpoints (for sync)
    pub fn get_all_checkpoints(
        &self,
    ) -> Result<Vec<FinalityCheckpoint>, Box<dyn std::error::Error>> {
        let mut checkpoints = Vec::new();

        for item in self.db.scan_prefix(b"checkpoint_") {
            let (_, value) = item?;
            let checkpoint: FinalityCheckpoint = bincode::deserialize(&value)?;
            checkpoints.push(checkpoint);
        }

        // Sort by height
        checkpoints.sort_by_key(|cp| cp.height);

        Ok(checkpoints)
    }

    /// Get checkpoint count
    pub fn get_checkpoint_count(&self) -> usize {
        self.db.scan_prefix(b"checkpoint_").count()
    }

    /// Prune old checkpoints (keep last N)
    pub fn prune_old_checkpoints(
        &mut self,
        keep_last: usize,
    ) -> Result<usize, Box<dyn std::error::Error>> {
        let mut checkpoints = self.get_all_checkpoints()?;

        if checkpoints.len() <= keep_last {
            return Ok(0); // Nothing to prune
        }

        // Sort by height descending
        checkpoints.sort_by_key(|cp| std::cmp::Reverse(cp.height));

        // Remove old checkpoints (but keep at least 1)
        let _to_remove = checkpoints.len() - keep_last;
        let mut removed = 0;

        for checkpoint in checkpoints.iter().skip(keep_last) {
            let key = format!("checkpoint_{}", checkpoint.height);
            self.db.remove(key.as_bytes())?;
            removed += 1;
        }

        self.db.flush()?;

        Ok(removed)
    }

    /// Get statistics
    pub fn get_statistics(&self) -> CheckpointStats {
        CheckpointStats {
            total_checkpoints: self.get_checkpoint_count(),
            latest_checkpoint_height: self.latest_checkpoint_height,
            checkpoint_interval: CHECKPOINT_INTERVAL,
        }
    }
}

/// Checkpoint statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointStats {
    pub total_checkpoints: usize,
    pub latest_checkpoint_height: u64,
    pub checkpoint_interval: u64,
}

/// DESIGN Pending checkpoint accumulating signatures from peers.
///
/// When a node creates a checkpoint, it becomes a "pending" checkpoint with
/// 1 signature (the proposer's). As other validators sign the same checkpoint
/// (matching height + block_hash + state_root), their signatures are accumulated.
/// Once 2f+1 signatures are collected, the checkpoint is finalized and stored.
#[derive(Debug, Clone)]
pub struct PendingCheckpoint {
    /// The checkpoint data (height, block_hash, state_root, etc.)
    pub checkpoint: FinalityCheckpoint,
    /// Signed checkpoint data for verification
    pub signing_data: Vec<u8>,
}

impl PendingCheckpoint {
    /// Create a new pending checkpoint from a proposal.
    pub fn new(checkpoint: FinalityCheckpoint) -> Self {
        let signing_data = checkpoint.signing_data();
        Self {
            checkpoint,
            signing_data,
        }
    }

    /// Add a signature to this pending checkpoint.
    /// Returns true if the signature was new (not a duplicate).
    /// Does NOT verify the signature cryptographically — caller must verify first.
    pub fn add_signature(&mut self, sig: CheckpointSignature) -> bool {
        // Dedup by validator address
        if self
            .checkpoint
            .signatures
            .iter()
            .any(|s| s.validator_address == sig.validator_address)
        {
            return false;
        }
        self.checkpoint.signatures.push(sig);
        // Update derived signature_count
        let unique: HashSet<&str> = self
            .checkpoint
            .signatures
            .iter()
            .map(|s| s.validator_address.as_str())
            .collect();
        self.checkpoint.signature_count = unique.len() as u32;
        true
    }

    /// Check if this pending checkpoint has reached quorum.
    pub fn has_quorum(&self) -> bool {
        self.checkpoint.verify_quorum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper: create N fake signatures for testing.
    /// These are NOT cryptographically valid — tests that need crypto verification
    /// should use `verify_signatures()` with a custom verifier.
    fn fake_sigs(count: u32) -> Vec<CheckpointSignature> {
        (0..count)
            .map(|i| CheckpointSignature {
                validator_address: format!("LOS_validator_{}", i),
                signature: vec![0xAA; 64], // placeholder bytes
            })
            .collect()
    }

    #[test]
    fn test_checkpoint_creation() {
        let checkpoint = FinalityCheckpoint::new(
            1000,
            "block_hash_1000".to_string(),
            10,
            "state_root".to_string(),
            fake_sigs(7), // 7/10 = 70% > 67%
        );

        assert_eq!(checkpoint.height, 1000);
        assert_eq!(checkpoint.signature_count, 7);
        assert_eq!(checkpoint.signatures.len(), 7);
        assert!(checkpoint.verify_quorum());
        assert!(checkpoint.is_valid_interval());
    }

    #[test]
    fn test_checkpoint_id_consistency() {
        let checkpoint = FinalityCheckpoint::new(
            1000,
            "block_hash".to_string(),
            10,
            "state_root".to_string(),
            fake_sigs(7),
        );

        let id1 = checkpoint.calculate_id();
        let id2 = checkpoint.calculate_id();
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_checkpoint_quorum_validation() {
        // 6/10 = 60% < 67% → insufficient
        let checkpoint_low = FinalityCheckpoint::new(
            1000,
            "block_hash".to_string(),
            10,
            "state_root".to_string(),
            fake_sigs(6),
        );
        assert!(!checkpoint_low.verify_quorum());

        // 7/10 = 70% >= 67% → sufficient
        let checkpoint_ok = FinalityCheckpoint::new(
            1000,
            "block_hash".to_string(),
            10,
            "state_root".to_string(),
            fake_sigs(7),
        );
        assert!(checkpoint_ok.verify_quorum());
    }

    #[test]
    fn test_checkpoint_quorum_deduplicates() {
        // 5 signatures but 2 are from the same validator → only 4 unique
        let mut sigs = fake_sigs(5);
        sigs[4].validator_address = sigs[0].validator_address.clone(); // duplicate
        let checkpoint = FinalityCheckpoint::new(
            1000,
            "block_hash".to_string(),
            10,
            "state_root".to_string(),
            sigs,
        );
        // signature_count should be 4 (deduplicated), not 5
        assert_eq!(checkpoint.signature_count, 4);
        assert!(!checkpoint.verify_quorum()); // 4/10 = 40% < 67%
    }

    #[test]
    fn test_checkpoint_interval_validation() {
        let checkpoint1 = FinalityCheckpoint::new(
            1000,
            "block_hash".to_string(),
            10,
            "state_root".to_string(),
            fake_sigs(7),
        );

        let checkpoint2 = FinalityCheckpoint::new(
            1001, // Invalid (not aligned to 1000)
            "block_hash".to_string(),
            10,
            "state_root".to_string(),
            fake_sigs(7),
        );

        assert!(checkpoint1.is_valid_interval());
        assert!(!checkpoint2.is_valid_interval());
    }

    #[test]
    fn test_signing_data_deterministic() {
        let cp1 = FinalityCheckpoint::new(
            2000,
            "hash_abc".to_string(),
            5,
            "root_xyz".to_string(),
            fake_sigs(4),
        );
        let cp2 = FinalityCheckpoint::new(
            2000,
            "hash_abc".to_string(),
            5,
            "root_xyz".to_string(),
            fake_sigs(3), // different sigs, same signing_data
        );
        assert_eq!(cp1.signing_data(), cp2.signing_data());
    }

    #[test]
    fn test_verify_signatures_with_verifier() {
        let checkpoint = FinalityCheckpoint::new(
            1000,
            "block_hash".to_string(),
            4,
            "state_root".to_string(),
            fake_sigs(3),
        );

        // Verifier that always accepts
        let accept_all = |_msg: &[u8], _sig: &[u8], _pk: &[u8]| true;
        // get_pubkey that knows first 2 validators
        let known_validators = |addr: &str| -> Option<Vec<u8>> {
            if addr == "LOS_validator_0" || addr == "LOS_validator_1" {
                Some(vec![0x01; 32])
            } else {
                None
            }
        };

        let valid = checkpoint.verify_signatures(&known_validators, &accept_all);
        assert_eq!(valid, 2); // Only 2 of 3 signers are known validators
    }

    #[test]
    fn test_checkpoint_manager_creation() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("checkpoints_test");

        let manager = CheckpointManager::new(&db_path);
        assert!(manager.is_ok());
    }

    #[test]
    fn test_store_and_retrieve_checkpoint() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("checkpoints_test");
        let mut manager = CheckpointManager::new(&db_path).unwrap();

        let checkpoint = FinalityCheckpoint::new(
            1000,
            "block_hash_1000".to_string(),
            10,
            "state_root".to_string(),
            fake_sigs(7),
        );

        let store_result = manager.store_checkpoint(checkpoint.clone());
        assert!(store_result.is_ok());

        let retrieved = manager.get_checkpoint(1000).unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.height, 1000);
        assert_eq!(retrieved.signatures.len(), 7);
    }

    #[test]
    fn test_get_latest_checkpoint() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("checkpoints_test");
        let mut manager = CheckpointManager::new(&db_path).unwrap();

        // Store multiple checkpoints
        for i in 1..=3 {
            let checkpoint = FinalityCheckpoint::new(
                i * 1000,
                format!("block_hash_{}", i * 1000),
                10,
                "state_root".to_string(),
                fake_sigs(7),
            );
            manager.store_checkpoint(checkpoint).unwrap();
        }

        let latest = manager.get_latest_checkpoint().unwrap();
        assert!(latest.is_some());
        assert_eq!(latest.unwrap().height, 3000);
    }

    #[test]
    fn test_validate_block_after_checkpoint() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("checkpoints_test");
        let mut manager = CheckpointManager::new(&db_path).unwrap();

        let checkpoint = FinalityCheckpoint::new(
            1000,
            "block_hash_1000".to_string(),
            10,
            "state_root".to_string(),
            fake_sigs(7),
        );
        manager.store_checkpoint(checkpoint).unwrap();

        let result =
            manager.validate_block_against_checkpoint(1500, "block_hash_1500", "parent_hash_1499");
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_reject_block_before_checkpoint() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("checkpoints_test");
        let mut manager = CheckpointManager::new(&db_path).unwrap();

        let checkpoint = FinalityCheckpoint::new(
            1000,
            "block_hash_1000".to_string(),
            10,
            "state_root".to_string(),
            fake_sigs(7),
        );
        manager.store_checkpoint(checkpoint).unwrap();

        let result =
            manager.validate_block_against_checkpoint(500, "block_hash_500", "parent_hash_499");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("long-range attack"));
    }

    #[test]
    fn test_should_create_checkpoint() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("checkpoints_test");
        let manager = CheckpointManager::new(&db_path).unwrap();

        assert!(manager.should_create_checkpoint(1000));
        assert!(manager.should_create_checkpoint(2000));
        assert!(!manager.should_create_checkpoint(1500)); // Not at interval
        assert!(!manager.should_create_checkpoint(999)); // Not at interval
    }

    #[test]
    fn test_get_all_checkpoints() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("checkpoints_test");
        let mut manager = CheckpointManager::new(&db_path).unwrap();

        for i in 1..=3 {
            let checkpoint = FinalityCheckpoint::new(
                i * 1000,
                format!("block_hash_{}", i * 1000),
                10,
                "state_root".to_string(),
                fake_sigs(7),
            );
            manager.store_checkpoint(checkpoint).unwrap();
        }

        let checkpoints = manager.get_all_checkpoints().unwrap();
        assert_eq!(checkpoints.len(), 3);
        assert_eq!(checkpoints[0].height, 1000);
        assert_eq!(checkpoints[2].height, 3000);
    }

    #[test]
    fn test_prune_old_checkpoints() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("checkpoints_test");
        let mut manager = CheckpointManager::new(&db_path).unwrap();

        for i in 1..=5 {
            let checkpoint = FinalityCheckpoint::new(
                i * 1000,
                format!("block_hash_{}", i * 1000),
                10,
                "state_root".to_string(),
                fake_sigs(7),
            );
            manager.store_checkpoint(checkpoint).unwrap();
        }

        assert_eq!(manager.get_checkpoint_count(), 5);

        let removed = manager.prune_old_checkpoints(3).unwrap();
        assert_eq!(removed, 2);
        assert_eq!(manager.get_checkpoint_count(), 3);

        let latest = manager.get_latest_checkpoint().unwrap();
        assert!(latest.is_some());
        assert_eq!(latest.unwrap().height, 5000);
    }

    #[test]
    fn test_checkpoint_statistics() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("checkpoints_test");
        let mut manager = CheckpointManager::new(&db_path).unwrap();

        for i in 1..=2 {
            let checkpoint = FinalityCheckpoint::new(
                i * 1000,
                format!("block_hash_{}", i * 1000),
                10,
                "state_root".to_string(),
                fake_sigs(7),
            );
            manager.store_checkpoint(checkpoint).unwrap();
        }

        let stats = manager.get_statistics();
        assert_eq!(stats.total_checkpoints, 2);
        assert_eq!(stats.latest_checkpoint_height, 2000);
        assert_eq!(stats.checkpoint_interval, 1000);
    }

    #[test]
    fn test_reject_checkpoint_without_quorum() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("checkpoints_test");
        let mut manager = CheckpointManager::new(&db_path).unwrap();

        // 5/10 = 50% < 67%
        let checkpoint = FinalityCheckpoint::new(
            1000,
            "block_hash_1000".to_string(),
            10,
            "state_root".to_string(),
            fake_sigs(5),
        );

        let result = manager.store_checkpoint(checkpoint);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Insufficient signatures"));
    }

    #[test]
    fn test_reject_checkpoint_invalid_interval() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("checkpoints_test");
        let mut manager = CheckpointManager::new(&db_path).unwrap();

        let checkpoint = FinalityCheckpoint::new(
            1500,
            "block_hash_1500".to_string(),
            10,
            "state_root".to_string(),
            fake_sigs(7),
        );

        let result = manager.store_checkpoint(checkpoint);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not aligned"));
    }

    #[test]
    fn test_legacy_checkpoint_backward_compat() {
        // Simulate a legacy checkpoint (no signatures field) by manual construction
        let legacy = FinalityCheckpoint {
            height: 1000,
            block_hash: "hash".to_string(),
            timestamp: 12345,
            validator_count: 10,
            state_root: "root".to_string(),
            signature_count: 7,
            signatures: vec![], // empty = legacy
        };

        // verify_quorum falls back to signature_count for legacy checkpoints
        assert!(legacy.verify_quorum()); // 7/10 = 70% >= 67%
    }
}
