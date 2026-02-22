use crate::commands::contract_ops;
use crate::{print_error, print_info, print_success};
use colored::Colorize;
use std::path::Path;

/// Handle DEX subcommands.
pub async fn handle(
    action: crate::DexCommands,
    rpc: &str,
    config_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        crate::DexCommands::Pools => list_pools(rpc).await,
        crate::DexCommands::Pool { contract, pool_id } => pool_info(rpc, &contract, &pool_id).await,
        crate::DexCommands::Quote {
            contract,
            pool_id,
            token_in,
            amount_in,
        } => get_quote(rpc, &contract, &pool_id, &token_in, amount_in).await,
        crate::DexCommands::Position {
            contract,
            pool_id,
            user,
        } => get_position(rpc, &contract, &pool_id, &user).await,
        crate::DexCommands::Deploy { wallet, wasm } => {
            dex_deploy(&wallet, &wasm, rpc, config_dir).await
        }
        crate::DexCommands::CreatePool {
            wallet,
            contract,
            token_a,
            token_b,
            amount_a,
            amount_b,
            fee_bps,
        } => {
            dex_create_pool(
                &wallet, &contract, &token_a, &token_b, &amount_a, &amount_b, fee_bps, rpc,
                config_dir,
            )
            .await
        }
        crate::DexCommands::AddLiquidity {
            wallet,
            contract,
            pool_id,
            amount_a,
            amount_b,
            min_lp,
        } => {
            dex_add_liquidity(
                &wallet, &contract, &pool_id, &amount_a, &amount_b, &min_lp, rpc, config_dir,
            )
            .await
        }
        crate::DexCommands::RemoveLiquidity {
            wallet,
            contract,
            pool_id,
            lp_amount,
            min_a,
            min_b,
        } => {
            dex_remove_liquidity(
                &wallet, &contract, &pool_id, &lp_amount, &min_a, &min_b, rpc, config_dir,
            )
            .await
        }
        crate::DexCommands::Swap {
            wallet,
            contract,
            pool_id,
            token_in,
            amount_in,
            min_out,
            deadline,
        } => {
            dex_swap(
                &wallet, &contract, &pool_id, &token_in, &amount_in, &min_out, deadline, rpc,
                config_dir,
            )
            .await
        }
    }
}

async fn list_pools(rpc: &str) -> Result<(), Box<dyn std::error::Error>> {
    let url = format!("{}/dex/pools", rpc);
    let resp: serde_json::Value = reqwest::get(&url).await?.json().await?;

    if resp["status"] == "success" {
        let count = resp["count"].as_u64().unwrap_or(0);
        println!("{}", format!("DEX Pools ({})", count).cyan().bold());
        println!("{}", "─".repeat(70));

        if let Some(pools) = resp["pools"].as_array() {
            for pool in pools {
                let pid = pool["pool_id"].as_str().unwrap_or("?");
                let ta = pool["token_a"].as_str().unwrap_or("?");
                let tb = pool["token_b"].as_str().unwrap_or("?");
                let ra = pool["reserve_a"].as_u64().unwrap_or(0);
                let rb = pool["reserve_b"].as_u64().unwrap_or(0);
                let contract = pool["contract"].as_str().unwrap_or("?");
                println!(
                    "  {} {} / {} | Reserves: {} / {} | Contract: {}",
                    pid.yellow(),
                    ta.green(),
                    tb.green(),
                    ra.to_string().white(),
                    rb.to_string().white(),
                    &contract[..16.min(contract.len())],
                );
            }
        }
        if count == 0 {
            println!("  {}", "No pools found".dimmed());
        }
    } else {
        let msg = resp["msg"].as_str().unwrap_or("Unknown error");
        eprintln!("{} {}", "Error:".red().bold(), msg);
    }

    Ok(())
}

async fn pool_info(
    rpc: &str,
    contract: &str,
    pool_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let url = format!("{}/dex/pool/{}/{}", rpc, contract, pool_id);
    let resp: serde_json::Value = reqwest::get(&url).await?.json().await?;

    if resp["status"] == "success" {
        let pool = &resp["pool"];
        println!("{}", "Pool Info".cyan().bold());
        println!("{}", "─".repeat(50));
        println!(
            "  Pool ID:     {}",
            pool["pool_id"].as_str().unwrap_or("?").yellow()
        );
        println!(
            "  Token A:     {}",
            pool["token_a"].as_str().unwrap_or("?").green()
        );
        println!(
            "  Token B:     {}",
            pool["token_b"].as_str().unwrap_or("?").green()
        );
        println!("  Reserve A:   {}", pool["reserve_a"]);
        println!("  Reserve B:   {}", pool["reserve_b"]);
        println!("  Total LP:    {}", pool["total_lp"]);
        println!("  Fee (bps):   {}", pool["fee_bps"]);
        println!("  Creator:     {}", pool["creator"].as_str().unwrap_or("?"));
        println!("  Last Trade:  {}", pool["last_trade"]);
    } else {
        let msg = resp["msg"].as_str().unwrap_or("Unknown error");
        eprintln!("{} {}", "Error:".red().bold(), msg);
    }

    Ok(())
}

async fn get_quote(
    rpc: &str,
    contract: &str,
    pool_id: &str,
    token_in: &str,
    amount_in: u128,
) -> Result<(), Box<dyn std::error::Error>> {
    let url = format!(
        "{}/dex/quote/{}/{}/{}/{}",
        rpc, contract, pool_id, token_in, amount_in
    );
    let resp: serde_json::Value = reqwest::get(&url).await?.json().await?;

    if resp["status"] == "success" {
        let q = &resp["quote"];
        println!("{}", "Swap Quote".cyan().bold());
        println!("{}", "─".repeat(40));
        println!("  Amount Out:      {}", q["amount_out"].to_string().green());
        println!("  Fee:             {}", q["fee"].to_string().yellow());
        println!(
            "  Price Impact:    {} bps",
            q["price_impact_bps"].to_string().white()
        );
    } else {
        let msg = resp["msg"].as_str().unwrap_or("Unknown error");
        eprintln!("{} {}", "Error:".red().bold(), msg);
    }

    Ok(())
}

async fn get_position(
    rpc: &str,
    contract: &str,
    pool_id: &str,
    user: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let url = format!("{}/dex/position/{}/{}/{}", rpc, contract, pool_id, user);
    let resp: serde_json::Value = reqwest::get(&url).await?.json().await?;

    if resp["status"] == "success" {
        println!("{}", "LP Position".cyan().bold());
        println!("{}", "─".repeat(40));
        println!("  LP Shares:   {}", resp["lp_shares"].to_string().green());
        println!("  User:        {}", user);
        println!("  Pool:        {}", pool_id);
    } else {
        let msg = resp["msg"].as_str().unwrap_or("Unknown error");
        eprintln!("{} {}", "Error:".red().bold(), msg);
    }

    Ok(())
}
// ─────────────────────────────────────────────────────────────
// WRITE OPERATIONS — Deploy, CreatePool, AddLiquidity,
//                    RemoveLiquidity, Swap
// ─────────────────────────────────────────────────────────────

async fn dex_deploy(
    wallet: &str,
    wasm_path: &str,
    rpc: &str,
    config_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    print_info("Deploying DEX AMM contract...");

    let initial_state = std::collections::BTreeMap::new();
    let (contract_addr, block_hash) =
        contract_ops::deploy_contract(wallet, wasm_path, initial_state, 0, rpc, config_dir).await?;

    print_success(&format!("DEX contract deployed: {}", contract_addr));

    // Call init() to initialize the DEX
    print_info("Initializing DEX...");
    let result = contract_ops::call_contract(
        wallet,
        &contract_addr,
        "init",
        vec![],
        None,
        0,
        rpc,
        config_dir,
    )
    .await?;

    let exec = &result["result"];
    if exec["success"].as_bool() == Some(true) {
        println!();
        print_success("DEX deployed and initialized!");
        println!("  {}: {}", "Contract".bold(), contract_addr.green());
        println!("  {}: {}", "Block Hash".bold(), block_hash);
        println!();
    } else {
        let output = exec["output"].as_str().unwrap_or("unknown error");
        print_error(&format!("DEX init failed: {}", output));
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn dex_create_pool(
    wallet: &str,
    contract: &str,
    token_a: &str,
    token_b: &str,
    amount_a: &str,
    amount_b: &str,
    fee_bps: Option<String>,
    rpc: &str,
    config_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    print_info(&format!(
        "Creating pool: {} / {} ({} / {})",
        token_a, token_b, amount_a, amount_b
    ));

    let mut args = vec![
        token_a.to_string(),
        token_b.to_string(),
        amount_a.to_string(),
        amount_b.to_string(),
    ];
    if let Some(fee) = fee_bps {
        args.push(fee);
    }

    let result = contract_ops::call_contract(
        wallet,
        contract,
        "create_pool",
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
        print_success("Liquidity pool created!");
        println!("  {}: {}", "Contract".bold(), contract.green());
        println!(
            "  {}: {} / {}",
            "Pair".bold(),
            token_a.green(),
            token_b.green()
        );
        println!(
            "  {}: {} / {}",
            "Initial Reserves".bold(),
            amount_a.cyan(),
            amount_b.cyan()
        );
        if let Some(gas) = exec["gas_used"].as_u64() {
            println!("  {}: {}", "Gas Used".bold(), gas);
        }
        println!();
    } else {
        let output = exec["output"].as_str().unwrap_or("unknown error");
        print_error(&format!("Create pool failed: {}", output));
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn dex_add_liquidity(
    wallet: &str,
    contract: &str,
    pool_id: &str,
    amount_a: &str,
    amount_b: &str,
    min_lp: &str,
    rpc: &str,
    config_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    print_info(&format!("Adding liquidity to pool {}...", pool_id));

    let args = vec![
        pool_id.to_string(),
        amount_a.to_string(),
        amount_b.to_string(),
        min_lp.to_string(),
    ];

    let result = contract_ops::call_contract(
        wallet,
        contract,
        "add_liquidity",
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
        print_success("Liquidity added!");
        println!("  {}: {}", "Pool".bold(), pool_id.yellow());
        println!(
            "  {}: {} / {}",
            "Amounts".bold(),
            amount_a.cyan(),
            amount_b.cyan()
        );
        if let Some(gas) = exec["gas_used"].as_u64() {
            println!("  {}: {}", "Gas Used".bold(), gas);
        }
        println!();
    } else {
        let output = exec["output"].as_str().unwrap_or("unknown error");
        print_error(&format!("Add liquidity failed: {}", output));
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn dex_remove_liquidity(
    wallet: &str,
    contract: &str,
    pool_id: &str,
    lp_amount: &str,
    min_a: &str,
    min_b: &str,
    rpc: &str,
    config_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    print_info(&format!(
        "Removing {} LP tokens from pool {}...",
        lp_amount, pool_id
    ));

    let args = vec![
        pool_id.to_string(),
        lp_amount.to_string(),
        min_a.to_string(),
        min_b.to_string(),
    ];

    let result = contract_ops::call_contract(
        wallet,
        contract,
        "remove_liquidity",
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
        print_success("Liquidity removed!");
        println!("  {}: {}", "Pool".bold(), pool_id.yellow());
        println!("  {}: {}", "LP Burned".bold(), lp_amount.cyan());
        if let Some(gas) = exec["gas_used"].as_u64() {
            println!("  {}: {}", "Gas Used".bold(), gas);
        }
        println!();
    } else {
        let output = exec["output"].as_str().unwrap_or("unknown error");
        print_error(&format!("Remove liquidity failed: {}", output));
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn dex_swap(
    wallet: &str,
    contract: &str,
    pool_id: &str,
    token_in: &str,
    amount_in: &str,
    min_out: &str,
    deadline: Option<u64>,
    rpc: &str,
    config_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    print_info(&format!(
        "Swapping {} {} in pool {}...",
        amount_in, token_in, pool_id
    ));

    let deadline_str = deadline.map(|d| d.to_string()).unwrap_or_else(|| {
        // Default deadline: 5 minutes from now
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        (now + 300).to_string()
    });

    let args = vec![
        pool_id.to_string(),
        token_in.to_string(),
        amount_in.to_string(),
        min_out.to_string(),
        deadline_str,
    ];

    let result =
        contract_ops::call_contract(wallet, contract, "swap", args, None, 0, rpc, config_dir)
            .await?;

    let exec = &result["result"];
    if exec["success"].as_bool() == Some(true) {
        println!();
        print_success("Swap executed!");
        println!("  {}: {}", "Pool".bold(), pool_id.yellow());
        println!("  {}: {} {}", "Sold".bold(), amount_in.cyan(), token_in);
        println!("  {}: {} (minimum)", "Min Received".bold(), min_out.green());
        if let Some(gas) = exec["gas_used"].as_u64() {
            println!("  {}: {}", "Gas Used".bold(), gas);
        }
        println!();
    } else {
        let output = exec["output"].as_str().unwrap_or("unknown error");
        print_error(&format!("Swap failed: {}", output));
    }

    Ok(())
}
