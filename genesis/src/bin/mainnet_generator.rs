/// UNAUTHORITY Mainnet Genesis Generator v3.0
/// Uses CRYSTALS-Dilithium5 (Post-Quantum) via los_crypto
///
/// SECURITY CRITICAL - READ BEFORE RUNNING:
///
///   1. Run ONLY on an air-gapped machine (no network)
///   2. ALL keys are generated from OsRng (system randomness)
///   3. NO hardcoded seeds - every run produces unique keys
///   4. Back up the output JSON to encrypted offline storage IMMEDIATELY
///   5. NEVER commit mainnet genesis to version control
///   6. The .gitignore already blocks mainnet-genesis/ from being committed
///
/// v3.0 CHANGES:
///   - Corrected to 8 wallets: 4 Dev Treasury + 4 Bootstrap (~3.5% dev / ~96.5% public)
///   - Dev Treasury 1: 428,113 LOS, Dev Treasury 2: 245,710 LOS
///   - Dev Treasury 3: 50,000 LOS, Dev Treasury 4: 50,000 LOS
///   - Bootstrap: 4 x 1,000 LOS
///   - Domain separator: "los-dilithium5-keygen-v1"
///
/// After running, the output files are:
///   - mainnet-genesis/mainnet_wallets.json (FULL - contains private keys)
///   - mainnet-genesis/mainnet_public.json   (PUBLIC ONLY - safe to share)
use bip39::{Language, Mnemonic};
use rand::rngs::OsRng;
use rand::RngCore;
use std::fs;

const CIL_PER_LOS: u128 = 100_000_000_000;
const TOTAL_SUPPLY_LOS: u128 = 21_936_236;

// Genesis Allocation (~3.5% DEV / ~96.5% PUBLIC) per copilot-instructions.md
const DEV_TREASURY_1_LOS: u128 = 428_113;
const DEV_TREASURY_2_LOS: u128 = 245_710;
const DEV_TREASURY_3_LOS: u128 = 50_000;
const DEV_TREASURY_4_LOS: u128 = 50_000;
const DEV_TREASURY_TOTAL_LOS: u128 =
    DEV_TREASURY_1_LOS + DEV_TREASURY_2_LOS + DEV_TREASURY_3_LOS + DEV_TREASURY_4_LOS; // 773,823
const BOOTSTRAP_NODE_COUNT: usize = 4;
const BOOTSTRAP_NODE_STAKE_LOS: u128 = 1_000;
const TOTAL_BOOTSTRAP_LOS: u128 = BOOTSTRAP_NODE_STAKE_LOS * (BOOTSTRAP_NODE_COUNT as u128); // 4,000
const DEV_SUPPLY_TOTAL_LOS: u128 = DEV_TREASURY_TOTAL_LOS + TOTAL_BOOTSTRAP_LOS; // 777,823
const PUBLIC_SUPPLY_LOS: u128 = TOTAL_SUPPLY_LOS - DEV_SUPPLY_TOTAL_LOS; // 21,158,413

const DEV_TREASURY_COUNT: usize = 4;

/// Structured wallet data for building multiple output formats
#[allow(dead_code)]
struct WalletData {
    wallet_type: String,
    address: String,
    seed_phrase: String,
    public_key: String,
    private_key: String,
    balance_cil: u128,
    is_bootstrap: bool,
}

fn main() {
    eprintln!();
    eprintln!("!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!");
    eprintln!("!!  MAINNET GENESIS GENERATOR v3.0 - EXTREME SECURITY    !!");
    eprintln!("!!                                                        !!");
    eprintln!("!!  This produces REAL mainnet keys with REAL value.      !!");
    eprintln!("!!  Ensure you are on an AIR-GAPPED machine.              !!");
    eprintln!("!!  NEVER run this on a networked computer.               !!");
    eprintln!("!!  NEVER commit the output to git.                       !!");
    eprintln!("!!  Back up output to encrypted offline storage ONLY.     !!");
    eprintln!("!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!");
    eprintln!();

    // ===== SUPPLY VALIDATION =====
    assert_eq!(
        TOTAL_SUPPLY_LOS, 21_936_236,
        "TOTAL_SUPPLY must be 21,936,236 LOS"
    );
    assert_eq!(CIL_PER_LOS, 100_000_000_000, "CIL_PER_LOS must be 10^11");
    assert_eq!(
        DEV_TREASURY_TOTAL_LOS, 773_823,
        "Dev treasury must be 773,823 LOS"
    );
    assert_eq!(
        DEV_SUPPLY_TOTAL_LOS, 777_823,
        "Dev supply total must be 777,823 LOS"
    );
    assert_eq!(
        PUBLIC_SUPPLY_LOS, 21_158_413,
        "Public supply must be 21,158,413 LOS"
    );

    let dev_balances_los: [u128; 4] = [
        DEV_TREASURY_1_LOS,
        DEV_TREASURY_2_LOS,
        DEV_TREASURY_3_LOS,
        DEV_TREASURY_4_LOS,
    ];

    println!();
    println!("===========================================================");
    println!("  UNAUTHORITY MAINNET GENESIS GENERATOR v3.0");
    println!("  Dilithium5 Post-Quantum Crypto | OsRng Random Keys");
    println!("  8 Wallets: 4 Dev Treasury + 4 Bootstrap Validators");
    println!("  ~3.5% Dev / ~96.5% Public");
    println!("===========================================================");
    println!();

    let mut wallet_entries_full: Vec<String> = Vec::new();
    let mut wallet_entries_public: Vec<String> = Vec::new();
    let mut all_wallets: Vec<WalletData> = Vec::new();

    // ===== DEV TREASURY WALLETS =====
    println!("--- DEV TREASURY WALLETS ---\n");

    for (i, &balance_los) in dev_balances_los.iter().enumerate().take(DEV_TREASURY_COUNT) {
        let wallet_num = i + 1;

        // Generate 32 bytes of entropy from OsRng (256-bit = 24-word mnemonic)
        let mut entropy = [0u8; 32];
        OsRng.fill_bytes(&mut entropy);
        let mnemonic = Mnemonic::from_entropy_in(Language::English, &entropy)
            .expect("Failed to generate BIP39 mnemonic from entropy");
        let seed_phrase = mnemonic.to_string();

        // Derive Dilithium5 keypair deterministically from BIP39 seed
        let bip39_seed = mnemonic.to_seed("");
        let kp = los_crypto::generate_keypair_from_seed(&bip39_seed);
        let pk_hex = hex::encode(&kp.public_key);
        let sk_hex = hex::encode(&kp.secret_key);
        let address = los_crypto::public_key_to_address(&kp.public_key);

        let balance_cil = balance_los * CIL_PER_LOS;

        println!("Dev Treasury #{}:", wallet_num);
        println!("  Address:      {}", address);
        println!("  Balance:      {} LOS ({} CIL)", balance_los, balance_cil);
        println!(
            "  Seed Phrase:  {} ... (FIRST 4 WORDS SHOWN)",
            &seed_phrase
                .split_whitespace()
                .take(4)
                .collect::<Vec<_>>()
                .join(" ")
        );
        println!("  Public Key:   {}...\n", &pk_hex[..64]);

        // Full entry (with private key + seed phrase)
        wallet_entries_full.push(format!(
            "    {{\n      \"wallet_type\": \"DevTreasury({})\",\n      \"seed_phrase\": \"{}\",\n      \"address\": \"{}\",\n      \"balance_cil\": \"{}\",\n      \"balance_los\": \"{}\",\n      \"public_key\": \"{}\",\n      \"private_key\": \"{}\",\n      \"note\": \"Dev Treasury #{}\"\n    }}",
            wallet_num, seed_phrase, address, balance_cil,
            balance_los, pk_hex, sk_hex, wallet_num
        ));

        // Public entry (NO private key, NO seed phrase)
        wallet_entries_public.push(format!(
            "    {{\n      \"wallet_type\": \"DevTreasury({})\",\n      \"address\": \"{}\",\n      \"balance_cil\": \"{}\",\n      \"balance_los\": \"{}\",\n      \"public_key\": \"{}\",\n      \"note\": \"Dev Treasury #{}\"\n    }}",
            wallet_num, address, balance_cil,
            balance_los, pk_hex, wallet_num
        ));

        all_wallets.push(WalletData {
            wallet_type: format!("DevTreasury({})", wallet_num),
            address,
            seed_phrase,
            public_key: pk_hex,
            private_key: sk_hex,
            balance_cil,
            is_bootstrap: false,
        });
    }

    // ===== BOOTSTRAP VALIDATOR WALLETS =====
    println!("--- BOOTSTRAP VALIDATOR WALLETS ---\n");

    for i in 0..BOOTSTRAP_NODE_COUNT {
        let validator_num = i + 1;

        // Generate 32 bytes of entropy from OsRng (256-bit = 24-word mnemonic)
        let mut entropy = [0u8; 32];
        OsRng.fill_bytes(&mut entropy);
        let mnemonic = Mnemonic::from_entropy_in(Language::English, &entropy)
            .expect("Failed to generate BIP39 mnemonic from entropy");
        let seed_phrase = mnemonic.to_string();

        // Derive Dilithium5 keypair deterministically from BIP39 seed
        let bip39_seed = mnemonic.to_seed("");
        let kp = los_crypto::generate_keypair_from_seed(&bip39_seed);
        let pk_hex = hex::encode(&kp.public_key);
        let sk_hex = hex::encode(&kp.secret_key);
        let address = los_crypto::public_key_to_address(&kp.public_key);

        let balance_cil = BOOTSTRAP_NODE_STAKE_LOS * CIL_PER_LOS;

        println!("Bootstrap Validator #{}:", validator_num);
        println!("  Address:      {}", address);
        println!(
            "  Balance:      {} LOS ({} CIL)",
            BOOTSTRAP_NODE_STAKE_LOS, balance_cil
        );
        println!(
            "  Seed Phrase:  {} ... (FIRST 4 WORDS SHOWN)",
            &seed_phrase
                .split_whitespace()
                .take(4)
                .collect::<Vec<_>>()
                .join(" ")
        );
        println!("  Public Key:   {}...\n", &pk_hex[..64]);

        wallet_entries_full.push(format!(
            "    {{\n      \"wallet_type\": \"BootstrapNode({})\",\n      \"seed_phrase\": \"{}\",\n      \"address\": \"{}\",\n      \"balance_cil\": \"{}\",\n      \"balance_los\": \"{}\",\n      \"public_key\": \"{}\",\n      \"private_key\": \"{}\",\n      \"note\": \"Bootstrap Validator #{}\"\n    }}",
            validator_num, seed_phrase, address, balance_cil,
            BOOTSTRAP_NODE_STAKE_LOS, pk_hex, sk_hex, validator_num
        ));

        wallet_entries_public.push(format!(
            "    {{\n      \"wallet_type\": \"BootstrapNode({})\",\n      \"address\": \"{}\",\n      \"balance_cil\": \"{}\",\n      \"balance_los\": \"{}\",\n      \"public_key\": \"{}\",\n      \"note\": \"Bootstrap Validator #{}\"\n    }}",
            validator_num, address, balance_cil,
            BOOTSTRAP_NODE_STAKE_LOS, pk_hex, validator_num
        ));

        all_wallets.push(WalletData {
            wallet_type: format!("BootstrapNode({})", validator_num),
            address,
            seed_phrase,
            public_key: pk_hex,
            private_key: sk_hex,
            balance_cil,
            is_bootstrap: true,
        });
    }

    // ===== ALLOCATION SUMMARY =====
    println!("===========================================================");
    println!("MAINNET ALLOCATION SUMMARY");
    println!("===========================================================");
    println!(
        "Total Supply:     {} LOS ({} CIL)",
        TOTAL_SUPPLY_LOS,
        TOTAL_SUPPLY_LOS * CIL_PER_LOS
    );
    println!(
        "Dev Treasury:     {} LOS (T1: {} + T2: {} + T3: {} + T4: {})",
        DEV_TREASURY_TOTAL_LOS,
        DEV_TREASURY_1_LOS,
        DEV_TREASURY_2_LOS,
        DEV_TREASURY_3_LOS,
        DEV_TREASURY_4_LOS
    );
    println!(
        "Bootstrap:        {} x {} LOS = {} LOS",
        BOOTSTRAP_NODE_COUNT, BOOTSTRAP_NODE_STAKE_LOS, TOTAL_BOOTSTRAP_LOS
    );
    println!("Total Dev:        {} LOS (~3%)", DEV_SUPPLY_TOTAL_LOS);
    println!("Public:           {} LOS (~97%)", PUBLIC_SUPPLY_LOS);
    println!("===========================================================\n");

    // ===== BUILD NODE-COMPATIBLE genesis_config.json =====
    // This format matches what los-node/src/genesis.rs GenesisConfig expects
    let total_supply_cil = TOTAL_SUPPLY_LOS * CIL_PER_LOS;
    let dev_supply_total_cil = DEV_SUPPLY_TOTAL_LOS * CIL_PER_LOS;
    let genesis_timestamp = chrono::Utc::now().timestamp();

    // ===== WRITE FULL BACKUP JSON (PRIVATE) =====
    let full_json = format!(
        "{{\n  \"version\": \"2.0\",\n  \"network\": \"mainnet\",\n  \"description\": \"UNAUTHORITY MAINNET GENESIS - CONFIDENTIAL\",\n  \"warning\": \"CONTAINS PRIVATE KEYS - NEVER commit to git or share publicly!\",\n  \"crypto\": \"CRYSTALS-Dilithium5 (Post-Quantum)\",\n  \"total_supply_los\": \"{}\",\n  \"total_supply_cil\": \"{}\",\n  \"allocation\": {{\n    \"dev_treasury_total_los\": \"{}\",\n    \"dev_supply_total_los\": \"{}\",\n    \"public_supply_los\": \"{}\",\n    \"dev_percent\": \"~3%\"\n  }},\n  \"wallets\": [\n{}\n  ]\n}}",
        TOTAL_SUPPLY_LOS,
        total_supply_cil,
        DEV_TREASURY_TOTAL_LOS,
        DEV_SUPPLY_TOTAL_LOS,
        PUBLIC_SUPPLY_LOS,
        wallet_entries_full.join(",\n")
    );

    // ===== WRITE PUBLIC JSON (NO private keys, NO seeds) =====
    let public_json = format!(
        "{{\n  \"version\": \"2.0\",\n  \"network\": \"mainnet\",\n  \"description\": \"UNAUTHORITY MAINNET GENESIS - PUBLIC INFO\",\n  \"note\": \"Public addresses and balances only. No private keys or seed phrases.\",\n  \"crypto\": \"CRYSTALS-Dilithium5 (Post-Quantum)\",\n  \"total_supply_los\": \"{}\",\n  \"total_supply_cil\": \"{}\",\n  \"allocation\": {{\n    \"dev_treasury_total_los\": \"{}\",\n    \"dev_supply_total_los\": \"{}\",\n    \"public_supply_los\": \"{}\",\n    \"dev_percent\": \"~3%\"\n  }},\n  \"wallets\": [\n{}\n  ]\n}}",
        TOTAL_SUPPLY_LOS,
        total_supply_cil,
        DEV_TREASURY_TOTAL_LOS,
        DEV_SUPPLY_TOTAL_LOS,
        PUBLIC_SUPPLY_LOS,
        wallet_entries_public.join(",\n")
    );

    let output_dir = "mainnet-genesis";
    if let Err(e) = fs::create_dir_all(output_dir) {
        eprintln!("Failed to create output directory: {}", e);
        std::process::exit(1);
    }

    let full_path = format!("{}/mainnet_wallets.json", output_dir);
    let public_path = format!("{}/mainnet_public.json", output_dir);

    fs::write(&full_path, &full_json).expect("Failed to write mainnet_wallets.json");
    fs::write(&public_path, &public_json).expect("Failed to write mainnet_public.json");

    // ===== WRITE NODE-COMPATIBLE genesis_config.json =====
    // This matches los-node/src/genesis.rs GenesisConfig struct exactly:
    // - network_id: u64, network: String
    // - total_supply_cil: u128 (JSON number, not string)
    // - bootstrap_nodes: Vec with stake_cil (integer)
    // - dev_accounts: Vec with balance_cil (integer)
    let bootstrap_json_entries: Vec<String> = all_wallets
        .iter()
        .filter(|w| w.is_bootstrap)
        .map(|w| {
            format!(
                "    {{\n      \"address\": \"{}\",\n      \"stake_cil\": {},\n      \"public_key\": \"{}\"\n    }}",
                w.address, w.balance_cil, w.public_key
            )
        })
        .collect();

    let dev_json_entries: Vec<String> = all_wallets
        .iter()
        .filter(|w| !w.is_bootstrap)
        .map(|w| {
            format!(
                "    {{\n      \"address\": \"{}\",\n      \"balance_cil\": {},\n      \"public_key\": \"{}\"\n    }}",
                w.address, w.balance_cil, w.public_key
            )
        })
        .collect();

    let node_config = format!(
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
  "security_notice": "Private keys and seed phrases have been stripped. Backed up separately."
}}"#,
        genesis_timestamp,
        total_supply_cil,
        dev_supply_total_cil,
        bootstrap_json_entries.join(",\n"),
        dev_json_entries.join(",\n")
    );

    fs::write("genesis_config.json", &node_config).expect("Failed to write genesis_config.json");

    println!("OUTPUT FILES:");
    println!("  FULL (PRIVATE):        {}", full_path);
    println!("  PUBLIC ONLY:           {}", public_path);
    println!("  NODE CONFIG:           genesis_config.json");
    println!();
    eprintln!("!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!");
    eprintln!(
        "!!  BACK UP {} TO ENCRYPTED OFFLINE STORAGE NOW!  !!",
        full_path
    );
    eprintln!("!!  DELETE IT FROM THIS MACHINE AFTER BACKUP.             !!");
    eprintln!(
        "!!  The public file ({}) is safe to share.  !!",
        public_path
    );
    eprintln!("!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!");
}
