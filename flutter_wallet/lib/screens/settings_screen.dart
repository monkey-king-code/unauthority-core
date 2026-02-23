import '../utils/log.dart';
import '../utils/secure_clipboard.dart';
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../constants/blockchain.dart';
import '../services/wallet_service.dart';
import 'wallet_setup_screen.dart';

class SettingsScreen extends StatefulWidget {
  const SettingsScreen({super.key});

  @override
  State<SettingsScreen> createState() => _SettingsScreenState();
}

class _SettingsScreenState extends State<SettingsScreen> {
  bool _showSeedPhrase = false;
  String? _seedPhrase;

  /// SECURITY FIX M-02: Clear seed phrase reference on widget disposal.
  /// Removes the Dart String reference so GC can collect sooner.
  /// (Dart Strings are immutable — content persists until GC, but
  /// removing the reference is the best Dart can do.)
  @override
  void dispose() {
    _seedPhrase = null;
    _showSeedPhrase = false;
    super.dispose();
  }

  /// FIX H-02: Lazy-load seed phrase only when user taps reveal.
  /// Prevents keeping mnemonic in widget state longer than necessary.
  /// SECURITY FIX A-03: Require user confirmation before revealing seed phrase.
  /// This prevents shoulder-surfing and casual access on unlocked devices.
  Future<void> _revealSeedPhrase() async {
    losLog(
        '⚙️ [SettingsScreen._revealSeedPhrase] Toggling seed phrase visibility...');
    if (_seedPhrase != null) {
      // Already loaded — just toggle visibility (hide)
      setState(() => _showSeedPhrase = !_showSeedPhrase);
      return;
    }

    // SECURITY FIX A-03: Gate behind explicit user confirmation
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Row(
          children: [
            Icon(Icons.warning_amber_rounded, color: Colors.orange, size: 28),
            SizedBox(width: 8),
            Text('Security Warning'),
          ],
        ),
        content: const Text(
          'Your seed phrase gives FULL ACCESS to your wallet and all funds.\n\n'
          '• Never share it with anyone\n'
          '• Never enter it on websites\n'
          '• Make sure no one is watching your screen\n\n'
          'Are you sure you want to reveal it?',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('CANCEL'),
          ),
          ElevatedButton(
            onPressed: () => Navigator.pop(context, true),
            style: ElevatedButton.styleFrom(
              backgroundColor: Colors.orange,
            ),
            child: const Text('REVEAL SEED PHRASE'),
          ),
        ],
      ),
    );

    if (confirmed != true || !mounted) return;

    final walletService = context.read<WalletService>();
    final wallet = await walletService.getCurrentWallet(includeMnemonic: true);
    if (wallet != null && mounted) {
      setState(() {
        _seedPhrase = wallet['mnemonic'];
        _showSeedPhrase = true;
      });
      losLog('⚙️ [SettingsScreen._revealSeedPhrase] Seed phrase revealed');
    }
  }

  void _copySeedPhrase() {
    if (_seedPhrase != null) {
      // SECURITY FIX I-01: Use SecureClipboard with auto-clear (30s default)
      SecureClipboard.copy(_seedPhrase!);
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(
          content: Text('Seed phrase copied to clipboard (auto-clears in 30s)'),
          backgroundColor: Colors.orange,
          duration: Duration(seconds: 2),
        ),
      );
    }
  }

  Future<void> _confirmLogout() async {
    losLog('⚙️ [SettingsScreen._confirmLogout] Showing logout confirmation...');
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Logout'),
        content: const Text(
          'Are you sure you want to logout? Make sure you have backed up your seed phrase!',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('CANCEL'),
          ),
          ElevatedButton(
            onPressed: () => Navigator.pop(context, true),
            style: ElevatedButton.styleFrom(backgroundColor: Colors.red),
            child: const Text('LOGOUT'),
          ),
        ],
      ),
    );

    if (confirmed == true && mounted) {
      losLog('⚙️ [SettingsScreen._confirmLogout] Logout confirmed');
      final walletService = context.read<WalletService>();
      await walletService.clearWallet();

      if (mounted) {
        Navigator.of(context).pushAndRemoveUntil(
          MaterialPageRoute(builder: (_) => const WalletSetupScreen()),
          (route) => false,
        );
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('Settings'),
        centerTitle: true,
      ),
      body: ListView(
        children: [
          // Backup Seed Phrase
          Card(
            margin: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
            child: Column(
              children: [
                ListTile(
                  leading: const Icon(Icons.vpn_key, color: Colors.orange),
                  title: const Text('Backup Seed Phrase'),
                  subtitle: const Text('Save your recovery phrase'),
                  trailing: IconButton(
                    icon: Icon(_showSeedPhrase
                        ? Icons.visibility_off
                        : Icons.visibility),
                    onPressed: _revealSeedPhrase,
                  ),
                ),
                if (_showSeedPhrase && _seedPhrase != null) ...[
                  const Divider(),
                  Container(
                    margin: const EdgeInsets.all(16),
                    padding: const EdgeInsets.all(16),
                    decoration: BoxDecoration(
                      color: const Color(0x1AFF9800), // orange at 10% opacity
                      borderRadius: BorderRadius.circular(8),
                      border: Border.all(color: Colors.orange),
                    ),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        const Row(
                          children: [
                            Icon(Icons.warning, color: Colors.orange, size: 20),
                            SizedBox(width: 8),
                            Text(
                              'KEEP THIS SECRET!',
                              style: TextStyle(
                                color: Colors.orange,
                                fontWeight: FontWeight.bold,
                              ),
                            ),
                          ],
                        ),
                        // SECURITY FIX F-03: Screenshot/screen-recording warning
                        const SizedBox(height: 8),
                        Container(
                          padding: const EdgeInsets.all(8),
                          decoration: BoxDecoration(
                            color: const Color(0x1AF44336), // red at 10%
                            borderRadius: BorderRadius.circular(4),
                            border: Border.all(
                                color: Colors.red.withValues(alpha: 0.5)),
                          ),
                          child: const Row(
                            children: [
                              Icon(Icons.screen_lock_portrait,
                                  color: Colors.red, size: 16),
                              SizedBox(width: 8),
                              Expanded(
                                child: Text(
                                  'Disable screen recording & screenshots before viewing.',
                                  style: TextStyle(
                                      color: Colors.red, fontSize: 12),
                                ),
                              ),
                            ],
                          ),
                        ),
                        const SizedBox(height: 12),
                        SelectableText(
                          _seedPhrase!,
                          style: const TextStyle(
                            fontSize: 14,
                            fontFamily: 'monospace',
                          ),
                        ),
                        const SizedBox(height: 12),
                        ElevatedButton.icon(
                          onPressed: _copySeedPhrase,
                          icon: const Icon(Icons.copy, size: 18),
                          label: const Text('COPY'),
                          style: ElevatedButton.styleFrom(
                            backgroundColor: Colors.orange,
                          ),
                        ),
                      ],
                    ),
                  ),
                ],
              ],
            ),
          ),

          // About Section
          Card(
            margin: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
            child: Column(
              children: [
                ListTile(
                  leading: const Icon(Icons.info),
                  title: const Text('About LOS'),
                  subtitle: const Text('Blockchain information'),
                  onTap: () => _showAboutDialog(),
                ),
                const Divider(height: 1),
                ListTile(
                  leading: const Icon(Icons.help),
                  title: const Text('Help & Support'),
                  subtitle: const Text('Documentation and FAQ'),
                  onTap: () {
                    ScaffoldMessenger.of(context).showSnackBar(
                      const SnackBar(
                          content: Text('Documentation coming soon')),
                    );
                  },
                ),
              ],
            ),
          ),

          const SizedBox(height: 24),

          // Danger Zone
          Card(
            margin: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
            color: const Color(0x1AF44336), // red at 10% opacity
            child: ListTile(
              leading: const Icon(Icons.logout, color: Colors.red),
              title: const Text(
                'Logout',
                style:
                    TextStyle(color: Colors.red, fontWeight: FontWeight.bold),
              ),
              subtitle: const Text('Clear wallet data from this device'),
              onTap: _confirmLogout,
            ),
          ),

          const SizedBox(height: 16),
          const Center(
            child: Text(
              'LOS Wallet v${BlockchainConstants.version}\nBuilt with Flutter',
              textAlign: TextAlign.center,
              style: TextStyle(fontSize: 12, color: Colors.grey),
            ),
          ),
          const SizedBox(height: 32),
        ],
      ),
    );
  }

  void _showAboutDialog() {
    showDialog(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('About Unauthority (LOS)'),
        content: const SingleChildScrollView(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(
                'Unauthority is a privacy-focused blockchain with:',
                style: TextStyle(fontWeight: FontWeight.bold),
              ),
              SizedBox(height: 12),
              Text('• aBFT consensus with linear stake voting'),
              Text('• PoW mining distribution (~96.5% public)'),
              Text('• Fixed supply: 21.9 million LOS'),
              Text('• Tor-only mainnet for privacy'),
              Text('• WASM smart contracts (UVM)'),
              Text('• Linear validator staking (1 CIL = 1 vote)'),
              SizedBox(height: 12),
              Text(
                'Network Status:',
                style: TextStyle(fontWeight: FontWeight.bold),
              ),
              Text('• Testnet: Online (.onion)'),
              Text('• Mainnet: Online (.onion)'),
            ],
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('CLOSE'),
          ),
        ],
      ),
    );
  }
}
