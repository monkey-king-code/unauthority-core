use crate::commands::common::load_wallet_keypair;
use crate::{print_error, print_info, print_success, ValidatorCommands};
use colored::*;
use std::path::Path;

pub async fn handle(
    action: ValidatorCommands,
    rpc: &str,
    config_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        ValidatorCommands::Stake { amount, wallet } => {
            stake(amount, &wallet, rpc, config_dir).await?
        }
        ValidatorCommands::Unstake { wallet } => unstake(&wallet, rpc, config_dir).await?,
        ValidatorCommands::Status { address } => show_status(&address, rpc).await?,
        ValidatorCommands::List => list_validators(rpc).await?,
    }
    Ok(())
}

async fn stake(
    amount: u64,
    wallet_name: &str,
    rpc: &str,
    config_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if amount < 1000 {
        print_error("Minimum stake is 1,000 LOS!");
        return Ok(());
    }

    print_info(&format!(
        "Staking {} LOS from wallet '{}'...",
        amount, wallet_name
    ));

    // Load wallet & decrypt
    let (address, keypair) = load_wallet_keypair(wallet_name, config_dir)?;
    print_success("Wallet loaded.");

    // Sign a validator registration proof
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let message = format!("REGISTER_VALIDATOR:{}:{}", address, timestamp);
    let signature = los_crypto::sign_message(message.as_bytes(), &keypair.secret_key)
        .map_err(|e| format!("Signing failed: {:?}", e))?;

    // POST /register-validator
    let client = reqwest::Client::new();
    let url = format!("{}/register-validator", rpc);
    let payload = serde_json::json!({
        "address": address,
        "public_key": hex::encode(&keypair.public_key),
        "signature": hex::encode(&signature),
        "timestamp": timestamp,
        "stake_amount": amount,
    });

    let resp = client.post(&url).json(&payload).send().await?;
    let resp_data: serde_json::Value = resp.json().await?;

    if resp_data["status"].as_str() == Some("ok")
        || resp_data["status"].as_str() == Some("registered")
    {
        println!();
        print_success(&format!("Validator registered with {} LOS stake!", amount));
        println!("  {} {}", "Address:".bold(), address.green());
        println!(
            "  {} {}",
            "Status:".bold(),
            "Active validator".green().bold()
        );
    } else {
        let msg = resp_data["msg"].as_str().unwrap_or("Unknown error");
        print_error(&format!("Staking failed: {}", msg));
    }

    Ok(())
}

async fn unstake(
    wallet_name: &str,
    rpc: &str,
    config_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    print_info(&format!("Unstaking from wallet '{}'...", wallet_name));

    // Load wallet & decrypt
    let (address, keypair) = load_wallet_keypair(wallet_name, config_dir)?;
    print_success("Wallet loaded.");

    // Sign an unregister proof
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let message = format!("UNREGISTER_VALIDATOR:{}:{}", address, timestamp);
    let signature = los_crypto::sign_message(message.as_bytes(), &keypair.secret_key)
        .map_err(|e| format!("Signing failed: {:?}", e))?;

    // POST /unregister-validator (voluntary unstake)
    let client = reqwest::Client::new();
    let url = format!("{}/unregister-validator", rpc);
    let payload = serde_json::json!({
        "address": address,
        "public_key": hex::encode(&keypair.public_key),
        "signature": hex::encode(&signature),
        "timestamp": timestamp,
    });

    let resp = client.post(&url).json(&payload).send().await?;
    let resp_data: serde_json::Value = resp.json().await?;

    if resp_data["status"].as_str() == Some("ok")
        || resp_data["status"].as_str() == Some("unregistered")
    {
        println!();
        print_success("Validator unregistered successfully!");
        println!("  {} {}", "Address:".bold(), address.green());
        println!(
            "  {}",
            "Stake will be returned after cooldown period.".dimmed()
        );
    } else {
        let msg = resp_data["msg"].as_str().unwrap_or("Unknown error");
        print_error(&format!("Unstaking failed: {}", msg));
    }

    Ok(())
}

async fn show_status(address: &str, rpc: &str) -> Result<(), Box<dyn std::error::Error>> {
    print_info(&format!("Querying validator status for {}...", address));

    let client = reqwest::Client::new();
    let url = format!("{}/validators", rpc);

    match client.get(&url).send().await {
        Ok(response) => {
            if response.status().is_success() {
                let data: serde_json::Value = response.json().await?;

                // Find validator in list
                if let Some(validators) = data["validators"].as_array() {
                    if let Some(validator) = validators
                        .iter()
                        .find(|v| v["address"].as_str() == Some(address))
                    {
                        println!();
                        println!("{} {}", "Address:".bold(), address.green());
                        println!(
                            "{} {} LOS",
                            "Stake:".bold(),
                            validator["stake"].as_u64().unwrap_or(0).to_string().cyan()
                        );
                        println!(
                            "{} {}",
                            "Active:".bold(),
                            if validator["active"].as_bool().unwrap_or(false) {
                                "Yes".green()
                            } else {
                                "No".red()
                            }
                        );
                        print_success("Validator found!");
                    } else {
                        print_error("Validator not found in active set.");
                    }
                }
            } else {
                print_error(&format!("Failed to query: HTTP {}", response.status()));
            }
        }
        Err(e) => {
            print_error(&format!("Network error: {}", e));
        }
    }

    Ok(())
}

async fn list_validators(rpc: &str) -> Result<(), Box<dyn std::error::Error>> {
    print_info("Fetching active validators...");

    let client = reqwest::Client::new();
    let url = format!("{}/validators", rpc);

    match client.get(&url).send().await {
        Ok(response) => {
            if response.status().is_success() {
                let data: serde_json::Value = response.json().await?;

                if let Some(validators) = data["validators"].as_array() {
                    println!();
                    println!("{}", "Active Validators:".bold());
                    println!();

                    for (i, validator) in validators.iter().enumerate() {
                        let address = validator["address"].as_str().unwrap_or("Unknown");
                        let stake = validator["stake"].as_u64().unwrap_or(0);

                        println!("  {}. {}", (i + 1).to_string().cyan(), address.green());
                        println!("     {}: {} LOS", "Stake".dimmed(), stake);
                        println!();
                    }

                    println!(
                        "{} {} {}",
                        "Total:".bold(),
                        validators.len().to_string().cyan(),
                        "validator(s)".dimmed()
                    );
                }
            } else {
                print_error(&format!("Failed to query: HTTP {}", response.status()));
            }
        }
        Err(e) => {
            print_error(&format!("Network error: {}", e));
        }
    }

    Ok(())
}
