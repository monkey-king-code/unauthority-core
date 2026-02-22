use crate::{print_error, print_info, QueryCommands};
use colored::*;
use los_core::CIL_PER_LOS;

pub async fn handle(action: QueryCommands, rpc: &str) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        QueryCommands::Block { height } => query_block(height, rpc).await?,
        QueryCommands::Account { address } => query_account(&address, rpc).await?,
        QueryCommands::Info => query_info(rpc).await?,
        QueryCommands::Validators => query_validators(rpc).await?,
    }
    Ok(())
}

async fn query_block(height: u64, rpc: &str) -> Result<(), Box<dyn std::error::Error>> {
    print_info(&format!("Fetching block #{}...", height));

    let client = reqwest::Client::new();
    let url = format!("{}/block/{}", rpc, height);

    match client.get(&url).send().await {
        Ok(response) => {
            if response.status().is_success() {
                let data: serde_json::Value = response.json().await?;

                println!();
                println!("{} {}", "Block Height:".bold(), height.to_string().cyan());
                println!(
                    "{} {}",
                    "Hash:".bold(),
                    data["hash"].as_str().unwrap_or("Unknown").green()
                );
                println!(
                    "{} {}",
                    "Timestamp:".bold(),
                    data["timestamp"].as_u64().unwrap_or(0)
                );
                println!(
                    "{} {}",
                    "Proposer:".bold(),
                    data["proposer"].as_str().unwrap_or("Unknown")
                );
                println!(
                    "{} {}",
                    "Transactions:".bold(),
                    data["tx_count"].as_u64().unwrap_or(0)
                );
            } else {
                print_error(&format!("Block not found: HTTP {}", response.status()));
            }
        }
        Err(e) => {
            print_error(&format!("Network error: {}", e));
        }
    }

    Ok(())
}

async fn query_account(address: &str, rpc: &str) -> Result<(), Box<dyn std::error::Error>> {
    print_info(&format!("Fetching account {}...", address));

    let client = reqwest::Client::new();
    let url = format!("{}/account/{}", rpc, address);

    match client.get(&url).send().await {
        Ok(response) => {
            if response.status().is_success() {
                let data: serde_json::Value = response.json().await?;

                let balance_cil = data["balance"].as_u64().unwrap_or(0) as u128;
                // Use precise string formatting to avoid f64 precision loss for large balances.
                let balance_los_str = format!(
                    "{}.{:011}",
                    balance_cil / CIL_PER_LOS,
                    balance_cil % CIL_PER_LOS
                );

                println!();
                println!("{} {}", "Address:".bold(), address.green());
                println!(
                    "{} {} LOS",
                    "Balance:".bold(),
                    balance_los_str.cyan().bold()
                );
                println!(
                    "{} {}",
                    "Nonce:".bold(),
                    data["nonce"].as_u64().unwrap_or(0)
                );
            } else {
                print_error(&format!("Account not found: HTTP {}", response.status()));
            }
        }
        Err(e) => {
            print_error(&format!("Network error: {}", e));
        }
    }

    Ok(())
}

async fn query_info(rpc: &str) -> Result<(), Box<dyn std::error::Error>> {
    print_info("Fetching network info...");

    let client = reqwest::Client::new();
    let url = format!("{}/node-info", rpc);

    match client.get(&url).send().await {
        Ok(response) => {
            if response.status().is_success() {
                let data: serde_json::Value = response.json().await?;

                println!();
                println!("{}", "═══ NETWORK INFO ═══".cyan().bold());
                println!();
                println!(
                    "{} {}",
                    "Chain ID:".bold(),
                    data["chain_id"].as_str().unwrap_or("Unknown")
                );
                println!(
                    "{} {}",
                    "Version:".bold(),
                    data["version"].as_str().unwrap_or("Unknown")
                );
                println!(
                    "{} {}",
                    "Block Height:".bold(),
                    data["block_height"]
                        .as_u64()
                        .unwrap_or(0)
                        .to_string()
                        .cyan()
                );
                println!("{} {}", "Total Supply:".bold(), "21,936,236 LOS".green());
                println!(
                    "{} {}",
                    "Active Validators:".bold(),
                    data["validator_count"].as_u64().unwrap_or(0)
                );
                println!(
                    "{} {}",
                    "Peer Count:".bold(),
                    data["peer_count"].as_u64().unwrap_or(0)
                );
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

async fn query_validators(rpc: &str) -> Result<(), Box<dyn std::error::Error>> {
    print_info("Fetching validators...");

    let client = reqwest::Client::new();
    let url = format!("{}/validators", rpc);

    match client.get(&url).send().await {
        Ok(response) => {
            if response.status().is_success() {
                let data: serde_json::Value = response.json().await?;

                if let Some(validators) = data["validators"].as_array() {
                    println!();
                    println!("{}", "═══ ACTIVE VALIDATORS ═══".cyan().bold());
                    println!();

                    for (i, validator) in validators.iter().enumerate() {
                        let address = validator["address"].as_str().unwrap_or("Unknown");
                        let stake = validator["stake"].as_u64().unwrap_or(0);

                        println!(
                            "{}. {} ({})",
                            (i + 1).to_string().dimmed(),
                            address.green(),
                            format!("{} LOS", stake).cyan()
                        );
                    }

                    println!();
                    println!(
                        "{} {}",
                        "Total:".bold(),
                        validators.len().to_string().cyan()
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
