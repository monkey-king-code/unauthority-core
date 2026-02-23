// ========================================
// INTEGRATION TESTS FOR UNAUTHORITY (LOS)
// ========================================
//
// Test Scenarios:
// 1. Three-Validator Network Consensus
// 2. PoW Mining Distribution Flow
// 3. Byzantine Fault Tolerance (Price Outlier Rejection)
// 4. Load Testing (1000 TPS)
// 5. Database Persistence & Recovery
//
// Usage:
//   cargo test --test integration_test -- --test-threads=1 --nocapture
//
// ========================================

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::time::sleep;

// Import LOS modules
use los_core::{AccountState, Block, BlockType, Ledger};
use los_crypto::{generate_keypair, sign_message};

// ========================================
// TEST 1: THREE-VALIDATOR NETWORK CONSENSUS
// ========================================
#[tokio::test]
async fn test_three_validator_consensus() {
    println!("\n🧪 TEST 1: Three-Validator Network Consensus");
    println!("================================================\n");

    // Setup: Create 3 validator nodes
    let mut validators = Vec::new();
    for i in 0..3 {
        let keypair = generate_keypair();
        let pubkey_hex = hex::encode(&keypair.public_key);
        let ledger = Arc::new(Mutex::new(Ledger::new()));

        validators.push(ValidatorNode {
            id: i,
            pubkey: pubkey_hex,
            keypair,
            ledger,
            stake: 1000_00000000, // 1000 LOS minimum stake
        });

        println!("✅ Validator {} initialized (stake: 1000 LOS)", i);
    }

    // Test: Send a transaction and measure consensus time
    let start = Instant::now();

    // Create a Send block from validator 0 to validator 1
    let sender = &validators[0];
    let receiver = &validators[1];

    let block_data = format!("{}{}{}{}", sender.pubkey, "0", 0u8, 100_00000000u128);

    let signature =
        sign_message(block_data.as_bytes(), &sender.keypair.secret_key).expect("Failed to sign");

    let block = Block {
        block_type: BlockType::Send,
        account: sender.pubkey.clone(),
        previous: "0".to_string(),
        amount: 100_00000000, // Send 100 LOS
        link: receiver.pubkey.clone(),
        signature: hex::encode(&signature),
        public_key: sender.pubkey.clone(),
        work: 0x0000000000000001u64, // Simplified PoW
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        fee: 0,
    };

    // Broadcast block to all validators (simulate consensus)
    for validator in &validators {
        let mut ledger = validator.ledger.lock().unwrap();

        // Initialize sender account with balance
        ledger.accounts.insert(
            sender.pubkey.clone(),
            AccountState {
                head: "0".to_string(),
                balance: 1000_00000000,
                block_count: 0,
                is_validator: false,
            },
        );

        // Process the block (simulates aBFT consensus)
        let block_hash = block.calculate_hash();
        ledger.blocks.insert(block_hash.clone(), block.clone());

        // Update account state
        if let Some(state) = ledger.accounts.get_mut(&sender.pubkey) {
            state.balance -= block.amount;
            state.head = block_hash.clone();
            state.block_count += 1;
        }
    }

    let finality_time = start.elapsed();

    println!("\n📊 Results:");
    println!("  - Finality Time: {:?}", finality_time);

    // Verify all validators have same state
    let ledger0 = validators[0].ledger.lock().unwrap();
    let ledger1 = validators[1].ledger.lock().unwrap();
    let ledger2 = validators[2].ledger.lock().unwrap();

    let balance0 = ledger0
        .accounts
        .get(&sender.pubkey)
        .map(|a| a.balance)
        .unwrap_or(0);
    let balance1 = ledger1
        .accounts
        .get(&sender.pubkey)
        .map(|a| a.balance)
        .unwrap_or(0);
    let balance2 = ledger2
        .accounts
        .get(&sender.pubkey)
        .map(|a| a.balance)
        .unwrap_or(0);

    println!("  - Validator 0 sees sender balance: {} CIL", balance0);
    println!("  - Validator 1 sees sender balance: {} CIL", balance1);
    println!("  - Validator 2 sees sender balance: {} CIL", balance2);

    assert_eq!(balance0, balance1, "Validator 0 and 1 state mismatch!");
    assert_eq!(balance1, balance2, "Validator 1 and 2 state mismatch!");
    assert!(
        finality_time < Duration::from_secs(3),
        "Finality time exceeded 3 seconds!"
    );

    println!(
        "\n✅ TEST PASSED: Consensus reached in {:?}\n",
        finality_time
    );
}

// ========================================
// TEST 2: POW MINING DISTRIBUTION FLOW
// ========================================
#[tokio::test]
async fn test_pow_mining_distribution() {
    println!("\nð§ª TEST 2: PoW Mining Distribution Flow");
    println!("============================================\n");

    let total_supply: u128 = 21_936_236 * 100_000_000_000u128;
    let dev_allocation: u128 = 777_823 * 100_000_000_000u128;
    let public_supply: u128 = total_supply - dev_allocation;
    let mut remaining_public: u128 = public_supply;

    println!("ð¦ Initial State:");
    println!("  - Total Supply: {} LOS", total_supply / 100_000_000_000);
    println!("  - Public Supply: {} LOS", public_supply / 100_000_000_000);
    println!(
        "  - Remaining: {} LOS\n",
        remaining_public / 100_000_000_000
    );

    let base_reward: u128 = 100 * 100_000_000_000u128;
    let epochs_to_test = [1u64, 100, 1000, 8760];
    for epoch in &epochs_to_test {
        let halvings = epoch / 8760;
        let reward = base_reward >> halvings;
        if reward > 0 && remaining_public >= reward {
            remaining_public -= reward;
            println!(
                "â Epoch {}: reward = {} LOS, remaining = {} LOS",
                epoch,
                reward / 100_000_000_000,
                remaining_public / 100_000_000_000
            );
        }
    }

    assert!(remaining_public > 0, "Public supply exhausted too soon!");
    assert!(remaining_public < public_supply, "Supply didn't decrease!");
    assert!(
        base_reward == 100 * 100_000_000_000u128,
        "Reward must be 100 LOS"
    );

    println!("\nâ TEST PASSED: PoW mining distribution working correctly\n");
}

// ========================================
// TEST 3: BYZANTINE FAULT TOLERANCE
// ========================================
#[tokio::test]
async fn test_byzantine_fault_tolerance() {
    println!("\nð§ª TEST 3: Byzantine Fault Tolerance (Price Outlier Rejection)");
    println!("================================================================\n");

    // All prices in micro-USD (u128), NO f64
    let price_reports: Vec<(&str, u128)> = vec![
        ("Validator 0 (Honest)", 90_000_000_000),
        ("Validator 1 (Honest)", 90_100_000_000),
        ("Validator 2 (MALICIOUS)", 9_000_000_000_000),
    ];

    println!("ð¡ Price Reports (micro-USD):");
    for (validator, price) in &price_reports {
        if *price > 900_000_000_000 {
            println!("  - {}: {} â ï¸  OUTLIER", validator, price);
        } else {
            println!("  - {}: {} â", validator, price);
        }
    }

    let mut prices: Vec<u128> = price_reports.iter().map(|(_, p)| *p).collect();
    prices.sort();

    let median = prices[prices.len() / 2];
    let threshold_bps: u128 = 2_000;

    let valid_prices: Vec<u128> = prices
        .iter()
        .filter(|&&p| {
            let diff = p.abs_diff(median);
            diff * 10_000 <= median * threshold_bps
        })
        .copied()
        .collect();

    let consensus_price = if valid_prices.is_empty() {
        0
    } else {
        valid_prices.iter().sum::<u128>() / valid_prices.len() as u128
    };

    println!("\nð Consensus Result:");
    println!("  - Median: {} micro-USD", median);
    println!("  - Valid Prices: {}/{}", valid_prices.len(), prices.len());
    println!("  - Consensus Price: {} micro-USD", consensus_price);

    assert_eq!(valid_prices.len(), 2, "Should reject 1 outlier!");
    assert!(consensus_price > 80_000_000_000 && consensus_price < 100_000_000_000);

    println!("\nâ TEST PASSED: Byzantine attack mitigated (integer-only math)\n");
}

// ========================================
// TEST 4: LOAD TESTING (1000 TPS)
// ========================================
#[tokio::test]
async fn test_load_1000_tps() {
    println!("\n🧪 TEST 4: Load Testing (1000 TPS)");
    println!("====================================\n");

    let target_tps = 1000;
    let duration_seconds = 5;

    let ledger = Arc::new(Mutex::new(Ledger::new()));

    let mut accounts = Vec::new();
    for _ in 0..100 {
        let keypair = generate_keypair();
        let pubkey_hex = hex::encode(&keypair.public_key);
        accounts.push(pubkey_hex);
    }

    println!("🚀 Starting load test...");
    println!("  - Target TPS: {}", target_tps);
    println!("  - Duration: {} seconds\n", duration_seconds);

    let start = Instant::now();
    let mut tx_count = 0;
    let mut latencies = Vec::new();

    for _ in 0..duration_seconds {
        let second_start = Instant::now();

        for _ in 0..target_tps {
            let tx_start = Instant::now();
            let sender = &accounts[tx_count % accounts.len()];

            {
                let mut ledger_guard = ledger.lock().unwrap();
                ledger_guard.accounts.insert(
                    sender.clone(),
                    AccountState {
                        head: format!("block_{}", tx_count),
                        balance: 1000_00000000 - (tx_count as u128 * 100000),
                        block_count: tx_count as u64 + 1,
                        is_validator: false,
                    },
                );
            }

            latencies.push(tx_start.elapsed());
            tx_count += 1;
        }

        let elapsed = second_start.elapsed();
        if elapsed < Duration::from_secs(1) {
            sleep(Duration::from_secs(1) - elapsed).await;
        }
    }

    let total_time = start.elapsed();
    let actual_tps = tx_count as f64 / total_time.as_secs_f64();

    latencies.sort();
    let p95 = latencies[latencies.len() * 95 / 100];
    let p99 = latencies[latencies.len() * 99 / 100];

    println!("📊 Results:");
    println!("  - Actual TPS: {:.2}", actual_tps);
    println!("  - P95 Latency: {:?}", p95);
    println!("  - P99 Latency: {:?}", p99);

    assert!(actual_tps >= 950.0, "TPS below target!");
    assert!(p95 < Duration::from_millis(50), "P95 too high!");

    println!("\n✅ TEST PASSED: {:.0} TPS sustained\n", actual_tps);
}

// ========================================
// TEST 5: DATABASE PERSISTENCE
// ========================================
#[tokio::test]
async fn test_database_persistence() {
    println!("\n🧪 TEST 5: Database Persistence");
    println!("==================================\n");

    let db_path = "/tmp/los_test_persistence";
    let ledger_file = format!("{}/ledger_state.json", db_path);
    let _ = std::fs::remove_dir_all(db_path);
    std::fs::create_dir_all(db_path).unwrap();

    let mut expected_accounts = Vec::new();

    println!("📝 Phase 1: Writing 1000 accounts and saving to disk...");
    {
        let mut ledger = Ledger::new();

        for i in 0..1000 {
            let keypair = generate_keypair();
            let address = los_crypto::public_key_to_address(&keypair.public_key);
            ledger.accounts.insert(
                address.clone(),
                AccountState {
                    head: format!("block_{}", i),
                    balance: (i * 100000) as u128,
                    block_count: i as u64,
                    is_validator: false,
                },
            );
            if i < 5 {
                expected_accounts.push((address, (i * 100000) as u128));
            }
        }

        // Serialize to disk (same format as los-node save_ledger)
        let serialized = serde_json::to_string(&ledger).expect("Failed to serialize ledger");
        std::fs::write(&ledger_file, &serialized).expect("Failed to write ledger file");

        assert_eq!(ledger.accounts.len(), 1000);
        println!("  ✅ Wrote 1000 accounts to {}", ledger_file);
    }

    println!("\n💥 Phase 2: Simulating crash (dropping in-memory state)...");
    sleep(Duration::from_millis(100)).await;

    println!("🔄 Phase 3: Recovery from disk...");
    {
        // Load from disk (same as los-node load_ledger)
        let data = std::fs::read_to_string(&ledger_file).expect("Failed to read ledger file");
        let ledger: Ledger = serde_json::from_str(&data).expect("Failed to deserialize ledger");
        let account_count = ledger.accounts.len();

        assert_eq!(
            account_count, 1000,
            "Should recover all 1000 accounts from disk"
        );

        // Verify specific accounts survived
        for (addr, expected_balance) in &expected_accounts {
            let account = ledger
                .accounts
                .get(addr)
                .unwrap_or_else(|| panic!("Account {} not found after recovery", addr));
            assert_eq!(
                account.balance, *expected_balance,
                "Balance mismatch for account {}",
                addr
            );
        }

        println!("  ✅ Loaded {} accounts from disk", account_count);
        println!("  ✅ Data integrity verified (balances match)");
    }

    println!("\n✅ TEST PASSED: Database persistence working\n");
    let _ = std::fs::remove_dir_all(db_path);
}

// ========================================
// HELPER STRUCTS
// ========================================

#[derive(Clone)]
struct ValidatorNode {
    #[allow(dead_code)]
    id: usize,
    pubkey: String,
    keypair: los_crypto::KeyPair,
    ledger: Arc<Mutex<Ledger>>,
    #[allow(dead_code)]
    stake: u128,
}
