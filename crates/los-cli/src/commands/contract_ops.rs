// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// SHARED CONTRACT OPERATIONS — Deploy & Call helpers (client-signed)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use crate::commands::common::load_wallet_keypair;
use crate::print_info;
use base64::Engine as _;
use los_core::{
    Block, BlockType, DEFAULT_GAS_LIMIT, GAS_PRICE_CIL, MIN_CALL_FEE_CIL, MIN_DEPLOY_FEE_CIL,
    MIN_POW_DIFFICULTY_BITS,
};
use std::collections::BTreeMap;
use std::path::Path;

/// Compute PoW nonce for anti-spam (16 leading zero bits).
fn compute_pow(block: &mut Block) {
    let mut nonce: u64 = 0;
    loop {
        block.work = nonce;
        let hash_hex = block.signing_hash();
        let hash_bytes = hex::decode(&hash_hex).unwrap_or_default();

        let mut zero_bits: u32 = 0;
        for byte in &hash_bytes {
            if *byte == 0 {
                zero_bits += 8;
            } else {
                zero_bits += byte.leading_zeros();
                break;
            }
        }

        if zero_bits >= MIN_POW_DIFFICULTY_BITS {
            return;
        }
        nonce += 1;
    }
}

/// Query account's current head block hash from the node.
async fn query_previous(
    client: &reqwest::Client,
    rpc: &str,
    address: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let url = format!("{}/balance/{}", rpc, address);
    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Ok("0".to_string());
    }
    let data: serde_json::Value = resp.json().await?;
    Ok(data["head"].as_str().unwrap_or("0").to_string())
}

/// Deploy a WASM contract (client-signed).
///
/// Returns `(contract_address, block_hash)` on success.
pub async fn deploy_contract(
    wallet_name: &str,
    wasm_path: &str,
    initial_state: BTreeMap<String, String>,
    amount_cil: u128,
    rpc: &str,
    config_dir: &Path,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    // 1. Load wallet
    let (sender_addr, keypair) = load_wallet_keypair(wallet_name, config_dir)?;

    // 2. Read WASM bytecode
    let bytecode = std::fs::read(wasm_path)
        .map_err(|e| format!("Failed to read WASM file '{}': {}", wasm_path, e))?;
    let bytecode_b64 = base64::engine::general_purpose::STANDARD.encode(&bytecode);

    print_info(&format!(
        "WASM bytecode: {} bytes ({})",
        bytecode.len(),
        wasm_path
    ));

    // 3. Query previous block hash
    let client = reqwest::Client::new();
    let previous = query_previous(&client, rpc, &sender_addr).await?;

    // 4. Build ContractDeploy block
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();

    let code_hash = { hex::encode(&blake3::hash(&bytecode).as_bytes()[0..32]) };
    let link = format!("DEPLOY:{}", code_hash);

    let mut block = Block {
        account: sender_addr.clone(),
        previous,
        block_type: BlockType::ContractDeploy,
        amount: amount_cil,
        link,
        signature: String::new(),
        public_key: hex::encode(&keypair.public_key),
        work: 0,
        timestamp,
        fee: MIN_DEPLOY_FEE_CIL,
    };

    // 5. PoW
    print_info("Computing Proof-of-Work...");
    compute_pow(&mut block);

    // 6. Sign
    print_info("Signing with Dilithium5...");
    let signing_hash = block.signing_hash();
    let signature = los_crypto::sign_message(signing_hash.as_bytes(), &keypair.secret_key)
        .map_err(|e| format!("Signing failed: {:?}", e))?;
    block.signature = hex::encode(&signature);

    // 7. Submit
    print_info("Broadcasting deploy transaction...");
    let url = format!("{}/deploy-contract", rpc);

    let initial_state_opt = if initial_state.is_empty() {
        None
    } else {
        Some(&initial_state)
    };

    let payload = serde_json::json!({
        "owner": sender_addr,
        "bytecode": bytecode_b64,
        "initial_state": initial_state_opt,
        "amount_cil": amount_cil,
        "signature": block.signature,
        "public_key": block.public_key,
        "previous": block.previous,
        "work": block.work,
        "timestamp": block.timestamp,
        "fee": block.fee,
    });

    let resp = client.post(&url).json(&payload).send().await?;
    let data: serde_json::Value = resp.json().await?;

    if data["status"].as_str() == Some("success") {
        let contract_addr = data["contract_address"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        let block_hash = data["block_hash"].as_str().unwrap_or("unknown").to_string();
        Ok((contract_addr, block_hash))
    } else {
        let msg = data["msg"].as_str().unwrap_or("Unknown error");
        Err(format!("Deploy failed: {}", msg).into())
    }
}

/// Call a smart contract function (client-signed).
///
/// Returns the full JSON response on success.
#[allow(clippy::too_many_arguments)]
pub async fn call_contract(
    wallet_name: &str,
    contract_address: &str,
    function: &str,
    args: Vec<String>,
    gas_limit: Option<u64>,
    amount_cil: u128,
    rpc: &str,
    config_dir: &Path,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    // 1. Load wallet
    let (sender_addr, keypair) = load_wallet_keypair(wallet_name, config_dir)?;

    // 2. Query previous block hash
    let client = reqwest::Client::new();
    let previous = query_previous(&client, rpc, &sender_addr).await?;

    // 3. Build ContractCall block
    let gas = gas_limit.unwrap_or(DEFAULT_GAS_LIMIT);
    let fee = MIN_CALL_FEE_CIL.max((gas as u128).saturating_mul(GAS_PRICE_CIL));
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();

    let args_json = serde_json::to_string(&args)?;
    let args_b64 = base64::engine::general_purpose::STANDARD.encode(args_json.as_bytes());
    let link = format!("CALL:{}:{}:{}", contract_address, function, args_b64);

    let mut block = Block {
        account: sender_addr.clone(),
        previous,
        block_type: BlockType::ContractCall,
        amount: amount_cil,
        link,
        signature: String::new(),
        public_key: hex::encode(&keypair.public_key),
        work: 0,
        timestamp,
        fee,
    };

    // 4. PoW
    print_info("Computing Proof-of-Work...");
    compute_pow(&mut block);

    // 5. Sign
    print_info("Signing with Dilithium5...");
    let signing_hash = block.signing_hash();
    let signature = los_crypto::sign_message(signing_hash.as_bytes(), &keypair.secret_key)
        .map_err(|e| format!("Signing failed: {:?}", e))?;
    block.signature = hex::encode(&signature);

    // 6. Submit
    print_info("Broadcasting contract call...");
    let url = format!("{}/call-contract", rpc);
    let payload = serde_json::json!({
        "contract_address": contract_address,
        "function": function,
        "args": args,
        "gas_limit": gas,
        "caller": sender_addr,
        "amount_cil": amount_cil,
        "signature": block.signature,
        "public_key": block.public_key,
        "previous": block.previous,
        "work": block.work,
        "timestamp": block.timestamp,
        "fee": fee,
    });

    let resp = client.post(&url).json(&payload).send().await?;
    let data: serde_json::Value = resp.json().await?;

    if data["status"].as_str() == Some("success") {
        Ok(data)
    } else {
        let msg = data["msg"].as_str().unwrap_or("Unknown error");
        Err(format!("Contract call failed: {}", msg).into())
    }
}
