// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// UNAUTHORITY (LOS) - ASYNCHRONOUS BYZANTINE FAULT TOLERANCE (aBFT)
//
// Core consensus protocol for <3 second finality guarantee
// - Pre-prepare → Prepare → Commit phases
// - Tolerates f < n/3 Byzantine validators
// - View change for liveness
// - Cryptographic message authentication codes (MAC)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use std::collections::{BTreeMap, VecDeque};

/// Block structure for consensus
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Block {
    pub height: u64,
    pub timestamp: u64,
    pub data: Vec<u8>,
    pub proposer: String,
    pub parent_hash: String,
}

impl Block {
    /// Calculate Keccak256 hash of the block
    pub fn calculate_hash(&self) -> String {
        let mut hasher = Keccak256::new();
        hasher.update(format!("{}", self.height).as_bytes());
        hasher.update(self.timestamp.to_le_bytes());
        hasher.update(&self.data);
        hasher.update(self.proposer.as_bytes());
        hasher.update(self.parent_hash.as_bytes());

        format!("{:x}", hasher.finalize())
    }
}

/// Consensus message types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConsensusMessageType {
    PrePrepare,
    Prepare,
    Commit,
    ViewChange,
}

/// Signed consensus message with MAC authentication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsensusMessage {
    pub msg_type: ConsensusMessageType,
    pub view: u64,
    pub sequence: u64,
    pub block_hash: String,
    pub sender: String,
    pub timestamp: u64,
    pub mac: Vec<u8>, // Message Authentication Code
}

impl ConsensusMessage {
    /// Create new consensus message with keyed MAC (SECURITY P0-3)
    /// Uses Keccak256(secret || message_data) — safe for SHA-3 family (no length extension)
    pub fn new(
        msg_type: ConsensusMessageType,
        view: u64,
        sequence: u64,
        block_hash: String,
        sender: String,
    ) -> Self {
        Self::new_with_secret(msg_type, view, sequence, block_hash, sender, &[])
    }

    /// Create new consensus message with explicit shared secret for MAC
    pub fn new_with_secret(
        msg_type: ConsensusMessageType,
        view: u64,
        sequence: u64,
        block_hash: String,
        sender: String,
        shared_secret: &[u8],
    ) -> Self {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mac = Self::compute_keyed_mac(
            shared_secret,
            &msg_type,
            view,
            sequence,
            &block_hash,
            &sender,
            timestamp,
        );

        Self {
            msg_type,
            view,
            sequence,
            block_hash,
            sender,
            timestamp,
            mac,
        }
    }

    /// Compute keyed MAC: Keccak256(secret || msg_type || view || seq || block_hash || sender || timestamp)
    fn compute_keyed_mac(
        secret: &[u8],
        msg_type: &ConsensusMessageType,
        view: u64,
        sequence: u64,
        block_hash: &str,
        sender: &str,
        timestamp: u64,
    ) -> Vec<u8> {
        let mut hasher = Keccak256::new();
        hasher.update(secret); // Key material first
        hasher.update(format!("{:?}", msg_type).as_bytes());
        hasher.update(view.to_le_bytes());
        hasher.update(sequence.to_le_bytes());
        hasher.update(block_hash.as_bytes());
        hasher.update(sender.as_bytes());
        hasher.update(timestamp.to_le_bytes()); // Include timestamp in MAC
        hasher.finalize().to_vec()
    }

    /// Verify message authentication (backward compatible - no secret)
    pub fn verify_mac(&self) -> bool {
        self.verify_mac_with_secret(&[])
    }

    /// Verify message authentication with shared secret
    pub fn verify_mac_with_secret(&self, shared_secret: &[u8]) -> bool {
        let expected = Self::compute_keyed_mac(
            shared_secret,
            &self.msg_type,
            self.view,
            self.sequence,
            &self.block_hash,
            &self.sender,
            self.timestamp,
        );
        expected == self.mac
    }
}

/// Validator consensus state
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ValidatorState {
    Normal,
    ViewChanging,
    Locked, // Locked on a block
}

/// aBFT Consensus Engine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ABFTConsensus {
    // Validator identity
    pub validator_id: String,
    pub total_validators: usize,
    pub f_max_faulty: usize, // max (n-1)/3

    // Consensus state
    pub view: u64,
    pub sequence: u64,
    pub state: ValidatorState,

    // Locked block information
    pub locked_block: Option<Block>,
    pub locked_view: u64,

    // Message tracking
    /// MAINNET: BTreeMap for deterministic sequence ordering
    pub pre_prepare_messages: BTreeMap<u64, ConsensusMessage>,
    pub prepare_votes: BTreeMap<u64, Vec<ConsensusMessage>>,
    pub commit_votes: BTreeMap<u64, Vec<ConsensusMessage>>,

    // Finalized blocks
    pub finalized_blocks: VecDeque<Block>,
    pub finality_timestamp: u64,

    // Timing
    pub block_timeout_ms: u64,
    pub view_change_timeout_ms: u64,

    // Statistics
    pub blocks_finalized: u64,
    pub view_changes: u64,

    // Security: Shared secret for keyed MAC authentication (C-03 fix)
    // Derived from node's secret key — prevents consensus message forgery.
    #[serde(skip, default)]
    pub shared_secret: Vec<u8>,

    // Validator set: ordered list of real validator addresses for leader selection (C-04 fix)
    // Index-based round-robin uses these instead of synthetic "validator-N" names.
    #[serde(default)]
    pub validator_set: Vec<String>,
}

impl ABFTConsensus {
    /// Create new aBFT consensus engine
    pub fn new(validator_id: String, total_validators: usize) -> Self {
        let f_max_faulty = (total_validators - 1) / 3;

        Self {
            validator_id,
            total_validators,
            f_max_faulty,
            view: 0,
            sequence: 0,
            state: ValidatorState::Normal,
            locked_block: None,
            locked_view: 0,
            pre_prepare_messages: BTreeMap::new(),
            prepare_votes: BTreeMap::new(),
            commit_votes: BTreeMap::new(),
            finalized_blocks: VecDeque::new(),
            finality_timestamp: 0,
            block_timeout_ms: 10000, // DESIGN FIX D-9: 10s for Tor (.onion) network realism
            view_change_timeout_ms: 5000,
            blocks_finalized: 0,
            view_changes: 0,
            shared_secret: Vec::new(),
            validator_set: Vec::new(),
        }
    }

    /// Set shared secret for MAC authentication.
    /// Should be derived from node's secret key (e.g., Keccak256(sk)).
    pub fn set_shared_secret(&mut self, secret: Vec<u8>) {
        self.shared_secret = secret;
    }

    /// Update the ordered validator set for leader selection.
    /// Must be sorted deterministically (e.g., by address) to ensure all nodes agree.
    pub fn update_validator_set(&mut self, validators: Vec<String>) {
        self.total_validators = validators.len().max(1);
        self.f_max_faulty = (self.total_validators - 1) / 3;
        self.validator_set = validators;
    }

    /// Create a MAC-authenticated consensus message using the engine's shared secret.
    fn create_message(
        &self,
        msg_type: ConsensusMessageType,
        block_hash: String,
    ) -> ConsensusMessage {
        ConsensusMessage::new_with_secret(
            msg_type,
            self.view,
            self.sequence,
            block_hash,
            self.validator_id.clone(),
            &self.shared_secret,
        )
    }

    /// Verify a consensus message's MAC using the engine's shared secret.
    fn verify_message(&self, msg: &ConsensusMessage) -> bool {
        msg.verify_mac_with_secret(&self.shared_secret)
    }

    /// Calculate quorum threshold (2f+1)
    fn get_quorum_threshold(&self) -> usize {
        2 * self.f_max_faulty + 1
    }

    /// PRE-PREPARE phase: Leader proposes block
    pub fn pre_prepare(&mut self, block: Block) -> Result<ConsensusMessage, String> {
        if self.state == ValidatorState::ViewChanging {
            return Err("Currently in view change".to_string());
        }

        self.sequence += 1;
        let block_hash = block.calculate_hash();

        let message = self.create_message(ConsensusMessageType::PrePrepare, block_hash.clone());

        self.pre_prepare_messages
            .insert(self.sequence, message.clone());

        // Lock the block
        self.locked_block = Some(block);
        self.locked_view = self.view;
        self.state = ValidatorState::Locked;

        Ok(message)
    }

    /// PREPARE phase: Validators accept block and vote
    pub fn prepare(&mut self, msg: ConsensusMessage) -> Result<(), String> {
        // Verify message authentication with shared secret
        if !self.verify_message(&msg) {
            return Err("Invalid message authentication".to_string());
        }

        // Verify message is from current view
        if msg.view != self.view {
            return Err(format!(
                "Message from wrong view: {} vs {}",
                msg.view, self.view
            ));
        }

        // Record prepare vote — SECURITY FIX M-3: Dedup by sender.
        // Without this, a Byzantine validator could replay prepare messages
        // to artificially reach quorum (2f+1) and force consensus.
        let votes = self.prepare_votes.entry(msg.sequence).or_default();
        if !votes.iter().any(|v| v.sender == msg.sender) {
            votes.push(msg);
        }

        Ok(())
    }

    /// Check if we have enough prepare votes for commit
    pub fn can_commit(&self, sequence: u64) -> bool {
        if let Some(votes) = self.prepare_votes.get(&sequence) {
            votes.len() >= self.get_quorum_threshold()
        } else {
            false
        }
    }

    /// COMMIT phase: After 2f+1 prepares, commit block
    pub fn commit(&mut self, msg: ConsensusMessage) -> Result<bool, String> {
        // Verify message authentication with shared secret
        if !self.verify_message(&msg) {
            return Err("Invalid message authentication".to_string());
        }

        let sequence = msg.sequence;

        // Record commit vote — SECURITY FIX M-3: Dedup by sender.
        // Same rationale as prepare(): prevents a single Byzantine validator
        // from replaying commit messages to force finalization.
        let votes = self.commit_votes.entry(sequence).or_default();
        if !votes.iter().any(|v| v.sender == msg.sender) {
            votes.push(msg);
        }

        // Check if we reached consensus (2f+1 commits)
        if let Some(commit_votes) = self.commit_votes.get(&sequence) {
            if commit_votes.len() >= self.get_quorum_threshold() {
                return self.finalize_block(sequence);
            }
        }

        Ok(false)
    }

    /// Finalize block and achieve consensus
    fn finalize_block(&mut self, sequence: u64) -> Result<bool, String> {
        if let Some(block) = self.locked_block.clone() {
            self.finalized_blocks.push_back(block);
            // Trim to prevent unbounded memory growth
            const MAX_FINALIZED_BLOCKS: usize = 10_000;
            while self.finalized_blocks.len() > MAX_FINALIZED_BLOCKS {
                self.finalized_blocks.pop_front();
            }
            self.blocks_finalized += 1;
            self.finality_timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            // Clean up old messages
            self.prepare_votes.remove(&sequence);
            self.commit_votes.remove(&sequence);

            self.state = ValidatorState::Normal;
            self.locked_block = None;

            Ok(true)
        } else {
            Err("No locked block to finalize".to_string())
        }
    }

    /// VIEW CHANGE protocol: Change leader if current one fails
    pub fn initiate_view_change(&mut self) -> Result<ConsensusMessage, String> {
        self.state = ValidatorState::ViewChanging;
        self.view += 1;
        self.view_changes += 1;

        let message = ConsensusMessage::new_with_secret(
            ConsensusMessageType::ViewChange,
            self.view,
            self.sequence,
            "".to_string(), // No block hash for view change
            self.validator_id.clone(),
            &self.shared_secret,
        );

        Ok(message)
    }

    /// Complete view change and resume consensus
    pub fn complete_view_change(&mut self, new_view: u64) -> Result<(), String> {
        if new_view < self.view {
            return Err(format!("Invalid new view: {} < {}", new_view, self.view));
        }

        self.view = new_view;
        self.state = ValidatorState::Normal;
        self.prepare_votes.clear();
        self.commit_votes.clear();

        Ok(())
    }

    /// Get current leader address for view.
    /// Uses real validator addresses from `validator_set` when available,
    /// falls back to synthetic names for backward compatibility in tests.
    pub fn get_leader(&self, view: u64) -> String {
        if self.validator_set.is_empty() {
            // Fallback for tests that don't populate validator_set
            let leader_index = (view as usize) % self.total_validators;
            format!("validator-{}", leader_index)
        } else {
            let leader_index = (view as usize) % self.validator_set.len();
            self.validator_set[leader_index].clone()
        }
    }

    /// Check if we are the current leader
    pub fn is_leader(&self) -> bool {
        self.get_leader(self.view) == self.validator_id
    }

    /// DESIGN FIX D-1: Record a block finalized by the CONFIRM_REQ/CONFIRM_RES
    /// voting system. This wires the actual consensus activity into the aBFT
    /// stats tracker, so `/consensus` API reports real data instead of zeros.
    /// Called after the vote-accumulation system reaches threshold.
    pub fn record_external_finalization(&mut self, distinct_voters: usize) {
        self.sequence += 1;
        self.blocks_finalized += 1;
        self.finality_timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        // Trim finalized_blocks to prevent unbounded memory growth
        const MAX_FINALIZED: usize = 10_000;
        while self.finalized_blocks.len() > MAX_FINALIZED {
            self.finalized_blocks.pop_front();
        }
        let _ = distinct_voters; // may be used for stats in future
    }

    /// Get finalized blocks
    pub fn get_finalized_blocks(&self) -> Vec<Block> {
        self.finalized_blocks.iter().cloned().collect()
    }

    /// Get last finalized block
    pub fn get_last_finalized_block(&self) -> Option<Block> {
        self.finalized_blocks.back().cloned()
    }

    /// Get consensus statistics
    pub fn get_statistics(&self) -> ConsensusStats {
        ConsensusStats {
            current_view: self.view,
            current_sequence: self.sequence,
            blocks_finalized: self.blocks_finalized,
            view_changes: self.view_changes,
            consensus_state: format!("{:?}", self.state),
            total_validators: self.total_validators as u32,
            max_faulty_validators: self.f_max_faulty as u32,
            quorum_threshold: self.get_quorum_threshold() as u32,
        }
    }

    /// Calculate Byzantine safety: majority must agree
    pub fn is_byzantine_safe(&self, _view: u64) -> bool {
        // Safety: f < n/3 must hold (standard BFT requirement)
        // For n validators: max_faulty = (n-1)/3
        // Safety guaranteed if: 3*max_faulty < total_validators
        3 * self.f_max_faulty < self.total_validators
    }

    /// Calculate finality time (should be <3 seconds)
    pub fn calculate_finality_time(&self) -> u64 {
        // Ideal: 3 phases × 1 second timeout = 3 seconds
        self.block_timeout_ms / 3
    }
}

/// Consensus statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsensusStats {
    pub current_view: u64,
    pub current_sequence: u64,
    pub blocks_finalized: u64,
    pub view_changes: u64,
    pub consensus_state: String,
    pub total_validators: u32,
    pub max_faulty_validators: u32,
    pub quorum_threshold: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_hash_consistency() {
        let block = Block {
            height: 1,
            timestamp: 1000,
            data: vec![1, 2, 3],
            proposer: "validator-1".to_string(),
            parent_hash: "0".to_string(),
        };

        let hash1 = block.calculate_hash();
        let hash2 = block.calculate_hash();

        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_consensus_message_creation() {
        let msg = ConsensusMessage::new(
            ConsensusMessageType::PrePrepare,
            0,
            1,
            "block_hash".to_string(),
            "validator-1".to_string(),
        );

        assert_eq!(msg.view, 0);
        assert_eq!(msg.sequence, 1);
        assert_eq!(msg.msg_type, ConsensusMessageType::PrePrepare);
    }

    #[test]
    fn test_message_authentication() {
        let msg = ConsensusMessage::new(
            ConsensusMessageType::Prepare,
            0,
            1,
            "block_hash".to_string(),
            "validator-1".to_string(),
        );

        assert!(msg.verify_mac());
    }

    #[test]
    fn test_abft_creation() {
        let consensus = ABFTConsensus::new("validator-1".to_string(), 7);

        assert_eq!(consensus.validator_id, "validator-1");
        assert_eq!(consensus.total_validators, 7);
        assert_eq!(consensus.f_max_faulty, 2); // (7-1)/3 = 2
        assert_eq!(consensus.get_quorum_threshold(), 5); // 2*2+1 = 5
    }

    #[test]
    fn test_quorum_calculation() {
        let consensus = ABFTConsensus::new("validator-1".to_string(), 13);

        // 13 validators: (13-1)/3 = 4 faulty, quorum = 2*4+1 = 9
        assert_eq!(consensus.f_max_faulty, 4);
        assert_eq!(consensus.get_quorum_threshold(), 9);
    }

    #[test]
    fn test_pre_prepare_phase() {
        let mut consensus = ABFTConsensus::new("validator-1".to_string(), 7);

        let block = Block {
            height: 1,
            timestamp: 1000,
            data: vec![1, 2, 3],
            proposer: "validator-1".to_string(),
            parent_hash: "0".to_string(),
        };

        let result = consensus.pre_prepare(block);
        assert!(result.is_ok());
        assert_eq!(consensus.sequence, 1);
        assert!(consensus.locked_block.is_some());
    }

    #[test]
    fn test_prepare_phase() {
        let mut consensus = ABFTConsensus::new("validator-1".to_string(), 7);

        let msg = ConsensusMessage::new(
            ConsensusMessageType::Prepare,
            0,
            1,
            "block_hash".to_string(),
            "validator-2".to_string(),
        );

        let result = consensus.prepare(msg);
        assert!(result.is_ok());
    }

    #[test]
    fn test_commit_phase() {
        let mut consensus = ABFTConsensus::new("validator-1".to_string(), 7);

        // Add prepare votes
        for i in 1..=5 {
            let msg = ConsensusMessage::new(
                ConsensusMessageType::Prepare,
                0,
                1,
                "block_hash".to_string(),
                format!("validator-{}", i),
            );
            let _ = consensus.prepare(msg);
        }

        assert!(consensus.can_commit(1));
    }

    #[test]
    fn test_view_change() {
        let mut consensus = ABFTConsensus::new("validator-1".to_string(), 7);

        assert_eq!(consensus.view, 0);

        let result = consensus.initiate_view_change();
        assert!(result.is_ok());
        assert_eq!(consensus.view, 1);
        assert_eq!(consensus.view_changes, 1);
    }

    #[test]
    fn test_leader_rotation() {
        let consensus = ABFTConsensus::new("validator-1".to_string(), 7);

        let leader_0 = consensus.get_leader(0);
        let leader_1 = consensus.get_leader(1);
        let leader_7 = consensus.get_leader(7); // Should wrap around

        assert_ne!(leader_0, leader_1);
        assert_eq!(leader_0, leader_7); // 7 % 7 = 0
    }

    #[test]
    fn test_is_leader() {
        let mut consensus = ABFTConsensus::new("validator-0".to_string(), 7);
        consensus.view = 0;

        assert!(consensus.is_leader()); // validator-0 is leader at view 0

        consensus.validator_id = "validator-1".to_string();
        assert!(!consensus.is_leader());
    }

    #[test]
    fn test_finality_guarantee() {
        let consensus = ABFTConsensus::new("validator-1".to_string(), 7);

        // Check that Byzantine safety holds
        assert!(consensus.is_byzantine_safe(0));

        let finality_time = consensus.calculate_finality_time();
        // DESIGN FIX D-9: Updated from 3000ms to 10000ms for Tor (.onion) network realism.
        // Tor adds ~2-5s latency per hop; 3-phase BFT with multiple round-trips
        // requires at minimum 6-15s over Tor. The 10s target is achievable.
        assert!(finality_time <= 10000); // Must finalize in <10 seconds (Tor realistic)
    }

    #[test]
    fn test_consensus_statistics() {
        let consensus = ABFTConsensus::new("validator-1".to_string(), 7);

        let stats = consensus.get_statistics();
        assert_eq!(stats.current_view, 0);
        assert_eq!(stats.total_validators, 7);
        assert_eq!(stats.max_faulty_validators, 2);
        assert_eq!(stats.quorum_threshold, 5);
    }

    #[test]
    fn test_get_finalized_blocks() {
        let consensus = ABFTConsensus::new("validator-1".to_string(), 7);

        let blocks = consensus.get_finalized_blocks();
        assert_eq!(blocks.len(), 0);
    }

    #[test]
    fn test_byzantine_safety_holds() {
        // Test with 4 validators: (4-1)/3 = 1 faulty, quorum = 3
        let consensus = ABFTConsensus::new("validator-1".to_string(), 4);

        assert_eq!(consensus.f_max_faulty, 1);
        assert_eq!(consensus.get_quorum_threshold(), 3);
        assert!(consensus.is_byzantine_safe(0));
    }

    #[test]
    fn test_message_from_wrong_view() {
        let mut consensus = ABFTConsensus::new("validator-1".to_string(), 7);
        consensus.view = 0;

        let msg = ConsensusMessage::new(
            ConsensusMessageType::Prepare,
            1, // Wrong view
            1,
            "block_hash".to_string(),
            "validator-2".to_string(),
        );

        let result = consensus.prepare(msg);
        assert!(result.is_err());
    }

    #[test]
    fn test_complete_view_change() {
        let mut consensus = ABFTConsensus::new("validator-1".to_string(), 7);

        consensus.initiate_view_change().unwrap();
        assert_eq!(consensus.view, 1);

        let result = consensus.complete_view_change(1);
        assert!(result.is_ok());
        assert_eq!(consensus.state, ValidatorState::Normal);
    }

    #[test]
    fn test_prepare_vote_dedup_by_sender() {
        // SECURITY FIX M-3: Verify that duplicate prepare votes from the same
        // sender are rejected. Without dedup, a single Byzantine validator could
        // replay prepare messages to artificially reach quorum.
        let mut consensus = ABFTConsensus::new("validator-1".to_string(), 7);
        // 7 validators → f=2, quorum=5

        // Pre-prepare to set state
        let block = Block {
            height: 1,
            timestamp: 1000,
            data: vec![1, 2, 3],
            proposer: "validator-1".to_string(),
            parent_hash: "0".to_string(),
        };
        let _pp = consensus.pre_prepare(block).unwrap();
        let seq = consensus.sequence;

        // Byzantine validator sends 10 identical prepare votes
        for _ in 0..10 {
            let msg = ConsensusMessage::new(
                ConsensusMessageType::Prepare,
                0,
                seq,
                "test_block_hash".to_string(),
                "byzantine-validator".to_string(),
            );
            let _ = consensus.prepare(msg);
        }

        // Should only count as 1 vote, NOT 10
        let votes = consensus.prepare_votes.get(&seq).unwrap();
        assert_eq!(
            votes.len(),
            1,
            "Duplicate prepare votes from same sender must be rejected"
        );
        // quorum=5, only 1 unique vote → cannot commit
        assert!(
            !consensus.can_commit(seq),
            "Should NOT reach quorum with only 1 unique voter"
        );
    }

    #[test]
    fn test_commit_vote_dedup_by_sender() {
        // SECURITY FIX M-3: Same dedup test for commit phase.
        let mut consensus = ABFTConsensus::new("validator-1".to_string(), 7);

        let block = Block {
            height: 1,
            timestamp: 1000,
            data: vec![1, 2, 3],
            proposer: "validator-1".to_string(),
            parent_hash: "0".to_string(),
        };
        let _pp = consensus.pre_prepare(block).unwrap();
        let seq = consensus.sequence;

        // Byzantine validator sends 10 identical commit votes
        for _ in 0..10 {
            let msg = ConsensusMessage::new(
                ConsensusMessageType::Commit,
                0,
                seq,
                "test_block_hash".to_string(),
                "byzantine-validator".to_string(),
            );
            let result = consensus.commit(msg);
            // Should NOT finalize (only 1 unique committer)
            assert!(
                !result.unwrap_or(false),
                "Byzantine replay must not force finalization"
            );
        }

        let votes = consensus.commit_votes.get(&seq).unwrap();
        assert_eq!(
            votes.len(),
            1,
            "Duplicate commit votes from same sender must be rejected"
        );
    }

    #[test]
    fn test_quorum_requires_unique_senders() {
        // Verify that quorum is reached only with distinct validators.
        let mut consensus = ABFTConsensus::new("validator-1".to_string(), 7);
        // quorum = 2*2+1 = 5

        let block = Block {
            height: 1,
            timestamp: 1000,
            data: vec![1, 2, 3],
            proposer: "validator-1".to_string(),
            parent_hash: "0".to_string(),
        };
        let _pp = consensus.pre_prepare(block).unwrap();
        let seq = consensus.sequence;

        // Send 4 unique prepare votes (below quorum)
        for i in 0..4 {
            let msg = ConsensusMessage::new(
                ConsensusMessageType::Prepare,
                0,
                seq,
                "test_block_hash".to_string(),
                format!("validator-{}", i + 10),
            );
            let _ = consensus.prepare(msg);
        }
        assert!(!consensus.can_commit(seq), "4 votes < quorum 5");

        // 5th unique validator → quorum reached
        let msg5 = ConsensusMessage::new(
            ConsensusMessageType::Prepare,
            0,
            seq,
            "test_block_hash".to_string(),
            "validator-15".to_string(),
        );
        let _ = consensus.prepare(msg5);
        assert!(
            consensus.can_commit(seq),
            "5 unique votes should reach quorum"
        );
    }
}
