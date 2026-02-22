// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// UNAUTHORITY (LOS) - TRANSACTION MEMPOOL
//
// Manages pending transactions before inclusion in blocks.
// - Priority queue based on fees and stake
// - Anti-spam protection with duplicate detection
// - Automatic transaction expiration
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use los_core::Block;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};

/// Maximum transactions in mempool
const MAX_MEMPOOL_SIZE: usize = 10_000;

/// Transaction expires after 24 hours
const TX_EXPIRATION_SECS: u64 = 86_400;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MempoolTransaction {
    pub block: Block,
    pub received_at: u64,
    pub priority: u64,
    pub fee: u64,
}

#[derive(Debug, Clone)]
pub struct Mempool {
    /// Transactions indexed by hash
    transactions: HashMap<String, MempoolTransaction>,

    /// Priority queue: priority -> tx_hash
    /// Higher priority = processed first
    priority_queue: BTreeMap<u64, Vec<String>>,

    /// Track transactions by sender address
    by_sender: HashMap<String, Vec<String>>,

    /// Statistics
    pub total_received: u64,
    pub total_accepted: u64,
    pub total_rejected: u64,
    pub total_expired: u64,
}

impl Mempool {
    pub fn new() -> Self {
        Self {
            transactions: HashMap::new(),
            priority_queue: BTreeMap::new(),
            by_sender: HashMap::new(),
            total_received: 0,
            total_accepted: 0,
            total_rejected: 0,
            total_expired: 0,
        }
    }

    /// Add transaction to mempool
    /// Returns Ok(tx_hash) if accepted, Err(reason) if rejected
    pub fn add_transaction(
        &mut self,
        block: Block,
        fee: u64,
        priority: u64,
    ) -> Result<String, String> {
        self.total_received += 1;

        let tx_hash = block.calculate_hash();

        // Check if already in mempool
        if self.transactions.contains_key(&tx_hash) {
            self.total_rejected += 1;
            return Err("Transaction already in mempool".to_string());
        }

        // Check mempool size limit
        if self.transactions.len() >= MAX_MEMPOOL_SIZE {
            // Try to evict lowest priority transaction
            if let Some(lowest_priority) = self.priority_queue.keys().next().cloned() {
                if lowest_priority < priority {
                    self.evict_lowest_priority();
                } else {
                    self.total_rejected += 1;
                    return Err("Mempool full and transaction priority too low".to_string());
                }
            }
        }

        // Validate basic block structure
        if block.account.is_empty() {
            self.total_rejected += 1;
            return Err("Invalid block: empty account".to_string());
        }

        if block.signature.is_empty() {
            self.total_rejected += 1;
            return Err("Invalid block: missing signature".to_string());
        }

        // Create mempool transaction
        let mempool_tx = MempoolTransaction {
            block: block.clone(),
            received_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            priority,
            fee,
        };

        // Add to main storage
        self.transactions.insert(tx_hash.clone(), mempool_tx);

        // Add to priority queue
        self.priority_queue
            .entry(priority)
            .or_default()
            .push(tx_hash.clone());

        // Track by sender
        self.by_sender
            .entry(block.account.clone())
            .or_default()
            .push(tx_hash.clone());

        self.total_accepted += 1;

        Ok(tx_hash)
    }

    /// Get transaction by hash
    pub fn get_transaction(&self, tx_hash: &str) -> Option<&MempoolTransaction> {
        self.transactions.get(tx_hash)
    }

    /// Remove transaction from mempool (after inclusion in block or rejection)
    pub fn remove_transaction(&mut self, tx_hash: &str) -> Option<MempoolTransaction> {
        if let Some(tx) = self.transactions.remove(tx_hash) {
            // Remove from priority queue
            if let Some(hashes) = self.priority_queue.get_mut(&tx.priority) {
                hashes.retain(|h| h != tx_hash);
                if hashes.is_empty() {
                    self.priority_queue.remove(&tx.priority);
                }
            }

            // Remove from sender tracking
            if let Some(hashes) = self.by_sender.get_mut(&tx.block.account) {
                hashes.retain(|h| h != tx_hash);
                if hashes.is_empty() {
                    self.by_sender.remove(&tx.block.account);
                }
            }

            return Some(tx);
        }
        None
    }

    /// Get next N transactions with highest priority
    pub fn get_next_transactions(&self, count: usize) -> Vec<String> {
        let mut result = Vec::new();

        // Iterate priority queue from highest to lowest
        for (_, hashes) in self.priority_queue.iter().rev() {
            for hash in hashes {
                result.push(hash.clone());
                if result.len() >= count {
                    return result;
                }
            }
        }

        result
    }

    /// Get all transactions from a sender
    pub fn get_transactions_by_sender(&self, address: &str) -> Vec<String> {
        self.by_sender.get(address).cloned().unwrap_or_default()
    }

    /// Remove expired transactions (older than 24 hours)
    pub fn remove_expired(&mut self) -> usize {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let expired: Vec<String> = self
            .transactions
            .iter()
            .filter(|(_, tx)| now.saturating_sub(tx.received_at) > TX_EXPIRATION_SECS)
            .map(|(hash, _)| hash.clone())
            .collect();

        let count = expired.len();
        for hash in expired {
            self.remove_transaction(&hash);
        }

        self.total_expired += count as u64;
        count
    }

    /// Evict lowest priority transaction
    fn evict_lowest_priority(&mut self) {
        if let Some((_, hashes)) = self.priority_queue.iter().next() {
            if let Some(hash) = hashes.first() {
                let hash = hash.clone();
                self.remove_transaction(&hash);
            }
        }
    }

    /// Get mempool statistics
    pub fn stats(&self) -> MempoolStats {
        MempoolStats {
            size: self.transactions.len(),
            total_received: self.total_received,
            total_accepted: self.total_accepted,
            total_rejected: self.total_rejected,
            total_expired: self.total_expired,
            unique_senders: self.by_sender.len(),
        }
    }

    /// Check if mempool contains transaction
    pub fn contains(&self, tx_hash: &str) -> bool {
        self.transactions.contains_key(tx_hash)
    }

    /// Get current size
    pub fn len(&self) -> usize {
        self.transactions.len()
    }

    /// Check if mempool is empty
    pub fn is_empty(&self) -> bool {
        self.transactions.is_empty()
    }

    /// Clear all transactions
    pub fn clear(&mut self) {
        self.transactions.clear();
        self.priority_queue.clear();
        self.by_sender.clear();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MempoolStats {
    pub size: usize,
    pub total_received: u64,
    pub total_accepted: u64,
    pub total_rejected: u64,
    pub total_expired: u64,
    pub unique_senders: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use los_core::BlockType;

    fn create_test_block(account: &str, amount: u128) -> Block {
        Block {
            account: account.to_string(),
            previous: "0".to_string(),
            block_type: BlockType::Send,
            amount,
            link: "target_address".to_string(),
            signature: "test_signature".to_string(),
            public_key: "test_pubkey".to_string(),
            work: 0,
            timestamp: 1234567890,
            fee: 0,
        }
    }

    #[test]
    fn test_add_transaction() {
        let mut mempool = Mempool::new();
        let block = create_test_block("sender1", 1000);

        let result = mempool.add_transaction(block, 100, 1000);
        assert!(result.is_ok());
        assert_eq!(mempool.len(), 1);
    }

    #[test]
    fn test_duplicate_rejection() {
        let mut mempool = Mempool::new();
        let block = create_test_block("sender1", 1000);

        mempool.add_transaction(block.clone(), 100, 1000).unwrap();
        let result = mempool.add_transaction(block, 100, 1000);

        assert!(result.is_err());
        assert_eq!(mempool.len(), 1);
    }

    #[test]
    fn test_priority_queue() {
        let mut mempool = Mempool::new();

        let block1 = create_test_block("sender1", 1000);
        let block2 = create_test_block("sender2", 2000);
        let block3 = create_test_block("sender3", 3000);

        mempool.add_transaction(block1, 100, 500).unwrap(); // Low priority
        mempool.add_transaction(block2, 200, 1000).unwrap(); // High priority
        mempool.add_transaction(block3, 150, 750).unwrap(); // Medium priority

        let next = mempool.get_next_transactions(3);
        assert_eq!(next.len(), 3);

        // Highest priority should be first
        let first_tx = mempool.get_transaction(&next[0]).unwrap();
        assert_eq!(first_tx.priority, 1000);
    }

    #[test]
    fn test_remove_transaction() {
        let mut mempool = Mempool::new();
        let block = create_test_block("sender1", 1000);

        let hash = mempool.add_transaction(block, 100, 1000).unwrap();
        assert_eq!(mempool.len(), 1);

        let removed = mempool.remove_transaction(&hash);
        assert!(removed.is_some());
        assert_eq!(mempool.len(), 0);
    }

    #[test]
    fn test_get_by_sender() {
        let mut mempool = Mempool::new();

        let block1 = create_test_block("sender1", 1000);
        let block2 = create_test_block("sender1", 2000);
        let block3 = create_test_block("sender2", 3000);

        mempool.add_transaction(block1, 100, 1000).unwrap();
        mempool.add_transaction(block2, 100, 1000).unwrap();
        mempool.add_transaction(block3, 100, 1000).unwrap();

        let sender1_txs = mempool.get_transactions_by_sender("sender1");
        assert_eq!(sender1_txs.len(), 2);

        let sender2_txs = mempool.get_transactions_by_sender("sender2");
        assert_eq!(sender2_txs.len(), 1);
    }
}
