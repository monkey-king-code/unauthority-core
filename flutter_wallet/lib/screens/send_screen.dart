import '../utils/log.dart';
import '../utils/secure_clipboard.dart';
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/wallet_service.dart';
import '../services/api_service.dart';
import '../services/block_construction_service.dart';
import '../constants/blockchain.dart';
import '../utils/address_validator.dart';

class SendScreen extends StatefulWidget {
  const SendScreen({super.key});

  @override
  State<SendScreen> createState() => _SendScreenState();
}

class _SendScreenState extends State<SendScreen> {
  final _formKey = GlobalKey<FormState>();
  final _toController = TextEditingController();
  final _amountController = TextEditingController();
  bool _isLoading = false;

  Future<void> _sendTransaction() async {
    if (!_formKey.currentState!.validate()) return;

    setState(() => _isLoading = true);
    losLog('💸 [Send] Starting send transaction...');

    try {
      final walletService = context.read<WalletService>();
      final apiService = context.read<ApiService>();
      final wallet = await walletService.getCurrentWallet();

      if (wallet == null) throw Exception('No wallet found');
      losLog('💸 [Send] From: ${wallet['address']}');

      // FIX C11-01: Backend expects LOS in `amount` field.
      // Support decimal amounts (e.g., 0.5 LOS) — BlockConstructionService
      // converts to CIL with full 10^11 precision.
      final amountStr = _amountController.text.trim();
      final amountCil = BlockchainConstants.losStringToCil(amountStr);
      if (amountCil <= 0) {
        throw Exception('Please enter a valid amount greater than 0');
      }

      // FIX H-06: Prevent sending to own address
      final toAddress = _toController.text.trim();
      losLog('💸 [Send] To: $toAddress, Amount: $amountStr LOS');
      if (toAddress == wallet['address']) {
        throw Exception('Cannot send to your own address');
      }

      // Balance validation: compare CIL integers (no f64 precision loss)
      try {
        final account = await apiService.getBalance(wallet['address']!);
        if (amountCil > account.balance) {
          throw Exception(
              'Insufficient balance: have ${BlockchainConstants.formatCilAsLos(account.balance)} LOS');
        }
      } catch (e) {
        if (e.toString().contains('Insufficient balance')) rethrow;
        // If balance check fails (network), let the backend reject
      }

      // Use BlockConstructionService for full client-side block construction
      // (PoW + Dilithium5 signing) — required for external addresses on L2+
      final blockService = BlockConstructionService(
        api: apiService,
        wallet: walletService,
      );

      // Check if wallet has signing keys (Dilithium5 keypair)
      final hasKeys = wallet['public_key'] != null;

      Map<String, dynamic> result;
      if (hasKeys) {
        // Full client-side signing via BlockConstructionService
        losLog('💸 [Send] Client-side signing with Dilithium5...');
        result = await blockService.sendTransaction(
          to: toAddress,
          amountLosStr: amountStr,
        );
      } else {
        // SECURITY FIX H-02: Refuse unsigned node-signed transactions on mainnet.
        // Address-only imports cannot produce valid signatures — node-signed
        // transactions are only acceptable on functional testnet.
        if (apiService.environment == NetworkEnvironment.mainnet) {
          throw Exception(
            'Mainnet requires signed transactions. '
            'Please import your wallet with a seed phrase or private key.',
          );
        }

        // Address-only import — no keys, let node sign (TESTNET ONLY)
        losLog(
            '💸 [Send] No signing keys — node-signed (functional testnet)...');
        // FIX: Use amountCil for sub-LOS precision (0.5 LOS = 50_000_000_000 CIL).
        final amountCilInt =
            BlockchainConstants.losStringToCil(_amountController.text.trim());
        result = await apiService.sendTransaction(
          from: wallet['address']!,
          to: toAddress,
          amount: amountCil ~/ BlockchainConstants.cilPerLos,
          amountCil: amountCilInt,
        );
      }

      if (!mounted) return;
      final txHash = result['tx_hash'] ?? result['txid'] ?? 'N/A';
      losLog('💸 [Send] SUCCESS: $txHash');

      Navigator.pop(context);

      // Show success dialog with copyable TX hash
      if (context.mounted) {
        showDialog(
          context: context,
          builder: (ctx) => AlertDialog(
            title: const Row(
              children: [
                Icon(Icons.check_circle, color: Colors.green, size: 28),
                SizedBox(width: 8),
                Text('Transaction Sent!'),
              ],
            ),
            content: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                const Text('TX Hash:',
                    style: TextStyle(fontSize: 12, color: Colors.grey)),
                const SizedBox(height: 4),
                Container(
                  padding: const EdgeInsets.all(12),
                  decoration: BoxDecoration(
                    color: Colors.grey.withValues(alpha: 0.1),
                    borderRadius: BorderRadius.circular(8),
                  ),
                  child: SelectableText(
                    txHash,
                    style: const TextStyle(
                      fontSize: 11,
                      fontFamily: 'monospace',
                    ),
                  ),
                ),
              ],
            ),
            actions: [
              TextButton.icon(
                icon: const Icon(Icons.copy, size: 18),
                label: const Text('Copy TX Hash'),
                onPressed: () {
                  SecureClipboard.copyPublic(txHash);
                  ScaffoldMessenger.of(ctx).showSnackBar(
                    const SnackBar(
                      content: Text('TX Hash copied to clipboard'),
                      duration: Duration(seconds: 2),
                    ),
                  );
                },
              ),
              TextButton(
                onPressed: () => Navigator.pop(ctx),
                child: const Text('OK'),
              ),
            ],
          ),
        );
      }
    } catch (e) {
      if (!mounted) return;
      losLog('💸 [Send] ERROR: $e');
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text(e.toString()), backgroundColor: Colors.red),
      );
    } finally {
      if (mounted) setState(() => _isLoading = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('Send LOS'),
        centerTitle: true,
      ),
      body: SafeArea(
        child: Padding(
          padding: const EdgeInsets.all(24.0),
          child: Form(
            key: _formKey,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                const Icon(Icons.send, size: 80, color: Colors.blue),
                const SizedBox(height: 32),
                TextFormField(
                  controller: _toController,
                  decoration: const InputDecoration(
                    labelText: 'To Address',
                    hintText: 'LOS...',
                    border: OutlineInputBorder(),
                    prefixIcon: Icon(Icons.person),
                  ),
                  validator: (value) {
                    if (value == null || value.trim().isEmpty) {
                      return 'Please enter recipient address';
                    }
                    // FIX C11-09: Use AddressValidator for consistent
                    // hex/Base58 character validation across all screens
                    return AddressValidator.getValidationError(value.trim());
                  },
                ),
                const SizedBox(height: 16),
                TextFormField(
                  controller: _amountController,
                  decoration: const InputDecoration(
                    labelText: 'Amount (LOS)',
                    hintText: '0.5',
                    helperText: 'Supports decimals (e.g., 0.5 LOS)',
                    border: OutlineInputBorder(),
                    prefixIcon: Icon(Icons.attach_money),
                  ),
                  keyboardType:
                      const TextInputType.numberWithOptions(decimal: true),
                  validator: (value) {
                    if (value == null || value.trim().isEmpty) {
                      return 'Please enter amount';
                    }
                    final amount = double.tryParse(value.trim());
                    if (amount == null || amount <= 0) {
                      return 'Please enter a number greater than 0';
                    }
                    return null;
                  },
                ),
                const SizedBox(height: 32),
                ElevatedButton(
                  onPressed: _isLoading ? null : _sendTransaction,
                  style: ElevatedButton.styleFrom(
                    padding: const EdgeInsets.all(16),
                  ),
                  child: _isLoading
                      ? const CircularProgressIndicator()
                      : const Text('SEND TRANSACTION',
                          style: TextStyle(fontSize: 16)),
                ),
                const SizedBox(height: 16),
                const Text(
                  '⚠️ Make sure the address is correct. Transactions cannot be reversed!',
                  style: TextStyle(fontSize: 12, color: Colors.orange),
                  textAlign: TextAlign.center,
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  @override
  void dispose() {
    _toController.dispose();
    _amountController.dispose();
    super.dispose();
  }
}
