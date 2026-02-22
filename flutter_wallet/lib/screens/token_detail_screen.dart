/// USP-01 Token Detail Screen
///
/// Shows token metadata, user balance, and actions (send, approve, burn).
library;

import 'package:flutter/material.dart';
import '../utils/secure_clipboard.dart';
import 'package:provider/provider.dart';
import '../services/api_service.dart';
import '../services/wallet_service.dart';
import '../models/token.dart';
import '../utils/log.dart';
import 'token_send_screen.dart';

class TokenDetailScreen extends StatefulWidget {
  final Token token;

  const TokenDetailScreen({super.key, required this.token});

  @override
  State<TokenDetailScreen> createState() => _TokenDetailScreenState();
}

class _TokenDetailScreenState extends State<TokenDetailScreen> {
  String _balance = '0';
  bool _isLoading = true;
  String? _walletAddress;

  @override
  void initState() {
    super.initState();
    _loadBalance();
  }

  Future<void> _loadBalance() async {
    setState(() => _isLoading = true);
    try {
      final api = context.read<ApiService>();
      final wallet = context.read<WalletService>();
      final walletInfo = await wallet.getCurrentWallet();
      _walletAddress = walletInfo?['address'];

      if (_walletAddress != null) {
        final bal = await api.getTokenBalance(
            widget.token.contractAddress, _walletAddress!);
        if (!mounted) return;
        setState(() {
          _balance = bal.balance;
          _isLoading = false;
        });
      }
    } catch (e) {
      losLog('❌ Token balance error: $e');
      if (!mounted) return;
      setState(() => _isLoading = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final token = widget.token;

    return Scaffold(
      appBar: AppBar(
        title: Text(token.symbol),
      ),
      body: RefreshIndicator(
        onRefresh: _loadBalance,
        child: ListView(
          padding: const EdgeInsets.all(16),
          children: [
            // Balance Card
            Card(
              child: Padding(
                padding: const EdgeInsets.all(24),
                child: Column(
                  children: [
                    CircleAvatar(
                      radius: 32,
                      backgroundColor: token.isWrapped
                          ? Colors.orange.withValues(alpha: 0.2)
                          : Colors.purple.withValues(alpha: 0.2),
                      child: Icon(
                        token.isWrapped ? Icons.swap_horiz : Icons.token,
                        size: 32,
                        color: token.isWrapped ? Colors.orange : Colors.purple,
                      ),
                    ),
                    const SizedBox(height: 16),
                    Text(
                      token.name,
                      style: const TextStyle(
                          fontSize: 20, fontWeight: FontWeight.bold),
                    ),
                    const SizedBox(height: 8),
                    if (_isLoading)
                      const CircularProgressIndicator()
                    else
                      Text(
                        '$_balance ${token.symbol}',
                        style: const TextStyle(
                            fontSize: 32, fontWeight: FontWeight.bold),
                      ),
                    if (token.isWrapped) ...[
                      const SizedBox(height: 8),
                      Container(
                        padding: const EdgeInsets.symmetric(
                            horizontal: 12, vertical: 4),
                        decoration: BoxDecoration(
                          color: Colors.orange.withValues(alpha: 0.2),
                          borderRadius: BorderRadius.circular(12),
                        ),
                        child: Text(
                          'Wrapped ${token.wrappedOrigin}',
                          style: TextStyle(
                            color: Colors.orange.shade400,
                            fontWeight: FontWeight.bold,
                            fontSize: 12,
                          ),
                        ),
                      ),
                    ],
                  ],
                ),
              ),
            ),

            const SizedBox(height: 16),

            // Action Buttons
            Row(
              children: [
                Expanded(
                  child: ElevatedButton.icon(
                    onPressed: () => Navigator.push(
                      context,
                      MaterialPageRoute(
                        builder: (_) => TokenSendScreen(token: token),
                      ),
                    ).then((_) => _loadBalance()),
                    icon: const Icon(Icons.send),
                    label: const Text('Send'),
                    style: ElevatedButton.styleFrom(
                      padding: const EdgeInsets.symmetric(vertical: 16),
                    ),
                  ),
                ),
                const SizedBox(width: 12),
                Expanded(
                  child: OutlinedButton.icon(
                    onPressed: () => _showBurnDialog(),
                    icon: const Icon(Icons.local_fire_department),
                    label: const Text('Burn'),
                    style: OutlinedButton.styleFrom(
                      padding: const EdgeInsets.symmetric(vertical: 16),
                    ),
                  ),
                ),
              ],
            ),

            const SizedBox(height: 24),

            // Token Info
            Card(
              child: Column(
                children: [
                  _InfoRow(label: 'Symbol', value: token.symbol),
                  const Divider(height: 1),
                  _InfoRow(label: 'Decimals', value: '${token.decimals}'),
                  const Divider(height: 1),
                  _InfoRow(label: 'Total Supply', value: token.totalSupply),
                  const Divider(height: 1),
                  _InfoRow(label: 'Standard', value: 'USP-01'),
                  const Divider(height: 1),
                  _InfoRow(
                    label: 'Contract',
                    value: token.shortAddress,
                    onTap: () {
                      SecureClipboard.copyPublic(token.contractAddress);
                      ScaffoldMessenger.of(context).showSnackBar(
                        const SnackBar(
                            content: Text('Contract address copied')),
                      );
                    },
                  ),
                  if (token.owner.isNotEmpty) ...[
                    const Divider(height: 1),
                    _InfoRow(
                      label: 'Owner',
                      value: token.owner.length > 16
                          ? '${token.owner.substring(0, 10)}...'
                          : token.owner,
                    ),
                  ],
                  if (token.isWrapped && token.wrappedOrigin.isNotEmpty) ...[
                    const Divider(height: 1),
                    _InfoRow(
                        label: 'Wrapped Asset', value: token.wrappedOrigin),
                  ],
                  if (token.maxSupply != '0') ...[
                    const Divider(height: 1),
                    _InfoRow(label: 'Max Supply', value: token.maxSupply),
                  ],
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }

  Future<void> _showBurnDialog() async {
    final controller = TextEditingController();
    final api = context.read<ApiService>();
    final result = await showDialog<String>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text('Burn ${widget.token.symbol}'),
        content: TextField(
          controller: controller,
          keyboardType: const TextInputType.numberWithOptions(decimal: false),
          decoration: InputDecoration(
            labelText: 'Amount',
            hintText: 'Enter amount to burn',
            suffixText: widget.token.symbol,
          ),
        ),
        actions: [
          TextButton(
              onPressed: () => Navigator.pop(ctx), child: const Text('Cancel')),
          ElevatedButton(
            onPressed: () => Navigator.pop(ctx, controller.text.trim()),
            style: ElevatedButton.styleFrom(
              backgroundColor: Colors.red.shade700,
            ),
            child: const Text('Burn'),
          ),
        ],
      ),
    );

    if (result == null || result.isEmpty) return;

    try {
      await api.callContract(
        contractAddress: widget.token.contractAddress,
        function: 'burn',
        args: [result],
        caller: _walletAddress,
      );
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('Burned $result ${widget.token.symbol}')),
      );
      _loadBalance();
    } catch (e) {
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('Burn failed: $e')),
      );
    }
  }
}

class _InfoRow extends StatelessWidget {
  final String label;
  final String value;
  final VoidCallback? onTap;

  const _InfoRow({required this.label, required this.value, this.onTap});

  @override
  Widget build(BuildContext context) {
    return InkWell(
      onTap: onTap,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
        child: Row(
          mainAxisAlignment: MainAxisAlignment.spaceBetween,
          children: [
            Text(label, style: const TextStyle(color: Colors.grey)),
            Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(value,
                    style: const TextStyle(fontWeight: FontWeight.w500)),
                if (onTap != null) ...[
                  const SizedBox(width: 4),
                  const Icon(Icons.copy, size: 14, color: Colors.grey),
                ],
              ],
            ),
          ],
        ),
      ),
    );
  }
}
