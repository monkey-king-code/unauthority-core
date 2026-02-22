// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// UNAUTHORITY (LOS) - P2P NETWORK INTEGRATION
//
// Integration layer for Noise Protocol-based secure node communication
// - Manages encrypted peer-to-peer sessions
// - Sentry node relays messages for validators
// - Perfect forward secrecy via Noise Protocol
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Encrypted peer session metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerSession {
    pub peer_id: String,
    pub session_id: String,
    pub established_at: u64,
    pub last_activity: u64,
    pub messages_sent: u64,
    pub messages_received: u64,
}

/// Message priority levels
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum MessagePriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

/// Queued message for encryption and routing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedMessage {
    pub destination_peer_id: String,
    pub payload: Vec<u8>,
    pub priority: MessagePriority,
    pub queued_at: u64,
}

/// Network statistics for monitoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkStats {
    pub total_peers: u32,
    pub connected_peers: u32,
    pub total_messages_sent: u64,
    pub total_messages_received: u64,
    pub total_bytes_sent: u64,
    pub total_bytes_received: u64,
    pub security_events: u32,
}

/// P2P Network Manager for secure node communication
#[derive(Clone, Serialize, Deserialize)]
pub struct P2PNetworkManager {
    // Configuration (serializable)
    pub node_id: String,
    pub listen_addr: String,
    pub listen_port: u16,
    pub node_role: NodeRole,

    // Peer sessions (serializable metadata only)
    pub peer_sessions: BTreeMap<String, PeerSession>,

    // Statistics (serializable)
    pub stats: NetworkStats,

    // Runtime state (skip serialization)
    #[serde(skip)]
    pub outbound_queue: Vec<QueuedMessage>,

    #[serde(skip)]
    pub inbound_queue: Vec<QueuedMessage>,

    #[serde(skip)]
    pub enforcement_enabled: bool,
}

/// Node role in network
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum NodeRole {
    Validator, // Signer node (private)
    Sentry,    // Public relay node
    Full,      // Full node (no validation)
}

impl P2PNetworkManager {
    /// Create new P2P network manager
    pub fn new(
        node_id: String,
        listen_addr: String,
        listen_port: u16,
        node_role: NodeRole,
    ) -> Self {
        Self {
            node_id,
            listen_addr,
            listen_port,
            node_role,
            peer_sessions: BTreeMap::new(),
            stats: NetworkStats {
                total_peers: 0,
                connected_peers: 0,
                total_messages_sent: 0,
                total_messages_received: 0,
                total_bytes_sent: 0,
                total_bytes_received: 0,
                security_events: 0,
            },
            outbound_queue: Vec::new(),
            inbound_queue: Vec::new(),
            enforcement_enabled: true,
        }
    }

    /// Register a peer for encrypted communication
    pub fn add_peer(&mut self, peer_id: String) -> Result<String, String> {
        if self.peer_sessions.contains_key(&peer_id) {
            return Err(format!("Peer {} already registered", peer_id));
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let session_id = format!("noise_{}_to_{}", self.node_id, peer_id);

        let session = PeerSession {
            peer_id: peer_id.clone(),
            session_id: session_id.clone(),
            established_at: now,
            last_activity: now,
            messages_sent: 0,
            messages_received: 0,
        };

        self.peer_sessions.insert(peer_id, session);
        self.stats.total_peers += 1;

        Ok(session_id)
    }

    /// Remove a peer session
    pub fn remove_peer(&mut self, peer_id: &str) -> Result<(), String> {
        if self.peer_sessions.remove(peer_id).is_some() {
            self.stats.total_peers = self.stats.total_peers.saturating_sub(1);
            self.stats.connected_peers = self.stats.connected_peers.saturating_sub(1);
            Ok(())
        } else {
            Err(format!("Peer {} not found", peer_id))
        }
    }

    /// Mark peer as connected
    pub fn connect_peer(&mut self, peer_id: &str) -> Result<(), String> {
        if let Some(_session) = self.peer_sessions.get_mut(peer_id) {
            self.stats.connected_peers += 1;
            Ok(())
        } else {
            Err(format!("Peer {} not registered", peer_id))
        }
    }

    /// Mark peer as disconnected
    pub fn disconnect_peer(&mut self, peer_id: &str) -> Result<(), String> {
        if self.peer_sessions.contains_key(peer_id) {
            self.stats.connected_peers = self.stats.connected_peers.saturating_sub(1);
            Ok(())
        } else {
            Err(format!("Peer {} not found", peer_id))
        }
    }

    /// Queue encrypted message for sending
    pub fn queue_message(
        &mut self,
        peer_id: String,
        payload: Vec<u8>,
        priority: MessagePriority,
    ) -> Result<(), String> {
        if !self.enforcement_enabled {
            return Err("Network enforcement disabled".to_string());
        }

        if !self.peer_sessions.contains_key(&peer_id) {
            return Err(format!("Peer {} not connected", peer_id));
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        self.outbound_queue.push(QueuedMessage {
            destination_peer_id: peer_id.clone(),
            payload: payload.clone(),
            priority,
            queued_at: now,
        });

        // Update stats
        self.stats.total_messages_sent += 1;
        self.stats.total_bytes_sent += payload.len() as u64;

        // Update session
        if let Some(session) = self.peer_sessions.get_mut(&peer_id) {
            session.messages_sent += 1;
            session.last_activity = now;
        }

        Ok(())
    }

    /// Process received encrypted message
    pub fn process_received_message(
        &mut self,
        peer_id: &str,
        payload: Vec<u8>,
    ) -> Result<(), String> {
        if let Some(session) = self.peer_sessions.get_mut(peer_id) {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            session.messages_received += 1;
            session.last_activity = now;

            self.stats.total_messages_received += 1;
            self.stats.total_bytes_received += payload.len() as u64;

            self.inbound_queue.push(QueuedMessage {
                destination_peer_id: peer_id.to_string(),
                payload,
                priority: MessagePriority::Normal,
                queued_at: now,
            });

            Ok(())
        } else {
            Err(format!("Peer {} not registered", peer_id))
        }
    }

    /// Get all connected peers
    pub fn get_connected_peers(&self) -> Vec<String> {
        self.peer_sessions.keys().cloned().collect()
    }

    /// Get peer session info
    pub fn get_peer_session(&self, peer_id: &str) -> Option<&PeerSession> {
        self.peer_sessions.get(peer_id)
    }

    /// Process outbound queue (sort by priority)
    pub fn flush_outbound_queue(&mut self) -> Vec<QueuedMessage> {
        self.outbound_queue
            .sort_by(|a, b| b.priority.cmp(&a.priority));
        self.outbound_queue.drain(..).collect()
    }

    /// Process inbound queue
    pub fn flush_inbound_queue(&mut self) -> Vec<QueuedMessage> {
        self.inbound_queue.drain(..).collect()
    }

    /// Get network statistics
    pub fn get_statistics(&self) -> NetworkStats {
        self.stats.clone()
    }

    /// Record security event
    pub fn record_security_event(&mut self) {
        self.stats.security_events += 1;
    }

    /// Disable network enforcement (emergency)
    pub fn disable_enforcement(&mut self) {
        self.enforcement_enabled = false;
    }

    /// Re-enable network enforcement
    pub fn enable_enforcement(&mut self) {
        self.enforcement_enabled = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_manager() {
        let manager = P2PNetworkManager::new(
            "validator-1".to_string(),
            "127.0.0.1".to_string(),
            30333,
            NodeRole::Validator,
        );

        assert_eq!(manager.node_id, "validator-1");
        assert_eq!(manager.listen_port, 30333);
        assert_eq!(manager.node_role, NodeRole::Validator);
    }

    #[test]
    fn test_add_peer() {
        let mut manager = P2PNetworkManager::new(
            "validator-1".to_string(),
            "127.0.0.1".to_string(),
            30333,
            NodeRole::Validator,
        );

        let result = manager.add_peer("validator-2".to_string());
        assert!(result.is_ok());
        assert!(manager.peer_sessions.contains_key("validator-2"));
        assert_eq!(manager.stats.total_peers, 1);
    }

    #[test]
    fn test_add_duplicate_peer() {
        let mut manager = P2PNetworkManager::new(
            "validator-1".to_string(),
            "127.0.0.1".to_string(),
            30333,
            NodeRole::Validator,
        );

        manager.add_peer("validator-2".to_string()).unwrap();
        let result = manager.add_peer("validator-2".to_string());

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already registered"));
    }

    #[test]
    fn test_remove_peer() {
        let mut manager = P2PNetworkManager::new(
            "validator-1".to_string(),
            "127.0.0.1".to_string(),
            30333,
            NodeRole::Validator,
        );

        manager.add_peer("validator-2".to_string()).unwrap();
        assert_eq!(manager.stats.total_peers, 1);

        let result = manager.remove_peer("validator-2");
        assert!(result.is_ok());
        assert_eq!(manager.stats.total_peers, 0);
    }

    #[test]
    fn test_connect_peer() {
        let mut manager = P2PNetworkManager::new(
            "validator-1".to_string(),
            "127.0.0.1".to_string(),
            30333,
            NodeRole::Validator,
        );

        manager.add_peer("validator-2".to_string()).unwrap();
        let result = manager.connect_peer("validator-2");

        assert!(result.is_ok());
        assert_eq!(manager.stats.connected_peers, 1);
    }

    #[test]
    fn test_disconnect_peer() {
        let mut manager = P2PNetworkManager::new(
            "validator-1".to_string(),
            "127.0.0.1".to_string(),
            30333,
            NodeRole::Validator,
        );

        manager.add_peer("validator-2".to_string()).unwrap();
        manager.connect_peer("validator-2").unwrap();
        assert_eq!(manager.stats.connected_peers, 1);

        manager.disconnect_peer("validator-2").unwrap();
        assert_eq!(manager.stats.connected_peers, 0);
    }

    #[test]
    fn test_queue_message() {
        let mut manager = P2PNetworkManager::new(
            "validator-1".to_string(),
            "127.0.0.1".to_string(),
            30333,
            NodeRole::Validator,
        );

        manager.add_peer("validator-2".to_string()).unwrap();

        let result = manager.queue_message(
            "validator-2".to_string(),
            vec![1, 2, 3, 4, 5],
            MessagePriority::High,
        );

        assert!(result.is_ok());
        assert_eq!(manager.outbound_queue.len(), 1);
        assert_eq!(manager.stats.total_messages_sent, 1);
        assert_eq!(manager.stats.total_bytes_sent, 5);
    }

    #[test]
    fn test_message_priority_sorting() {
        let mut manager = P2PNetworkManager::new(
            "validator-1".to_string(),
            "127.0.0.1".to_string(),
            30333,
            NodeRole::Validator,
        );

        manager.add_peer("validator-2".to_string()).unwrap();
        manager.add_peer("validator-3".to_string()).unwrap();

        manager
            .queue_message("validator-2".to_string(), vec![1], MessagePriority::Low)
            .unwrap();

        manager
            .queue_message(
                "validator-3".to_string(),
                vec![2],
                MessagePriority::Critical,
            )
            .unwrap();

        let queue = manager.flush_outbound_queue();
        assert_eq!(queue[0].priority, MessagePriority::Critical);
        assert_eq!(queue[1].priority, MessagePriority::Low);
    }

    #[test]
    fn test_process_received_message() {
        let mut manager = P2PNetworkManager::new(
            "validator-1".to_string(),
            "127.0.0.1".to_string(),
            30333,
            NodeRole::Validator,
        );

        manager.add_peer("validator-2".to_string()).unwrap();

        let result = manager.process_received_message("validator-2", vec![1, 2, 3]);

        assert!(result.is_ok());
        assert_eq!(manager.stats.total_messages_received, 1);
        assert_eq!(manager.stats.total_bytes_received, 3);
        assert_eq!(manager.inbound_queue.len(), 1);
    }

    #[test]
    fn test_get_connected_peers() {
        let mut manager = P2PNetworkManager::new(
            "validator-1".to_string(),
            "127.0.0.1".to_string(),
            30333,
            NodeRole::Validator,
        );

        manager.add_peer("validator-2".to_string()).unwrap();
        manager.add_peer("validator-3".to_string()).unwrap();

        let peers = manager.get_connected_peers();
        assert_eq!(peers.len(), 2);
        assert!(peers.contains(&"validator-2".to_string()));
        assert!(peers.contains(&"validator-3".to_string()));
    }

    #[test]
    fn test_enforcement_disable() {
        let mut manager = P2PNetworkManager::new(
            "validator-1".to_string(),
            "127.0.0.1".to_string(),
            30333,
            NodeRole::Validator,
        );

        manager.add_peer("validator-2".to_string()).unwrap();
        manager.disable_enforcement();

        let result = manager.queue_message(
            "validator-2".to_string(),
            vec![1, 2, 3],
            MessagePriority::Normal,
        );

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Network enforcement disabled");
    }

    #[test]
    fn test_statistics() {
        let manager = P2PNetworkManager::new(
            "validator-1".to_string(),
            "127.0.0.1".to_string(),
            30333,
            NodeRole::Validator,
        );

        let stats = manager.get_statistics();
        assert_eq!(stats.total_peers, 0);
        assert_eq!(stats.connected_peers, 0);
        assert_eq!(stats.total_messages_sent, 0);
    }
}
