import '../utils/log.dart';
import '../utils/secure_clipboard.dart';
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import 'package:intl/intl.dart';
import '../services/api_service.dart';

/// Look up a transaction or block by hash, or search for an address.
class ExplorerScreen extends StatefulWidget {
  const ExplorerScreen({super.key});

  @override
  State<ExplorerScreen> createState() => _ExplorerScreenState();
}

class _ExplorerScreenState extends State<ExplorerScreen> {
  final _searchController = TextEditingController();
  Map<String, dynamic>? _result;
  String? _resultType; // 'transaction', 'block', 'search'
  bool _isLoading = false;
  String? _error;

  Future<void> _search() async {
    final query = _searchController.text.trim();
    if (query.isEmpty) return;
    losLog('🔍 [Explorer] Searching: "$query"');

    setState(() {
      _isLoading = true;
      _error = null;
      _result = null;
      _resultType = null;
    });

    try {
      final api = context.read<ApiService>();

      // Heuristic: 64-char hex → try transaction first, then block
      // LOS... address → use search endpoint
      if (query.length == 64 && RegExp(r'^[a-fA-F0-9]+$').hasMatch(query)) {
        // Try transaction lookup first
        try {
          final txResult = await api.getTransaction(query);
          if (txResult['status'] == 'found' ||
              txResult.containsKey('transaction')) {
            losLog('🔍 [Explorer] Found transaction: $query');
            if (!mounted) return;
            setState(() {
              _result = txResult;
              _resultType = 'transaction';
              _isLoading = false;
            });
            return;
          }
        } catch (e) {
          losLog('🔍 [Explorer] Not a transaction ($e), trying block...');
        }

        // Try block lookup
        try {
          final blockResult = await api.getBlock(query);
          if (blockResult['status'] == 'found' ||
              blockResult.containsKey('block')) {
            losLog('🔍 [Explorer] Found block: $query');
            if (!mounted) return;
            setState(() {
              _result = blockResult;
              _resultType = 'block';
              _isLoading = false;
            });
            return;
          }
        } catch (e) {
          losLog(
              '🔍 [Explorer] Not a block either ($e), using search fallback...');
        }
      }

      // Fallback: use search endpoint
      final searchResult = await api.search(query);
      losLog(
          '🔍 [Explorer] Search result: ${searchResult['count'] ?? 0} matches');
      if (!mounted) return;
      setState(() {
        _result = searchResult;
        _resultType = 'search';
        _isLoading = false;
      });
    } catch (e) {
      losLog('❌ [Explorer] Search error: $e');
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
        title: const Text('Explorer'),
        centerTitle: true,
      ),
      body: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          children: [
            // Search bar
            Row(
              children: [
                Expanded(
                  child: TextField(
                    controller: _searchController,
                    decoration: const InputDecoration(
                      hintText: 'Transaction hash, block hash, or address',
                      border: OutlineInputBorder(),
                      prefixIcon: Icon(Icons.search),
                      contentPadding:
                          EdgeInsets.symmetric(horizontal: 12, vertical: 14),
                    ),
                    onSubmitted: (_) => _search(),
                  ),
                ),
                const SizedBox(width: 8),
                ElevatedButton(
                  onPressed: _isLoading ? null : _search,
                  style: ElevatedButton.styleFrom(
                    padding: const EdgeInsets.symmetric(
                        horizontal: 16, vertical: 14),
                  ),
                  child: _isLoading
                      ? const SizedBox(
                          width: 20,
                          height: 20,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        )
                      : const Icon(Icons.search),
                ),
              ],
            ),
            const SizedBox(height: 16),

            // Results
            Expanded(
              child: _error != null
                  ? Center(
                      child: Text('Error: $_error',
                          style: const TextStyle(color: Colors.red)))
                  : _result == null
                      ? const Center(
                          child: Text(
                            'Search for a transaction hash, block hash,\nor LOS address',
                            textAlign: TextAlign.center,
                            style: TextStyle(color: Colors.grey),
                          ),
                        )
                      : _buildResult(),
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildResult() {
    switch (_resultType) {
      case 'transaction':
        return _buildTransactionResult();
      case 'block':
        return _buildBlockResult();
      case 'search':
        return _buildSearchResult();
      default:
        return const Center(child: Text('No results'));
    }
  }

  Widget _buildTransactionResult() {
    final tx = _result!['transaction'] as Map<String, dynamic>? ?? _result!;
    final dateTime = tx['timestamp'] != null
        ? DateTime.fromMillisecondsSinceEpoch((tx['timestamp'] as int) * 1000)
        : null;

    return ListView(
      children: [
        const Text('Transaction Found',
            style: TextStyle(
                fontSize: 18,
                fontWeight: FontWeight.bold,
                color: Colors.green)),
        const SizedBox(height: 12),
        _resultCard([
          _resultRow('Hash', tx['hash']?.toString() ?? 'N/A', copyable: true),
          _resultRow('Type', (tx['type'] ?? 'N/A').toString().toUpperCase()),
          _resultRow('From', tx['from']?.toString() ?? 'N/A', copyable: true),
          _resultRow('To', tx['to']?.toString() ?? 'N/A', copyable: true),
          _resultRow('Amount', '${tx['amount'] ?? 'N/A'}'),
          if (tx['amount_cil'] != null)
            _resultRow('Amount (CIL)', '${tx['amount_cil']} CIL'),
          if (dateTime != null)
            _resultRow(
                'Time', DateFormat('MMM dd, yyyy HH:mm:ss').format(dateTime)),
          _resultRow(
              'Confirmed', (tx['confirmed'] == true) ? 'Yes' : 'Pending'),
        ]),
      ],
    );
  }

  Widget _buildBlockResult() {
    final block = _result!['block'] as Map<String, dynamic>? ?? _result!;
    final dateTime = block['timestamp'] != null
        ? DateTime.fromMillisecondsSinceEpoch(
            (block['timestamp'] as int) * 1000)
        : null;

    return ListView(
      children: [
        const Text('Block Found',
            style: TextStyle(
                fontSize: 18, fontWeight: FontWeight.bold, color: Colors.blue)),
        const SizedBox(height: 12),
        _resultCard([
          _resultRow('Hash', block['hash']?.toString() ?? 'N/A',
              copyable: true),
          _resultRow('Account', block['account']?.toString() ?? 'N/A',
              copyable: true),
          _resultRow('Type', (block['type'] ?? 'N/A').toString().toUpperCase()),
          _resultRow('Amount', '${block['amount'] ?? 'N/A'}'),
          if (block['amount_cil'] != null)
            _resultRow('Amount (CIL)', '${block['amount_cil']} CIL'),
          _resultRow('Previous', block['previous']?.toString() ?? 'N/A',
              copyable: true),
          if (dateTime != null)
            _resultRow(
                'Time', DateFormat('MMM dd, yyyy HH:mm:ss').format(dateTime)),
        ]),
      ],
    );
  }

  Widget _buildSearchResult() {
    final results = _result!['results'] as List<dynamic>? ?? [];
    final count = _result!['count'] ?? results.length;

    if (results.isEmpty) {
      return Center(
        child: Text('No results found for "${_searchController.text.trim()}"',
            style: const TextStyle(color: Colors.grey)),
      );
    }

    return ListView(
      children: [
        Text('$count result(s) found',
            style: const TextStyle(fontSize: 16, fontWeight: FontWeight.bold)),
        const SizedBox(height: 12),
        ...results.map((r) {
          final result = r as Map<String, dynamic>;
          return Card(
            child: ListTile(
              leading: Icon(
                result['type'] == 'account' ? Icons.person : Icons.token,
                color: Colors.blue,
              ),
              title: Text(
                result['address']?.toString() ?? 'N/A',
                style: const TextStyle(fontSize: 12, fontFamily: 'monospace'),
                overflow: TextOverflow.ellipsis,
              ),
              subtitle: Text(
                // Backend /search returns balance as LOS integer (already divided by CIL_PER_LOS).
                // Display directly — do NOT pass through cilToLos which would divide again.
                'Balance: ${result['balance'] ?? 0} LOS  •  Blocks: ${result['block_count'] ?? 0}',
                style: const TextStyle(fontSize: 11),
              ),
              trailing: IconButton(
                icon: const Icon(Icons.copy, size: 18),
                onPressed: () {
                  SecureClipboard.copyPublic(
                      result['address']?.toString() ?? '');
                  ScaffoldMessenger.of(context).showSnackBar(
                    const SnackBar(content: Text('Address copied')),
                  );
                },
              ),
            ),
          );
        }),
      ],
    );
  }

  Widget _resultCard(List<Widget> children) {
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(12),
        child: Column(children: children),
      ),
    );
  }

  Widget _resultRow(String label, String value, {bool copyable = false}) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            width: 100,
            child: Text(label,
                style: const TextStyle(color: Colors.grey, fontSize: 12)),
          ),
          Expanded(
            child: Text(
              value,
              style: const TextStyle(fontSize: 12, fontFamily: 'monospace'),
            ),
          ),
          if (copyable)
            GestureDetector(
              onTap: () {
                SecureClipboard.copyPublic(value);
                ScaffoldMessenger.of(context).showSnackBar(
                  SnackBar(content: Text('$label copied')),
                );
              },
              child: const Icon(Icons.copy, size: 14, color: Colors.grey),
            ),
        ],
      ),
    );
  }

  @override
  void dispose() {
    _searchController.dispose();
    super.dispose();
  }
}
