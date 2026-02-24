// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// UNAUTHORITY (LOS) - DATABASE MODULE
//
// sled embedded database for persistent blockchain state.
// Provides ACID-compliant atomic operations for blocks, accounts, and metadata.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use los_core::{AccountState, Block, Ledger};
use sled::{Db, Tree};
use std::path::Path;
use std::sync::Arc;

#[cfg(unix)]
use std::os::unix::io::AsRawFd;

const DB_PATH: &str = "los_database";
const TREE_BLOCKS: &str = "blocks";
const TREE_ACCOUNTS: &str = "accounts";
const TREE_META: &str = "metadata";
const TREE_FAUCET_COOLDOWNS: &str = "faucet_cooldowns";
const TREE_PEERS: &str = "known_peers";
const TREE_CONTRACTS: &str = "contracts"; // Smart contract VM state

/// Database wrapper with ACID guarantees
pub struct LosDatabase {
    db: Arc<Db>,
}

impl LosDatabase {
    /// Pre-check: try a NON-BLOCKING flock on the sled db file.
    ///
    /// sled internally uses `flock(LOCK_EX)` (blocking) which can hang
    /// forever if a UE zombie holds the lock. This function probes with
    /// `LOCK_NB` to detect that scenario BEFORE calling `sled::open()`.
    ///
    /// Returns:
    /// - Ok(true)  — lock is available (or db file doesn't exist yet)
    /// - Ok(false) — lock is held by another process
    /// - Err(_)    — some other I/O error
    #[cfg(unix)]
    fn is_db_lock_available(path: &Path) -> Result<bool, String> {
        let db_file = path.join("db");
        if !db_file.exists() {
            return Ok(true); // New database — no lock contention possible
        }

        let file = std::fs::OpenOptions::new()
            .read(true)
            .open(&db_file)
            .map_err(|e| format!("Cannot open db file for lock check: {}", e))?;

        let fd = file.as_raw_fd();
        // LOCK_EX | LOCK_NB: exclusive, non-blocking
        let ret = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
        if ret == 0 {
            // We got the lock — release it immediately (sled will re-acquire)
            unsafe { libc::flock(fd, libc::LOCK_UN) };
            Ok(true)
        } else {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::WouldBlock {
                Ok(false) // Lock held by another process
            } else {
                Err(format!("flock probe failed: {}", err))
            }
        }
    }

    #[cfg(not(unix))]
    fn is_db_lock_available(_path: &Path) -> Result<bool, String> {
        Ok(true) // Non-Unix: skip check, rely on sled's own error
    }

    /// Open or create database.
    ///
    /// **Anti-zombie design:**
    /// 1. Pre-checks the sled flock with NON-BLOCKING flock(LOCK_NB).
    ///    If another process (including UE zombies) holds the lock,
    ///    returns an error immediately instead of blocking in kernel I/O
    ///    (which would turn THIS process into ANOTHER UE zombie).
    /// 2. Retries up to 3 times with exponential backoff (500ms, 1s, 2s)
    ///    for transient lock-release delays (e.g. after normal SIGTERM).
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let path_ref = path.as_ref();
        let retry_delays_ms: [u64; 3] = [500, 1000, 2000];

        // ── Anti-zombie pre-check ───────────────────────────────────────
        // Probe the sled flock with LOCK_NB BEFORE calling sled::open().
        // If a UE zombie holds the lock, we fail fast instead of blocking
        // in kernel I/O (which would make US another UE zombie).
        match Self::is_db_lock_available(path_ref) {
            Ok(true) => { /* Lock available — proceed with sled::open */ }
            Ok(false) => {
                eprintln!(
                    "⚠️  Database flock held by another process at {} — \
                     will retry with backoff (NOT blocking in kernel I/O)",
                    path_ref.display()
                );
                // Don't call sled::open yet — go straight to retry loop
                // which uses the non-blocking probe on each iteration
                for (i, delay_ms) in retry_delays_ms.iter().enumerate() {
                    std::thread::sleep(std::time::Duration::from_millis(*delay_ms));
                    eprintln!(
                        "🔄 Lock probe retry {}/{} after {}ms...",
                        i + 1,
                        retry_delays_ms.len(),
                        delay_ms
                    );
                    match Self::is_db_lock_available(path_ref) {
                        Ok(true) => break, // Lock released — fall through to sled::open
                        Ok(false) if i + 1 == retry_delays_ms.len() => {
                            return Err(format!(
                                "Database lock permanently held at {} — another los-node \
                                 (possibly a UE zombie) still holds the flock. \
                                 Fix: remove the database directory and resync from peers.",
                                path_ref.display()
                            ));
                        }
                        Ok(false) => continue,
                        Err(e) => {
                            eprintln!("⚠️ flock probe error: {}", e);
                            break; // Fall through to sled::open and let it handle
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!(
                    "⚠️ flock probe error: {} — falling through to sled::open",
                    e
                );
            }
        }

        // First attempt — fast path (no delay)
        match sled::open(path_ref) {
            Ok(db) => return Ok(LosDatabase { db: Arc::new(db) }),
            Err(e) if Self::is_lock_error(&e) => {
                eprintln!(
                    "⚠️  Database lock held at {} — retrying ({} attempts remain)",
                    path_ref.display(),
                    retry_delays_ms.len()
                );
            }
            Err(e) => return Err(format!("Failed to open database: {}", e)),
        }

        // Retry with exponential backoff — only for lock errors
        for (i, delay_ms) in retry_delays_ms.iter().enumerate() {
            std::thread::sleep(std::time::Duration::from_millis(*delay_ms));
            eprintln!(
                "🔄 Database lock retry {}/{} after {}ms...",
                i + 1,
                retry_delays_ms.len(),
                delay_ms
            );

            match sled::open(path_ref) {
                Ok(db) => {
                    eprintln!("✅ Database lock acquired on retry {}", i + 1);
                    return Ok(LosDatabase { db: Arc::new(db) });
                }
                Err(e) if Self::is_lock_error(&e) => {
                    if i + 1 == retry_delays_ms.len() {
                        return Err(format!(
                            "Failed to open database after {} retries: {} \
                             (another los-node process may still be running)",
                            retry_delays_ms.len(),
                            e
                        ));
                    }
                }
                Err(e) => return Err(format!("Failed to open database: {}", e)),
            }
        }

        unreachable!("retry loop should return in all branches")
    }

    /// Check if a sled error is a lock/resource-busy error.
    fn is_lock_error(e: &sled::Error) -> bool {
        let msg = e.to_string();
        // sled wraps IO errors: "IO error: ... Resource temporarily unavailable"
        // or "IO error: ... Would block" (varies by OS)
        msg.contains("Resource temporarily unavailable")
            || msg.contains("WouldBlock")
            || msg.contains("Would block")
            || msg.contains("lock")
            || msg.contains("EAGAIN")
            || msg.contains("EWOULDBLOCK")
    }

    /// Open with default path
    pub fn open_default() -> Result<Self, String> {
        Self::open(DB_PATH)
    }

    /// Flush all pending writes to disk.
    ///
    /// Called during graceful shutdown (SIGTERM handler) BEFORE std::process::exit().
    /// This ensures dirty pages are written without relying on sled::Drop,
    /// which can hang in kernel I/O and cause macOS UE (Uninterruptible Exit) zombies.
    pub fn flush(&self) -> Result<(), String> {
        self.db
            .flush()
            .map_err(|e| format!("Failed to flush database: {}", e))?;
        Ok(())
    }

    /// Get blocks tree
    fn blocks_tree(&self) -> Result<Tree, String> {
        self.db
            .open_tree(TREE_BLOCKS)
            .map_err(|e| format!("Failed to open blocks tree: {}", e))
    }

    /// Get accounts tree
    fn accounts_tree(&self) -> Result<Tree, String> {
        self.db
            .open_tree(TREE_ACCOUNTS)
            .map_err(|e| format!("Failed to open accounts tree: {}", e))
    }

    /// Get metadata tree
    fn meta_tree(&self) -> Result<Tree, String> {
        self.db
            .open_tree(TREE_META)
            .map_err(|e| format!("Failed to open metadata tree: {}", e))
    }

    /// Save complete ledger state (TRULY ATOMIC — cross-tree transaction)
    /// Uses sled's transaction API so blocks, accounts, and metadata
    /// are committed as a single atomic unit. A crash mid-save won't
    /// leave partial state.
    pub fn save_ledger(&self, ledger: &Ledger) -> Result<(), String> {
        use sled::Transactional;

        let blocks_tree = self.blocks_tree()?;
        let accounts_tree = self.accounts_tree()?;
        let meta_tree = self.meta_tree()?;

        // Pre-serialize all data outside the transaction (transactions should be fast)
        let mut block_entries: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(ledger.blocks.len());
        for (hash, block) in &ledger.blocks {
            let block_json = serde_json::to_vec(block)
                .map_err(|e| format!("Failed to serialize block: {}", e))?;
            block_entries.push((hash.as_bytes().to_vec(), block_json));
        }

        let mut account_entries: Vec<(Vec<u8>, Vec<u8>)> =
            Vec::with_capacity(ledger.accounts.len());
        for (addr, state) in &ledger.accounts {
            let state_json = serde_json::to_vec(state)
                .map_err(|e| format!("Failed to serialize account: {}", e))?;
            account_entries.push((addr.as_bytes().to_vec(), state_json));
        }

        let distribution_json = serde_json::to_vec(&ledger.distribution)
            .map_err(|e| format!("Failed to serialize distribution: {}", e))?;

        // Atomic cross-tree transaction: all-or-nothing commit
        (&blocks_tree, &accounts_tree, &meta_tree)
            .transaction(|(tx_blocks, tx_accounts, tx_meta)| {
                for (key, value) in &block_entries {
                    tx_blocks.insert(key.as_slice(), value.as_slice())?;
                }
                for (key, value) in &account_entries {
                    tx_accounts.insert(key.as_slice(), value.as_slice())?;
                }
                tx_meta.insert(b"distribution".as_ref(), distribution_json.as_slice())?;
                // Persist accumulated_fees_cil (lives on Ledger, not DistributionState)
                tx_meta.insert(
                    b"accumulated_fees_cil".as_ref(),
                    &ledger.accumulated_fees_cil.to_le_bytes() as &[u8],
                )?;
                // Persist total_slashed_cil — without this, supply audit breaks after restart
                // because the invariant includes slashed CIL in the accounting equation.
                tx_meta.insert(
                    b"total_slashed_cil".as_ref(),
                    &ledger.total_slashed_cil.to_le_bytes() as &[u8],
                )?;
                Ok(())
            })
            .map_err(|e: sled::transaction::TransactionError<()>| {
                format!("Atomic save failed: {:?}", e)
            })?;

        // Flush to disk (durability guarantee)
        self.db
            .flush()
            .map_err(|e| format!("Failed to flush to disk: {}", e))?;

        Ok(())
    }

    /// Load complete ledger state
    pub fn load_ledger(&self) -> Result<Ledger, String> {
        let blocks_tree = self.blocks_tree()?;
        let accounts_tree = self.accounts_tree()?;
        let meta_tree = self.meta_tree()?;

        let mut ledger = Ledger::new();

        // 1. Load all blocks
        for item in blocks_tree.iter() {
            let (key, value) = item.map_err(|e| format!("Failed to read block: {}", e))?;

            let hash = String::from_utf8(key.to_vec())
                .map_err(|e| format!("Invalid block hash: {}", e))?;

            let block: Block = serde_json::from_slice(&value)
                .map_err(|e| format!("Failed to deserialize block: {}", e))?;

            ledger.blocks.insert(hash, block);
        }

        // 2. Load all accounts
        for item in accounts_tree.iter() {
            let (key, value) = item.map_err(|e| format!("Failed to read account: {}", e))?;

            let addr = String::from_utf8(key.to_vec())
                .map_err(|e| format!("Invalid account address: {}", e))?;

            let state: AccountState = serde_json::from_slice(&value)
                .map_err(|e| format!("Failed to deserialize account: {}", e))?;

            ledger.accounts.insert(addr, state);
        }

        // 3. Load metadata
        if let Some(dist_bytes) = meta_tree
            .get(b"distribution")
            .map_err(|e| format!("Failed to read distribution: {}", e))?
        {
            ledger.distribution = serde_json::from_slice(&dist_bytes)
                .map_err(|e| format!("Failed to deserialize distribution: {}", e))?;
        }

        // Restore accumulated_fees_cil from persistent storage
        if let Some(fee_bytes) = meta_tree
            .get(b"accumulated_fees_cil")
            .map_err(|e| format!("Failed to read accumulated_fees: {}", e))?
        {
            if fee_bytes.len() >= 16 {
                let mut buf = [0u8; 16];
                buf.copy_from_slice(&fee_bytes[..16]);
                ledger.accumulated_fees_cil = u128::from_le_bytes(buf);
            }
        }

        // Restore total_slashed_cil from persistent storage
        if let Some(slash_bytes) = meta_tree
            .get(b"total_slashed_cil")
            .map_err(|e| format!("Failed to read total_slashed_cil: {}", e))?
        {
            if slash_bytes.len() >= 16 {
                let mut buf = [0u8; 16];
                buf.copy_from_slice(&slash_bytes[..16]);
                ledger.total_slashed_cil = u128::from_le_bytes(buf);
            }
        }

        // 4. Rebuild claimed_sends index from loaded Receive blocks (O(1) double-receive check)
        for block in ledger.blocks.values() {
            if block.block_type == los_core::BlockType::Receive {
                ledger.claimed_sends.insert(block.link.clone());
            }
        }

        Ok(ledger)
    }

    /// Save single block (ATOMIC)
    #[allow(dead_code)]
    pub fn save_block(&self, hash: &str, block: &Block) -> Result<(), String> {
        let tree = self.blocks_tree()?;

        let block_json =
            serde_json::to_vec(block).map_err(|e| format!("Failed to serialize block: {}", e))?;

        tree.insert(hash.as_bytes(), block_json)
            .map_err(|e| format!("Failed to save block: {}", e))?;

        tree.flush()
            .map_err(|e| format!("Failed to flush block: {}", e))?;

        Ok(())
    }

    /// Get single block
    #[allow(dead_code)]
    pub fn get_block(&self, hash: &str) -> Result<Option<Block>, String> {
        let tree = self.blocks_tree()?;

        if let Some(bytes) = tree
            .get(hash.as_bytes())
            .map_err(|e| format!("Failed to read block: {}", e))?
        {
            let block: Block = serde_json::from_slice(&bytes)
                .map_err(|e| format!("Failed to deserialize block: {}", e))?;
            Ok(Some(block))
        } else {
            Ok(None)
        }
    }

    /// Save account state (ATOMIC)
    #[allow(dead_code)]
    pub fn save_account(&self, addr: &str, state: &AccountState) -> Result<(), String> {
        let tree = self.accounts_tree()?;

        let state_json =
            serde_json::to_vec(state).map_err(|e| format!("Failed to serialize account: {}", e))?;

        tree.insert(addr.as_bytes(), state_json)
            .map_err(|e| format!("Failed to save account: {}", e))?;

        tree.flush()
            .map_err(|e| format!("Failed to flush account: {}", e))?;

        Ok(())
    }

    /// Get account state
    #[allow(dead_code)]
    pub fn get_account(&self, addr: &str) -> Result<Option<AccountState>, String> {
        let tree = self.accounts_tree()?;

        if let Some(bytes) = tree
            .get(addr.as_bytes())
            .map_err(|e| format!("Failed to read account: {}", e))?
        {
            let state: AccountState = serde_json::from_slice(&bytes)
                .map_err(|e| format!("Failed to deserialize account: {}", e))?;
            Ok(Some(state))
        } else {
            Ok(None)
        }
    }

    /// Get database statistics
    pub fn stats(&self) -> DatabaseStats {
        let blocks_count = self.blocks_tree().ok().map(|t| t.len()).unwrap_or(0);

        let accounts_count = self.accounts_tree().ok().map(|t| t.len()).unwrap_or(0);

        let size_on_disk = self.db.size_on_disk().unwrap_or(0);

        DatabaseStats {
            blocks_count,
            accounts_count,
            size_on_disk,
        }
    }

    /// Check if database is empty (first run)
    pub fn is_empty(&self) -> bool {
        self.blocks_tree()
            .ok()
            .map(|t| t.is_empty())
            .unwrap_or(true)
    }

    /// Create backup snapshot
    #[allow(dead_code)]
    pub fn create_snapshot(&self, path: &str) -> Result<(), String> {
        self.db
            .flush()
            .map_err(|e| format!("Failed to flush before snapshot: {}", e))?;

        // sled snapshots are not directly supported, use export instead
        let blocks = self.blocks_tree()?;
        let accounts = self.accounts_tree()?;

        let backup_data = serde_json::json!({
            "blocks_count": blocks.len(),
            "accounts_count": accounts.len(),
            "timestamp": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        });

        std::fs::write(
            format!("{}/snapshot_meta.json", path),
            serde_json::to_string_pretty(&backup_data).unwrap_or_else(|_| "{}".to_string()),
        )
        .map_err(|e| format!("Failed to write snapshot metadata: {}", e))?;

        Ok(())
    }

    /// Clear all data (DANGER - for testing only)
    #[allow(dead_code)]
    pub fn clear_all(&self) -> Result<(), String> {
        let blocks = self.blocks_tree()?;
        let accounts = self.accounts_tree()?;
        let meta = self.meta_tree()?;

        blocks
            .clear()
            .map_err(|e| format!("Failed to clear blocks: {}", e))?;
        accounts
            .clear()
            .map_err(|e| format!("Failed to clear accounts: {}", e))?;
        meta.clear()
            .map_err(|e| format!("Failed to clear metadata: {}", e))?;

        self.db
            .flush()
            .map_err(|e| format!("Failed to flush after clear: {}", e))?;

        Ok(())
    }

    // --- Smart Contract VM State Persistence ---

    /// Get contracts tree
    fn contracts_tree(&self) -> Result<Tree, String> {
        self.db
            .open_tree(TREE_CONTRACTS)
            .map_err(|e| format!("Failed to open contracts tree: {}", e))
    }

    /// Save entire VM state (all contracts + nonce maps) as a single blob.
    /// The WasmEngine.serialize_all() output is stored under key "vm_state".
    pub fn save_contracts(&self, vm_state_bytes: &[u8]) -> Result<(), String> {
        let tree = self.contracts_tree()?;
        tree.insert(b"vm_state", vm_state_bytes)
            .map_err(|e| format!("Failed to save contracts: {}", e))?;
        tree.flush()
            .map_err(|e| format!("Failed to flush contracts: {}", e))?;
        Ok(())
    }

    /// Load VM state blob. Returns None if no contracts have been deployed yet.
    pub fn load_contracts(&self) -> Result<Option<Vec<u8>>, String> {
        let tree = self.contracts_tree()?;
        match tree.get(b"vm_state") {
            Ok(Some(bytes)) => Ok(Some(bytes.to_vec())),
            Ok(None) => Ok(None),
            Err(e) => Err(format!("Failed to load contracts: {}", e)),
        }
    }

    // --- Faucet Cooldown Persistence ---

    /// Get faucet cooldowns tree
    fn faucet_tree(&self) -> Result<Tree, String> {
        self.db
            .open_tree(TREE_FAUCET_COOLDOWNS)
            .map_err(|e| format!("Failed to open faucet cooldowns tree: {}", e))
    }

    /// Record faucet claim timestamp for an address (persistent across restarts)
    pub fn record_faucet_claim(&self, address: &str) -> Result<(), String> {
        let tree = self.faucet_tree()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        tree.insert(address.as_bytes(), &now.to_le_bytes())
            .map_err(|e| format!("Failed to record faucet claim: {}", e))?;
        Ok(())
    }

    /// Check if address is in faucet cooldown period
    /// Returns Ok(()) if allowed, Err(seconds_remaining) if in cooldown
    pub fn check_faucet_cooldown(&self, address: &str, cooldown_secs: u64) -> Result<(), u64> {
        let tree = self.faucet_tree().map_err(|_| 0u64)?;

        if let Ok(Some(bytes)) = tree.get(address.as_bytes()) {
            if bytes.len() == 8 {
                let last_claim = u64::from_le_bytes(bytes.as_ref().try_into().unwrap_or([0u8; 8]));
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                let elapsed = now.saturating_sub(last_claim);
                if elapsed < cooldown_secs {
                    return Err(cooldown_secs - elapsed);
                }
            }
        }

        Ok(())
    }

    // --- Persistent Peer Storage ---

    /// Get known peers tree
    fn peers_tree(&self) -> Result<Tree, String> {
        self.db
            .open_tree(TREE_PEERS)
            .map_err(|e| format!("Failed to open peers tree: {}", e))
    }

    /// Save known peer (short_addr → full_addr mapping)
    pub fn save_peer(&self, short_addr: &str, full_addr: &str) -> Result<(), String> {
        let tree = self.peers_tree()?;
        tree.insert(short_addr.as_bytes(), full_addr.as_bytes())
            .map_err(|e| format!("Failed to save peer: {}", e))?;
        Ok(())
    }

    /// Load all known peers from disk
    pub fn load_peers(&self) -> Result<std::collections::HashMap<String, String>, String> {
        let tree = self.peers_tree()?;
        let mut peers = std::collections::HashMap::new();

        for item in tree.iter() {
            let (key, value) = item.map_err(|e| format!("Failed to read peer: {}", e))?;
            let short =
                String::from_utf8(key.to_vec()).map_err(|e| format!("Invalid peer key: {}", e))?;
            let full = String::from_utf8(value.to_vec())
                .map_err(|e| format!("Invalid peer value: {}", e))?;
            peers.insert(short, full);
        }

        Ok(peers)
    }

    /// Remove a peer from persistent storage
    #[allow(dead_code)]
    pub fn remove_peer(&self, short_addr: &str) -> Result<(), String> {
        let tree = self.peers_tree()?;
        tree.remove(short_addr.as_bytes())
            .map_err(|e| format!("Failed to remove peer: {}", e))?;
        Ok(())
    }
}

/// Database statistics
#[derive(Debug, Clone)]
pub struct DatabaseStats {
    pub blocks_count: usize,
    pub accounts_count: usize,
    pub size_on_disk: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use los_core::{BlockType, CIL_PER_LOS};

    #[test]
    fn test_database_open() {
        let db = LosDatabase::open("test_db_open").unwrap();
        assert!(db.is_empty());

        // Cleanup
        std::fs::remove_dir_all("test_db_open").ok();
    }

    #[test]
    fn test_save_and_load_ledger() {
        let db = LosDatabase::open("test_db_ledger").unwrap();

        // Create test ledger
        let mut ledger = Ledger::new();
        ledger.accounts.insert(
            "test_account".to_string(),
            AccountState {
                head: "genesis".to_string(),
                balance: 1000 * CIL_PER_LOS,
                block_count: 1,
                is_validator: false,
            },
        );

        // Save
        db.save_ledger(&ledger).unwrap();

        // Load
        let loaded = db.load_ledger().unwrap();
        assert_eq!(loaded.accounts.len(), 1);
        assert_eq!(
            loaded.accounts.get("test_account").unwrap().balance,
            1000 * CIL_PER_LOS
        );

        // Cleanup
        std::fs::remove_dir_all("test_db_ledger").ok();
    }

    #[test]
    fn test_save_single_block() {
        let db = LosDatabase::open("test_db_block").unwrap();

        let block = Block {
            account: "test".to_string(),
            previous: "0".to_string(),
            link: "genesis".to_string(),
            block_type: BlockType::Send,
            amount: 100,
            signature: "sig123".to_string(),
            public_key: "pubkey123".to_string(),
            work: 0,
            timestamp: 1234567890,
            fee: 0,
        };

        // Save
        db.save_block("block_hash_123", &block).unwrap();

        // Load
        let loaded = db.get_block("block_hash_123").unwrap().unwrap();
        assert_eq!(loaded.account, "test");
        assert_eq!(loaded.amount, 100);

        // Cleanup
        std::fs::remove_dir_all("test_db_block").ok();
    }

    #[test]
    fn test_atomic_batch() {
        let db = LosDatabase::open("test_db_atomic").unwrap();

        let mut ledger = Ledger::new();

        // Add multiple accounts
        for i in 0..10 {
            ledger.accounts.insert(
                format!("account_{}", i),
                AccountState {
                    head: "genesis".to_string(),
                    balance: (i as u128) * CIL_PER_LOS,
                    block_count: 0,
                    is_validator: false,
                },
            );
        }

        // Save atomically
        db.save_ledger(&ledger).unwrap();

        // Verify all saved
        let loaded = db.load_ledger().unwrap();
        assert_eq!(loaded.accounts.len(), 10);

        // Cleanup
        std::fs::remove_dir_all("test_db_atomic").ok();
    }

    #[test]
    fn test_database_stats() {
        let db = LosDatabase::open("test_db_stats").unwrap();

        let mut ledger = Ledger::new();
        ledger.accounts.insert(
            "test".to_string(),
            AccountState {
                head: "0".to_string(),
                balance: 100,
                block_count: 0,
                is_validator: false,
            },
        );

        db.save_ledger(&ledger).unwrap();

        let stats = db.stats();
        assert_eq!(stats.accounts_count, 1);
        assert!(stats.size_on_disk > 0);

        // Cleanup
        std::fs::remove_dir_all("test_db_stats").ok();
    }
}
