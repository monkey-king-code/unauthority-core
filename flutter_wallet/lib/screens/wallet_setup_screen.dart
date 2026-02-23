import '../utils/log.dart';
import '../utils/secure_clipboard.dart';
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/wallet_service.dart';
import '../utils/address_validator.dart';
import 'home_screen.dart';

class WalletSetupScreen extends StatefulWidget {
  const WalletSetupScreen({super.key});

  @override
  State<WalletSetupScreen> createState() => _WalletSetupScreenState();
}

class _WalletSetupScreenState extends State<WalletSetupScreen> {
  final _formKey = GlobalKey<FormState>();
  final _mnemonicController = TextEditingController();
  final _addressController = TextEditingController();
  bool _isLoading = false;
  // 0 = main menu, 1 = import mnemonic, 2 = import address
  int _mode = 0;

  Future<void> _generateWallet() async {
    losLog('💰 [WalletSetupScreen._generateWallet] Generating new wallet...');
    setState(() => _isLoading = true);

    try {
      final walletService = context.read<WalletService>();
      final wallet = await walletService.generateWallet();
      losLog(
          '💰 [WalletSetupScreen._generateWallet] SUCCESS address=${wallet['address']}');

      if (!mounted) return;

      await showDialog(
        context: context,
        barrierDismissible: false,
        builder: (context) => AlertDialog(
          title: const Text('⚠️ Backup Your Seed Phrase'),
          content: SingleChildScrollView(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                const Text(
                  'Write down these 24 words in order. This is the ONLY way to recover your wallet!',
                  style: TextStyle(color: Colors.redAccent),
                ),
                const SizedBox(height: 16),
                Container(
                  padding: const EdgeInsets.all(12),
                  decoration: BoxDecoration(
                    color: Colors.black26,
                    borderRadius: BorderRadius.circular(8),
                  ),
                  child: SelectableText(
                    wallet['mnemonic']!,
                    style: const TextStyle(fontSize: 14),
                  ),
                ),
                const SizedBox(height: 8),
                Align(
                  alignment: Alignment.centerRight,
                  child: TextButton.icon(
                    icon: const Icon(Icons.copy, size: 16),
                    label: const Text('Copy Seed Phrase'),
                    style: TextButton.styleFrom(
                      foregroundColor: Colors.orangeAccent,
                    ),
                    onPressed: () {
                      SecureClipboard.copy(wallet['mnemonic']!);
                      ScaffoldMessenger.of(context).showSnackBar(
                        const SnackBar(
                          content:
                              Text('Seed phrase copied (auto-clears in 30s)'),
                          backgroundColor: Colors.orange,
                          duration: Duration(seconds: 2),
                        ),
                      );
                    },
                  ),
                ),
                const SizedBox(height: 8),
                Row(
                  children: [
                    Expanded(
                      child: Text(
                        'Address: ${wallet['address']}',
                        style:
                            const TextStyle(fontSize: 12, color: Colors.grey),
                      ),
                    ),
                  ],
                ),
              ],
            ),
          ),
          actions: [
            TextButton(
              onPressed: () {
                Navigator.pop(context);
                Navigator.pushReplacement(
                  context,
                  MaterialPageRoute(builder: (_) => const HomeScreen()),
                );
              },
              child: const Text('I HAVE SAVED IT'),
            ),
          ],
        ),
      );
    } catch (e) {
      losLog('💰 [WalletSetupScreen._generateWallet] ERROR: $e');
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('Error: $e'), backgroundColor: Colors.red),
      );
    } finally {
      if (mounted) setState(() => _isLoading = false);
    }
  }

  Future<void> _importWallet() async {
    if (!_formKey.currentState!.validate()) return;

    losLog(
        '💰 [WalletSetupScreen._importWallet] Importing wallet from mnemonic...');
    setState(() => _isLoading = true);

    try {
      final walletService = context.read<WalletService>();
      await walletService.importWallet(_mnemonicController.text.trim());
      losLog('💰 [WalletSetupScreen._importWallet] SUCCESS');

      if (!mounted) return;

      Navigator.pushReplacement(
        context,
        MaterialPageRoute(builder: (_) => const HomeScreen()),
      );
    } catch (e) {
      losLog('💰 [WalletSetupScreen._importWallet] ERROR: $e');
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('Error: $e'), backgroundColor: Colors.red),
      );
    } finally {
      if (mounted) setState(() => _isLoading = false);
    }
  }

  /// Import by address only (testnet genesis accounts)
  Future<void> _importByAddress() async {
    if (!_formKey.currentState!.validate()) return;

    losLog('💰 [WalletSetupScreen._importByAddress] Importing by address...');
    setState(() => _isLoading = true);

    try {
      final walletService = context.read<WalletService>();
      await walletService.importByAddress(_addressController.text.trim());
      losLog('💰 [WalletSetupScreen._importByAddress] SUCCESS');

      if (!mounted) return;

      Navigator.pushReplacement(
        context,
        MaterialPageRoute(builder: (_) => const HomeScreen()),
      );
    } catch (e) {
      losLog('💰 [WalletSetupScreen._importByAddress] ERROR: $e');
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('Error: $e'), backgroundColor: Colors.red),
      );
    } finally {
      if (mounted) setState(() => _isLoading = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('LOS Wallet Setup'),
        centerTitle: true,
        leading: _mode != 0
            ? IconButton(
                icon: const Icon(Icons.arrow_back),
                onPressed: () => setState(() => _mode = 0),
              )
            : null,
      ),
      body: SafeArea(
        child: Padding(
          padding: const EdgeInsets.all(24.0),
          child: _mode == 0
              ? _buildMainMenu()
              : _mode == 1
                  ? _buildImportMnemonic()
                  : _buildImportAddress(),
        ),
      ),
    );
  }

  Widget _buildMainMenu() {
    return Column(
      mainAxisAlignment: MainAxisAlignment.center,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Icon(
          Icons.account_balance_wallet,
          size: 80,
          color: Theme.of(context).colorScheme.primary,
        ),
        const SizedBox(height: 24),
        Text(
          'Welcome to LOS Wallet',
          style: Theme.of(context).textTheme.headlineMedium,
          textAlign: TextAlign.center,
        ),
        const SizedBox(height: 48),

        // Create New Wallet
        ElevatedButton.icon(
          onPressed: _isLoading ? null : _generateWallet,
          icon: const Icon(Icons.add),
          style: ElevatedButton.styleFrom(
            padding: const EdgeInsets.all(16),
          ),
          label: _isLoading
              ? const CircularProgressIndicator()
              : const Text('CREATE NEW WALLET', style: TextStyle(fontSize: 16)),
        ),
        const SizedBox(height: 16),

        // Import from Seed Phrase
        OutlinedButton.icon(
          onPressed: _isLoading ? null : () => setState(() => _mode = 1),
          icon: const Icon(Icons.vpn_key),
          style: OutlinedButton.styleFrom(
            padding: const EdgeInsets.all(16),
          ),
          label:
              const Text('IMPORT SEED PHRASE', style: TextStyle(fontSize: 16)),
        ),
        const SizedBox(height: 16),

        // Import by Address — testnet only (hidden on mainnet builds)
        if (const String.fromEnvironment('NETWORK', defaultValue: 'mainnet') !=
            'mainnet') ...[
          OutlinedButton.icon(
            onPressed: _isLoading ? null : () => setState(() => _mode = 2),
            icon: const Icon(Icons.account_circle),
            style: OutlinedButton.styleFrom(
              padding: const EdgeInsets.all(16),
              foregroundColor: Colors.orange,
            ),
            label: const Text('IMPORT BY ADDRESS (TESTNET)',
                style: TextStyle(fontSize: 14)),
          ),
          const SizedBox(height: 16),
          const Text(
            'Import by Address: Use a pre-funded testnet genesis address\nwithout needing a seed phrase.',
            style: TextStyle(fontSize: 11, color: Colors.grey),
            textAlign: TextAlign.center,
          ),
        ],
      ],
    );
  }

  Widget _buildImportMnemonic() {
    return Column(
      mainAxisAlignment: MainAxisAlignment.center,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(
          'Import Wallet',
          style: Theme.of(context).textTheme.headlineMedium,
          textAlign: TextAlign.center,
        ),
        const SizedBox(height: 32),
        Form(
          key: _formKey,
          child: TextFormField(
            controller: _mnemonicController,
            maxLines: 3,
            decoration: const InputDecoration(
              labelText: 'Seed Phrase (12 or 24 words)',
              hintText: 'word1 word2 word3 ...',
              border: OutlineInputBorder(),
            ),
            validator: (value) {
              if (value == null || value.trim().isEmpty) {
                return 'Please enter your seed phrase';
              }
              final words = value.trim().split(RegExp(r'\s+'));
              if (words.length != 12 && words.length != 24) {
                return 'Seed phrase must be 12 or 24 words';
              }
              return null;
            },
          ),
        ),
        const SizedBox(height: 24),
        ElevatedButton(
          onPressed: _isLoading ? null : _importWallet,
          style: ElevatedButton.styleFrom(
            padding: const EdgeInsets.all(16),
          ),
          child: _isLoading
              ? const CircularProgressIndicator()
              : const Text('IMPORT WALLET', style: TextStyle(fontSize: 16)),
        ),
      ],
    );
  }

  Widget _buildImportAddress() {
    return Column(
      mainAxisAlignment: MainAxisAlignment.center,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(
          'Import by Address',
          style: Theme.of(context).textTheme.headlineMedium,
          textAlign: TextAlign.center,
        ),
        const SizedBox(height: 8),
        const Text(
          'For testnet genesis accounts. You can view balance and request faucet.\nTransactions are signed by the node in L1 testnet mode.',
          style: TextStyle(fontSize: 12, color: Colors.orange),
          textAlign: TextAlign.center,
        ),
        const SizedBox(height: 32),
        Form(
          key: _formKey,
          child: TextFormField(
            controller: _addressController,
            decoration: const InputDecoration(
              labelText: 'LOS Address',
              hintText: 'LOS...',
              border: OutlineInputBorder(),
              prefixIcon: Icon(Icons.account_circle),
            ),
            validator: (value) {
              if (value == null || value.trim().isEmpty) {
                return 'Please enter LOS address';
              }
              // Use AddressValidator for consistent validation
              // across all screens (supports both hex and Base58 formats).
              return AddressValidator.getValidationError(value.trim());
            },
          ),
        ),
        const SizedBox(height: 24),
        ElevatedButton(
          onPressed: _isLoading ? null : _importByAddress,
          style: ElevatedButton.styleFrom(
            padding: const EdgeInsets.all(16),
            backgroundColor: Colors.orange,
          ),
          child: _isLoading
              ? const CircularProgressIndicator()
              : const Text('IMPORT ADDRESS', style: TextStyle(fontSize: 16)),
        ),
      ],
    );
  }

  @override
  void dispose() {
    _mnemonicController.dispose();
    _addressController.dispose();
    super.dispose();
  }
}
