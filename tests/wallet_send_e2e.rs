/// E2E wallet send test: simulates Flutter wallet send from test wallet.
///
/// Uses the test wallet mnemonic to generate a Dilithium5 keypair,
/// constructs a PoW-signed send block, and submits to a running testnet node.
/// If this passes, the backend sign+verify is correct.
/// If it fails, there's a fundamental algorithm mismatch.
///
/// Test wallets:
/// Wallet1: "leisure steel artwork again silly fantasy ability confirm cigar naive upon like snow tank web jazz okay slot pony midnight spray fence input unveil"
///          → LOSWnrDcEDq9uXGgnfi5XEiEMPsrknCRYTKq1
///
/// Run: cargo test -p los-node wallet_send_e2e --test wallet_send_e2e -- --nocapture
use bip39::Mnemonic;
use los_core::{Block, BlockType, CHAIN_ID};
use std::str::FromStr;

const NODE_URL: &str = "http://127.0.0.1:7030";
const TEST_MNEMONIC_1: &str = "leisure steel artwork again silly fantasy ability confirm cigar naive upon like snow tank web jazz okay slot pony midnight spray fence input unveil";
const EXPECTED_ADDR_1: &str = "LOSWnrDcEDq9uXGgnfi5XEiEMPsrknCRYTKq1";
const DEST_ADDR: &str = "LOSX84MQjCL6ZaGCktyUxjj11XZ12Jkqq4JYR";
const AMOUNT_LOS: u64 = 100; // 100 LOS test send

fn solve_pow(blk: &mut Block, bits: u32) {
    for nonce in 0u64..50_000_000 {
        blk.work = nonce;
        if blk.verify_pow_bits(bits) {
            return;
        }
    }
    panic!("PoW failed");
}

trait VerifyPow {
    fn verify_pow_bits(&self, bits: u32) -> bool;
}

impl VerifyPow for Block {
    fn verify_pow_bits(&self, bits: u32) -> bool {
        let hash = self.signing_hash();
        let hash_bytes = hex::decode(&hash).unwrap_or_default();
        let mut zero_bits = 0u32;
        for byte in &hash_bytes {
            if *byte == 0 {
                zero_bits += 8;
            } else {
                zero_bits += byte.leading_zeros();
                break;
            }
        }
        zero_bits >= bits
    }
}

#[test]
#[ignore] // requires running testnet node at 127.0.0.1:7030
fn test_wallet_send_from_mnemonic() {
    // 1. Derive BIP39 seed from mnemonic (same as Flutter bip39.mnemonicToSeed)
    let mnemonic = Mnemonic::from_str(TEST_MNEMONIC_1).expect("Invalid mnemonic");
    let seed = mnemonic.to_seed(""); // empty passphrase, same as Flutter

    println!("[Step 1] BIP39 seed derived, {} bytes", seed.len());

    // 2. Generate Dilithium5 keypair deterministically (same as Flutter + los_crypto)
    let keypair = los_crypto::generate_keypair_from_seed(&seed);
    let pk_hex = hex::encode(&keypair.public_key);
    println!(
        "[Step 2] PK len = {} bytes, hex = {}...{}",
        keypair.public_key.len(),
        &pk_hex[..8],
        &pk_hex[pk_hex.len() - 8..]
    );

    // 3. Derive LOS address and check it matches expected
    let address = los_crypto::public_key_to_address(&keypair.public_key);
    println!("[Step 3] Address = {}", address);
    assert_eq!(
        address, EXPECTED_ADDR_1,
        "Address mismatch! Expected {} got {}",
        EXPECTED_ADDR_1, address
    );
    println!("✅ Address matches expected wallet");

    // 4. Fetch account state from running node
    let client = reqwest::blocking::Client::new();
    let account_resp = client
        .get(format!("{}/account/{}", NODE_URL, address))
        .send()
        .expect("GET account failed — is node running?");
    let account: serde_json::Value = account_resp.json().expect("Parse account JSON");
    let balance_cil: u128 = account["balance_cil"].as_u64().unwrap_or(0) as u128;
    let head_block = account["head_block"].as_str().unwrap_or("0").to_string();
    println!(
        "[Step 4] balance_cil = {}, head = {}",
        balance_cil, head_block
    );
    assert!(
        balance_cil > 0,
        "Wallet has no balance — fund it first with faucet"
    );

    // 5. Fetch node info for chain_id, pow bits, fee
    let info_resp = client
        .get(format!("{}/node-info", NODE_URL))
        .send()
        .expect("GET node-info failed");
    let node_info: serde_json::Value = info_resp.json().expect("Parse node-info JSON");
    let chain_id_node = node_info["protocol"]["chain_id_numeric"]
        .as_u64()
        .unwrap_or(2);
    let pow_bits = node_info["protocol"]["pow_difficulty_bits"]
        .as_u64()
        .unwrap_or(16) as u32;
    let base_fee = node_info["protocol"]["base_fee_cil"]
        .as_u64()
        .unwrap_or(100_000) as u128;
    println!(
        "[Step 5] chain_id={}, pow_bits={}, base_fee={}",
        chain_id_node, pow_bits, base_fee
    );
    assert_eq!(
        chain_id_node, CHAIN_ID,
        "CHAIN_ID mismatch! node={} backend={}",
        chain_id_node, CHAIN_ID
    );

    // 6. Build the block
    let amount_cil = AMOUNT_LOS as u128 * los_core::CIL_PER_LOS;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut blk = Block {
        account: address.clone(),
        previous: head_block.clone(),
        block_type: BlockType::Send,
        amount: amount_cil,
        link: DEST_ADDR.to_string(),
        signature: String::new(),
        public_key: pk_hex.clone(),
        work: 0,
        timestamp,
        fee: base_fee,
    };

    // 7. Mine PoW
    println!("[Step 7] Mining PoW ({} bits)...", pow_bits);
    let pow_start = std::time::Instant::now();
    solve_pow(&mut blk, pow_bits);
    println!(
        "[Step 7] PoW found: nonce={}, time={:?}",
        blk.work,
        pow_start.elapsed()
    );

    // 8. Compute signing_hash
    let signing_hash = blk.signing_hash();
    println!("[Step 8] signing_hash = {}", signing_hash);

    // 9. Sign with Dilithium5 (same as Flutter DilithiumService.sign(utf8.encode(signingHash)))
    let sig_bytes = los_crypto::sign_message(signing_hash.as_bytes(), &keypair.secret_key)
        .expect("Signing failed");
    let sig_hex = hex::encode(&sig_bytes);
    println!(
        "[Step 9] sig bytes = {}, sig hex len = {}",
        sig_bytes.len(),
        sig_hex.len()
    );

    // 10. Set signature on block, verify locally
    blk.signature = sig_hex.clone();
    let local_verify = blk.verify_signature();
    println!("[Step 10] Local verify_signature() = {}", local_verify);
    assert!(local_verify, "Local sign+verify FAILED — crypto issue!");

    // 11. Submit to node
    let body = serde_json::json!({
        "from": address,
        "target": DEST_ADDR,
        "amount": AMOUNT_LOS,
        "amount_cil": amount_cil,
        "signature": sig_hex,
        "public_key": pk_hex,
        "previous": head_block,
        "work": blk.work,
        "timestamp": timestamp,
        "fee": base_fee,
    });

    println!("[Step 11] Submitting to {}/send ...", NODE_URL);
    let resp = client
        .post(format!("{}/send", NODE_URL))
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .expect("POST /send failed");

    let result: serde_json::Value = resp.json().expect("Parse send response");
    println!(
        "[Step 11] Response = {}",
        serde_json::to_string_pretty(&result).unwrap_or_default()
    );

    assert_eq!(
        result["status"].as_str().unwrap_or(""),
        "success",
        "Send FAILED: {}",
        result["msg"].as_str().unwrap_or("unknown error")
    );

    println!("✅ E2E wallet send SUCCESS!");
}
