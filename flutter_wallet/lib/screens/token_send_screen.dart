// USP-01 Token Send / Approve Screen
//
// Allows transferring tokens to another address or setting an approval.
// All amounts are kept as integer strings — no floating-point.
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/api_service.dart';
import '../services/wallet_service.dart';
import '../models/token.dart';
import '../utils/address_validator.dart';
import '../utils/log.dart';

class TokenSendScreen extends StatefulWidget {
  final Token token;

  const TokenSendScreen({super.key, required this.token});

  @override
  State<TokenSendScreen> createState() => _TokenSendScreenState();
}

class _TokenSendScreenState extends State<TokenSendScreen> {
  final _formKey = GlobalKey<FormState>();
  final _toController = TextEditingController();
  final _amountController = TextEditingController();
  bool _isSending = false;

  /// True = transfer, False = approve
  bool _isTransfer = true;

  @override
  void dispose() {
    _toController.dispose();
    _amountController.dispose();
    super.dispose();
  }

  Future<void> _submit() async {
    if (!_formKey.currentState!.validate()) return;

    setState(() => _isSending = true);
    final to = _toController.text.trim();
    final amount = _amountController.text.trim();

    try {
      final api = context.read<ApiService>();
      final wallet = context.read<WalletService>();
      final walletInfo = await wallet.getCurrentWallet();
      final myAddress = walletInfo?['address'];

      if (myAddress == null) throw Exception('No wallet found');
      if (to == myAddress) throw Exception('Cannot send to your own address');

      if (_isTransfer) {
        losLog('🪙 [TokenSend] transfer ${widget.token.symbol}: $amount → $to');
        await api.callContract(
          contractAddress: widget.token.contractAddress,
          function: 'transfer',
          args: [to, amount],
          caller: myAddress,
        );
      } else {
        losLog(
            '🪙 [TokenSend] approve ${widget.token.symbol}: $amount for $to');
        await api.callContract(
          contractAddress: widget.token.contractAddress,
          function: 'approve',
          args: [to, amount],
          caller: myAddress,
        );
      }

      if (!mounted) return;
      final action = _isTransfer ? 'Sent' : 'Approved';
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('$action $amount ${widget.token.symbol}')),
      );
      Navigator.pop(context);
    } catch (e) {
      losLog('❌ [TokenSend] Error: $e');
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('Failed: $e')),
      );
    } finally {
      if (mounted) setState(() => _isSending = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title:
            Text('${_isTransfer ? 'Send' : 'Approve'} ${widget.token.symbol}'),
      ),
      body: Padding(
        padding: const EdgeInsets.all(16),
        child: Form(
          key: _formKey,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              // Transfer / Approve toggle
              SegmentedButton<bool>(
                segments: const [
                  ButtonSegment(
                      value: true,
                      label: Text('Transfer'),
                      icon: Icon(Icons.send)),
                  ButtonSegment(
                      value: false,
                      label: Text('Approve'),
                      icon: Icon(Icons.check_circle)),
                ],
                selected: {_isTransfer},
                onSelectionChanged: (v) =>
                    setState(() => _isTransfer = v.first),
              ),
              const SizedBox(height: 24),

              // Recipient / Spender
              TextFormField(
                controller: _toController,
                decoration: InputDecoration(
                  labelText:
                      _isTransfer ? 'Recipient Address' : 'Spender Address',
                  prefixIcon: const Icon(Icons.account_circle),
                  border: const OutlineInputBorder(),
                ),
                validator: (v) {
                  if (v == null || v.trim().isEmpty) {
                    return 'Address is required';
                  }
                  if (!AddressValidator.isValidAddress(v.trim())) {
                    return 'Invalid LOS address';
                  }
                  return null;
                },
              ),
              const SizedBox(height: 16),

              // Amount
              TextFormField(
                controller: _amountController,
                keyboardType:
                    const TextInputType.numberWithOptions(decimal: false),
                decoration: InputDecoration(
                  labelText: 'Amount',
                  prefixIcon: const Icon(Icons.toll),
                  suffixText: widget.token.symbol,
                  border: const OutlineInputBorder(),
                ),
                validator: (v) {
                  if (v == null || v.trim().isEmpty) {
                    return 'Amount is required';
                  }
                  final n = int.tryParse(v.trim());
                  if (n == null || n <= 0) {
                    return 'Enter a valid positive integer';
                  }
                  return null;
                },
              ),
              const SizedBox(height: 24),

              // Submit
              SizedBox(
                height: 52,
                child: ElevatedButton(
                  onPressed: _isSending ? null : _submit,
                  child: _isSending
                      ? const SizedBox(
                          width: 24,
                          height: 24,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        )
                      : Text(
                          _isTransfer ? 'Send Tokens' : 'Set Approval',
                          style: const TextStyle(fontSize: 16),
                        ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
