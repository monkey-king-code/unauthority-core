/// USP-01 Token List Screen
///
/// Displays all USP-01 tokens the user holds, with their balances.
/// Allows navigation to token detail / send screens.
library;

import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/api_service.dart';
import '../services/wallet_service.dart';
import '../models/token.dart';
import '../utils/log.dart';
import 'token_detail_screen.dart';

class TokensScreen extends StatefulWidget {
  const TokensScreen({super.key});

  @override
  State<TokensScreen> createState() => _TokensScreenState();
}

class _TokensScreenState extends State<TokensScreen> {
  List<Token> _tokens = [];
  Map<String, String> _balances = {}; // contractAddress → balance
  bool _isLoading = true;
  String? _error;

  @override
  void initState() {
    super.initState();
    _loadTokens();
  }

  Future<void> _loadTokens() async {
    setState(() {
      _isLoading = true;
      _error = null;
    });
    try {
      final api = context.read<ApiService>();
      final wallet = context.read<WalletService>();
      final walletInfo = await wallet.getCurrentWallet();
      final address = walletInfo?['address'] ?? '';

      // Fetch all tokens
      final tokens = await api.getTokens();

      // Fetch balances for the current wallet
      final balances = <String, String>{};
      for (final token in tokens) {
        try {
          final bal = await api.getTokenBalance(token.contractAddress, address);
          balances[token.contractAddress] = bal.balance;
        } catch (e) {
          losLog('⚠️ Failed to get balance for ${token.symbol}: $e');
          balances[token.contractAddress] = '0';
        }
      }

      if (!mounted) return;
      setState(() {
        _tokens = tokens;
        _balances = balances;
        _isLoading = false;
      });
    } catch (e) {
      losLog('❌ _loadTokens error: $e');
      if (!mounted) return;
      setState(() {
        _error = e.toString();
        _isLoading = false;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('USP-01 Tokens'),
        actions: [
          IconButton(
            icon: const Icon(Icons.refresh),
            onPressed: _loadTokens,
          ),
        ],
      ),
      body: _buildBody(),
    );
  }

  Widget _buildBody() {
    if (_isLoading) {
      return const Center(child: CircularProgressIndicator());
    }
    if (_error != null) {
      return Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(Icons.error_outline, size: 48, color: Colors.red.shade400),
            const SizedBox(height: 16),
            Text('Failed to load tokens',
                style: TextStyle(color: Colors.red.shade400)),
            const SizedBox(height: 8),
            Text(_error!,
                style: const TextStyle(fontSize: 12, color: Colors.grey)),
            const SizedBox(height: 16),
            ElevatedButton(onPressed: _loadTokens, child: const Text('Retry')),
          ],
        ),
      );
    }
    if (_tokens.isEmpty) {
      return Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(Icons.token, size: 64, color: Colors.grey.shade600),
            const SizedBox(height: 16),
            const Text('No tokens deployed yet',
                style: TextStyle(color: Colors.grey, fontSize: 16)),
            const SizedBox(height: 8),
            const Text(
              'USP-01 tokens will appear here\nonce deployed on the network.',
              textAlign: TextAlign.center,
              style: TextStyle(color: Colors.grey, fontSize: 12),
            ),
          ],
        ),
      );
    }

    return RefreshIndicator(
      onRefresh: _loadTokens,
      child: ListView.builder(
        padding: const EdgeInsets.all(16),
        itemCount: _tokens.length,
        itemBuilder: (context, index) {
          final token = _tokens[index];
          final balance = _balances[token.contractAddress] ?? '0';
          return _TokenCard(
            token: token,
            balance: balance,
            onTap: () => Navigator.push(
              context,
              MaterialPageRoute(
                builder: (_) => TokenDetailScreen(token: token),
              ),
            ).then((_) => _loadTokens()),
          );
        },
      ),
    );
  }
}

class _TokenCard extends StatelessWidget {
  final Token token;
  final String balance;
  final VoidCallback onTap;

  const _TokenCard({
    required this.token,
    required this.balance,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    return Card(
      margin: const EdgeInsets.only(bottom: 12),
      child: ListTile(
        onTap: onTap,
        leading: CircleAvatar(
          backgroundColor: token.isWrapped
              ? Colors.orange.withValues(alpha: 0.2)
              : Colors.purple.withValues(alpha: 0.2),
          child: Icon(
            token.isWrapped ? Icons.swap_horiz : Icons.token,
            color: token.isWrapped ? Colors.orange : Colors.purple,
          ),
        ),
        title: Text(
          token.symbol,
          style: const TextStyle(fontWeight: FontWeight.bold),
        ),
        subtitle: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(token.name, style: const TextStyle(fontSize: 12)),
            if (token.isWrapped)
              Text(
                'Wrapped ${token.wrappedOrigin}',
                style: TextStyle(fontSize: 10, color: Colors.orange.shade400),
              ),
          ],
        ),
        trailing: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          crossAxisAlignment: CrossAxisAlignment.end,
          children: [
            Text(
              balance,
              style: const TextStyle(
                fontWeight: FontWeight.bold,
                fontSize: 16,
              ),
            ),
            Text(
              token.symbol,
              style: const TextStyle(fontSize: 11, color: Colors.grey),
            ),
          ],
        ),
      ),
    );
  }
}
