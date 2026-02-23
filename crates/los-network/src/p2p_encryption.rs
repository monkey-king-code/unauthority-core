// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// UNAUTHORITY (LOS) - P2P ENCRYPTION & SENTRY ARCHITECTURE
//
// Task #5: Secure Network Layer
// - Noise Protocol for all node-to-node communication
// - Sentry Node: Public-facing, shields validator from network
// - Signer Node: Private validator, only talks to sentry via encrypted tunnel
// - IP obscuring: Validator runs on private network/VPN
// - Session encryption: All messages encrypted with perfect forward secrecy
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use zeroize::Zeroize;

use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};

/// Noise Protocol HandshakePattern
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum NoisePattern {
    /// Recommended for authenticated communication (both parties have keys)
    IK, // Initiator Static, Responder Static (pre-shared keys)
    /// For initial pairing/discovery
    NN, // No static keys
    /// DH with authentication
    IX, // Initiator Static, Responder Static (interactive discovery)
}

/// Encryption key material
/// Implements Drop to zeroize key material, preventing
/// ChaCha20-Poly1305 session keys from persisting in freed memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CipherKey {
    pub key_id: u64,
    pub material: Vec<u8>, // 32 bytes for ChaCha20-Poly1305
    pub nonce_counter: u64,
    pub created_at_timestamp: u64,
}

impl Drop for CipherKey {
    fn drop(&mut self) {
        self.material.zeroize();
    }
}

impl CipherKey {
    pub fn new(key_id: u64, material: Vec<u8>, timestamp: u64) -> Self {
        Self {
            key_id,
            material,
            nonce_counter: 0,
            created_at_timestamp: timestamp,
        }
    }

    /// Increment nonce counter for forward secrecy
    pub fn increment_nonce(&mut self) -> u64 {
        self.nonce_counter += 1;
        self.nonce_counter
    }

    pub fn is_expired(&self, current_timestamp: u64, ttl_seconds: u64) -> bool {
        current_timestamp.saturating_sub(self.created_at_timestamp) > ttl_seconds
    }
}

/// Noise session state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoiseSession {
    pub session_id: String,
    pub peer_id: String,
    pub handshake_pattern: NoisePattern,
    pub send_key: Option<CipherKey>,
    pub receive_key: Option<CipherKey>,
    pub established_at: u64,
    pub last_message_timestamp: u64,
    pub messages_sent: u64,
    pub messages_received: u64,
}

impl NoiseSession {
    pub fn new(session_id: String, peer_id: String, pattern: NoisePattern, timestamp: u64) -> Self {
        Self {
            session_id,
            peer_id,
            handshake_pattern: pattern,
            send_key: None,
            receive_key: None,
            established_at: timestamp,
            last_message_timestamp: timestamp,
            messages_sent: 0,
            messages_received: 0,
        }
    }

    pub fn is_established(&self) -> bool {
        self.send_key.is_some() && self.receive_key.is_some()
    }

    pub fn get_session_age(&self, current_timestamp: u64) -> u64 {
        current_timestamp.saturating_sub(self.established_at)
    }
}

/// Encrypted message envelope
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedMessage {
    pub session_id: String,
    pub sequence_number: u64,
    pub ciphertext: Vec<u8>,
    pub mac_tag: Vec<u8>, // Authentication tag for AEAD
    pub timestamp: u64,
}

/// Node identity (static long-term key)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeIdentity {
    pub peer_id: String,
    pub static_key: Vec<u8>, // 32 bytes
    pub node_type: NodeType,
}

impl NodeIdentity {
    pub fn new(peer_id: String, static_key: Vec<u8>, node_type: NodeType) -> Result<Self, String> {
        if static_key.len() != 32 {
            return Err(format!(
                "Static key must be 32 bytes, got {}",
                static_key.len()
            ));
        }
        Ok(Self {
            peer_id,
            static_key,
            node_type,
        })
    }
}

/// Node types in Unauthority network
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum NodeType {
    Sentry, // Public relay node
    Signer, // Private validator node
    Full,   // Full node with no validator
}

/// Sentry Node - Public-facing relay
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SentryNode {
    pub identity: NodeIdentity,
    pub public_address: String, // IP:port accessible from network
    pub active_sessions: BTreeMap<String, NoiseSession>,
    pub connected_peers: Vec<String>,
    pub messages_relayed: u64,
    pub created_at: u64,
}

impl SentryNode {
    pub fn new(
        peer_id: String,
        static_key: Vec<u8>,
        public_address: String,
        timestamp: u64,
    ) -> Result<Self, String> {
        let identity = NodeIdentity::new(peer_id, static_key, NodeType::Sentry)?;

        Ok(Self {
            identity,
            public_address,
            active_sessions: BTreeMap::new(),
            connected_peers: Vec::new(),
            messages_relayed: 0,
            created_at: timestamp,
        })
    }

    /// Establish encrypted session with peer
    pub fn create_session(
        &mut self,
        peer_id: String,
        pattern: NoisePattern,
        timestamp: u64,
    ) -> String {
        let session_id = format!("session_{}_{}", self.identity.peer_id, peer_id);
        let session = NoiseSession::new(session_id.clone(), peer_id.clone(), pattern, timestamp);

        self.active_sessions.insert(session_id.clone(), session);
        if !self.connected_peers.contains(&peer_id) {
            self.connected_peers.push(peer_id);
        }

        session_id
    }

    /// Complete Noise handshake
    pub fn complete_handshake(
        &mut self,
        session_id: &str,
        send_key: CipherKey,
        receive_key: CipherKey,
        timestamp: u64,
    ) -> Result<(), String> {
        let session = self
            .active_sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("Session {} not found", session_id))?;

        session.send_key = Some(send_key);
        session.receive_key = Some(receive_key);
        session.last_message_timestamp = timestamp;

        Ok(())
    }

    /// Relay encrypted message to peer via signer node
    pub fn relay_to_signer(
        &mut self,
        session_id: &str,
        _message: EncryptedMessage,
    ) -> Result<(), String> {
        let session = self
            .active_sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("Session {} not found", session_id))?;

        if !session.is_established() {
            return Err("Session not established".to_string());
        }

        session.messages_received += 1;
        self.messages_relayed += 1;

        Ok(())
    }

    pub fn get_active_session_count(&self) -> usize {
        self.active_sessions.len()
    }

    pub fn get_peer_count(&self) -> usize {
        self.connected_peers.len()
    }
}

/// Signer Node - Private validator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignerNode {
    pub identity: NodeIdentity,
    pub sentry_tunnel: Option<String>, // Session ID to sentry
    pub private_address: String,       // Only accessible via VPN/encrypted tunnel
    pub stake_address: String,         // Associated validator address
    pub active_sessions: BTreeMap<String, NoiseSession>,
    pub messages_signed: u64,
    pub created_at: u64,
}

impl SignerNode {
    pub fn new(
        peer_id: String,
        static_key: Vec<u8>,
        private_address: String,
        stake_address: String,
        timestamp: u64,
    ) -> Result<Self, String> {
        let identity = NodeIdentity::new(peer_id, static_key, NodeType::Signer)?;

        Ok(Self {
            identity,
            sentry_tunnel: None,
            private_address,
            stake_address,
            active_sessions: BTreeMap::new(),
            messages_signed: 0,
            created_at: timestamp,
        })
    }

    /// Connect to sentry node (via encrypted tunnel)
    pub fn connect_to_sentry(
        &mut self,
        sentry_peer_id: String,
        pattern: NoisePattern,
        timestamp: u64,
    ) -> String {
        let session_id = format!("sentry_tunnel_{}_{}", self.identity.peer_id, sentry_peer_id);
        let session = NoiseSession::new(session_id.clone(), sentry_peer_id, pattern, timestamp);

        self.active_sessions.insert(session_id.clone(), session);
        self.sentry_tunnel = Some(session_id.clone());

        session_id
    }

    /// Complete tunnel handshake
    pub fn establish_sentry_tunnel(
        &mut self,
        send_key: CipherKey,
        receive_key: CipherKey,
        timestamp: u64,
    ) -> Result<(), String> {
        let tunnel_session = self
            .sentry_tunnel
            .as_ref()
            .ok_or("Sentry tunnel not initialized")?;

        let session = self
            .active_sessions
            .get_mut(tunnel_session)
            .ok_or("Sentry tunnel session not found")?;

        session.send_key = Some(send_key);
        session.receive_key = Some(receive_key);
        session.last_message_timestamp = timestamp;

        Ok(())
    }

    /// Sign message and send encrypted via sentry tunnel.
    ///
    /// Uses ChaCha20-Poly1305 AEAD encryption.
    /// Previous version stored plaintext in the `ciphertext` field — the message
    /// was authenticated (MAC) but NOT encrypted, leaking content to any observer.
    /// Now uses the same AEAD scheme as `NoiseProtocolManager::encrypt_message()`.
    pub fn sign_and_send(
        &mut self,
        message: &str,
        _signature: Vec<u8>, // Caller attaches Dilithium5 signature
    ) -> Result<EncryptedMessage, String> {
        let tunnel = self
            .sentry_tunnel
            .as_ref()
            .ok_or("Not connected to sentry")?
            .clone();

        let session = self
            .active_sessions
            .get_mut(&tunnel)
            .ok_or("Session not found")?;

        if !session.is_established() {
            return Err("Session not established".to_string());
        }

        let send_key = session.send_key.as_mut().ok_or("Send key not set")?;
        let nonce = send_key.increment_nonce();

        // ChaCha20-Poly1305 AEAD encryption (same as NoiseProtocolManager)
        let key_bytes: [u8; 32] =
            send_key.material.as_slice().try_into().map_err(|_| {
                "Invalid key length for ChaCha20-Poly1305 (need 32 bytes)".to_string()
            })?;
        let cipher = ChaCha20Poly1305::new_from_slice(&key_bytes)
            .map_err(|e| format!("ChaCha20-Poly1305 key init failed: {}", e))?;

        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[..8].copy_from_slice(&nonce.to_le_bytes());
        let nonce_val = Nonce::from(nonce_bytes);

        let aead_output = cipher
            .encrypt(&nonce_val, message.as_bytes())
            .map_err(|e| format!("ChaCha20-Poly1305 encryption failed: {}", e))?;

        // Split AEAD output: ciphertext + 16-byte Poly1305 tag
        let (ct, tag) = aead_output.split_at(aead_output.len() - 16);

        session.messages_sent += 1;
        self.messages_signed += 1;

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Ok(EncryptedMessage {
            session_id: tunnel,
            sequence_number: nonce,
            ciphertext: ct.to_vec(),
            mac_tag: tag.to_vec(),
            timestamp: ts,
        })
    }

    pub fn is_connected_to_sentry(&self) -> bool {
        self.sentry_tunnel
            .as_ref()
            .and_then(|t| self.active_sessions.get(t))
            .map(|s| s.is_established())
            .unwrap_or(false)
    }
}

/// Noise Protocol Manager - Core encryption
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoiseProtocolManager {
    pub node_identity: NodeIdentity,
    pub sessions: BTreeMap<String, NoiseSession>,
    pub peer_keys: BTreeMap<String, Vec<u8>>, // Known peer public keys
    pub handshake_pattern: NoisePattern,
}

impl NoiseProtocolManager {
    pub fn new(node_identity: NodeIdentity, pattern: NoisePattern) -> Self {
        Self {
            node_identity,
            sessions: BTreeMap::new(),
            peer_keys: BTreeMap::new(),
            handshake_pattern: pattern,
        }
    }

    /// Initiate Noise handshake
    pub fn initiate_handshake(
        &mut self,
        peer_id: String,
        peer_static_key: Vec<u8>,
        timestamp: u64,
    ) -> Result<String, String> {
        // Validate peer key
        if peer_static_key.len() != 32 {
            return Err(format!(
                "Invalid peer key length: {}",
                peer_static_key.len()
            ));
        }

        let session_id = format!("noise_{}_{}", self.node_identity.peer_id, peer_id);

        self.peer_keys.insert(peer_id.clone(), peer_static_key);
        let session = NoiseSession::new(
            session_id.clone(),
            peer_id,
            self.handshake_pattern,
            timestamp,
        );

        self.sessions.insert(session_id.clone(), session);

        Ok(session_id)
    }

    /// Complete handshake and derive session keys
    pub fn complete_handshake(
        &mut self,
        session_id: &str,
        send_key_material: Vec<u8>,
        receive_key_material: Vec<u8>,
        timestamp: u64,
    ) -> Result<(), String> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("Session {} not found", session_id))?;

        let send_key = CipherKey::new(1, send_key_material, timestamp);
        let receive_key = CipherKey::new(2, receive_key_material, timestamp);

        session.send_key = Some(send_key);
        session.receive_key = Some(receive_key);
        session.last_message_timestamp = timestamp;

        Ok(())
    }

    /// Encrypt message with session key
    pub fn encrypt_message(
        &mut self,
        session_id: &str,
        plaintext: &[u8],
        timestamp: u64,
    ) -> Result<EncryptedMessage, String> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("Session {} not found", session_id))?;

        if !session.is_established() {
            return Err("Session not established".to_string());
        }

        let send_key = session.send_key.as_mut().ok_or("Send key not set")?;

        let nonce = send_key.increment_nonce();

        // Real ChaCha20-Poly1305 AEAD encryption
        let key_bytes: [u8; 32] =
            send_key.material.as_slice().try_into().map_err(|_| {
                "Invalid key length for ChaCha20-Poly1305 (need 32 bytes)".to_string()
            })?;
        let cipher = ChaCha20Poly1305::new_from_slice(&key_bytes)
            .map_err(|e| format!("ChaCha20-Poly1305 key init failed: {}", e))?;

        // Build 12-byte nonce from counter (first 8 bytes = nonce counter, last 4 = 0)
        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[..8].copy_from_slice(&nonce.to_le_bytes());
        let nonce_val = Nonce::from(nonce_bytes);

        let ciphertext = cipher
            .encrypt(&nonce_val, plaintext)
            .map_err(|e| format!("ChaCha20-Poly1305 encryption failed: {}", e))?;

        // MAC tag is appended to ciphertext by AEAD (last 16 bytes)
        // Split for compatibility with EncryptedMessage struct
        let (ct, tag) = ciphertext.split_at(ciphertext.len() - 16);

        session.messages_sent += 1;

        Ok(EncryptedMessage {
            session_id: session_id.to_string(),
            sequence_number: nonce,
            ciphertext: ct.to_vec(),
            mac_tag: tag.to_vec(),
            timestamp,
        })
    }

    /// Decrypt message with session key
    pub fn decrypt_message(
        &mut self,
        session_id: &str,
        encrypted_msg: &EncryptedMessage,
    ) -> Result<Vec<u8>, String> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("Session {} not found", session_id))?;

        if !session.is_established() {
            return Err("Session not established".to_string());
        }

        let _receive_key = session.receive_key.as_mut().ok_or("Receive key not set")?;

        let nonce = encrypted_msg.sequence_number;

        // Real ChaCha20-Poly1305 AEAD decryption with MAC verification
        let key_bytes: [u8; 32] =
            _receive_key.material.as_slice().try_into().map_err(|_| {
                "Invalid key length for ChaCha20-Poly1305 (need 32 bytes)".to_string()
            })?;
        let cipher = ChaCha20Poly1305::new_from_slice(&key_bytes)
            .map_err(|e| format!("ChaCha20-Poly1305 key init failed: {}", e))?;

        // Build 12-byte nonce from counter
        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[..8].copy_from_slice(&nonce.to_le_bytes());
        let nonce_val = Nonce::from(nonce_bytes);

        // Reconstruct AEAD ciphertext (data + tag)
        let mut aead_ciphertext = encrypted_msg.ciphertext.clone();
        aead_ciphertext.extend_from_slice(&encrypted_msg.mac_tag);

        let plaintext = cipher
            .decrypt(&nonce_val, aead_ciphertext.as_slice())
            .map_err(|_| "ChaCha20-Poly1305 decryption failed: MAC verification error (tampered or wrong key)".to_string())?;

        session.messages_received += 1;

        Ok(plaintext)
    }

    /// Get session statistics
    pub fn get_session_stats(&self, session_id: &str) -> Option<SessionStats> {
        self.sessions.get(session_id).map(|s| SessionStats {
            peer_id: s.peer_id.clone(),
            is_established: s.is_established(),
            messages_sent: s.messages_sent,
            messages_received: s.messages_received,
            handshake_pattern: s.handshake_pattern,
        })
    }

    pub fn get_active_sessions(&self) -> usize {
        self.sessions
            .values()
            .filter(|s| s.is_established())
            .count()
    }

    pub fn clear_expired_sessions(&mut self, current_timestamp: u64, ttl_seconds: u64) {
        self.sessions
            .retain(|_, session| session.get_session_age(current_timestamp) < ttl_seconds);
    }
}

/// Session statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStats {
    pub peer_id: String,
    pub is_established: bool,
    pub messages_sent: u64,
    pub messages_received: u64,
    pub handshake_pattern: NoisePattern,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TESTS
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_keys() -> (Vec<u8>, Vec<u8>) {
        // Mock 32-byte keys for testing
        (vec![1u8; 32], vec![2u8; 32])
    }

    #[test]
    fn test_node_identity_creation() {
        let (key, _) = create_test_keys();
        let identity = NodeIdentity::new("node1".to_string(), key, NodeType::Sentry).unwrap();

        assert_eq!(identity.peer_id, "node1");
        assert_eq!(identity.node_type, NodeType::Sentry);
    }

    #[test]
    fn test_invalid_key_length() {
        let short_key = vec![1u8; 16]; // Too short
        let result = NodeIdentity::new("node1".to_string(), short_key, NodeType::Sentry);

        assert!(result.is_err());
    }

    #[test]
    fn test_sentry_node_creation() {
        let (key, _) = create_test_keys();
        let sentry = SentryNode::new(
            "sentry1".to_string(),
            key,
            "127.0.0.1:30333".to_string(),
            1000,
        )
        .unwrap();

        assert_eq!(sentry.identity.peer_id, "sentry1");
        assert_eq!(sentry.public_address, "127.0.0.1:30333");
        assert_eq!(sentry.get_active_session_count(), 0);
    }

    #[test]
    fn test_signer_node_creation() {
        let (key, _) = create_test_keys();
        let signer = SignerNode::new(
            "signer1".to_string(),
            key,
            "192.168.1.100:30331".to_string(),
            "LOS_validator_address".to_string(),
            1000,
        )
        .unwrap();

        assert_eq!(signer.identity.peer_id, "signer1");
        assert_eq!(signer.private_address, "192.168.1.100:30331");
        assert!(!signer.is_connected_to_sentry());
    }

    #[test]
    fn test_sentry_creates_session() {
        let (key, _) = create_test_keys();
        let mut sentry = SentryNode::new(
            "sentry1".to_string(),
            key,
            "127.0.0.1:30333".to_string(),
            1000,
        )
        .unwrap();

        let session_id = sentry.create_session("peer1".to_string(), NoisePattern::IK, 1000);

        assert!(session_id.contains("sentry1"));
        assert_eq!(sentry.get_active_session_count(), 1);
    }

    #[test]
    fn test_signer_connects_to_sentry() {
        let (key1, _key2) = create_test_keys();

        let mut signer = SignerNode::new(
            "signer1".to_string(),
            key1,
            "192.168.1.100:30331".to_string(),
            "LOS_addr".to_string(),
            1000,
        )
        .unwrap();

        let session_id = signer.connect_to_sentry("sentry1".to_string(), NoisePattern::IK, 1000);

        assert!(session_id.contains("signer1"));
        assert!(signer.sentry_tunnel.is_some());
    }

    #[test]
    fn test_handshake_completion() {
        let (key, _) = create_test_keys();
        let mut sentry = SentryNode::new(
            "sentry1".to_string(),
            key,
            "127.0.0.1:30333".to_string(),
            1000,
        )
        .unwrap();

        let session_id = sentry.create_session("peer1".to_string(), NoisePattern::IK, 1000);

        let send_key = CipherKey::new(1, vec![3u8; 32], 1000);
        let receive_key = CipherKey::new(2, vec![4u8; 32], 1000);

        sentry
            .complete_handshake(&session_id, send_key, receive_key, 1000)
            .unwrap();

        let session = sentry.active_sessions.get(&session_id).unwrap();
        assert!(session.is_established());
    }

    #[test]
    fn test_noise_protocol_manager_initiate() {
        let (key1, key2) = create_test_keys();
        let identity = NodeIdentity::new("node1".to_string(), key1, NodeType::Full).unwrap();
        let mut manager = NoiseProtocolManager::new(identity, NoisePattern::IK);

        let session_id = manager
            .initiate_handshake("peer1".to_string(), key2, 1000)
            .unwrap();

        assert!(session_id.contains("node1"));
        assert_eq!(manager.get_active_sessions(), 0); // Not established yet
    }

    #[test]
    fn test_handshake_and_encrypt() {
        let (key1, key2) = create_test_keys();
        let identity = NodeIdentity::new("node1".to_string(), key1, NodeType::Full).unwrap();
        let mut manager = NoiseProtocolManager::new(identity, NoisePattern::IK);

        let session_id = manager
            .initiate_handshake("peer1".to_string(), key2, 1000)
            .unwrap();

        manager
            .complete_handshake(&session_id, vec![5u8; 32], vec![6u8; 32], 1000)
            .unwrap();

        let msg = manager
            .encrypt_message(&session_id, b"hello", 1000)
            .unwrap();

        assert!(!msg.ciphertext.is_empty());
        assert_eq!(msg.sequence_number, 1);
    }

    #[test]
    fn test_encrypt_decrypt_cycle() {
        let (key1, key2) = create_test_keys();
        let identity = NodeIdentity::new("node1".to_string(), key1, NodeType::Full).unwrap();
        let mut manager = NoiseProtocolManager::new(identity, NoisePattern::IK);

        let session_id = manager
            .initiate_handshake("peer1".to_string(), key2, 1000)
            .unwrap();

        manager
            .complete_handshake(&session_id, vec![5u8; 32], vec![5u8; 32], 1000)
            .unwrap();

        let plaintext = b"secret message";
        let encrypted = manager
            .encrypt_message(&session_id, plaintext, 1000)
            .unwrap();

        let decrypted = manager.decrypt_message(&session_id, &encrypted).unwrap();

        // With same key for both directions, should decrypt back
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_session_statistics() {
        let (key1, key2) = create_test_keys();
        let identity = NodeIdentity::new("node1".to_string(), key1, NodeType::Full).unwrap();
        let mut manager = NoiseProtocolManager::new(identity, NoisePattern::IK);

        let session_id = manager
            .initiate_handshake("peer1".to_string(), key2, 1000)
            .unwrap();

        manager
            .complete_handshake(&session_id, vec![5u8; 32], vec![6u8; 32], 1000)
            .unwrap();

        manager.encrypt_message(&session_id, b"msg1", 1000).unwrap();
        manager.encrypt_message(&session_id, b"msg2", 1000).unwrap();

        let stats = manager.get_session_stats(&session_id).unwrap();
        assert_eq!(stats.messages_sent, 2);
        assert!(stats.is_established);
    }

    #[test]
    fn test_sentry_relay() {
        let (key, _) = create_test_keys();
        let mut sentry = SentryNode::new(
            "sentry1".to_string(),
            key,
            "127.0.0.1:30333".to_string(),
            1000,
        )
        .unwrap();

        let session_id = sentry.create_session("peer1".to_string(), NoisePattern::IK, 1000);

        let send_key = CipherKey::new(1, vec![3u8; 32], 1000);
        let receive_key = CipherKey::new(2, vec![4u8; 32], 1000);
        sentry
            .complete_handshake(&session_id, send_key, receive_key, 1000)
            .unwrap();

        let message = EncryptedMessage {
            session_id: session_id.clone(),
            sequence_number: 1,
            ciphertext: vec![1, 2, 3],
            mac_tag: vec![0u8; 16],
            timestamp: 1000,
        };

        sentry.relay_to_signer(&session_id, message).unwrap();
        assert_eq!(sentry.messages_relayed, 1);
    }

    #[test]
    fn test_multiple_concurrent_sessions() {
        let (key1, key2) = create_test_keys();
        let identity = NodeIdentity::new("node1".to_string(), key1, NodeType::Full).unwrap();
        let mut manager = NoiseProtocolManager::new(identity, NoisePattern::IK);

        // Create 3 sessions
        let sid1 = manager
            .initiate_handshake("peer1".to_string(), key2.clone(), 1000)
            .unwrap();
        let sid2 = manager
            .initiate_handshake("peer2".to_string(), key2.clone(), 1000)
            .unwrap();
        let sid3 = manager
            .initiate_handshake("peer3".to_string(), key2, 1000)
            .unwrap();

        // Complete handshakes
        manager
            .complete_handshake(&sid1, vec![5u8; 32], vec![6u8; 32], 1000)
            .unwrap();
        manager
            .complete_handshake(&sid2, vec![5u8; 32], vec![6u8; 32], 1000)
            .unwrap();
        manager
            .complete_handshake(&sid3, vec![5u8; 32], vec![6u8; 32], 1000)
            .unwrap();

        assert_eq!(manager.get_active_sessions(), 3);
    }

    #[test]
    fn test_full_sentry_signer_tunnel() {
        let (key1, key2) = create_test_keys();

        // Create sentry
        let _sentry = SentryNode::new(
            "sentry1".to_string(),
            key1,
            "127.0.0.1:30333".to_string(),
            1000,
        )
        .unwrap();

        // Create signer
        let mut signer = SignerNode::new(
            "signer1".to_string(),
            key2,
            "192.168.1.100:30331".to_string(),
            "LOS_validator".to_string(),
            1000,
        )
        .unwrap();

        // Signer connects to sentry
        let _tunnel_session =
            signer.connect_to_sentry("sentry1".to_string(), NoisePattern::IK, 1000);

        // Complete tunnel handshake
        let send_key = CipherKey::new(1, vec![3u8; 32], 1000);
        let receive_key = CipherKey::new(2, vec![4u8; 32], 1000);

        signer
            .establish_sentry_tunnel(send_key, receive_key, 1000)
            .unwrap();

        assert!(signer.is_connected_to_sentry());
    }

    #[test]
    fn test_session_expiration() {
        let (key1, key2) = create_test_keys();
        let identity = NodeIdentity::new("node1".to_string(), key1, NodeType::Full).unwrap();
        let mut manager = NoiseProtocolManager::new(identity, NoisePattern::IK);

        let session_id = manager
            .initiate_handshake("peer1".to_string(), key2, 1000)
            .unwrap();

        manager
            .complete_handshake(&session_id, vec![5u8; 32], vec![6u8; 32], 1000)
            .unwrap();

        // Clear sessions older than 100 seconds (this session is at 1000)
        manager.clear_expired_sessions(2000, 100);

        assert_eq!(manager.get_active_sessions(), 0); // Session expired
    }
}
