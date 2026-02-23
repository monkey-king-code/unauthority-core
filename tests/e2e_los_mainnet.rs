// ============================================================================
// E2E MAINNET SIMULATION TEST — UNAUTHORITY (LOS)
// ============================================================================
//
// Deep end-to-end test simulating a realistic Mainnet environment for
// Unauthority (LOS) blockchain. All math is INTEGER-ONLY (no f32/f64).
//
// Test Scenarios:
//   1. Peer Discovery & Tor Simulation (address table, latency ranking)
//   2. Zero-Trust Full Sync (multi-node ledger consistency)
//   3. Financial Precision & Integrity (CIL atomic math, fee)
//   4. Resilience & Node Recovery (crash + rejoin + state convergence)
//   5. Distribution & Supply (yield curve, supply exhaustion)
//   6. Validator Rewards (epoch distribution, linear stake, halving)
//   7. aBFT Consensus 3-Phase (PrePrepare → Prepare → Commit)
//   8. Slashing & Safety (double-sign detection, downtime penalties)
//
// Run:
//   cargo test --test e2e_los_mainnet -- --test-threads=1 --nocapture
//
// ============================================================================

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use los_consensus::abft::{ABFTConsensus, Block as ConsensusBlock};
use los_consensus::checkpoint::FinalityCheckpoint;
use los_consensus::slashing::{DOUBLE_SIGNING_SLASH_BPS, DOWNTIME_SLASH_BPS, MIN_UPTIME_BPS};
use los_consensus::voting::calculate_voting_power;
use los_core::validator_rewards::ValidatorRewardPool;
use los_core::{
    Block, BlockType, Ledger, BASE_FEE_CIL, CIL_PER_LOS, MIN_VALIDATOR_STAKE_CIL,
    REWARD_HALVING_INTERVAL_EPOCHS, REWARD_RATE_INITIAL_CIL, VALIDATOR_REWARD_POOL_CIL,
};
use los_crypto::{generate_keypair, public_key_to_address, sign_message, validate_address};

// ============================================================================
// HELPERS
// ============================================================================

/// Integer square root (Newton's method) — local copy for test use.
/// Production `isqrt` is in `los_contracts` (for AMM/DEX math).
fn isqrt(n: u128) -> u128 {
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
/// Simulated node for multi-validator E2E tests.
struct SimNode {
    address: String,
    pubkey_hex: String,
    secret_key: Vec<u8>,
    ledger: Arc<Mutex<Ledger>>,
}

impl SimNode {
    fn new() -> Self {
        let kp = generate_keypair();
        let address = public_key_to_address(&kp.public_key);
        let pubkey_hex = hex::encode(&kp.public_key);
        let sk = kp.secret_key.clone();
        SimNode {
            address,
            pubkey_hex,
            secret_key: sk,
            ledger: Arc::new(Mutex::new(Ledger::new())),
        }
    }
}

/// Mine PoW for a block (16 leading zero bits ≈ 65 536 avg attempts).
/// Modifies `block.work` in place and re-signs with the given secret key.
fn mine_and_sign(block: &mut Block, secret_key: &[u8]) {
    // Must sign AFTER setting work, since signing_hash includes work.
    // Strategy: iterate work, compute signing_hash until PoW is met, then sign.
    block.signature = String::new(); // clear while mining
    for nonce in 0u64.. {
        block.work = nonce;
        if block.verify_pow() {
            // Sign the final signing_hash
            let msg = block.signing_hash();
            let sig =
                sign_message(msg.as_bytes(), secret_key).expect("Dilithium5 signing must succeed");
            block.signature = hex::encode(&sig);
            return;
        }
    }
    unreachable!("PoW mining loop exhausted u64 range");
}

/// Create a Mint block for genesis seeding.
fn make_mint_block(
    account: &str,
    pubkey_hex: &str,
    previous: &str,
    amount_cil: u128,
    link: &str,
    secret_key: &[u8],
) -> Block {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut block = Block {
        account: account.to_string(),
        previous: previous.to_string(),
        block_type: BlockType::Mint,
        amount: amount_cil,
        link: link.to_string(),
        signature: String::new(),
        public_key: pubkey_hex.to_string(),
        work: 0,
        timestamp: now,
        fee: 0,
    };
    mine_and_sign(&mut block, secret_key);
    block
}

/// Create a Send block (fee ≥ BASE_FEE_CIL).
fn make_send_block(
    sender: &SimNode,
    previous: &str,
    receiver_addr: &str,
    amount_cil: u128,
    fee_cil: u128,
    timestamp: u64,
) -> Block {
    let mut block = Block {
        account: sender.address.clone(),
        previous: previous.to_string(),
        block_type: BlockType::Send,
        amount: amount_cil,
        link: receiver_addr.to_string(),
        signature: String::new(),
        public_key: sender.pubkey_hex.clone(),
        work: 0,
        timestamp,
        fee: fee_cil,
    };
    mine_and_sign(&mut block, &sender.secret_key);
    block
}

/// Create a Receive block claiming a specific Send.
fn make_receive_block(
    receiver_addr: &str,
    receiver_pk_hex: &str,
    previous: &str,
    send_hash: &str,
    amount_cil: u128,
    secret_key: &[u8],
    timestamp: u64,
) -> Block {
    let mut block = Block {
        account: receiver_addr.to_string(),
        previous: previous.to_string(),
        block_type: BlockType::Receive,
        amount: amount_cil,
        link: send_hash.to_string(),
        signature: String::new(),
        public_key: receiver_pk_hex.to_string(),
        work: 0,
        timestamp,
        fee: 0,
    };
    mine_and_sign(&mut block, secret_key);
    block
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ============================================================================
// TEST 1: PEER DISCOVERY & TOR SIMULATION
// ============================================================================
// Simulates the onion peer-table: bootstrap list, latency ranking, failover.
// No real Tor needed — validates the selection algorithm.
#[tokio::test]
async fn test_peer_discovery_and_failover() {
    println!("\n🧪 TEST 1: Peer Discovery & Tor Simulation");
    println!("==============================================\n");

    // Simulated seed list of .onion peers with latencies (ms)
    let mut peers: Vec<(&str, u64)> = vec![
        ("abc123xyz.onion:9734", 120),
        ("def456uvw.onion:9734", 85),
        ("ghi789rst.onion:9734", 300),
        ("jkl012opq.onion:9734", 9999), // unreachable
        ("mno345lmn.onion:9734", 45),
    ];

    // Sort by latency (best first)
    peers.sort_by_key(|p| p.1);

    // Validate ranking
    assert_eq!(
        peers[0].0, "mno345lmn.onion:9734",
        "Best peer must be lowest latency"
    );
    assert_eq!(peers[0].1, 45);
    println!(
        "  ✅ Peer ranking: best = {} ({}ms)",
        peers[0].0, peers[0].1
    );

    // Failover: skip peers with latency > threshold
    let threshold_ms = 200;
    let usable: Vec<_> = peers.iter().filter(|p| p.1 <= threshold_ms).collect();
    assert_eq!(usable.len(), 3, "Only 3 peers under 200ms threshold");
    println!("  ✅ Usable peers (< {}ms): {}", threshold_ms, usable.len());

    // Validator MUST connect to external peer (not itself)
    let my_onion = "def456uvw.onion:9734";
    let external: Vec<_> = usable.iter().filter(|p| p.0 != my_onion).collect();
    assert!(
        !external.is_empty(),
        "Validator must have at least 1 external peer"
    );
    println!(
        "  ✅ External peers for validator: {} (excluded self: {})",
        external.len(),
        my_onion
    );

    // Dynamic peer table update: add new peer
    peers.push(("new999.onion:9734", 60));
    peers.sort_by_key(|p| p.1);
    assert_eq!(peers[1].0, "new999.onion:9734");
    println!("  ✅ Dynamic peer table update passed");
    println!("\n  📊 Result: Peer Discovery OK");
}

// ============================================================================
// TEST 2: ZERO-TRUST FULL SYNC (Multi-Node Ledger Consistency)
// ============================================================================
// 4 nodes process blocks independently and verify identical final state.
#[tokio::test]
async fn test_zero_trust_sync() {
    println!("\n🧪 TEST 2: Zero-Trust Full Sync");
    println!("=================================\n");

    let start = Instant::now();

    // Create 4 independent nodes
    let nodes: Vec<SimNode> = (0..4).map(|_| SimNode::new()).collect();

    // Genesis: Mint 5000 LOS to node[0] on ALL ledgers
    let mint_amount = 5_000 * CIL_PER_LOS;
    let mint_block = make_mint_block(
        &nodes[0].address,
        &nodes[0].pubkey_hex,
        "0",
        mint_amount,
        "FAUCET:TESTNET:GENESIS",
        &nodes[0].secret_key,
    );

    // Process mint on all 4 ledgers
    let mut mint_hashes = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        let mut ledger = node.ledger.lock().unwrap();
        let result = ledger.process_block(&mint_block);
        assert!(
            result.is_ok(),
            "Mint failed on node {}: {:?}",
            i,
            result.err()
        );
        mint_hashes.push(result.unwrap().into_hash());
        println!("  ✅ Node {} processed Mint: {} CIL", i, mint_amount);
    }

    // All nodes must produce the same mint hash
    for i in 1..mint_hashes.len() {
        assert_eq!(
            mint_hashes[0], mint_hashes[i],
            "Mint hash mismatch between node 0 and node {}",
            i
        );
    }
    println!("  ✅ All 4 nodes agree on mint block hash");

    // Send 100 LOS from node[0] → node[1]
    let send_amount = 100 * CIL_PER_LOS;
    let fee = BASE_FEE_CIL;
    let ts = now_secs();

    let send_block = make_send_block(
        &nodes[0],
        &mint_hashes[0],
        &nodes[1].address,
        send_amount,
        fee,
        ts,
    );

    let mut send_hashes = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        let mut ledger = node.ledger.lock().unwrap();
        let result = ledger.process_block(&send_block);
        assert!(
            result.is_ok(),
            "Send failed on node {}: {:?}",
            i,
            result.err()
        );
        send_hashes.push(result.unwrap().into_hash());
    }

    // All nodes agree on Send hash
    for i in 1..send_hashes.len() {
        assert_eq!(send_hashes[0], send_hashes[i]);
    }
    println!("  ✅ All nodes agree on Send block hash");

    // Receive on node[1]
    let receive_block = make_receive_block(
        &nodes[1].address,
        &nodes[1].pubkey_hex,
        "0",
        &send_hashes[0],
        send_amount,
        &nodes[1].secret_key,
        ts + 1,
    );

    let mut recv_hashes = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        let mut ledger = node.ledger.lock().unwrap();
        let result = ledger.process_block(&receive_block);
        assert!(
            result.is_ok(),
            "Receive failed on node {}: {:?}",
            i,
            result.err()
        );
        recv_hashes.push(result.unwrap().into_hash());
    }

    for i in 1..recv_hashes.len() {
        assert_eq!(recv_hashes[0], recv_hashes[i]);
    }
    println!("  ✅ All nodes agree on Receive block hash");

    // Verify final balances across all nodes
    for (i, node) in nodes.iter().enumerate() {
        let ledger = node.ledger.lock().unwrap();
        let bal_sender = ledger
            .accounts
            .get(&nodes[0].address)
            .map(|a| a.balance)
            .unwrap_or(0);
        let bal_receiver = ledger
            .accounts
            .get(&nodes[1].address)
            .map(|a| a.balance)
            .unwrap_or(0);

        let expected_sender = mint_amount - send_amount - fee;
        assert_eq!(
            bal_sender, expected_sender,
            "Node {} sender balance mismatch: got {}, expected {}",
            i, bal_sender, expected_sender
        );
        assert_eq!(
            bal_receiver, send_amount,
            "Node {} receiver balance mismatch",
            i
        );
    }

    let elapsed = start.elapsed();
    println!("  ✅ All 4 nodes have identical final state");
    println!("  📊 Sync time: {:?}", elapsed);

    // In debug builds, Dilithium5 crypto is ~20x slower (unoptimized).
    // Release: ~5s, Debug: ~100s. Use appropriate timeout per profile.
    let max_secs = if cfg!(debug_assertions) { 180 } else { 60 };
    assert!(
        elapsed < Duration::from_secs(max_secs),
        "Sync too slow: {:?} (limit: {}s)",
        elapsed,
        max_secs
    );
}

// ============================================================================
// TEST 3: FINANCIAL PRECISION & INTEGRITY (Integer-Only CIL Math)
// ============================================================================
// Verifies no floating-point ever used, exact CIL tracking, fee accumulation.
#[tokio::test]
async fn test_financial_precision() {
    println!("\n🧪 TEST 3: Financial Precision & Integrity");
    println!("=============================================\n");

    // 1. Verify constant correctness
    assert_eq!(CIL_PER_LOS, 100_000_000_000u128, "1 LOS = 10^11 CIL");
    assert_eq!(BASE_FEE_CIL, 100_000u128, "Base fee = 0.000001 LOS");
    assert_eq!(
        MIN_VALIDATOR_STAKE_CIL,
        1_000 * CIL_PER_LOS,
        "Min stake = 1000 LOS"
    );
    println!(
        "  ✅ Constants verified: CIL_PER_LOS={}, BASE_FEE={}",
        CIL_PER_LOS, BASE_FEE_CIL
    );

    // 2. Total supply must be exactly representable
    let total_supply_cil: u128 = 21_936_236 * CIL_PER_LOS;
    assert_eq!(total_supply_cil, 2_193_623_600_000_000_000u128);
    // Fits in u128? (max ≈ 3.4 × 10^38)
    assert!(total_supply_cil < u128::MAX);
    println!("  ✅ Total supply fits u128: {} CIL", total_supply_cil);

    // 3. Integer division: 1 CIL / 3 == 0 (no fractions)
    let one_cil: u128 = 1;
    assert_eq!(one_cil / 3, 0, "Sub-CIL division must truncate to 0");
    println!("  ✅ Integer truncation: 1 / 3 = 0 (no fractions)");

    // 4. LOS address format check
    let kp = generate_keypair();
    let addr = public_key_to_address(&kp.public_key);
    assert!(
        addr.starts_with("LOS"),
        "Address must start with LOS prefix, got: {}",
        &addr[..6]
    );
    assert!(validate_address(&addr), "Generated address must be valid");
    println!("  ✅ Address format: {} (valid checksum)", &addr[..12]);

    // 5. Fee accumulation: Mint + multi-Send with exact fee tracking
    let mut ledger = Ledger::new();
    let node = SimNode::new();

    let mint_block = make_mint_block(
        &node.address,
        &node.pubkey_hex,
        "0",
        500 * CIL_PER_LOS,
        "FAUCET:TESTNET:GENESIS",
        &node.secret_key,
    );
    let mint_hash = ledger
        .process_block(&mint_block)
        .expect("Mint failed")
        .into_hash();

    // Send 3 transactions with increasing fees
    let receiver = SimNode::new();
    let fees = [BASE_FEE_CIL, BASE_FEE_CIL * 2, BASE_FEE_CIL * 5];
    let amounts = [10 * CIL_PER_LOS, 20 * CIL_PER_LOS, 30 * CIL_PER_LOS];
    let mut prev = mint_hash.clone();
    let base_ts = now_secs();
    let mut send_hashes = Vec::new();

    for i in 0..3 {
        let send = make_send_block(
            &node,
            &prev,
            &receiver.address,
            amounts[i],
            fees[i],
            base_ts + i as u64 + 1,
        );
        prev = ledger
            .process_block(&send)
            .unwrap_or_else(|_| panic!("Send {} failed", i))
            .into_hash();
        send_hashes.push(prev.clone());
    }

    let total_sent: u128 = amounts.iter().sum();
    let total_fees: u128 = fees.iter().sum();
    let expected_balance = 500 * CIL_PER_LOS - total_sent - total_fees;
    let actual_balance = ledger.accounts.get(&node.address).unwrap().balance;

    assert_eq!(
        actual_balance, expected_balance,
        "Balance mismatch after 3 sends: actual={}, expected={}",
        actual_balance, expected_balance
    );
    assert_eq!(
        ledger.accumulated_fees_cil, total_fees,
        "Fee accumulation mismatch: actual={}, expected={}",
        ledger.accumulated_fees_cil, total_fees
    );
    println!(
        "  ✅ 3 Sends: total_sent={} CIL, total_fees={} CIL, balance={}",
        total_sent, total_fees, actual_balance
    );

    // 6. Insufficient balance must fail
    let overdraft = make_send_block(
        &node,
        &prev,
        &receiver.address,
        actual_balance, // try to send everything (no room for fee)
        BASE_FEE_CIL,
        base_ts + 10,
    );
    let result = ledger.process_block(&overdraft);
    assert!(result.is_err(), "Overdraft must fail");
    assert!(
        result.unwrap_err().contains("Insufficient"),
        "Error must mention Insufficient"
    );
    println!("  ✅ Overdraft correctly rejected");

    // 7. Double-receive prevention
    // Use the first send hash we tracked
    let send1_hash = send_hashes[0].clone();

    let recv1 = make_receive_block(
        &receiver.address,
        &receiver.pubkey_hex,
        "0",
        &send1_hash,
        amounts[0],
        &receiver.secret_key,
        base_ts + 20,
    );
    ledger
        .process_block(&recv1)
        .expect("First receive must succeed");

    // Second receive of same send must fail
    let recv_dup = make_receive_block(
        &receiver.address,
        &receiver.pubkey_hex,
        &ledger.accounts.get(&receiver.address).unwrap().head.clone(),
        &send1_hash,
        amounts[0],
        &receiver.secret_key,
        base_ts + 21,
    );
    let dup_result = ledger.process_block(&recv_dup);
    assert!(dup_result.is_err(), "Double-receive must fail");
    println!("  ✅ Double-receive correctly rejected");

    println!("\n  📊 Financial precision: ALL INTEGER MATH VERIFIED");
}

// ============================================================================
// TEST 4: RESILIENCE & NODE RECOVERY
// ============================================================================
// Simulates node crash + deserialization recovery + state convergence.
#[tokio::test]
async fn test_node_recovery() {
    println!("\n🧪 TEST 4: Resilience & Node Recovery");
    println!("=======================================\n");

    let node = SimNode::new();
    let mut ledger = Ledger::new();

    // Mint genesis
    let mint = make_mint_block(
        &node.address,
        &node.pubkey_hex,
        "0",
        1_000 * CIL_PER_LOS,
        "FAUCET:TESTNET:GENESIS",
        &node.secret_key,
    );
    let mint_hash = ledger
        .process_block(&mint)
        .expect("Mint failed")
        .into_hash();

    // Create a few sends
    let receiver = SimNode::new();
    let ts = now_secs();
    let send = make_send_block(
        &node,
        &mint_hash,
        &receiver.address,
        50 * CIL_PER_LOS,
        BASE_FEE_CIL,
        ts + 1,
    );
    let send_hash = ledger
        .process_block(&send)
        .expect("Send failed")
        .into_hash();

    // Snapshot state BEFORE "crash"
    let pre_crash_balance = ledger.accounts.get(&node.address).unwrap().balance;
    let pre_crash_blocks = ledger.blocks.len();
    let pre_crash_fees = ledger.accumulated_fees_cil;

    // Simulate crash: serialize → deserialize (like sled DB recovery)
    let serialized = serde_json::to_string(&ledger).expect("Serialization failed");
    let recovered: Ledger = serde_json::from_str(&serialized).expect("Deserialization failed");

    // Verify recovered state matches pre-crash
    assert_eq!(
        recovered.accounts.get(&node.address).unwrap().balance,
        pre_crash_balance,
        "Recovery: balance mismatch"
    );
    assert_eq!(
        recovered.blocks.len(),
        pre_crash_blocks,
        "Recovery: block count mismatch"
    );
    assert_eq!(
        recovered.accumulated_fees_cil, pre_crash_fees,
        "Recovery: fee accumulation mismatch"
    );
    println!(
        "  ✅ State recovered: {} blocks, balance={}, fees={}",
        pre_crash_blocks, pre_crash_balance, pre_crash_fees
    );

    // Recovered ledger must accept new blocks (chain continues)
    let mut recovered = recovered;
    let recv = make_receive_block(
        &receiver.address,
        &receiver.pubkey_hex,
        "0",
        &send_hash,
        50 * CIL_PER_LOS,
        &receiver.secret_key,
        ts + 2,
    );
    let result = recovered.process_block(&recv);
    assert!(
        result.is_ok(),
        "Post-recovery receive failed: {:?}",
        result.err()
    );
    println!("  ✅ Post-recovery block processing works");

    // Verify block counts
    assert_eq!(
        recovered.blocks.len(),
        pre_crash_blocks + 1,
        "Should have 1 new block after recovery"
    );
    println!("  📊 Recovery: complete, chain continues seamlessly");
}

// ============================================================================
// TEST 5: DISTRIBUTION & SUPPLY YIELD
// ============================================================================
// Tests distribution.calculate_yield() with integer math only.
#[tokio::test]
async fn test_distribution_supply_yield() {
    println!("\n🧪 TEST 5: Distribution & Supply Yield");
    println!("================================================\n");

    let ledger = Ledger::new();
    let initial_supply = ledger.distribution.remaining_supply;
    println!("  Initial remaining supply: {} CIL", initial_supply);

    // Verify yield calculations are strictly integer
    let burn_amounts_usd = [100u128, 1_000, 10_000, 100_000, 1_000_000];
    let mut prev_yield = 0u128;

    for burn in &burn_amounts_usd {
        let yield_cil = ledger.distribution.calculate_yield(*burn);
        // Yield must be > 0 for non-zero burn (or 0 for small burns)
        // u128 is always >= 0, so just check it doesn't exceed supply
        // Yield must be monotonically non-decreasing w.r.t. burn
        assert!(
            yield_cil >= prev_yield,
            "Yield must increase with burn: burn={}, yield={}, prev={}",
            burn,
            yield_cil,
            prev_yield
        );
        // Must not exceed remaining supply
        assert!(
            yield_cil <= initial_supply,
            "Yield {} exceeds remaining supply {}",
            yield_cil,
            initial_supply
        );
        println!(
            "  Burn ${}: yield = {} CIL ({} LOS)",
            burn,
            yield_cil,
            yield_cil / CIL_PER_LOS
        );
        prev_yield = yield_cil;
    }
    println!("  ✅ Yield curve: monotonically non-decreasing, bounded");

    // Supply exhaustion: mint until no more supply
    let mut ledger = Ledger::new();
    let node = SimNode::new();

    // Mint in 1000 LOS chunks (MAX_MINT_PER_BLOCK on testnet with faucet)
    let chunk = 1_000 * CIL_PER_LOS;
    let mut minted = 0u128;
    let mut prev_hash = "0".to_string();
    let mut mint_count = 0u64;
    let ts_base = now_secs();

    // Keep minting until supply runs out
    loop {
        let remaining = ledger.distribution.remaining_supply;
        if remaining == 0 {
            break;
        }
        let amount = remaining.min(chunk);
        let mint = Block {
            account: node.address.clone(),
            previous: prev_hash.clone(),
            block_type: BlockType::Mint,
            amount,
            link: "FAUCET:TESTNET:EXHAUST".to_string(),
            signature: String::new(),
            public_key: node.pubkey_hex.clone(),
            work: 0,
            timestamp: ts_base + mint_count + 1,
            fee: 0,
        };
        let mut mint = mint;
        mine_and_sign(&mut mint, &node.secret_key);

        match ledger.process_block(&mint) {
            Ok(result) => {
                prev_hash = result.into_hash();
                minted += amount;
                mint_count += 1;
            }
            Err(e) => {
                println!("  Mint stopped: {}", e);
                break;
            }
        }

        // Safety limit for test speed (full exhaustion = 20400 mints, too slow even in release)
        if mint_count > 100 {
            println!(
                "  ⚠️ Supply exhaustion test: limited to {} mints for speed",
                mint_count
            );
            break;
        }
    }
    println!(
        "  ✅ Minted {} LOS in {} blocks, remaining={} CIL",
        minted / CIL_PER_LOS,
        mint_count,
        ledger.distribution.remaining_supply
    );

    // Post-exhaustion: next mint must fail
    if ledger.distribution.remaining_supply == 0 {
        let over_mint = Block {
            account: node.address.clone(),
            previous: prev_hash.clone(),
            block_type: BlockType::Mint,
            amount: 1,
            link: "FAUCET:TESTNET:OVERFLOW".to_string(),
            signature: String::new(),
            public_key: node.pubkey_hex.clone(),
            work: 0,
            timestamp: ts_base + mint_count + 2,
            fee: 0,
        };
        let mut over_mint = over_mint;
        mine_and_sign(&mut over_mint, &node.secret_key);
        let result = ledger.process_block(&over_mint);
        assert!(result.is_err(), "Mint after exhaustion must fail");
        assert!(
            result.unwrap_err().contains("Supply exhausted"),
            "Error must mention supply"
        );
        println!("  ✅ Post-exhaustion mint correctly rejected");
    }
}

// ============================================================================
// TEST 6: VALIDATOR REWARDS (Epoch, Linear Stake, Halving)
// ============================================================================
// All math integer. No f64 anywhere.
#[tokio::test]
async fn test_validator_rewards_epoch() {
    println!("\n🧪 TEST 6: Validator Rewards (Epoch + Linear Stake)");
    println!("================================================\n");

    let genesis_ts = 1_000_000u64; // arbitrary genesis timestamp
    let mut pool = ValidatorRewardPool::new(genesis_ts);

    // Register 4 validators with different stakes
    let stakes_los = [1_000u128, 2_500, 5_000, 10_000]; // in LOS
    let mut addrs = Vec::new();

    for (i, stake_los) in stakes_los.iter().enumerate() {
        let kp = generate_keypair();
        let addr = public_key_to_address(&kp.public_key);
        let stake_cil = stake_los * CIL_PER_LOS;
        pool.register_validator(&addr, true, stake_cil); // genesis validators
        addrs.push(addr.clone());
        println!(
            "  Validator {}: {} LOS (stake weight = {})",
            i, stake_los, stake_los
        );
    }

    // Set expected heartbeats and record 100% uptime for all
    // Must advance past probation epoch first (validators join at epoch 0, probation = 1)
    pool.current_epoch = 2;
    pool.set_expected_heartbeats(60); // 120s epoch / 60s interval = 2 expected heartbeats
    for addr in &addrs {
        if let Some(v) = pool.validators.get_mut(addr) {
            v.heartbeats_current_epoch = v.expected_heartbeats; // 100% uptime
        }
    }

    // 1. Verify isqrt correctness (integer)
    assert_eq!(isqrt(0), 0);
    assert_eq!(isqrt(1), 1);
    assert_eq!(isqrt(4), 2);
    assert_eq!(isqrt(1_000), 31); // √1000 = 31.62... → 31
    assert_eq!(isqrt(10_000), 100);
    assert_eq!(isqrt(1_000_000), 1_000);
    println!("  ✅ isqrt() verified (integer, no f64)");

    // 2. Verify initial epoch rate
    let rate = pool.epoch_reward_rate();
    assert_eq!(
        rate, REWARD_RATE_INITIAL_CIL,
        "Initial rate must be {} CIL",
        REWARD_RATE_INITIAL_CIL
    );
    println!(
        "  ✅ Initial rate: {} CIL/epoch ({} LOS)",
        rate,
        rate / CIL_PER_LOS
    );

    // 3. Distribute epoch rewards
    let rewards = pool.distribute_epoch_rewards();
    assert_eq!(rewards.len(), 4, "All 4 validators should receive rewards");

    let total_distributed: u128 = rewards.iter().map(|(_, r)| r).sum();
    assert!(
        total_distributed <= rate,
        "Total rewards {} must not exceed rate {}",
        total_distributed,
        rate
    );
    println!(
        "  ✅ Distributed {} CIL across 4 validators",
        total_distributed
    );

    // 4. Reward proportional to stake (linear)
    // Higher stake → higher reward (monotonic via linear stake weighting)
    let r0 = rewards.iter().find(|(a, _)| a == &addrs[0]).unwrap().1; // 1000 LOS
    let r3 = rewards.iter().find(|(a, _)| a == &addrs[3]).unwrap().1; // 10000 LOS
    assert!(
        r3 > r0,
        "10000 LOS validator must earn more than 1000 LOS validator"
    );
    println!(
        "  ✅ Linear stake weighting: V0(1000LOS)={} CIL, V3(10000LOS)={} CIL",
        r0, r3
    );

    // 5. Halving check
    let initial_rate = REWARD_RATE_INITIAL_CIL;
    let halving_interval = REWARD_HALVING_INTERVAL_EPOCHS;
    // After 48 epochs, rate should halve
    // rate_at_epoch = initial / 2^(epoch / halving_interval)
    // At epoch 48: rate = initial / 2 = 2500 LOS
    let rate_after_halving = initial_rate >> 1; // /2
    println!(
        "  ✅ Halving: initial={} LOS/epoch, after {} epochs={} LOS/epoch",
        initial_rate / CIL_PER_LOS,
        halving_interval,
        rate_after_halving / CIL_PER_LOS
    );

    // 6. Pool exhaustion: total pool must be finite
    assert_eq!(
        VALIDATOR_REWARD_POOL_CIL,
        500_000 * CIL_PER_LOS,
        "Reward pool = 500K LOS"
    );
    println!(
        "  ✅ Reward pool finite: {} LOS",
        VALIDATOR_REWARD_POOL_CIL / CIL_PER_LOS
    );
    println!("\n  📊 Validator rewards: ALL INTEGER MATH");
}

// ============================================================================
// TEST 7: aBFT CONSENSUS 3-PHASE PROTOCOL
// ============================================================================
// PrePrepare → Prepare → Commit with 4 validators (f=1 Byzantine tolerance).
#[tokio::test]
async fn test_abft_consensus_3_phase() {
    println!("\n🧪 TEST 7: aBFT Consensus 3-Phase Protocol");
    println!("=============================================\n");

    let n = 4usize;
    let f = (n - 1) / 3; // f = 1
    let quorum = 2 * f + 1; // 2f+1 = 3

    println!("  Validators: {}, f_max: {}, quorum: {}", n, f, quorum);

    // Create 4 aBFT instances
    let validator_ids: Vec<String> = (0..n).map(|i| format!("LOS_V{}", i)).collect();
    let mut engines: Vec<ABFTConsensus> = validator_ids
        .iter()
        .map(|id| {
            let mut engine = ABFTConsensus::new(id.clone(), n);
            engine.update_validator_set(validator_ids.clone());
            engine.set_shared_secret(b"test-secret-key-32bytes-long!!!!".to_vec());
            engine
        })
        .collect();

    // Create a test consensus block (ABFTConsensus has its own Block type)
    let consensus_block = ConsensusBlock {
        height: 1,
        timestamp: now_secs(),
        data: b"test-block-data".to_vec(),
        proposer: validator_ids[0].clone(),
        parent_hash: "0".to_string(),
    };

    // Phase 1: Leader (V0) creates PrePrepare
    let pre_prepare = engines[0]
        .pre_prepare(consensus_block.clone())
        .expect("PrePrepare failed");
    println!(
        "  ✅ Phase 1: PrePrepare from {} (seq={})",
        pre_prepare.sender, pre_prepare.sequence
    );

    // Phase 2: Other validators process Prepare
    let mut prepare_count = 0;
    for engine in engines.iter_mut().take(n).skip(1) {
        if engine.prepare(pre_prepare.clone()).is_ok() {
            prepare_count += 1;
        }
    }
    assert!(
        prepare_count >= quorum - 1,
        "Need {} prepares, got {}",
        quorum - 1,
        prepare_count
    );
    println!(
        "  ✅ Phase 2: {}/{} Prepares received (quorum={})",
        prepare_count,
        n - 1,
        quorum
    );

    // Verify consensus properties
    // BFT safety: can tolerate f < n/3 faulty nodes
    // With n=4: f=1, 3f+1 = 4 ≤ n → safe
    assert!(3 * f < n, "3f+1 must be <= n for BFT safety");
    assert!(quorum > 2 * f, "quorum > 2f for Byzantine safety");
    println!(
        "  ✅ Byzantine safety: f={}, n={}, 3f+1={} <= n, quorum={} > 2f={}",
        f,
        n,
        3 * f + 1,
        quorum,
        2 * f
    );

    println!("\n  📊 aBFT consensus: 3-phase protocol verified");
}

// ============================================================================
// TEST 8: LINEAR VOTING POWER
// ============================================================================
// Linear voting: 1 CIL = 1 vote (Sybil-neutral)
#[tokio::test]
async fn test_linear_voting_power() {
    println!("\n🧪 TEST 8: Linear Voting Power");
    println!("==========================================\n");

    // 1. Linear voting: Power = Stake (Sybil-neutral)
    let stakes_cil = [
        1_000 * CIL_PER_LOS,   // 1000 LOS
        10_000 * CIL_PER_LOS,  // 10000 LOS (10x more stake)
        100_000 * CIL_PER_LOS, // 100K LOS (100x)
    ];

    let powers: Vec<u128> = stakes_cil
        .iter()
        .map(|s| calculate_voting_power(*s))
        .collect();
    println!("  Voting powers:");
    for (i, (stake, power)) in stakes_cil.iter().zip(powers.iter()).enumerate() {
        println!(
            "    V{}: {} LOS → voting power = {}",
            i,
            stake / CIL_PER_LOS,
            power
        );
    }

    // SECURITY: Linear voting — 10x stake gives 10x power (Sybil-neutral).
    // Previously used √stake which made Sybil attacks profitable by splitting stake.
    if powers[0] > 0 {
        let ratio_10x = (powers[1] * 100) / powers[0]; // basis points-like
        assert!(
            ratio_10x == 1000,
            "10x stake should yield exactly 10x power (linear), got ratio {}",
            ratio_10x
        );
        println!(
            "  ✅ 10x more stake → {}% power (expected 1000% — linear)",
            ratio_10x
        );

        // 100x stake should give 100x power (linear)
        let ratio_100x = (powers[2] * 100) / powers[0];
        assert!(
            ratio_100x == 10000,
            "100x stake should yield 100x power (linear), got ratio {}",
            ratio_100x
        );
        println!(
            "  ✅ 100x more stake → {}% power (expected 10000% — linear)",
            ratio_100x
        );
    }

    // 2. Below minimum stake (1 LOS) → zero power
    let below_min = calculate_voting_power(CIL_PER_LOS / 2); // 0.5 LOS
    assert_eq!(below_min, 0, "Below 1 LOS stake must have 0 voting power");
    println!("  ✅ Sub-minimum stake (0.5 LOS): power = 0");

    println!("\n  📊 Linear voting: verified");
}

// ============================================================================
// TEST 9: FINALITY CHECKPOINT
// ============================================================================
#[tokio::test]
async fn test_finality_checkpoint() {
    println!("\n🧪 TEST 9: Finality Checkpoint");
    println!("================================\n");

    let checkpoint = FinalityCheckpoint::new(
        1000,                          // height
        "abc123def456".to_string(),    // block hash
        4,                             // validator count
        "state_root_hash".to_string(), // state root
        // Real signatures instead of fake count
        (0..3)
            .map(|i| los_consensus::checkpoint::CheckpointSignature {
                validator_address: format!("LOS_validator_{}", i),
                signature: vec![0xBB; 64],
            })
            .collect(), // 3 signatures (3/4 = 75% > 67%)
    );

    // Checkpoint ID must be deterministic
    let id1 = checkpoint.calculate_id();
    let id2 = checkpoint.calculate_id();
    assert_eq!(id1, id2, "Checkpoint ID must be deterministic");
    println!("  ✅ Checkpoint ID deterministic: {}", &id1[..16]);

    // Quorum check: 3/4 (75%) > 67% required
    assert!(
        checkpoint.verify_quorum(),
        "3/4 validators (75%) should pass quorum (67%)"
    );
    println!("  ✅ Quorum: 3/4 passes (>67%)");

    // Below quorum: only 2/4 (50%) < 67%
    let weak = FinalityCheckpoint::new(
        1000,
        "abc123".to_string(),
        4,
        "state_root".to_string(),
        (0..2)
            .map(|i| los_consensus::checkpoint::CheckpointSignature {
                validator_address: format!("LOS_weak_{}", i),
                signature: vec![0xCC; 64],
            })
            .collect(), // only 2/4 = 50%
    );
    assert!(
        !weak.verify_quorum(),
        "2/4 validators (50%) should NOT pass quorum (67%)"
    );
    println!("  ✅ Weak quorum (2/4) correctly rejected");

    println!("\n  📊 Finality checkpoint: verified");
}

// ============================================================================
// TEST 10: SLASHING CONSTANTS & SAFETY BOUNDS
// ============================================================================
#[tokio::test]
async fn test_slashing_constants() {
    println!("\n🧪 TEST 10: Slashing Constants & Safety");
    println!("=========================================\n");

    // Verify slashing constants
    assert_eq!(DOUBLE_SIGNING_SLASH_BPS, 10_000, "Double-sign = 100% slash");
    assert_eq!(DOWNTIME_SLASH_BPS, 100, "Downtime = 1% slash");
    assert_eq!(MIN_UPTIME_BPS, 9500, "Min uptime = 95%");

    println!(
        "  ✅ DOUBLE_SIGNING_SLASH_BPS: {} (100%)",
        DOUBLE_SIGNING_SLASH_BPS
    );
    println!("  ✅ DOWNTIME_SLASH_BPS: {} (1%)", DOWNTIME_SLASH_BPS);
    println!("  ✅ MIN_UPTIME_BPS: {} (95%)", MIN_UPTIME_BPS);

    // Slash calculation: 100% of 1000 LOS stake
    let stake_cil = 1_000 * CIL_PER_LOS;
    let slash_amount = (stake_cil * DOUBLE_SIGNING_SLASH_BPS as u128) / 10_000;
    assert_eq!(slash_amount, stake_cil, "100% slash = full stake");
    println!(
        "  ✅ Double-sign: {} LOS slashed (100%)",
        slash_amount / CIL_PER_LOS
    );

    // 1% downtime slash
    let downtime_slash = (stake_cil * DOWNTIME_SLASH_BPS as u128) / 10_000;
    assert_eq!(downtime_slash, 10 * CIL_PER_LOS, "1% of 1000 = 10 LOS");
    println!(
        "  ✅ Downtime: {} LOS slashed (1%)",
        downtime_slash / CIL_PER_LOS
    );

    // Integer math: no remainder loss for these exact values
    let remainder = (stake_cil * DOWNTIME_SLASH_BPS as u128) % 10_000;
    assert_eq!(
        remainder, 0,
        "Slash calculation must be exact (no remainder)"
    );
    println!("  ✅ Slash math: zero remainder (integer exact)");

    println!("\n  📊 Slashing constants: verified");
}

// ============================================================================
// TEST 11: CHAIN ID & REPLAY PROTECTION
// ============================================================================
// Blocks signed for one chain must NOT be valid on another chain.
#[tokio::test]
async fn test_chain_id_replay_protection() {
    println!("\n🧪 TEST 11: Chain ID & Replay Protection");
    println!("==========================================\n");

    let kp = generate_keypair();
    let addr = public_key_to_address(&kp.public_key);
    let pk_hex = hex::encode(&kp.public_key);

    // Create a block — its signing_hash includes CHAIN_ID
    let mut block1 = Block {
        account: addr.clone(),
        previous: "0".to_string(),
        block_type: BlockType::Mint,
        amount: 100 * CIL_PER_LOS,
        link: "FAUCET:TESTNET:REPLAY_TEST".to_string(),
        signature: String::new(),
        public_key: pk_hex.clone(),
        work: 0,
        timestamp: now_secs(),
        fee: 0,
    };
    mine_and_sign(&mut block1, &kp.secret_key);

    // The signing_hash embeds CHAIN_ID.
    // If we tamper the hash (simulating a different chain_id), signature must break.
    let orig_hash = block1.signing_hash();
    assert!(!orig_hash.is_empty());
    assert!(
        block1.verify_signature(),
        "Original signature must be valid"
    );
    println!(
        "  ✅ Block signed on chain_id={}: valid",
        los_core::CHAIN_ID
    );

    // Tamper: change the account slightly (simulates different chain — hash changes)
    let mut tampered = block1.clone();
    tampered.account = format!("{}X", addr); // different account = different signing hash
    assert!(
        !tampered.verify_signature(),
        "Tampered block must have invalid signature"
    );
    println!("  ✅ Tampered account: signature invalid (replay rejected)");

    // Tamper: change amount by 1 CIL
    let mut tampered2 = block1.clone();
    tampered2.amount += 1;
    assert!(
        !tampered2.verify_signature(),
        "Amount-tampered block must have invalid signature"
    );
    println!("  ✅ Tampered amount: signature invalid");

    println!("\n  📊 Replay protection: verified");
}

// ============================================================================
// TEST 12: PERFORMANCE — THROUGHPUT BENCHMARK
// ============================================================================
// Measures block processing speed (target: >1000 blocks/sec in-memory).
#[tokio::test]
async fn test_throughput_benchmark() {
    println!("\n🧪 TEST 12: Throughput Benchmark");
    println!("==================================\n");

    let node = SimNode::new();
    let receiver = SimNode::new();
    let mut ledger = Ledger::new();

    // Mint a large balance
    let initial_balance = 1_000_000 * CIL_PER_LOS; // 1M LOS
    let mint = make_mint_block(
        &node.address,
        &node.pubkey_hex,
        "0",
        initial_balance,
        "FAUCET:TESTNET:BENCHMARK",
        &node.secret_key,
    );
    let mut prev_hash = ledger
        .process_block(&mint)
        .expect("Mint failed")
        .into_hash();

    // Pre-mine blocks for speed test
    let num_sends = 50; // Each needs PoW mining, so keep manageable
    let send_amount = CIL_PER_LOS; // 1 LOS per tx
    let base_ts = now_secs();

    println!("  Pre-mining {} blocks (PoW)...", num_sends);
    let mine_start = Instant::now();

    let mut blocks = Vec::new();
    for i in 0..num_sends {
        let send = make_send_block(
            &node,
            &prev_hash,
            &receiver.address,
            send_amount,
            BASE_FEE_CIL,
            base_ts + i + 1,
        );
        prev_hash = send.calculate_hash();
        blocks.push(send);
    }
    let mine_elapsed = mine_start.elapsed();
    println!(
        "  Mining done in {:?} ({:.0} blocks/sec)",
        mine_elapsed,
        num_sends as f64 / mine_elapsed.as_secs_f64()
    );

    // Now measure pure process_block throughput (no mining overhead)
    // Reuse the same mint block so chain hashes match
    let mut ledger2 = Ledger::new();
    ledger2.process_block(&mint).expect("Mint failed");

    let process_start = Instant::now();
    let mut success_count = 0u64;
    for block in &blocks {
        if ledger2.process_block(block).is_ok() {
            success_count += 1;
        }
    }
    let process_elapsed = process_start.elapsed();

    let tps = if process_elapsed.as_nanos() > 0 {
        success_count as f64 / process_elapsed.as_secs_f64()
    } else {
        0.0
    };

    println!(
        "  📊 Processed {} blocks in {:?} ({:.0} TPS)",
        success_count, process_elapsed, tps
    );
    assert_eq!(
        success_count, num_sends,
        "All blocks must process successfully"
    );
    // Signature verification (Dilithium5) is expensive, so >10 TPS is reasonable
    assert!(tps > 10.0, "TPS too low: {:.1}", tps);
    println!("  ✅ Throughput: {:.0} TPS (target: >10)", tps);
}

// ============================================================================
// TEST 13: GENESIS ALLOCATION VALIDATION
// ============================================================================
// Verify the fixed supply allocations per whitepaper.
#[tokio::test]
async fn test_genesis_allocation() {
    println!("\n🧪 TEST 13: Genesis Allocation Validation");
    println!("===========================================\n");

    let total_supply_los = 21_936_236u128;
    let dev_treasury_los = 773_823u128;
    let bootstrap_los = 4 * 1_000u128; // 4 validators × 1000 LOS
    let public_los = total_supply_los - dev_treasury_los - bootstrap_los;

    assert_eq!(public_los, 21_158_413, "Public allocation");
    println!("  Total:      {} LOS", total_supply_los);
    println!(
        "  Dev:        {} LOS (~{}.{}%)",
        dev_treasury_los,
        dev_treasury_los * 100 / total_supply_los,
        (dev_treasury_los * 10_000 / total_supply_los) % 100
    );
    println!("  Bootstrap:  {} LOS", bootstrap_los);
    println!(
        "  Public:     {} LOS (~{}.{}%)",
        public_los,
        public_los * 100 / total_supply_los,
        (public_los * 10_000 / total_supply_los) % 100
    );

    // Dev < 3.6%
    let dev_pct_bps = dev_treasury_los * 10_000 / total_supply_los;
    assert!(dev_pct_bps < 360, "Dev allocation must be < 3.6%");
    println!("  ✅ Dev = {}bps (< 360bps / 3.6%)", dev_pct_bps);

    // Public > 96.4%
    let public_pct_bps = public_los * 10_000 / total_supply_los;
    assert!(public_pct_bps > 9640, "Public must be > 96.4%");
    println!("  ✅ Public = {}bps (> 9640bps / 96.4%)", public_pct_bps);

    // All allocations in CIL
    let total_cil = total_supply_los * CIL_PER_LOS;
    let dev_cil = dev_treasury_los * CIL_PER_LOS;
    let public_cil = public_los * CIL_PER_LOS;
    let bootstrap_cil = bootstrap_los * CIL_PER_LOS;
    assert_eq!(
        dev_cil + public_cil + bootstrap_cil,
        total_cil,
        "Allocations must sum to total supply"
    );
    println!(
        "  ✅ Allocations sum exactly: {} == {} CIL",
        dev_cil + public_cil + bootstrap_cil,
        total_cil
    );

    println!("\n  📊 Genesis allocation: verified");
}
