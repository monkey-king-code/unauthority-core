library;

import '../utils/log.dart';

/// LOS FLUTTER WALLET - GRADUATED TESTNET CONFIGURATION
///
/// Aligns with backend graduated testnet levels:
/// - Level 1 (Functional): UI/API testing only
/// - Level 2 (Consensus): Real consensus testing (same as mainnet behavior)
/// - Level 3 (Production): Full mainnet simulation
///
/// For wallet, Level 2 and Level 3 are identical from UX perspective.
/// The difference is backend behavior, not wallet behavior.

enum TestnetLevel {
  /// Level 1: Functional testing
  /// - Immediate transaction confirmation
  /// - Faucet available
  functional,

  /// Level 2/3: Consensus/Production testing
  /// - Real transaction confirmation delays (3 seconds)
  /// - Faucet may be available (Level 2) or disabled (Level 3)
  /// - Wallet behaves identically to mainnet
  consensus,
}

enum NetworkType { testnet, mainnet }

class WalletTestnetConfig {
  final NetworkType network;
  final TestnetLevel? testnetLevel; // null for mainnet
  final String apiUrl;
  final bool faucetAvailable;
  final Duration expectedConfirmationTime;

  const WalletTestnetConfig({
    required this.network,
    this.testnetLevel,
    required this.apiUrl,
    required this.faucetAvailable,
    required this.expectedConfirmationTime,
  });

  /// Level 1 (Functional): Current testnet - UI testing focus
  static WalletTestnetConfig functionalTestnet() {
    return const WalletTestnetConfig(
      network: NetworkType.testnet,
      testnetLevel: TestnetLevel.functional,
      apiUrl: '', // Loaded from NetworkConfig at runtime
      faucetAvailable: true,
      expectedConfirmationTime: Duration(milliseconds: 100), // Immediate
    );
  }

  /// Level 2/3 (Consensus/Production): Production-equivalent behavior
  static WalletTestnetConfig consensusTestnet() {
    return const WalletTestnetConfig(
      network: NetworkType.testnet,
      testnetLevel: TestnetLevel.consensus,
      apiUrl: '', // Loaded from NetworkConfig at runtime
      faucetAvailable: true, // May be disabled in Level 3
      expectedConfirmationTime: Duration(seconds: 3), // Real BFT finality
    );
  }

  /// Mainnet: Production network
  static WalletTestnetConfig mainnet() {
    return const WalletTestnetConfig(
      network: NetworkType.mainnet,
      testnetLevel: null,
      apiUrl: '', // Loaded from NetworkConfig at runtime
      faucetAvailable: false,
      expectedConfirmationTime: Duration(seconds: 3), // Real BFT finality
    );
  }

  /// Get human-readable network name
  String get networkName {
    if (network == NetworkType.mainnet) return 'Mainnet';

    switch (testnetLevel) {
      case TestnetLevel.functional:
        return 'Testnet (Functional)';
      case TestnetLevel.consensus:
        return 'Testnet (Consensus)';
      default:
        return 'Testnet';
    }
  }

  /// Get network badge color
  String get badgeColor {
    if (network == NetworkType.mainnet) return '#00C851'; // Green

    switch (testnetLevel) {
      case TestnetLevel.functional:
        return '#2196F3'; // Blue
      case TestnetLevel.consensus:
        return '#FF9800'; // Orange
      default:
        return '#9E9E9E'; // Gray
    }
  }

  /// Check if real consensus is enabled
  bool get isRealConsensus {
    return network == NetworkType.mainnet ||
        testnetLevel == TestnetLevel.consensus;
  }

  /// Get warning message if in testing mode
  String? get warningMessage {
    if (network == NetworkType.mainnet) return null;

    switch (testnetLevel) {
      case TestnetLevel.functional:
        return '⚠️ Functional Testing: Transactions finalize instantly (not production behavior)';
      case TestnetLevel.consensus:
        return '✅ Consensus Testing: Full production behavior (real BFT consensus)';
      default:
        return null;
    }
  }
}

/// Global wallet configuration
class WalletConfig {
  /// Build-time flag: --dart-define=NETWORK=testnet to override
  static const _networkMode =
      String.fromEnvironment('NETWORK', defaultValue: 'mainnet');
  static WalletTestnetConfig _current = _networkMode == 'mainnet'
      ? WalletTestnetConfig.mainnet()
      : WalletTestnetConfig.functionalTestnet();

  static WalletTestnetConfig get current => _current;

  /// Switch network configuration
  static void setConfig(WalletTestnetConfig config) {
    _current = config;
    losLog('🔄 Wallet switched to: ${config.networkName}');
    losLog('   API: ${config.apiUrl}');
    losLog('   Faucet: ${config.faucetAvailable ? "Available" : "Disabled"}');
    losLog(
        '   Confirmation: ${config.expectedConfirmationTime.inMilliseconds}ms');
  }

  /// Quick setters
  static void useFunctionalTestnet() =>
      setConfig(WalletTestnetConfig.functionalTestnet());
  static void useConsensusTestnet() =>
      setConfig(WalletTestnetConfig.consensusTestnet());
  static void useMainnet() => setConfig(WalletTestnetConfig.mainnet());

  /// Detect from environment variable (if set)
  static void detectFromEnvironment() {
    final networkMode =
        const String.fromEnvironment('NETWORK', defaultValue: 'mainnet');
    if (networkMode == 'mainnet') {
      useMainnet();
      return;
    }
    final env = const String.fromEnvironment('LOS_TESTNET_LEVEL',
        defaultValue: 'functional');

    switch (env) {
      case 'functional':
        useFunctionalTestnet();
        break;
      case 'consensus':
      case 'production':
        useConsensusTestnet();
        break;
      case 'mainnet':
        useMainnet();
        break;
      default:
        losLog('⚠️ Unknown LOS_TESTNET_LEVEL: $env, using functional');
        useFunctionalTestnet();
    }
  }
}
