use crate::commands::common::load_wallet_keypair;
use crate::{print_error, print_info, print_success, TxCommands};
use colored::*;
use los_core::{Block, BlockType, CIL_PER_LOS, MIN_POW_DIFFICULTY_BITS};
use std::path::Path;

pub async fn handle(
    action: TxCommands,
    rpc: &str,
    config_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        TxCommands::Send { to, amount, from } => {
            send_tx(&to, amount, &from, rpc, config_dir).await?
        }
        TxCommands::Status { hash } => query_status(&hash, rpc).await?,
    }
    Ok(())
}

/// Compute PoW nonce for anti-spam (16 leading zero bits)
pub(crate) fn compute_pow(block: &mut Block) {
    let mut nonce: u64 = 0;
    loop {
        block.work = nonce;
        let hash_hex = block.signing_hash();
        let hash_bytes = hex::decode(&hash_hex).unwrap_or_default();

        // Count leading zero bits
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

async fn send_tx(
    to: &str,
    amount: u64,
    from_wallet: &str,
    rpc: &str,
    config_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    print_info(&format!("Sending {} LOS to {}...", amount, to));

    // Validate recipient address
    if !los_crypto::validate_address(to) {
        print_error("Invalid recipient address format. Must be Base58Check with LOS prefix.");
        return Ok(());
    }

    // 1. Load wallet & decrypt keypair
    print_info("Loading wallet and decrypting keypair...");
    let (sender_addr, keypair) = load_wallet_keypair(from_wallet, config_dir)?;
    print_success("Wallet loaded.");

    // 2. Query sender's current account state (previous block hash + balance)
    let client = reqwest::Client::new();
    let account_url = format!("{}/balance/{}", rpc, sender_addr);
    let account_resp = client.get(&account_url).send().await?;
    if !account_resp.status().is_success() {
        print_error(&format!(
            "Failed to query account: HTTP {}",
            account_resp.status()
        ));
        return Ok(());
    }
    let account_data: serde_json::Value = account_resp.json().await?;
    let previous = account_data["head"].as_str().unwrap_or("0").to_string();
    // Use string-based balance to avoid f64 precision loss (C-02 fix)
    let balance_cil: u128 = account_data["balance_cil_str"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| account_data["balance_cil"].as_u64().unwrap_or(0) as u128);

    // 2b. Query fee estimate
    let fee_url = format!("{}/fee-estimate/{}", rpc, sender_addr);
    let fee_cil: u128 = match client.get(&fee_url).send().await {
        Ok(resp) if resp.status().is_success() => {
            let fee_data: serde_json::Value = resp.json().await?;
            fee_data["estimated_fee_cil"]
                .as_u64()
                .map(|v| v as u128)
                .unwrap_or(100_000) // Fallback to base fee
        }
        _ => 100_000, // Default base fee
    };

    let amount_cil = (amount as u128)
        .checked_mul(CIL_PER_LOS)
        .ok_or("Amount overflow")?;

    if balance_cil < amount_cil {
        print_error(&format!(
            "Insufficient balance: have {} CIL, need {} CIL",
            balance_cil, amount_cil
        ));
        return Ok(());
    }

    // 3. Build Send block
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();

    let mut block = Block {
        account: sender_addr.clone(),
        previous,
        block_type: BlockType::Send,
        amount: amount_cil,
        link: to.to_string(),
        signature: String::new(),
        public_key: hex::encode(&keypair.public_key),
        work: 0,
        timestamp,
        fee: fee_cil, // Include proper fee from fee-estimate
    };

    // 4. Compute PoW (anti-spam)
    print_info("Computing Proof-of-Work...");
    compute_pow(&mut block);
    print_success(&format!("PoW solved (nonce: {})", block.work));

    // 5. Sign with Dilithium5
    print_info("Signing with Dilithium5...");
    let signing_hash = block.signing_hash();
    let signature = los_crypto::sign_message(signing_hash.as_bytes(), &keypair.secret_key)
        .map_err(|e| format!("Signing failed: {:?}", e))?;
    block.signature = hex::encode(&signature);

    // 6. Submit to node via REST API
    print_info("Broadcasting transaction...");
    let send_url = format!("{}/send", rpc);
    let payload = serde_json::json!({
        "from": sender_addr,
        "target": to,
        "amount": amount,
        "amount_cil": amount_cil,
        "previous": block.previous,
        "signature": block.signature,
        "public_key": block.public_key,
        "work": block.work,
        "timestamp": block.timestamp,
        "fee": block.fee,
    });

    let resp = client.post(&send_url).json(&payload).send().await?;
    let resp_data: serde_json::Value = resp.json().await?;

    if resp_data["status"].as_str() == Some("ok")
        || resp_data["status"].as_str() == Some("confirmed")
        || resp_data["status"].as_str() == Some("success")
    {
        let block_hash = resp_data["hash"]
            .as_str()
            .or_else(|| resp_data["tx_hash"].as_str())
            .unwrap_or("unknown");
        println!();
        print_success("Transaction sent successfully!");
        println!("  {} {}", "Block Hash:".bold(), block_hash.green());
        println!(
            "  {} {} → {}",
            "Transfer:".bold(),
            sender_addr.dimmed(),
            to.green()
        );
        println!("  {} {} LOS", "Amount:".bold(), amount.to_string().cyan());
    } else {
        let msg = resp_data["msg"].as_str().unwrap_or("Unknown error");
        print_error(&format!("Transaction failed: {}", msg));
    }

    Ok(())
}

async fn query_status(tx_hash: &str, rpc: &str) -> Result<(), Box<dyn std::error::Error>> {
    print_info(&format!("Querying transaction {}...", tx_hash));

    let client = reqwest::Client::new();
    let url = format!("{}/tx/{}", rpc, tx_hash);

    match client.get(&url).send().await {
        Ok(response) => {
            if response.status().is_success() {
                let data: serde_json::Value = response.json().await?;

                println!();
                println!("{} {}", "Transaction Hash:".bold(), tx_hash.green());
                println!(
                    "{} {}",
                    "Status:".bold(),
                    if data["confirmed"].as_bool().unwrap_or(false) {
                        "Confirmed ✓".green()
                    } else {
                        "Pending...".yellow()
                    }
                );
                println!(
                    "{} {}",
                    "Block Height:".bold(),
                    data["block_height"].as_u64().unwrap_or(0)
                );
                println!(
                    "{} {} → {}",
                    "Transfer:".bold(),
                    data["from"].as_str().unwrap_or("Unknown").dimmed(),
                    data["to"].as_str().unwrap_or("Unknown").dimmed()
                );
                println!(
                    "{} {} LOS",
                    "Amount:".bold(),
                    data["amount"].as_u64().unwrap_or(0).to_string().cyan()
                );
            } else {
                print_error(&format!(
                    "Transaction not found: HTTP {}",
                    response.status()
                ));
            }
        }
        Err(e) => {
            print_error(&format!("Network error: {}", e));
        }
    }

    Ok(())
}
