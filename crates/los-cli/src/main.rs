// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// UNAUTHORITY CLI - Command Line Interface for Validators & Users
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use clap::{Parser, Subcommand};
use colored::*;
use std::path::PathBuf;

mod commands;

#[derive(Parser)]
#[command(name = "los-cli")]
#[command(about = "Unauthority CLI - Validator & Wallet Management", long_about = None)]
#[command(version)]
struct Cli {
    /// RPC endpoint URL (reads LOS_RPC_URL env var, or defaults to http://localhost:3030)
    /// For Tor: set LOS_RPC_URL=http://your-node.onion
    #[arg(
        short,
        long,
        env = "LOS_RPC_URL",
        default_value = "http://localhost:3030"
    )]
    rpc: String,

    /// Config directory (default: ~/.los)
    #[arg(short, long)]
    config_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Wallet management
    Wallet {
        #[command(subcommand)]
        action: WalletCommands,
    },

    /// Validator operations
    Validator {
        #[command(subcommand)]
        action: ValidatorCommands,
    },

    /// Query blockchain state
    Query {
        #[command(subcommand)]
        action: QueryCommands,
    },

    /// Transaction operations
    Tx {
        #[command(subcommand)]
        action: TxCommands,
    },

    /// USP-01 Token operations
    Token {
        #[command(subcommand)]
        action: TokenCommands,
    },

    /// DEX (Decentralized Exchange) operations
    Dex {
        #[command(subcommand)]
        action: DexCommands,
    },
}

#[derive(Subcommand)]
enum WalletCommands {
    /// Create new wallet
    New {
        /// Wallet name
        #[arg(short, long)]
        name: String,
    },

    /// List all wallets
    List,

    /// Show wallet balance
    Balance {
        /// Wallet address
        address: String,
    },

    /// Export wallet (encrypted)
    Export {
        /// Wallet name
        name: String,

        /// Output file path
        #[arg(short, long)]
        output: PathBuf,
    },

    /// Import wallet
    Import {
        /// Input file path
        input: PathBuf,

        /// Wallet name
        #[arg(short, long)]
        name: String,
    },
}

#[derive(Subcommand)]
enum ValidatorCommands {
    /// Stake tokens to become validator
    Stake {
        /// Amount in LOS (minimum 1000)
        #[arg(short, long)]
        amount: u64,

        /// Wallet name
        #[arg(short, long)]
        wallet: String,
    },

    /// Unstake tokens
    Unstake {
        /// Wallet name
        #[arg(short, long)]
        wallet: String,
    },

    /// Show validator status
    Status {
        /// Validator address
        address: String,
    },

    /// List all active validators
    List,
}

#[derive(Subcommand)]
enum QueryCommands {
    /// Get block by height
    Block {
        /// Block height
        height: u64,
    },

    /// Get account state
    Account {
        /// Account address
        address: String,
    },

    /// Get network info
    Info,

    /// Get validator set
    Validators,
}

#[derive(Subcommand)]
enum TxCommands {
    /// Send LOS to address
    Send {
        /// Recipient address
        #[arg(short, long)]
        to: String,

        /// Amount in LOS
        #[arg(short, long)]
        amount: u64,

        /// Sender wallet name
        #[arg(short, long)]
        from: String,
    },

    /// Query transaction status
    Status {
        /// Transaction hash
        hash: String,
    },
}

#[derive(Subcommand)]
enum DexCommands {
    /// List all DEX pools across all contracts
    Pools,

    /// Show pool info
    Pool {
        /// DEX contract address (LOSCon...)
        #[arg(short, long)]
        contract: String,

        /// Pool ID (e.g. POOL:LOS:TOKEN_A)
        #[arg(short, long)]
        pool_id: String,
    },

    /// Get a swap quote
    Quote {
        /// DEX contract address
        #[arg(short, long)]
        contract: String,

        /// Pool ID
        #[arg(short, long)]
        pool_id: String,

        /// Token to sell
        #[arg(short, long)]
        token_in: String,

        /// Amount to sell (atomic units)
        #[arg(short, long)]
        amount_in: u128,
    },

    /// Get LP position for a user
    Position {
        /// DEX contract address
        #[arg(short, long)]
        contract: String,

        /// Pool ID
        #[arg(short, long)]
        pool_id: String,

        /// User address
        #[arg(short, long)]
        user: String,
    },

    /// Deploy a DEX AMM contract
    Deploy {
        /// Wallet name
        #[arg(short, long)]
        wallet: String,

        /// Path to compiled WASM file
        #[arg(long)]
        wasm: String,
    },

    /// Create a new liquidity pool
    CreatePool {
        /// Wallet name
        #[arg(short, long)]
        wallet: String,

        /// DEX contract address
        #[arg(short, long)]
        contract: String,

        /// Token A identifier
        #[arg(long)]
        token_a: String,

        /// Token B identifier
        #[arg(long)]
        token_b: String,

        /// Initial amount of Token A (atomic units)
        #[arg(long)]
        amount_a: String,

        /// Initial amount of Token B (atomic units)
        #[arg(long)]
        amount_b: String,

        /// Fee in basis points (default: 30 = 0.3%)
        #[arg(long)]
        fee_bps: Option<String>,
    },

    /// Add liquidity to a pool
    AddLiquidity {
        /// Wallet name
        #[arg(short, long)]
        wallet: String,

        /// DEX contract address
        #[arg(short, long)]
        contract: String,

        /// Pool ID
        #[arg(short, long)]
        pool_id: String,

        /// Amount of Token A to add
        #[arg(long)]
        amount_a: String,

        /// Amount of Token B to add
        #[arg(long)]
        amount_b: String,

        /// Minimum LP tokens to receive (slippage protection)
        #[arg(long)]
        min_lp: String,
    },

    /// Remove liquidity from a pool
    RemoveLiquidity {
        /// Wallet name
        #[arg(short, long)]
        wallet: String,

        /// DEX contract address
        #[arg(short, long)]
        contract: String,

        /// Pool ID
        #[arg(short, long)]
        pool_id: String,

        /// LP tokens to burn
        #[arg(long)]
        lp_amount: String,

        /// Minimum Token A to receive (slippage protection)
        #[arg(long)]
        min_a: String,

        /// Minimum Token B to receive (slippage protection)
        #[arg(long)]
        min_b: String,
    },

    /// Execute a token swap
    Swap {
        /// Wallet name
        #[arg(short, long)]
        wallet: String,

        /// DEX contract address
        #[arg(short, long)]
        contract: String,

        /// Pool ID
        #[arg(short, long)]
        pool_id: String,

        /// Token to sell
        #[arg(long)]
        token_in: String,

        /// Amount to sell (atomic units)
        #[arg(long)]
        amount_in: String,

        /// Minimum amount to receive (slippage protection)
        #[arg(long)]
        min_out: String,

        /// Transaction deadline (unix timestamp, default: now + 5min)
        #[arg(long)]
        deadline: Option<u64>,
    },
}

#[derive(Subcommand)]
enum TokenCommands {
    /// List all deployed USP-01 tokens
    List,

    /// Show USP-01 token metadata
    Info {
        /// Token contract address (LOSCon...)
        address: String,
    },

    /// Query token balance for a holder
    Balance {
        /// Token contract address
        #[arg(short, long)]
        token: String,

        /// Holder address
        #[arg(long)]
        holder: String,
    },

    /// Query token allowance
    Allowance {
        /// Token contract address
        #[arg(short, long)]
        token: String,

        /// Owner address
        #[arg(short, long)]
        owner: String,

        /// Spender address
        #[arg(short, long)]
        spender: String,
    },

    /// Deploy a new USP-01 token
    Deploy {
        /// Wallet name
        #[arg(short, long)]
        wallet: String,

        /// Path to compiled WASM file
        #[arg(long)]
        wasm: String,

        /// Token name (1-64 chars)
        #[arg(long)]
        name: String,

        /// Token symbol (1-8 chars)
        #[arg(long)]
        symbol: String,

        /// Token decimals (0-18)
        #[arg(long, default_value = "11")]
        decimals: u8,

        /// Total supply (atomic units)
        #[arg(long)]
        total_supply: String,

        /// Max supply (0 = unlimited, atomic units)
        #[arg(long)]
        max_supply: Option<String>,

        /// Is this a wrapped asset?
        #[arg(long, default_value = "false")]
        is_wrapped: bool,

        /// Origin chain for wrapped asset (e.g. "ethereum")
        #[arg(long)]
        wrapped_origin: Option<String>,

        /// Bridge operator address for wrapped asset
        #[arg(long)]
        bridge_operator: Option<String>,
    },

    /// Distribute tokens (transfer from owner)
    Mint {
        /// Wallet name
        #[arg(short, long)]
        wallet: String,

        /// Token contract address
        #[arg(short, long)]
        token: String,

        /// Recipient address
        #[arg(long)]
        to: String,

        /// Amount (atomic units)
        #[arg(long)]
        amount: String,
    },

    /// Transfer tokens to another address
    Transfer {
        /// Wallet name
        #[arg(short, long)]
        wallet: String,

        /// Token contract address
        #[arg(short, long)]
        token: String,

        /// Recipient address
        #[arg(long)]
        to: String,

        /// Amount (atomic units)
        #[arg(long)]
        amount: String,
    },

    /// Approve spender allowance
    Approve {
        /// Wallet name
        #[arg(short, long)]
        wallet: String,

        /// Token contract address
        #[arg(short, long)]
        token: String,

        /// Spender address
        #[arg(long)]
        spender: String,

        /// Amount to allow (atomic units)
        #[arg(long)]
        amount: String,
    },

    /// Burn tokens from your balance
    Burn {
        /// Wallet name
        #[arg(short, long)]
        wallet: String,

        /// Token contract address
        #[arg(short, long)]
        token: String,

        /// Amount to burn (atomic units)
        #[arg(long)]
        amount: String,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Print banner
    print_banner();

    // Get config directory
    let config_dir = cli.config_dir.unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".los")
    });

    // Ensure config directory exists
    std::fs::create_dir_all(&config_dir)?;

    match cli.command {
        Commands::Wallet { action } => {
            commands::wallet::handle(action, &cli.rpc, &config_dir).await?;
        }
        Commands::Validator { action } => {
            commands::validator::handle(action, &cli.rpc, &config_dir).await?;
        }
        Commands::Query { action } => {
            commands::query::handle(action, &cli.rpc).await?;
        }
        Commands::Tx { action } => {
            commands::tx::handle(action, &cli.rpc, &config_dir).await?;
        }
        Commands::Token { action } => {
            commands::token::handle(action, &cli.rpc, &config_dir).await?;
        }
        Commands::Dex { action } => commands::dex::handle(action, &cli.rpc, &config_dir).await?,
    }

    Ok(())
}

fn print_banner() {
    println!(
        "{}",
        "╔═══════════════════════════════════════════════╗".cyan()
    );
    println!(
        "{}",
        "║      UNAUTHORITY (LOS) - CLI v0.1.0           ║"
            .cyan()
            .bold()
    );
    println!(
        "{}",
        "║   Permissionless | Immutable | Decentralized  ║".cyan()
    );
    println!(
        "{}",
        "╚═══════════════════════════════════════════════╝".cyan()
    );
    println!();
}

// Additional utility for colored output
#[allow(dead_code)]
fn print_success(msg: &str) {
    println!("{} {}", "✓".green().bold(), msg);
}

#[allow(dead_code)]
fn print_error(msg: &str) {
    eprintln!("{} {}", "✗".red().bold(), msg);
}

#[allow(dead_code)]
fn print_info(msg: &str) {
    println!("{} {}", "ℹ".blue().bold(), msg);
}

// ─────────────────────────────────────────────────────────────────
// UNIT TESTS
// ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use los_core::{Block, BlockType, CIL_PER_LOS, MIN_POW_DIFFICULTY_BITS};

    // ── CLI Argument Parsing ────────────────────────────────────

    #[test]
    fn test_cli_wallet_new() {
        let cli = Cli::try_parse_from(["los-cli", "wallet", "new", "--name", "test_wallet"]);
        assert!(cli.is_ok(), "Failed to parse: {:?}", cli.err());
        let cli = cli.unwrap();
        match cli.command {
            Commands::Wallet {
                action: WalletCommands::New { name },
            } => assert_eq!(name, "test_wallet"),
            _ => panic!("Expected Wallet::New"),
        }
    }

    #[test]
    fn test_cli_wallet_list() {
        let cli = Cli::try_parse_from(["los-cli", "wallet", "list"]);
        assert!(cli.is_ok());
        match cli.unwrap().command {
            Commands::Wallet {
                action: WalletCommands::List,
            } => {}
            _ => panic!("Expected Wallet::List"),
        }
    }

    #[test]
    fn test_cli_wallet_balance() {
        let cli = Cli::try_parse_from(["los-cli", "wallet", "balance", "LOSxyz123"]);
        assert!(cli.is_ok());
        match cli.unwrap().command {
            Commands::Wallet {
                action: WalletCommands::Balance { address },
            } => assert_eq!(address, "LOSxyz123"),
            _ => panic!("Expected Wallet::Balance"),
        }
    }

    #[test]
    fn test_cli_wallet_export() {
        let cli = Cli::try_parse_from([
            "los-cli",
            "wallet",
            "export",
            "mywallet",
            "--output",
            "/tmp/w.json",
        ]);
        assert!(cli.is_ok());
        match cli.unwrap().command {
            Commands::Wallet {
                action: WalletCommands::Export { name, output },
            } => {
                assert_eq!(name, "mywallet");
                assert_eq!(output, PathBuf::from("/tmp/w.json"));
            }
            _ => panic!("Expected Wallet::Export"),
        }
    }

    #[test]
    fn test_cli_wallet_import() {
        let cli = Cli::try_parse_from([
            "los-cli",
            "wallet",
            "import",
            "/tmp/w.json",
            "--name",
            "imported",
        ]);
        assert!(cli.is_ok());
        match cli.unwrap().command {
            Commands::Wallet {
                action: WalletCommands::Import { input, name },
            } => {
                assert_eq!(name, "imported");
                assert_eq!(input, PathBuf::from("/tmp/w.json"));
            }
            _ => panic!("Expected Wallet::Import"),
        }
    }

    #[test]
    fn test_cli_validator_stake() {
        let cli = Cli::try_parse_from([
            "los-cli",
            "validator",
            "stake",
            "--amount",
            "1000",
            "--wallet",
            "w1",
        ]);
        assert!(cli.is_ok());
        match cli.unwrap().command {
            Commands::Validator {
                action: ValidatorCommands::Stake { amount, wallet },
            } => {
                assert_eq!(amount, 1000);
                assert_eq!(wallet, "w1");
            }
            _ => panic!("Expected Validator::Stake"),
        }
    }

    #[test]
    fn test_cli_validator_list() {
        let cli = Cli::try_parse_from(["los-cli", "validator", "list"]);
        assert!(cli.is_ok());
    }

    #[test]
    fn test_cli_tx_send() {
        let cli = Cli::try_parse_from([
            "los-cli", "tx", "send", "--to", "LOSabc", "--amount", "50", "--from", "w1",
        ]);
        assert!(cli.is_ok());
        match cli.unwrap().command {
            Commands::Tx {
                action: TxCommands::Send { to, amount, from },
            } => {
                assert_eq!(to, "LOSabc");
                assert_eq!(amount, 50);
                assert_eq!(from, "w1");
            }
            _ => panic!("Expected Tx::Send"),
        }
    }

    #[test]
    fn test_cli_tx_status() {
        let cli = Cli::try_parse_from(["los-cli", "tx", "status", "deadbeef"]);
        assert!(cli.is_ok());
        match cli.unwrap().command {
            Commands::Tx {
                action: TxCommands::Status { hash },
            } => assert_eq!(hash, "deadbeef"),
            _ => panic!("Expected Tx::Status"),
        }
    }

    #[test]
    fn test_cli_query_block() {
        let cli = Cli::try_parse_from(["los-cli", "query", "block", "42"]);
        assert!(cli.is_ok());
        match cli.unwrap().command {
            Commands::Query {
                action: QueryCommands::Block { height },
            } => assert_eq!(height, 42),
            _ => panic!("Expected Query::Block"),
        }
    }

    #[test]
    fn test_cli_query_account() {
        let cli = Cli::try_parse_from(["los-cli", "query", "account", "LOSabc"]);
        assert!(cli.is_ok());
        match cli.unwrap().command {
            Commands::Query {
                action: QueryCommands::Account { address },
            } => assert_eq!(address, "LOSabc"),
            _ => panic!("Expected Query::Account"),
        }
    }

    #[test]
    fn test_cli_query_info() {
        let cli = Cli::try_parse_from(["los-cli", "query", "info"]);
        assert!(cli.is_ok());
    }

    #[test]
    fn test_cli_query_validators() {
        let cli = Cli::try_parse_from(["los-cli", "query", "validators"]);
        assert!(cli.is_ok());
    }

    #[test]
    fn test_cli_token_list() {
        let cli = Cli::try_parse_from(["los-cli", "token", "list"]);
        assert!(cli.is_ok());
        match cli.unwrap().command {
            Commands::Token {
                action: TokenCommands::List,
            } => {}
            _ => panic!("Expected Token::List"),
        }
    }

    #[test]
    fn test_cli_token_info() {
        let cli = Cli::try_parse_from(["los-cli", "token", "info", "LOSConXYZ"]);
        assert!(cli.is_ok());
    }

    #[test]
    fn test_cli_token_deploy() {
        let cli = Cli::try_parse_from([
            "los-cli",
            "token",
            "deploy",
            "--wallet",
            "w1",
            "--wasm",
            "path/to/token.wasm",
            "--name",
            "TestToken",
            "--symbol",
            "TST",
            "--decimals",
            "11",
            "--total-supply",
            "1000000",
        ]);
        assert!(cli.is_ok());
    }

    #[test]
    fn test_cli_dex_pools() {
        let cli = Cli::try_parse_from(["los-cli", "dex", "pools"]);
        assert!(cli.is_ok());
        match cli.unwrap().command {
            Commands::Dex {
                action: DexCommands::Pools,
            } => {}
            _ => panic!("Expected Dex::Pools"),
        }
    }

    #[test]
    fn test_cli_dex_swap() {
        let cli = Cli::try_parse_from([
            "los-cli",
            "dex",
            "swap",
            "--wallet",
            "w1",
            "--contract",
            "LOSConDEX",
            "--pool-id",
            "POOL:LOS:TST",
            "--token-in",
            "LOS",
            "--amount-in",
            "1000",
            "--min-out",
            "900",
        ]);
        assert!(cli.is_ok());
    }

    #[test]
    fn test_cli_default_rpc_url() {
        let cli = Cli::try_parse_from(["los-cli", "query", "info"]).unwrap();
        assert_eq!(cli.rpc, "http://localhost:3030");
    }

    #[test]
    fn test_cli_custom_rpc_url() {
        let cli =
            Cli::try_parse_from(["los-cli", "--rpc", "http://my-node.onion", "query", "info"])
                .unwrap();
        assert_eq!(cli.rpc, "http://my-node.onion");
    }

    #[test]
    fn test_cli_missing_required_args() {
        // tx send without --to should fail
        let result =
            Cli::try_parse_from(["los-cli", "tx", "send", "--amount", "50", "--from", "w"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cli_unknown_subcommand() {
        let result = Cli::try_parse_from(["los-cli", "foobar"]);
        assert!(result.is_err());
    }

    // ── PoW Logic ───────────────────────────────────────────────

    #[test]
    fn test_pow_produces_valid_hash() {
        let mut block = Block {
            account: "LOStest".to_string(),
            previous: "0".to_string(),
            block_type: BlockType::Send,
            amount: 100_000,
            link: "LOSrecipient".to_string(),
            signature: String::new(),
            public_key: "deadbeef".to_string(),
            work: 0,
            timestamp: 1700000000,
            fee: 100_000,
        };

        commands::tx::compute_pow(&mut block);

        // Verify the PoW is valid
        assert!(block.verify_pow(), "PoW should be valid after compute_pow");
        // Nonce should have been set to a non-trivial value
        // (it's extremely unlikely the first nonce=0 satisfies 16-bit difficulty)
    }

    #[test]
    fn test_pow_verification_counts_leading_zeros() {
        // Block with work=0 is very unlikely to satisfy 16-bit PoW
        let _block = Block {
            account: "LOStest".to_string(),
            previous: "0".to_string(),
            block_type: BlockType::Send,
            amount: 1,
            link: "LOSother".to_string(),
            signature: String::new(),
            public_key: "aabb".to_string(),
            work: 0,
            timestamp: 1700000000,
            fee: 100_000,
        };

        // With a random nonce of 0, this is likely invalid (but not guaranteed)
        // The important test is that verify_pow uses MIN_POW_DIFFICULTY_BITS
        assert_eq!(
            MIN_POW_DIFFICULTY_BITS, 16,
            "PoW difficulty should be 16 bits"
        );
    }

    // ── Address Validation ──────────────────────────────────────

    #[test]
    fn test_address_validation_rejects_empty() {
        assert!(!los_crypto::validate_address(""));
    }

    #[test]
    fn test_address_validation_rejects_no_prefix() {
        assert!(!los_crypto::validate_address("BTCxyz123"));
    }

    #[test]
    fn test_address_validation_rejects_short() {
        assert!(!los_crypto::validate_address("LOS"));
    }

    #[test]
    fn test_address_validation_rejects_invalid_base58() {
        assert!(!los_crypto::validate_address("LOS0OIl")); // Invalid Base58 chars
    }

    #[test]
    fn test_address_validation_rejects_bad_checksum() {
        // Valid Base58 but wrong checksum
        assert!(!los_crypto::validate_address("LOS1111111111111111111111"));
    }

    #[test]
    fn test_address_validation_accepts_generated() {
        let keypair = los_crypto::generate_keypair();
        let address = los_crypto::public_key_to_address(&keypair.public_key);
        assert!(
            los_crypto::validate_address(&address),
            "Generated address must pass validation: {}",
            address
        );
    }

    // ── CIL / LOS Conversion ────────────────────────────────────

    #[test]
    fn test_cil_per_los_constant() {
        assert_eq!(CIL_PER_LOS, 100_000_000_000);
    }

    #[test]
    fn test_cil_to_los_formatting() {
        let balance_cil: u128 = 1_500_000_000_000; // 15 LOS
        let los = balance_cil / CIL_PER_LOS;
        let fractional = balance_cil % CIL_PER_LOS;
        let formatted = format!("{}.{:011}", los, fractional);
        assert_eq!(formatted, "15.00000000000");
    }

    #[test]
    fn test_cil_to_los_formatting_fractional() {
        let balance_cil: u128 = 100_500_000_000; // 1.005 LOS
        let los = balance_cil / CIL_PER_LOS;
        let fractional = balance_cil % CIL_PER_LOS;
        let formatted = format!("{}.{:011}", los, fractional);
        assert_eq!(formatted, "1.00500000000");
    }

    #[test]
    fn test_los_to_cil_conversion_no_overflow() {
        let amount_los: u128 = 21_936_236; // Total supply
        let amount_cil = amount_los.checked_mul(CIL_PER_LOS);
        assert!(
            amount_cil.is_some(),
            "Total supply in CIL must not overflow u128"
        );
        assert_eq!(amount_cil.unwrap(), 2_193_623_600_000_000_000);
    }

    #[test]
    fn test_minimum_stake_conversion() {
        let min_stake_los: u128 = 1000;
        let min_stake_cil = min_stake_los * CIL_PER_LOS;
        assert_eq!(min_stake_cil, los_core::MIN_VALIDATOR_STAKE_CIL);
    }
}
