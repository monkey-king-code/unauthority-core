import '../utils/log.dart';
import '../utils/secure_clipboard.dart';
import 'package:flutter/material.dart';
import 'package:qr_flutter/qr_flutter.dart';
import 'package:provider/provider.dart';
import '../services/wallet_service.dart';

class ReceiveScreen extends StatefulWidget {
  const ReceiveScreen({super.key});

  @override
  State<ReceiveScreen> createState() => _ReceiveScreenState();
}

class _ReceiveScreenState extends State<ReceiveScreen> {
  String? _address;
  bool _isLoading = true;

  @override
  void initState() {
    super.initState();
    _loadAddress();
  }

  Future<void> _loadAddress() async {
    losLog('💰 [ReceiveScreen._loadAddress] Loading address...');
    try {
      final walletService = context.read<WalletService>();
      final wallet = await walletService.getCurrentWallet();

      if (wallet != null && mounted) {
        setState(() {
          _address = wallet['address'];
          _isLoading = false;
        });
        losLog('💰 [ReceiveScreen._loadAddress] Address: $_address');
      }
    } catch (e) {
      if (mounted) {
        setState(() => _isLoading = false);
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Error: $e'), backgroundColor: Colors.red),
        );
      }
    }
  }

  void _copyAddress() {
    if (_address != null) {
      SecureClipboard.copyPublic(_address!);
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(
          content: Text('Address copied to clipboard'),
          backgroundColor: Colors.green,
          duration: Duration(seconds: 2),
        ),
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('Receive LOS'),
        centerTitle: true,
      ),
      body: _isLoading
          ? const Center(child: CircularProgressIndicator())
          : _address == null
              ? const Center(child: Text('No wallet address found'))
              : Center(
                  child: SingleChildScrollView(
                    padding: const EdgeInsets.all(24),
                    child: Column(
                      mainAxisAlignment: MainAxisAlignment.center,
                      children: [
                        const Text(
                          'Scan QR Code to Receive',
                          style: TextStyle(
                            fontSize: 20,
                            fontWeight: FontWeight.bold,
                          ),
                        ),
                        const SizedBox(height: 32),

                        // QR Code
                        Container(
                          padding: const EdgeInsets.all(16),
                          decoration: BoxDecoration(
                            color: Colors.white,
                            borderRadius: BorderRadius.circular(16),
                            boxShadow: [
                              BoxShadow(
                                color: Colors.black.withValues(alpha: 0.1),
                                blurRadius: 20,
                                spreadRadius: 5,
                              ),
                            ],
                          ),
                          child: QrImageView(
                            data: _address!,
                            version: QrVersions.auto,
                            size: 280,
                            backgroundColor: Colors.white,
                            eyeStyle: const QrEyeStyle(
                              eyeShape: QrEyeShape.square,
                              color: Colors.black,
                            ),
                            dataModuleStyle: const QrDataModuleStyle(
                              dataModuleShape: QrDataModuleShape.square,
                              color: Colors.black,
                            ),
                            errorCorrectionLevel: QrErrorCorrectLevel.H,
                          ),
                        ),

                        const SizedBox(height: 32),

                        // Address Display
                        Card(
                          child: Padding(
                            padding: const EdgeInsets.all(16),
                            child: Column(
                              crossAxisAlignment: CrossAxisAlignment.start,
                              children: [
                                const Text(
                                  'Your LOS Address',
                                  style: TextStyle(
                                    fontSize: 12,
                                    color: Colors.grey,
                                    fontWeight: FontWeight.bold,
                                  ),
                                ),
                                const SizedBox(height: 8),
                                SelectableText(
                                  _address!,
                                  style: const TextStyle(
                                    fontSize: 14,
                                    fontFamily: 'monospace',
                                    fontWeight: FontWeight.bold,
                                  ),
                                ),
                              ],
                            ),
                          ),
                        ),

                        const SizedBox(height: 24),

                        // Copy Button
                        ElevatedButton.icon(
                          onPressed: _copyAddress,
                          icon: const Icon(Icons.copy),
                          label: const Text('COPY ADDRESS'),
                          style: ElevatedButton.styleFrom(
                            padding: const EdgeInsets.symmetric(
                              horizontal: 32,
                              vertical: 16,
                            ),
                            backgroundColor: Theme.of(context).primaryColor,
                          ),
                        ),

                        const SizedBox(height: 24),

                        // Warning
                        Container(
                          padding: const EdgeInsets.all(16),
                          decoration: BoxDecoration(
                            color: Colors.orange.withValues(alpha: 0.1),
                            borderRadius: BorderRadius.circular(8),
                            border: Border.all(color: Colors.orange),
                          ),
                          child: const Row(
                            children: [
                              Icon(Icons.warning,
                                  color: Colors.orange, size: 24),
                              SizedBox(width: 12),
                              Expanded(
                                child: Text(
                                  'Only send LOS tokens to this address. Sending other tokens may result in permanent loss.',
                                  style: TextStyle(
                                      fontSize: 12, color: Colors.orange),
                                ),
                              ),
                            ],
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
    );
  }
}
