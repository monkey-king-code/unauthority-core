/// UNAUTHORITY Testnet Genesis Generator v11.0
/// Uses CRYSTALS-Dilithium5 (Post-Quantum) via los_crypto
///
/// DETERMINISTIC: Keypairs ARE derived from BIP39 seeds via
/// domain-separated SHA-256 → ChaCha20 DRBG → pqcrypto_dilithium::keypair()
/// Same seed phrase → same keypair → same address → importable in wallet!
///
/// v11.0 CHANGES:
///   - Corrected to 8 wallets: 4 Dev Treasury + 4 Bootstrap (~3.5% dev / ~96.5% public)
///   - Dev Treasury 1: 428,113 LOS, Dev Treasury 2: 245,710 LOS
///   - Dev Treasury 3: 50,000 LOS, Dev Treasury 4: 50,000 LOS
///   - Bootstrap: 4 x 1,000 LOS
///   - Domain separator: "los-dilithium5-keygen-v1"
///
/// SECURITY: This binary is for TESTNET ONLY. Mainnet genesis must be
/// generated offline with fresh random keys, NEVER from committed seeds.
/// The seed phrases below are PUBLIC testnet keys — they have zero value.
use bip39::{Language, Mnemonic};
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

// TESTNET v11.0 SEED PHRASES — 8 wallets (4 dev treasury + 4 bootstrap)
// PUBLIC — Safe to share. These seeds have ZERO real-world value.
const TESTNET_SEEDS: [&str; 8] = [
    // Dev Treasury #1 (428,113 LOS)
    "PURGED_MNEMONIC",
    // Dev Treasury #2 (245,710 LOS)
    "PURGED_MNEMONIC",
    // Dev Treasury #3 (50,000 LOS)
    "PURGED_MNEMONIC",
    // Dev Treasury #4 (50,000 LOS)
    "year coconut innocent alert ugly nice leave agree similar easily neglect simple home illegal method riot pudding clean thumb actual install quantum magic distance",
    // Bootstrap Validator #1 (1,000 LOS)
    "hurt shuffle ring barely want stock neither siren vapor stomach desert design antenna spread envelope remove joy faith veteran dinosaur they spin guess various",
    // Bootstrap Validator #2 (1,000 LOS)
    "material toy scissors input illness sadness dignity tenant verb pond fashion beef grant swear elbow jacket embrace rather wolf quote own genuine junior junk",
    // Bootstrap Validator #3 (1,000 LOS)
    "index load clarify exhibit about moral chef phone beyond canoe asset timber pear sample boil motion flag slush range exhibit fossil lock toe cute",
    // Bootstrap Validator #4 (1,000 LOS)
    "electric social attract amount powder ramp drop mixed cup unique witness ramp execute pelican circle fuel evolve domain west nothing wrist salt nasty crowd",
];

fn main() {
    println!("\n╔════════════════════════════════════════════════════════════╗");
    println!("║   UNAUTHORITY TESTNET GENESIS GENERATOR v11.0             ║");
    println!("║   Dilithium5 Post-Quantum Crypto                          ║");
    println!("║   ~3.5% Dev / ~96.5% Public — 8 Wallets                   ║");
    println!("║   PUBLIC - Safe to Share and Commit                       ║");
    println!("╚════════════════════════════════════════════════════════════╝");
    println!("\n8 Wallets: 4 Dev Treasury + 4 Bootstrap Validators\n");

    // Supply validation assertions
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
    let mut wallet_entries: Vec<String> = Vec::new();

    println!("===================================================");
    println!("TESTNET DEV TREASURY WALLETS (Dilithium5 Post-Quantum)");
    println!("===================================================\n");

    for (i, &seed_phrase) in TESTNET_SEEDS[..DEV_TREASURY_COUNT].iter().enumerate() {
        let wallet_num = i + 1;

        // Validate seed phrase is valid BIP39
        let mnemonic = Mnemonic::parse_in_normalized(Language::English, seed_phrase)
            .expect("Invalid BIP39 seed phrase");

        // DETERMINISTIC: Derive Dilithium5 keypair from BIP39 seed
        let bip39_seed = mnemonic.to_seed("");
        let kp = los_crypto::generate_keypair_from_seed(&bip39_seed);
        let pk_hex = hex::encode(&kp.public_key);
        let sk_hex = hex::encode(&kp.secret_key);
        let address = los_crypto::public_key_to_address(&kp.public_key);

        let balance_los = dev_balances_los[i];
        let balance_cil = balance_los * CIL_PER_LOS;

        println!("Dev Treasury #{}:", wallet_num);
        println!("  Address:      {}", address);
        println!("  Balance:      {} LOS ({} CIL)", balance_los, balance_cil);
        println!("  Seed Phrase:  {}", seed_phrase);
        println!("  Public Key:   {}...\n", &pk_hex[..64]);

        wallet_entries.push(format!(
            "    {{\n      \"wallet_type\": \"DevTreasury({})\",\n      \"seed_phrase\": \"{}\",\n      \"address\": \"{}\",\n      \"balance_cil\": \"{}\",\n      \"balance_los\": \"{}\",\n      \"public_key\": \"{}\",\n      \"private_key\": \"{}\",\n      \"note\": \"Dev Treasury #{}\"\n    }}",
            wallet_num, seed_phrase, address, balance_cil,
            balance_los, pk_hex, sk_hex, wallet_num
        ));
    }

    println!("===================================================");
    println!("TESTNET BOOTSTRAP VALIDATORS (Dilithium5 Post-Quantum)");
    println!("===================================================\n");

    for i in 0..BOOTSTRAP_NODE_COUNT {
        let validator_num = i + 1;
        let seed_index = DEV_TREASURY_COUNT + i;
        let seed_phrase = TESTNET_SEEDS[seed_index];

        let mnemonic_parsed = Mnemonic::parse_in_normalized(Language::English, seed_phrase)
            .expect("Invalid BIP39 seed phrase");

        // DETERMINISTIC: Derive Dilithium5 keypair from BIP39 seed
        let bip39_seed = mnemonic_parsed.to_seed("");
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
        println!("  Seed Phrase:  {}", seed_phrase);
        println!("  Public Key:   {}...\n", &pk_hex[..64]);

        wallet_entries.push(format!(
            "    {{\n      \"wallet_type\": \"BootstrapNode({})\",\n      \"seed_phrase\": \"{}\",\n      \"address\": \"{}\",\n      \"balance_cil\": \"{}\",\n      \"balance_los\": \"{}\",\n      \"public_key\": \"{}\",\n      \"private_key\": \"{}\",\n      \"note\": \"Bootstrap Validator #{}\"\n    }}",
            validator_num, seed_phrase, address, balance_cil,
            BOOTSTRAP_NODE_STAKE_LOS, pk_hex, sk_hex, validator_num
        ));
    }

    println!("===================================================");
    println!("ALLOCATION SUMMARY (TESTNET)");
    println!("===================================================");
    println!("Total Supply:     {} LOS", TOTAL_SUPPLY_LOS);
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
    println!("===================================================\n");

    // Build JSON manually
    let wallets_json = wallet_entries.join(",\n");
    let json = format!(
        "{{\n  \"version\": \"11.0\",\n  \"network\": \"testnet\",\n  \"description\": \"Public testnet genesis v11.0 - 8 wallets (4 dev treasury + 4 bootstrap validators)\",\n  \"warning\": \"FOR TESTNET ONLY - NEVER use these seeds on mainnet!\",\n  \"crypto\": \"CRYSTALS-Dilithium5 (Post-Quantum)\",\n  \"note\": \"BIP39 seeds deterministically derive Dilithium5 keypairs. Same seed = same address.\",\n  \"allocation\": {{\n    \"total_supply_los\": \"{}\",\n    \"dev_treasury_total_los\": \"{}\",\n    \"dev_supply_total_los\": \"{}\",\n    \"public_supply_los\": \"{}\",\n    \"dev_percent\": \"~3.5%\"\n  }},\n  \"wallets\": [\n{}\n  ]\n}}",
        TOTAL_SUPPLY_LOS,
        DEV_TREASURY_TOTAL_LOS,
        DEV_SUPPLY_TOTAL_LOS,
        PUBLIC_SUPPLY_LOS,
        wallets_json
    );

    let output_path = "testnet-genesis/testnet_wallets.json";
    if let Err(e) = fs::create_dir_all("testnet-genesis") {
        eprintln!("Warning: Could not create directory: {}", e);
    }
    fs::write(output_path, &json).expect("Failed to write testnet_wallets.json");

    println!("Testnet genesis saved to: {}", output_path);
    println!("8 wallets with Dilithium5 post-quantum addresses");
    println!("Safe to commit to git - these are PUBLIC testnet seeds\n");
}
