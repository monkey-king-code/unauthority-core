import 'package:flutter/material.dart';
import '../config/testnet_config.dart';

/// Network Badge Widget
/// Shows current network/testnet level with appropriate styling
class NetworkBadge extends StatelessWidget {
  const NetworkBadge({super.key});

  Color _parseColor(String hexColor) {
    hexColor = hexColor.replaceAll('#', '');
    return Color(int.parse('FF$hexColor', radix: 16));
  }

  @override
  Widget build(BuildContext context) {
    final config = WalletConfig.current;

    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
      decoration: BoxDecoration(
        color: _parseColor(config.badgeColor).withValues(alpha: 0.1),
        border: Border.all(
          color: _parseColor(config.badgeColor),
          width: 1.5,
        ),
        borderRadius: BorderRadius.circular(20),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(
            Icons.circle,
            size: 8,
            color: _parseColor(config.badgeColor),
          ),
          const SizedBox(width: 6),
          Text(
            config.networkName,
            style: TextStyle(
              color: _parseColor(config.badgeColor),
              fontWeight: FontWeight.bold,
              fontSize: 12,
            ),
          ),
        ],
      ),
    );
  }
}

/// Network Warning Banner
/// Shows warning message for non-production testnet levels
class NetworkWarningBanner extends StatelessWidget {
  const NetworkWarningBanner({super.key});

  @override
  Widget build(BuildContext context) {
    final config = WalletConfig.current;
    final warning = config.warningMessage;

    if (warning == null) return const SizedBox.shrink();

    return Container(
      width: double.infinity,
      padding: const EdgeInsets.all(12),
      margin: const EdgeInsets.all(16),
      decoration: BoxDecoration(
        color: config.testnetLevel == TestnetLevel.functional
            ? Colors.blue.withValues(alpha: 0.1)
            : Colors.orange.withValues(alpha: 0.1),
        border: Border.all(
          color: config.testnetLevel == TestnetLevel.functional
              ? Colors.blue
              : Colors.orange,
          width: 1,
        ),
        borderRadius: BorderRadius.circular(8),
      ),
      child: Row(
        children: [
          Icon(
            config.testnetLevel == TestnetLevel.functional
                ? Icons.info_outline
                : Icons.check_circle_outline,
            color: config.testnetLevel == TestnetLevel.functional
                ? Colors.blue
                : Colors.orange,
            size: 20,
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Text(
              warning,
              style: TextStyle(
                color: config.testnetLevel == TestnetLevel.functional
                    ? Colors.blue[900]
                    : Colors.orange[900],
                fontSize: 12,
                fontWeight: FontWeight.w500,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// Network Settings Screen
/// Advanced network settings — testnet level switching.
/// Mainnet/Testnet toggle is in the main Settings screen (SegmentedButton).
class NetworkSettingsScreen extends StatelessWidget {
  const NetworkSettingsScreen({super.key});

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('Advanced Network Settings'),
        backgroundColor: const Color.fromARGB(255, 0, 123, 254),
        foregroundColor: Colors.white,
      ),
      body: ListView(
        children: [
          const Padding(
            padding: EdgeInsets.all(16.0),
            child: Text(
              'TESTNET LEVELS',
              style: TextStyle(
                fontSize: 12,
                fontWeight: FontWeight.bold,
                color: Colors.grey,
              ),
            ),
          ),
          const Padding(
            padding: EdgeInsets.symmetric(horizontal: 16.0),
            child: Text(
              'These settings only affect Testnet mode.\n'
              'Switch between Testnet/Mainnet in the main Settings screen.',
              style: TextStyle(fontSize: 12, color: Colors.grey),
            ),
          ),
          const SizedBox(height: 8),
          ListTile(
            leading: const Icon(Icons.science, color: Colors.blue),
            title: const Text('Level 1: Functional Testing'),
            subtitle: const Text('Immediate finalization, faucet enabled'),
            trailing:
                WalletConfig.current.testnetLevel == TestnetLevel.functional
                    ? const Icon(Icons.check_circle, color: Colors.green)
                    : null,
            enabled: WalletConfig.current.network == NetworkType.testnet,
            onTap: () {
              if (WalletConfig.current.network != NetworkType.testnet) return;
              WalletConfig.useFunctionalTestnet();
              ScaffoldMessenger.of(context).showSnackBar(
                const SnackBar(content: Text('Switched to Functional Testnet')),
              );
              Navigator.pop(context);
            },
          ),
          ListTile(
            leading: const Icon(Icons.flash_on, color: Colors.orange),
            title: const Text('Level 2/3: Consensus Testing'),
            subtitle: const Text('Real BFT consensus, production behavior'),
            trailing:
                WalletConfig.current.testnetLevel == TestnetLevel.consensus
                    ? const Icon(Icons.check_circle, color: Colors.green)
                    : null,
            enabled: WalletConfig.current.network == NetworkType.testnet,
            onTap: () {
              if (WalletConfig.current.network != NetworkType.testnet) return;
              WalletConfig.useConsensusTestnet();
              ScaffoldMessenger.of(context).showSnackBar(
                const SnackBar(content: Text('Switched to Consensus Testnet')),
              );
              Navigator.pop(context);
            },
          ),
          const Padding(
            padding: EdgeInsets.all(16.0),
            child: Card(
              child: Padding(
                padding: EdgeInsets.all(16.0),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      'About Testnet Levels',
                      style: TextStyle(fontWeight: FontWeight.bold),
                    ),
                    SizedBox(height: 8),
                    Text(
                      '• Level 1: UI testing, instant finalization\n'
                      '• Level 2/3: Real consensus, 3-second finality\n\n'
                      'Backend automatically matches wallet level.\n'
                      'Use Level 1 for development, Level 2/3 for production testing.',
                      style: TextStyle(fontSize: 12, color: Colors.grey),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}
