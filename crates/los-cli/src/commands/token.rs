use crate::commands::contract_ops;
use crate::{print_error, print_info, print_success, TokenCommands};
use colored::*;
use std::path::Path;

pub async fn handle(
    action: TokenCommands,
    rpc: &str,
    config_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        TokenCommands::List => list_tokens(rpc).await?,
        TokenCommands::Info { address } => token_info(&address, rpc).await?,
        TokenCommands::Balance { token, holder } => token_balance(&token, &holder, rpc).await?,
        TokenCommands::Allowance {
            token,
            owner,
            spender,
        } => token_allowance(&token, &owner, &spender, rpc).await?,
        TokenCommands::Deploy {
            wallet,
            wasm,
            name,
            symbol,
            decimals,
            total_supply,
            max_supply,
            is_wrapped,
            wrapped_origin,
            bridge_operator,
        } => {
            token_deploy(
                &wallet,
                &wasm,
                &name,
                &symbol,
                decimals,
                &total_supply,
                max_supply.as_deref(),
                is_wrapped,
                wrapped_origin.as_deref(),
                bridge_operator.as_deref(),
                rpc,
                config_dir,
            )
            .await?
        }
        TokenCommands::Mint {
            wallet,
            token,
            to,
            amount,
        } => token_mint(&wallet, &token, &to, &amount, rpc, config_dir).await?,
        TokenCommands::Transfer {
            wallet,
            token,
            to,
            amount,
        } => token_transfer(&wallet, &token, &to, &amount, rpc, config_dir).await?,
        TokenCommands::Approve {
            wallet,
            token,
            spender,
            amount,
        } => token_approve(&wallet, &token, &spender, &amount, rpc, config_dir).await?,
        TokenCommands::Burn {
            wallet,
            token,
            amount,
        } => token_burn(&wallet, &token, &amount, rpc, config_dir).await?,
    }
    Ok(())
}

async fn list_tokens(rpc: &str) -> Result<(), Box<dyn std::error::Error>> {
    print_info("Querying USP-01 tokens...");

    let client = reqwest::Client::new();
    let url = format!("{}/tokens", rpc);

    match client.get(&url).send().await {
        Ok(response) => {
            if response.status().is_success() {
                let data: serde_json::Value = response.json().await?;
                let count = data["count"].as_u64().unwrap_or(0);

                if count == 0 {
                    print_info("No USP-01 tokens deployed yet.");
                    return Ok(());
                }

                println!("{}", "USP-01 Tokens:".bold());
                println!();

                if let Some(tokens) = data["tokens"].as_array() {
                    for token in tokens {
                        let name = token["name"].as_str().unwrap_or("Unknown");
                        let symbol = token["symbol"].as_str().unwrap_or("???");
                        let contract = token["contract"].as_str().unwrap_or("Unknown");
                        let total_supply = token["total_supply"].as_u64().unwrap_or(0);
                        let decimals = token["decimals"].as_u64().unwrap_or(0);
                        let is_wrapped = token["is_wrapped"].as_bool().unwrap_or(false);

                        println!("  {} {} ({})", "•".cyan(), name.bold(), symbol.yellow());
                        println!("    {}: {}", "Contract".dimmed(), contract.green());
                        println!(
                            "    {}: {} ({} decimals)",
                            "Supply".dimmed(),
                            total_supply.to_string().cyan(),
                            decimals
                        );
                        if is_wrapped {
                            let origin = token["wrapped_origin"].as_str().unwrap_or("unknown");
                            println!(
                                "    {}: {} ({})",
                                "Type".dimmed(),
                                "Wrapped Asset".yellow(),
                                origin
                            );
                        }
                        println!();
                    }
                }

                println!(
                    "{} {} {}",
                    "Total:".bold(),
                    count.to_string().cyan(),
                    "token(s)".dimmed()
                );
            } else {
                print_error(&format!("Server error: {}", response.status()));
            }
        }
        Err(e) => print_error(&format!("Connection failed: {}", e)),
    }
    Ok(())
}

async fn token_info(address: &str, rpc: &str) -> Result<(), Box<dyn std::error::Error>> {
    print_info(&format!("Querying token info for {}...", address));

    let client = reqwest::Client::new();
    let url = format!("{}/token/{}", rpc, address);

    match client.get(&url).send().await {
        Ok(response) => {
            if response.status().is_success() {
                let data: serde_json::Value = response.json().await?;

                if data["status"].as_str() == Some("error") {
                    print_error(data["msg"].as_str().unwrap_or("Unknown error"));
                    return Ok(());
                }

                let token = &data["token"];
                let name = token["name"].as_str().unwrap_or("Unknown");
                let symbol = token["symbol"].as_str().unwrap_or("???");
                let decimals = token["decimals"].as_u64().unwrap_or(0);
                let total_supply = token["total_supply"].as_u64().unwrap_or(0);
                let max_supply = token["max_supply"].as_u64().unwrap_or(0);
                let is_wrapped = token["is_wrapped"].as_bool().unwrap_or(false);
                let owner = token["owner"].as_str().unwrap_or("Unknown");
                let contract = token["contract"].as_str().unwrap_or(address);

                println!();
                println!("{}", "USP-01 Token Info".bold().underline());
                println!();
                println!(
                    "  {}: {} ({})",
                    "Name".bold(),
                    name.green(),
                    symbol.yellow()
                );
                println!("  {}: {}", "Contract".bold(), contract.green());
                println!("  {}: {}", "Owner".bold(), owner);
                println!("  {}: {}", "Decimals".bold(), decimals);
                println!(
                    "  {}: {}",
                    "Total Supply".bold(),
                    total_supply.to_string().cyan()
                );
                if max_supply > 0 {
                    println!(
                        "  {}: {}",
                        "Max Supply".bold(),
                        max_supply.to_string().cyan()
                    );
                }
                if is_wrapped {
                    let origin = token["wrapped_origin"].as_str().unwrap_or("unknown");
                    let bridge = token["bridge_operator"].as_str().unwrap_or("none");
                    println!(
                        "  {}: {} ({})",
                        "Type".bold(),
                        "Wrapped Asset".yellow(),
                        origin
                    );
                    println!("  {}: {}", "Bridge Operator".bold(), bridge);
                }
                println!("  {}: {}", "Standard".bold(), "USP-01".cyan());
                println!();
            } else {
                print_error(&format!("Server error: {}", response.status()));
            }
        }
        Err(e) => print_error(&format!("Connection failed: {}", e)),
    }
    Ok(())
}

async fn token_balance(
    token: &str,
    holder: &str,
    rpc: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    print_info(&format!("Querying token balance for {}...", holder));

    let client = reqwest::Client::new();
    let url = format!("{}/token/{}/balance/{}", rpc, token, holder);

    match client.get(&url).send().await {
        Ok(response) => {
            if response.status().is_success() {
                let data: serde_json::Value = response.json().await?;

                if data["status"].as_str() == Some("error") {
                    print_error(data["msg"].as_str().unwrap_or("Unknown error"));
                    return Ok(());
                }

                let balance = data["balance"].as_str().unwrap_or("0");
                println!();
                println!("  {}: {}", "Token".bold(), token.green());
                println!("  {}: {}", "Holder".bold(), holder);
                println!("  {}: {}", "Balance".bold(), balance.cyan().bold());
                println!();

                print_success("Balance retrieved successfully");
            } else {
                print_error(&format!("Server error: {}", response.status()));
            }
        }
        Err(e) => print_error(&format!("Connection failed: {}", e)),
    }
    Ok(())
}

async fn token_allowance(
    token: &str,
    owner: &str,
    spender: &str,
    rpc: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    print_info(&format!("Querying allowance: {} -> {}...", owner, spender));

    let client = reqwest::Client::new();
    let url = format!("{}/token/{}/allowance/{}/{}", rpc, token, owner, spender);

    match client.get(&url).send().await {
        Ok(response) => {
            if response.status().is_success() {
                let data: serde_json::Value = response.json().await?;

                if data["status"].as_str() == Some("error") {
                    print_error(data["msg"].as_str().unwrap_or("Unknown error"));
                    return Ok(());
                }

                let allowance = data["allowance"].as_str().unwrap_or("0");
                println!();
                println!("  {}: {}", "Token".bold(), token.green());
                println!("  {}: {}", "Owner".bold(), owner);
                println!("  {}: {}", "Spender".bold(), spender);
                println!("  {}: {}", "Allowance".bold(), allowance.cyan().bold());
                println!();

                print_success("Allowance retrieved successfully");
            } else {
                print_error(&format!("Server error: {}", response.status()));
            }
        }
        Err(e) => print_error(&format!("Connection failed: {}", e)),
    }
    Ok(())
}
// ─────────────────────────────────────────────────────────────
// WRITE OPERATIONS — Deploy, Mint, Transfer, Approve, Burn
// ─────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn token_deploy(
    wallet: &str,
    wasm_path: &str,
    name: &str,
    symbol: &str,
    decimals: u8,
    total_supply: &str,
    max_supply: Option<&str>,
    is_wrapped: bool,
    wrapped_origin: Option<&str>,
    bridge_operator: Option<&str>,
    rpc: &str,
    config_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    print_info(&format!("Deploying USP-01 token: {} ({})...", name, symbol));

    // Deploy WASM with empty initial state (init function sets metadata)
    let initial_state = std::collections::BTreeMap::new();

    let (contract_addr, block_hash) =
        contract_ops::deploy_contract(wallet, wasm_path, initial_state, 0, rpc, config_dir).await?;

    print_success(&format!("Contract deployed: {}", contract_addr));

    // Call init() on the deployed contract
    print_info("Initializing USP-01 token...");
    let mut args = vec![
        name.to_string(),
        symbol.to_string(),
        decimals.to_string(),
        total_supply.to_string(),
    ];
    // Optional args: is_wrapped(4), wrapped_origin(5), max_supply(6), bridge_operator(7)
    args.push(if is_wrapped {
        "1".to_string()
    } else {
        "0".to_string()
    });
    args.push(wrapped_origin.unwrap_or("").to_string());
    args.push(max_supply.unwrap_or("0").to_string());
    args.push(bridge_operator.unwrap_or("").to_string());

    let result = contract_ops::call_contract(
        wallet,
        &contract_addr,
        "init",
        args,
        None,
        0,
        rpc,
        config_dir,
    )
    .await?;

    let exec = &result["result"];
    if exec["success"].as_bool() == Some(true) {
        println!();
        print_success("USP-01 Token deployed and initialized!");
        println!("  {}: {}", "Contract".bold(), contract_addr.green());
        println!("  {}: {}", "Block Hash".bold(), block_hash);
        println!(
            "  {}: {} ({})",
            "Token".bold(),
            name.green(),
            symbol.yellow()
        );
        println!("  {}: {}", "Decimals".bold(), decimals);
        println!("  {}: {}", "Total Supply".bold(), total_supply.cyan());
        if is_wrapped {
            println!(
                "  {}: {} ({})",
                "Type".bold(),
                "Wrapped Asset".yellow(),
                wrapped_origin.unwrap_or("unknown")
            );
        }
        println!();
    } else {
        let output = exec["output"].as_str().unwrap_or("unknown error");
        print_error(&format!("Token init failed: {}", output));
    }

    Ok(())
}

async fn token_mint(
    wallet: &str,
    token_contract: &str,
    to: &str,
    amount: &str,
    rpc: &str,
    config_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    print_info(&format!("Distributing {} tokens to {}...", amount, to));
    print_info("Note: USP-01 uses fixed supply. This transfers tokens from your balance.");

    let args = vec![to.to_string(), amount.to_string()];

    let result = contract_ops::call_contract(
        wallet,
        token_contract,
        "transfer",
        args,
        None,
        0,
        rpc,
        config_dir,
    )
    .await?;

    let exec = &result["result"];
    if exec["success"].as_bool() == Some(true) {
        println!();
        print_success("Tokens distributed successfully!");
        println!("  {}: {}", "Contract".bold(), token_contract.green());
        println!("  {}: {}", "Recipient".bold(), to);
        println!("  {}: {}", "Amount".bold(), amount.cyan().bold());
        println!();
    } else {
        let output = exec["output"].as_str().unwrap_or("unknown error");
        print_error(&format!("Mint/transfer failed: {}", output));
    }

    Ok(())
}

async fn token_transfer(
    wallet: &str,
    token_contract: &str,
    to: &str,
    amount: &str,
    rpc: &str,
    config_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    print_info(&format!("Transferring {} tokens to {}...", amount, to));

    let args = vec![to.to_string(), amount.to_string()];

    let result = contract_ops::call_contract(
        wallet,
        token_contract,
        "transfer",
        args,
        None,
        0,
        rpc,
        config_dir,
    )
    .await?;

    let exec = &result["result"];
    if exec["success"].as_bool() == Some(true) {
        println!();
        print_success("Transfer successful!");
        println!("  {}: {}", "Contract".bold(), token_contract.green());
        println!("  {}: {}", "To".bold(), to.green());
        println!("  {}: {}", "Amount".bold(), amount.cyan().bold());
        if let Some(gas) = exec["gas_used"].as_u64() {
            println!("  {}: {}", "Gas Used".bold(), gas);
        }
        println!();
    } else {
        let output = exec["output"].as_str().unwrap_or("unknown error");
        print_error(&format!("Transfer failed: {}", output));
    }

    Ok(())
}

async fn token_approve(
    wallet: &str,
    token_contract: &str,
    spender: &str,
    amount: &str,
    rpc: &str,
    config_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    print_info(&format!(
        "Approving {} to spend {} tokens...",
        spender, amount
    ));

    let args = vec![spender.to_string(), amount.to_string()];

    let result = contract_ops::call_contract(
        wallet,
        token_contract,
        "approve",
        args,
        None,
        0,
        rpc,
        config_dir,
    )
    .await?;

    let exec = &result["result"];
    if exec["success"].as_bool() == Some(true) {
        println!();
        print_success("Approval set!");
        println!("  {}: {}", "Contract".bold(), token_contract.green());
        println!("  {}: {}", "Spender".bold(), spender);
        println!("  {}: {}", "Allowance".bold(), amount.cyan().bold());
        println!();
    } else {
        let output = exec["output"].as_str().unwrap_or("unknown error");
        print_error(&format!("Approve failed: {}", output));
    }

    Ok(())
}

async fn token_burn(
    wallet: &str,
    token_contract: &str,
    amount: &str,
    rpc: &str,
    config_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    print_info(&format!("Burning {} tokens...", amount));

    let args = vec![amount.to_string()];

    let result = contract_ops::call_contract(
        wallet,
        token_contract,
        "burn",
        args,
        None,
        0,
        rpc,
        config_dir,
    )
    .await?;

    let exec = &result["result"];
    if exec["success"].as_bool() == Some(true) {
        println!();
        print_success("Tokens burned!");
        println!("  {}: {}", "Contract".bold(), token_contract.green());
        println!("  {}: {}", "Burned".bold(), amount.cyan().bold());
        println!();
    } else {
        let output = exec["output"].as_str().unwrap_or("unknown error");
        print_error(&format!("Burn failed: {}", output));
    }

    Ok(())
}
