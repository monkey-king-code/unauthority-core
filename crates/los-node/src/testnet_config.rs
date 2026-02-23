/// UNAUTHORITY TESTNET CONFIGURATION
///
/// Implements graduated testnet modes that progressively test production features.
/// Goal: Ensure testnet success = mainnet success guarantee.
///
/// ARCHITECTURE:
/// - Level 1: UI/API Testing (current TESTNET_MODE)
/// - Level 2: Consensus Testing (real aBFT, mock economics)
/// - Level 3: Production Simulation (full production code, testnet data)

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TestnetLevel {
    /// Level 1: Functional testing only
    /// - Bypass consensus (immediate finalization)
    /// - Mock burn verification
    /// - UI/API testing focus
    Functional,

    /// Level 2: Consensus testing  
    /// - Real aBFT consensus
    /// - Real price aggregation (smart contract)
    /// - Testnet economic parameters
    Consensus,

    /// Level 3: Production simulation
    /// - Identical to mainnet code
    /// - Real validator staking/rewards
    /// - Only network/domain differs from mainnet
    Production,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TestnetConfig {
    pub level: TestnetLevel,
    pub enable_faucet: bool,
    /// Consensus quorum threshold in basis points (6700 = 67%)
    pub consensus_threshold_bps: u32,
    pub signature_validation: bool,
    pub byzantine_testing: bool,
    pub economic_incentives: bool,
}

impl TestnetConfig {
    /// Level 1: Current testnet (functional testing)
    pub fn functional() -> Self {
        Self {
            level: TestnetLevel::Functional,
            enable_faucet: true,
            consensus_threshold_bps: 0,  // Immediate finalization
            signature_validation: false, // Allow any signature
            byzantine_testing: false,
            economic_incentives: false,
        }
    }

    /// Level 2: Consensus testing (production logic, testnet parameters)
    pub fn consensus_testing() -> Self {
        Self {
            level: TestnetLevel::Consensus,
            enable_faucet: true,
            consensus_threshold_bps: 6700, // Real BFT threshold (67%)
            signature_validation: true,    // Real Dilithium5 (post-quantum) validation
            byzantine_testing: true,       // Enable byzantine scenarios
            economic_incentives: false,    // No real staking rewards yet
        }
    }

    /// Level 3: Production simulation (identical to mainnet)
    pub fn production_simulation() -> Self {
        Self {
            level: TestnetLevel::Production,
            enable_faucet: false,          // No faucet in production
            consensus_threshold_bps: 6700, // Real BFT (67%)
            signature_validation: true,    // Full validation
            byzantine_testing: true,       // Byzantine resistance
            economic_incentives: true,     // Real validator economics
        }
    }

    /// Check if feature should be enabled
    pub fn should_enable_consensus(&self) -> bool {
        matches!(
            self.level,
            TestnetLevel::Consensus | TestnetLevel::Production
        )
    }

    #[allow(dead_code)]
    pub fn should_validate_signatures(&self) -> bool {
        self.signature_validation
    }

    pub fn should_enable_faucet(&self) -> bool {
        self.enable_faucet
    }

    #[allow(dead_code)]
    pub fn should_test_byzantine_behavior(&self) -> bool {
        self.byzantine_testing
    }

    #[allow(dead_code)]
    pub fn get_consensus_threshold_bps(&self) -> u32 {
        self.consensus_threshold_bps
    }
}

/// Global testnet configuration
///
/// MAINNET BUILD: Always returns Production config regardless of environment variables.
/// This is the MASTER SAFETY GATE — all bypass checks flow through get_testnet_config(),
/// so forcing Production level here eliminates ALL testnet bypasses at once:
///   - should_enable_consensus() → true (no immediate finalization)
///   - should_validate_signatures() → true (no unsigned blocks)
///   - should_enable_faucet() → false (no free tokens)
///   - Mint cap → enforced (no TESTNET: prefix bypass)
///
/// TESTNET BUILD: Reads LOS_TESTNET_LEVEL env var, defaults to Consensus (Level 2).
static TESTNET_CONFIG: std::sync::LazyLock<TestnetConfig> = std::sync::LazyLock::new(|| {
    // MAINNET: Hardcoded to Production. No env var can weaken this.
    if los_core::is_mainnet_build() {
        println!("🔒 MAINNET BUILD: All security enforced (consensus, signatures, mint cap)");
        println!("   Faucet: DISABLED | Consensus: ENABLED | Signatures: REQUIRED");
        return TestnetConfig::production_simulation();
    }

    // TESTNET: Allow level selection via environment variable
    match std::env::var("LOS_TESTNET_LEVEL").as_deref() {
        Ok("functional") => {
            println!("🧪 TESTNET Level 1: Functional testing (instant finalization, mock burns)");
            TestnetConfig::functional()
        }
        Ok("consensus") => {
            println!(
                "🧪 TESTNET Level 2: Consensus testing (real aBFT, real signatures, PoW mining)"
            );
            TestnetConfig::consensus_testing()
        }
        Ok("production") => {
            println!(
                "🧪 TESTNET Level 3: Production simulation (identical to mainnet, full security)"
            );
            TestnetConfig::production_simulation()
        }
        _ => {
            // Default: Consensus testing — this is the minimum for multi-node testnet over Tor
            // Single-node dev should explicitly set LOS_TESTNET_LEVEL=functional
            println!("🧪 TESTNET: Defaulting to Level 2 (Consensus) for multi-node testing");
            println!("   Set LOS_TESTNET_LEVEL=functional for single-node dev mode");
            println!("   Set LOS_TESTNET_LEVEL=production for mainnet-equivalent testing");
            TestnetConfig::consensus_testing()
        }
    }
});

pub fn get_testnet_config() -> &'static TestnetConfig {
    &TESTNET_CONFIG
}

/// Check if we're in any testnet mode (vs mainnet)
/// Returns false only when LOS_NETWORK=mainnet is explicitly set
#[allow(dead_code)]
pub fn is_testnet() -> bool {
    match std::env::var("LOS_NETWORK").as_deref() {
        Ok("mainnet") => false,
        _ => true, // Default: testnet
    }
}

/// Check if we're using production-equivalent code
#[allow(dead_code)]
pub fn is_production_simulation() -> bool {
    matches!(get_testnet_config().level, TestnetLevel::Production)
}

/// Migration helper: Convert old flags to new system
#[allow(dead_code)]
pub fn legacy_testnet_mode() -> bool {
    // Backward compatibility with old TESTNET_MODE flag
    !get_testnet_config().should_enable_consensus()
}

#[allow(dead_code)]
pub fn legacy_dev_mode() -> bool {
    // Backward compatibility with old DEV_MODE flag
    matches!(get_testnet_config().level, TestnetLevel::Functional)
}
