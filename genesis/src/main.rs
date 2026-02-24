use bip39::Mnemonic;
use rand::Rng;
use std::fs::File;
use std::io::Write;

const CIL_PER_LOS: u128 = 100_000_000_000; // 10^11 CIL per LOS
const TOTAL_SUPPLY_CIL: u128 = 21_936_236 * CIL_PER_LOS;

// Genesis Allocation (~3.5% DEV / ~96.5% PUBLIC)
// Dev Treasury 1:  428,113 LOS
// Dev Treasury 2:  245,710 LOS
// Dev Treasury 3:   50,000 LOS
// Dev Treasury 4:   50,000 LOS
// Bootstrap Nodes: 4 × 1,000 = 4,000 LOS
// Total Dev:       777,823 LOS (~3.5%)
// Public:          21,158,413 LOS (~96.5%)
const DEV_TREASURY_1_CIL: u128 = 428_113 * CIL_PER_LOS;
const DEV_TREASURY_2_CIL: u128 = 245_710 * CIL_PER_LOS;
const DEV_TREASURY_3_CIL: u128 = 50_000 * CIL_PER_LOS;
const DEV_TREASURY_4_CIL: u128 = 50_000 * CIL_PER_LOS;
const DEV_TREASURY_TOTAL_CIL: u128 =
    DEV_TREASURY_1_CIL + DEV_TREASURY_2_CIL + DEV_TREASURY_3_CIL + DEV_TREASURY_4_CIL; // 773,823 LOS
const BOOTSTRAP_NODE_COUNT: usize = 4;
const ALLOCATION_PER_BOOTSTRAP_NODE_CIL: u128 = 1_000 * CIL_PER_LOS;
const TOTAL_BOOTSTRAP_ALLOCATION_CIL: u128 =
    ALLOCATION_PER_BOOTSTRAP_NODE_CIL * (BOOTSTRAP_NODE_COUNT as u128);
const DEV_SUPPLY_TOTAL_CIL: u128 = DEV_TREASURY_TOTAL_CIL + TOTAL_BOOTSTRAP_ALLOCATION_CIL; // 777,823 LOS

#[derive(Clone)]
struct DevWallet {
    wallet_type: WalletType,
    address: String,
    seed_phrase: String,
    private_key: String,
    public_key: String,
    balance_cil: u128,
}

#[derive(Clone, Debug)]
enum WalletType {
    DevTreasury(u8),
    BootstrapNode(u8),
}

fn main() {
    println!("\n╔════════════════════════════════════════════════════════════╗");
    println!("║   UNAUTHORITY GENESIS GENERATOR v5.0 (PRODUCTION)         ║");
    println!("╚════════════════════════════════════════════════════════════╝");
    println!("\n8 Wallets: 4 Dev Treasury + 4 Bootstrap Validators (~3.5% Dev / ~96.5% Public)\n");

    // Supply validation
    assert_eq!(DEV_TREASURY_1_CIL / CIL_PER_LOS, 428_113);
    assert_eq!(DEV_TREASURY_2_CIL / CIL_PER_LOS, 245_710);
    assert_eq!(DEV_TREASURY_3_CIL / CIL_PER_LOS, 50_000);
    assert_eq!(DEV_TREASURY_4_CIL / CIL_PER_LOS, 50_000);
    assert_eq!(DEV_TREASURY_TOTAL_CIL / CIL_PER_LOS, 773_823);
    assert_eq!(DEV_SUPPLY_TOTAL_CIL / CIL_PER_LOS, 777_823);
    let public_los = (TOTAL_SUPPLY_CIL - DEV_SUPPLY_TOTAL_CIL) / CIL_PER_LOS;
    assert_eq!(public_los, 21_158_413);

    let mut wallets: Vec<DevWallet> = Vec::new();
    let mut total_allocated_cil: u128 = 0;

    // Dev Treasury #1 (428,113 LOS)
    {
        let (seed_phrase, priv_key, pub_key) = generate_keys("dev-treasury-1");
        let address = derive_address(&pub_key);
        wallets.push(DevWallet {
            wallet_type: WalletType::DevTreasury(1),
            address,
            seed_phrase,
            private_key: priv_key,
            public_key: pub_key,
            balance_cil: DEV_TREASURY_1_CIL,
        });
        total_allocated_cil += DEV_TREASURY_1_CIL;
    }

    // Dev Treasury #2 (245,710 LOS)
    {
        let (seed_phrase, priv_key, pub_key) = generate_keys("dev-treasury-2");
        let address = derive_address(&pub_key);
        wallets.push(DevWallet {
            wallet_type: WalletType::DevTreasury(2),
            address,
            seed_phrase,
            private_key: priv_key,
            public_key: pub_key,
            balance_cil: DEV_TREASURY_2_CIL,
        });
        total_allocated_cil += DEV_TREASURY_2_CIL;
    }

    // Dev Treasury #3 (50,000 LOS)
    {
        let (seed_phrase, priv_key, pub_key) = generate_keys("dev-treasury-3");
        let address = derive_address(&pub_key);
        wallets.push(DevWallet {
            wallet_type: WalletType::DevTreasury(3),
            address,
            seed_phrase,
            private_key: priv_key,
            public_key: pub_key,
            balance_cil: DEV_TREASURY_3_CIL,
        });
        total_allocated_cil += DEV_TREASURY_3_CIL;
    }

    // Dev Treasury #4 (50,000 LOS)
    {
        let (seed_phrase, priv_key, pub_key) = generate_keys("dev-treasury-4");
        let address = derive_address(&pub_key);
        wallets.push(DevWallet {
            wallet_type: WalletType::DevTreasury(4),
            address,
            seed_phrase,
            private_key: priv_key,
            public_key: pub_key,
            balance_cil: DEV_TREASURY_4_CIL,
        });
        total_allocated_cil += DEV_TREASURY_4_CIL;
    }

    // Bootstrap Validators #1-#4 (1,000 LOS each)
    for i in 1..=BOOTSTRAP_NODE_COUNT {
        let (seed_phrase, priv_key, pub_key) = generate_keys(&format!("bootstrap-node-{}", i));
        let address = derive_address(&pub_key);
        wallets.push(DevWallet {
            wallet_type: WalletType::BootstrapNode(i as u8),
            address,
            seed_phrase,
            private_key: priv_key,
            public_key: pub_key,
            balance_cil: ALLOCATION_PER_BOOTSTRAP_NODE_CIL,
        });
        total_allocated_cil += ALLOCATION_PER_BOOTSTRAP_NODE_CIL;
    }

    println!("═══════════════════════════════════════════════════════════");
    println!("DEV TREASURY WALLETS");
    println!("═══════════════════════════════════════════════════════════\n");
    for wallet in wallets
        .iter()
        .filter(|w| matches!(w.wallet_type, WalletType::DevTreasury(_)))
    {
        print_wallet(wallet);
    }

    println!("═══════════════════════════════════════════════════════════");
    println!("BOOTSTRAP VALIDATOR NODES");
    println!("═══════════════════════════════════════════════════════════\n");
    for wallet in wallets
        .iter()
        .filter(|w| matches!(w.wallet_type, WalletType::BootstrapNode(_)))
    {
        print_wallet(wallet);
    }

    println!("═══════════════════════════════════════════════════════════");
    println!("SUPPLY VERIFICATION");
    println!("═══════════════════════════════════════════════════════════");
    println!(
        "Target:    {} CIL ({} LOS)",
        DEV_SUPPLY_TOTAL_CIL,
        DEV_SUPPLY_TOTAL_CIL / CIL_PER_LOS
    );
    println!(
        "Allocated: {} CIL ({} LOS)",
        total_allocated_cil,
        total_allocated_cil / CIL_PER_LOS
    );

    if total_allocated_cil == DEV_SUPPLY_TOTAL_CIL {
        println!("Status: ✅ MATCH\n");
    } else {
        println!("Status: ❌ MISMATCH!\n");
        std::process::exit(1);
    }

    println!("═══════════════════════════════════════════════════════════");
    println!("🔒 SECURITY INSTRUCTIONS (CRITICAL)");
    println!("═══════════════════════════════════════════════════════════");
    println!("1. BACKUP ALL SEED PHRASES IMMEDIATELY (write on paper)");
    println!("2. Store genesis_config.json in ENCRYPTED cold storage");
    println!("3. NEVER commit genesis_config.json to public Git");
    println!("4. For Bootstrap Nodes:");
    println!("   - Open Validator Dashboard");
    println!("   - Click 'Import Existing Keys'");
    println!("   - Paste seed phrase OR private key");
    println!("   - Node registers as validator with >= 1 LOS (reward eligibility requires >= 1000 LOS)\n");

    generate_config(&wallets);

    println!("✅ Genesis config saved: genesis/genesis_config.json");
    println!("⚠️  WARNING: This file contains private keys! Keep secure!\n");
}

fn generate_keys(label: &str) -> (String, String, String) {
    let mut rng = rand::thread_rng();
    let entropy: [u8; 32] = rng.gen();
    let mnemonic = Mnemonic::from_entropy(&entropy).expect("Failed to generate mnemonic");
    let seed_phrase = mnemonic.to_string();
    let bip39_seed = mnemonic.to_seed("");
    let keypair = los_crypto::generate_keypair_from_seed(&bip39_seed);
    let private_key = hex::encode(&keypair.secret_key);
    let public_key = hex::encode(&keypair.public_key);
    println!(
        "✓ Generated deterministic Dilithium5 keypair for: {}",
        label
    );
    (seed_phrase, private_key, public_key)
}

fn derive_address(pub_key_hex: &str) -> String {
    let public_key = hex::decode(pub_key_hex).expect("Failed to decode public key hex");
    los_crypto::public_key_to_address(&public_key)
}

fn print_wallet(w: &DevWallet) {
    let label = match &w.wallet_type {
        WalletType::DevTreasury(n) => format!("DEV TREASURY #{}", n),
        WalletType::BootstrapNode(n) => format!("BOOTSTRAP NODE #{}", n),
    };
    let balance_los = w.balance_cil / CIL_PER_LOS;
    println!("┌─────────────────────────────────────────────────────────┐");
    println!("│ Type: {:<50} │", label);
    println!("├─────────────────────────────────────────────────────────┤");
    println!("│ Address:  {:<46} │", w.address);
    println!("│ Balance:  {:<46} │", format!("{} LOS", balance_los));
    println!("├─────────────────────────────────────────────────────────┤");
    println!("│ SEED PHRASE (24 words):                                 │");
    let words: Vec<&str> = w.seed_phrase.split_whitespace().collect();
    for chunk in words.chunks(6) {
        println!("│ {:<56} │", chunk.join(" "));
    }
    println!("├─────────────────────────────────────────────────────────┤");
    println!(
        "│ Private Key: {}...{} │",
        &w.private_key[0..24],
        &w.private_key[w.private_key.len() - 24..]
    );
    println!(
        "│ Public Key:  {}...{} │",
        &w.public_key[0..24],
        &w.public_key[w.public_key.len() - 24..]
    );
    println!("└─────────────────────────────────────────────────────────┘");
    println!();
}

fn generate_config(wallets: &[DevWallet]) {
    let bootstrap: Vec<_> = wallets
        .iter()
        .filter(|w| matches!(w.wallet_type, WalletType::BootstrapNode(_)))
        .map(|w| {
            format!(
                r#"    {{
      "address": "{}",
      "stake_cil": {},
      "seed_phrase": "{}",
      "private_key": "{}",
      "public_key": "{}"
    }}"#,
                w.address, w.balance_cil, w.seed_phrase, w.private_key, w.public_key
            )
        })
        .collect();

    let dev: Vec<_> = wallets
        .iter()
        .filter(|w| matches!(w.wallet_type, WalletType::DevTreasury(_)))
        .map(|w| {
            format!(
                r#"    {{
      "address": "{}",
      "balance_cil": {},
      "seed_phrase": "{}",
      "private_key": "{}",
      "public_key": "{}"
    }}"#,
                w.address, w.balance_cil, w.seed_phrase, w.private_key, w.public_key
            )
        })
        .collect();

    let config = format!(
        r#"{{
  "network_id": 1,
  "network": "mainnet",
  "chain_name": "Unauthority",
  "ticker": "LOS",
  "genesis_timestamp": {},
  "total_supply_cil": {},
  "dev_supply_cil": {},
  "bootstrap_nodes": [
{}
  ],
  "dev_accounts": [
{}
  ],
  "security_notice": "⚠️ CRITICAL: This file contains private keys! Store in encrypted cold storage. NEVER commit to public repository!"
}}
"#,
        chrono::Utc::now().timestamp(),
        TOTAL_SUPPLY_CIL,
        DEV_SUPPLY_TOTAL_CIL,
        bootstrap.join(",\n"),
        dev.join(",\n")
    );

    let mut file = File::create("genesis_config.json").expect("Failed to create config");
    file.write_all(config.as_bytes())
        .expect("Failed to write config");
}
