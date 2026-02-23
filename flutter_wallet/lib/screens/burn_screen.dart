import '../utils/log.dart';
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../config/testnet_config.dart';
import '../services/wallet_service.dart';
import '../services/api_service.dart';

/// Burn Bridge Screen
///
/// Matches backend `BurnRequest`: `{ coin_type: "eth"|"btc", txid: String, recipient_address: Option<String> }`
class BurnScreen extends StatefulWidget {
  const BurnScreen({super.key});

  @override
  State<BurnScreen> createState() => _BurnScreenState();
}

class _BurnScreenState extends State<BurnScreen> {
  final _formKey = GlobalKey<FormState>();
  final _txidController = TextEditingController();
  String _selectedCoin = 'btc'; // "btc" or "eth"
  bool _isLoading = false;

  Future<void> _submitBurn() async {
    if (!_formKey.currentState!.validate()) return;

    setState(() => _isLoading = true);
    losLog('🔥 [Burn] Starting burn submission...');

    try {
      final walletService = context.read<WalletService>();
      final apiService = context.read<ApiService>();
      final wallet = await walletService.getCurrentWallet();

      if (wallet == null) throw Exception('No wallet found');
      final recipientAddress = wallet['address']!;
      final publicKeyHex = await walletService.getPublicKeyHex();
      // Sanitize TXID — must match backend: trim, strip 0x, lowercase
      var cleanTxid = _txidController.text.trim();
      if (cleanTxid.startsWith('0x') || cleanTxid.startsWith('0X')) {
        cleanTxid = cleanTxid.substring(2);
      }
      cleanTxid = cleanTxid.toLowerCase();

      losLog('🔥 [Burn] Coin: $_selectedCoin, TXID: $cleanTxid');
      losLog('🔥 [Burn] Recipient: $recipientAddress');

      // Sign burn message for authenticated burns.
      // Backend expects: "BURN:{coin_type}:{txid}:{recipient}"
      String? signature;
      if (publicKeyHex != null) {
        final burnMessage = 'BURN:$_selectedCoin:$cleanTxid:$recipientAddress';
        losLog('🔥 [Burn] Signing message: $burnMessage');
        signature = await walletService.signTransaction(burnMessage);
        losLog('🔥 [Burn] Signature: ${signature.length} hex chars');
      }

      final result = await apiService.submitBurn(
        coinType: _selectedCoin,
        txid: cleanTxid,
        recipientAddress: recipientAddress,
        signature: signature,
        publicKey: publicKeyHex,
      );

      if (!mounted) return;
      losLog('🔥 [Burn] SUCCESS: ${result['msg']}');

      Navigator.pop(context);
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: Text(
              'Burn submitted! ${result['msg'] ?? 'Pending validator verification'}'),
          backgroundColor: Colors.green,
          duration: const Duration(seconds: 5),
        ),
      );
    } catch (e) {
      if (!mounted) return;
      losLog('🔥 [Burn] ERROR: $e');
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
        title: const Text('Burn Bridge'),
        centerTitle: true,
      ),
      body: SafeArea(
        child: Padding(
          padding: const EdgeInsets.all(24.0),
          child: Form(
            key: _formKey,
            child: ListView(
              children: [
                const Icon(
                  Icons.local_fire_department,
                  size: 80,
                  color: Colors.orange,
                ),
                const SizedBox(height: 16),
                const Text(
                  'Burn BTC or ETH to receive LOS',
                  style: TextStyle(fontSize: 18, fontWeight: FontWeight.bold),
                  textAlign: TextAlign.center,
                ),
                const SizedBox(height: 8),
                const Text(
                  'Submit your burn transaction ID. Oracle validators will verify and mint LOS to your wallet.',
                  style: TextStyle(fontSize: 12, color: Colors.grey),
                  textAlign: TextAlign.center,
                ),
                const SizedBox(height: 32),

                // Coin Type Selector
                const Text('Select Coin Type',
                    style: TextStyle(fontWeight: FontWeight.bold)),
                const SizedBox(height: 8),
                SegmentedButton<String>(
                  segments: const [
                    ButtonSegment(
                      value: 'btc',
                      label: Text('BTC'),
                      icon: Icon(Icons.currency_bitcoin),
                    ),
                    ButtonSegment(
                      value: 'eth',
                      label: Text('ETH'),
                      icon: Icon(Icons.currency_exchange),
                    ),
                  ],
                  selected: {_selectedCoin},
                  onSelectionChanged: (Set<String> selected) {
                    setState(() => _selectedCoin = selected.first);
                  },
                ),

                const SizedBox(height: 24),

                // TXID Input
                TextFormField(
                  controller: _txidController,
                  decoration: InputDecoration(
                    labelText: _selectedCoin == 'btc'
                        ? 'Bitcoin Transaction ID'
                        : 'Ethereum Transaction Hash',
                    hintText: _selectedCoin == 'btc'
                        ? '64-character hex TXID'
                        : '0x... (66 characters)',
                    border: const OutlineInputBorder(),
                    prefixIcon: Icon(_selectedCoin == 'btc'
                        ? Icons.currency_bitcoin
                        : Icons.currency_exchange),
                  ),
                  validator: (value) {
                    if (value == null || value.trim().isEmpty) {
                      return 'Please enter transaction ID';
                    }
                    final txid = value.trim();
                    if (_selectedCoin == 'btc') {
                      if (txid.length != 64) {
                        return 'BTC TXID must be exactly 64 characters';
                      }
                      if (!RegExp(r'^[a-fA-F0-9]+$').hasMatch(txid)) {
                        return 'Invalid hex characters in TXID';
                      }
                    } else {
                      // ETH
                      if (!txid.startsWith('0x') || txid.length != 66) {
                        return 'ETH TXID must start with 0x and be 66 characters';
                      }
                    }
                    return null;
                  },
                ),

                const SizedBox(height: 12),

                // Test TXID buttons (for testnet convenience — hidden on mainnet)
                if (WalletConfig.current.network != NetworkType.mainnet)
                  Card(
                    color: const Color(0xFF1A2332),
                    child: Padding(
                      padding: const EdgeInsets.all(12),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          const Text(
                            '🧪 Test TXIDs (Testnet)',
                            style: TextStyle(
                              fontSize: 12,
                              fontWeight: FontWeight.bold,
                              color: Colors.blueGrey,
                            ),
                          ),
                          const SizedBox(height: 8),
                          Wrap(
                            spacing: 8,
                            runSpacing: 8,
                            children: [
                              _TestTxidChip(
                                label: 'BTC Test',
                                txid:
                                    '2096b844178ecc776e050be7886e618ee111e2a68fcf70b28928b82b5f97dcc9',
                                coinType: 'btc',
                                onTap: () {
                                  setState(() {
                                    _selectedCoin = 'btc';
                                    _txidController.text =
                                        '2096b844178ecc776e050be7886e618ee111e2a68fcf70b28928b82b5f97dcc9';
                                  });
                                },
                              ),
                              _TestTxidChip(
                                label: 'ETH Test',
                                txid:
                                    '0x459ccd6fe488b0f826aef198ad5625d0275f5de1b77b905f85d6e71460c1f1aa',
                                coinType: 'eth',
                                onTap: () {
                                  setState(() {
                                    _selectedCoin = 'eth';
                                    _txidController.text =
                                        '0x459ccd6fe488b0f826aef198ad5625d0275f5de1b77b905f85d6e71460c1f1aa';
                                  });
                                },
                              ),
                            ],
                          ),
                        ],
                      ),
                    ),
                  ),

                const SizedBox(height: 32),

                ElevatedButton(
                  onPressed: _isLoading ? null : _submitBurn,
                  style: ElevatedButton.styleFrom(
                    backgroundColor: Colors.deepOrange,
                    padding: const EdgeInsets.all(16),
                  ),
                  child: _isLoading
                      ? const CircularProgressIndicator()
                      : const Text('SUBMIT BURN PROOF',
                          style: TextStyle(fontSize: 16)),
                ),
                const SizedBox(height: 16),
                const Card(
                  color: Color(0xFFB71C1C),
                  child: Padding(
                    padding: EdgeInsets.all(12.0),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          '⚠️ Important:',
                          style: TextStyle(fontWeight: FontWeight.bold),
                        ),
                        SizedBox(height: 8),
                        Text(
                          '• Send BTC/ETH to the burn address FIRST\n'
                          '• Validators verify on-chain automatically\n'
                          '• LOS amount is calculated from live prices\n'
                          '• Max 1,000 LOS per block (Mint cap protection)\n'
                          '• Rate limit: 1 burn per 5 minutes\n'
                          '• False submissions are rejected automatically',
                          style: TextStyle(fontSize: 12),
                        ),
                      ],
                    ),
                  ),
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
    _txidController.dispose();
    super.dispose();
  }
}

/// Chip button for quickly inserting test TXIDs
class _TestTxidChip extends StatelessWidget {
  final String label;
  final String txid;
  final String coinType;
  final VoidCallback onTap;

  const _TestTxidChip({
    required this.label,
    required this.txid,
    required this.coinType,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    return ActionChip(
      avatar: Icon(
        coinType == 'btc' ? Icons.currency_bitcoin : Icons.currency_exchange,
        size: 16,
        color: coinType == 'btc' ? Colors.orange : Colors.blue,
      ),
      label: Text(label, style: const TextStyle(fontSize: 11)),
      onPressed: onTap,
      backgroundColor: const Color(0xFF0D1B2A),
      side: BorderSide(
        color: coinType == 'btc'
            ? Colors.orange.withAlpha(80)
            : Colors.blue.withAlpha(80),
      ),
    );
  }
}
