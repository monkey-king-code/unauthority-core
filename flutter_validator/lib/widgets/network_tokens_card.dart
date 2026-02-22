// Network Token & DEX Overview Widget for Validator Dashboard.
//
// Read-only: shows registered USP-01 token count and DEX pool summary.
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/api_service.dart';
import '../models/network_tokens.dart';
import '../utils/log.dart';

class NetworkTokensCard extends StatefulWidget {
  const NetworkTokensCard({super.key});

  @override
  State<NetworkTokensCard> createState() => _NetworkTokensCardState();
}

class _NetworkTokensCardState extends State<NetworkTokensCard> {
  List<Token> _tokens = [];
  List<DexPool> _pools = [];
  bool _isLoading = true;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    try {
      final api = context.read<ApiService>();
      final tokens = await api.getTokens();
      final pools = await api.getDexPools();
      if (!mounted) return;
      setState(() {
        _tokens = tokens;
        _pools = pools;
        _isLoading = false;
      });
    } catch (e) {
      losLog('⚠ NetworkTokensCard load: $e');
      if (!mounted) return;
      setState(() => _isLoading = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                const Icon(Icons.token, size: 20),
                const SizedBox(width: 8),
                const Text('Network Assets',
                    style:
                        TextStyle(fontSize: 16, fontWeight: FontWeight.bold)),
                const Spacer(),
                if (_isLoading)
                  const SizedBox(
                      width: 16,
                      height: 16,
                      child: CircularProgressIndicator(strokeWidth: 2)),
              ],
            ),
            const Divider(),
            Row(
              children: [
                Expanded(
                  child: _StatTile(
                    label: 'USP-01 Tokens',
                    value: '${_tokens.length}',
                    icon: Icons.toll,
                  ),
                ),
                Expanded(
                  child: _StatTile(
                    label: 'DEX Pools',
                    value: '${_pools.length}',
                    icon: Icons.water_drop,
                  ),
                ),
                Expanded(
                  child: _StatTile(
                    label: 'Wrapped',
                    value: '${_tokens.where((t) => t.isWrapped).length}',
                    icon: Icons.swap_horiz,
                  ),
                ),
              ],
            ),
            if (_tokens.isNotEmpty) ...[
              const SizedBox(height: 12),
              const Text('Registered Tokens',
                  style: TextStyle(fontSize: 12, color: Colors.grey)),
              const SizedBox(height: 4),
              ...(_tokens.take(5).map((t) => Padding(
                    padding: const EdgeInsets.symmetric(vertical: 2),
                    child: Row(
                      children: [
                        Icon(
                          t.isWrapped ? Icons.swap_horiz : Icons.toll,
                          size: 14,
                          color: t.isWrapped ? Colors.orange : Colors.purple,
                        ),
                        const SizedBox(width: 6),
                        Text(t.symbol,
                            style:
                                const TextStyle(fontWeight: FontWeight.w500)),
                        const SizedBox(width: 8),
                        Expanded(
                          child: Text(t.name,
                              overflow: TextOverflow.ellipsis,
                              style: const TextStyle(
                                  fontSize: 12, color: Colors.grey)),
                        ),
                        Text('Supply: ${t.totalSupply}',
                            style: const TextStyle(
                                fontSize: 11, color: Colors.grey)),
                      ],
                    ),
                  ))),
              if (_tokens.length > 5)
                Padding(
                  padding: const EdgeInsets.only(top: 4),
                  child: Text('+${_tokens.length - 5} more',
                      style: const TextStyle(fontSize: 11, color: Colors.grey)),
                ),
            ],
            if (_pools.isNotEmpty) ...[
              const SizedBox(height: 12),
              const Text('Active Pools',
                  style: TextStyle(fontSize: 12, color: Colors.grey)),
              const SizedBox(height: 4),
              ...(_pools.take(3).map((p) => Padding(
                    padding: const EdgeInsets.symmetric(vertical: 2),
                    child: Row(
                      children: [
                        const Icon(Icons.water_drop,
                            size: 14, color: Colors.blue),
                        const SizedBox(width: 6),
                        Text(p.pairLabel,
                            style:
                                const TextStyle(fontWeight: FontWeight.w500)),
                        const Spacer(),
                        Text('Fee: ${p.feeDisplay}',
                            style: const TextStyle(
                                fontSize: 11, color: Colors.grey)),
                      ],
                    ),
                  ))),
            ],
          ],
        ),
      ),
    );
  }
}

class _StatTile extends StatelessWidget {
  final String label;
  final String value;
  final IconData icon;

  const _StatTile(
      {required this.label, required this.value, required this.icon});

  @override
  Widget build(BuildContext context) {
    return Column(
      children: [
        Icon(icon, size: 24, color: Colors.grey),
        const SizedBox(height: 4),
        Text(value,
            style: const TextStyle(fontSize: 20, fontWeight: FontWeight.bold)),
        Text(label, style: const TextStyle(fontSize: 11, color: Colors.grey)),
      ],
    );
  }
}
