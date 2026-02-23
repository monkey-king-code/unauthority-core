import '../utils/log.dart';
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import 'package:intl/intl.dart';
import '../services/wallet_service.dart';
import '../services/api_service.dart';
import '../models/account.dart';
import 'transaction_detail_screen.dart';

class HistoryScreen extends StatefulWidget {
  const HistoryScreen({super.key});

  @override
  State<HistoryScreen> createState() => _HistoryScreenState();
}

class _HistoryScreenState extends State<HistoryScreen> {
  String? _address;
  List<Transaction> _transactions = [];
  bool _isLoading = true;
  String? _error;

  @override
  void initState() {
    super.initState();
    _loadTransactionHistory();
  }

  Future<void> _loadTransactionHistory() async {
    losLog(
        '💰 [HistoryScreen._loadTransactionHistory] Loading transaction history...');
    try {
      final walletService = context.read<WalletService>();
      final apiService = context.read<ApiService>();
      final wallet = await walletService.getCurrentWallet();

      if (!mounted) return;
      if (wallet == null) {
        setState(() => _error = 'No wallet found');
        return;
      }

      setState(() {
        _address = wallet['address'];
      });

      final history = await apiService.getHistory(_address!);

      if (!mounted) return;
      losLog(
          '💰 [HistoryScreen._loadTransactionHistory] Loaded ${history.length} transactions for $_address');
      setState(() {
        _transactions = history;
        _isLoading = false;
      });
    } catch (e) {
      losLog('💰 [HistoryScreen._loadTransactionHistory] ERROR: $e');
      if (!mounted) return;
      setState(() {
        _error = e.toString();
        _isLoading = false;
      });
    }
  }

  String _truncateAddress(String address) {
    if (address.length <= 20) return address;
    return '${address.substring(0, 8)}...${address.substring(address.length - 8)}';
  }

  Color _getTransactionColor(Transaction tx) {
    if (tx.type.toLowerCase() == 'mint') return Colors.blue;
    if (tx.from == _address) return Colors.red;
    return Colors.green;
  }

  IconData _getTransactionIcon(Transaction tx) {
    if (tx.type.toLowerCase() == 'mint') return Icons.add_circle;
    if (tx.from == _address) return Icons.arrow_upward;
    return Icons.arrow_downward;
  }

  String _getTransactionTitle(Transaction tx) {
    if (tx.type.toLowerCase() == 'mint') {
      if (tx.to.contains('ETH:')) return 'ETH Burn Reward';
      if (tx.to.contains('BTC:')) return 'BTC Burn Reward';
      return 'Minted';
    }
    if (tx.from == _address) return 'Sent';
    return 'Received';
  }

  String _getTransactionSubtitle(Transaction tx) {
    if (tx.type.toLowerCase() == 'mint') {
      if (tx.to.contains('ETH:') || tx.to.contains('BTC:')) {
        // Extract TXID from burn details
        final parts = tx.to.split(':');
        if (parts.length >= 2) {
          final txid = parts[1];
          final display =
              txid.length > 16 ? '${txid.substring(0, 16)}...' : txid;
          return 'Burn TXID: $display';
        }
      }
      return 'System Mint';
    }

    final otherParty = tx.from == _address ? tx.to : tx.from;
    final direction = tx.from == _address ? 'To' : 'From';
    return '$direction: ${_truncateAddress(otherParty)}';
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final colorScheme = theme.colorScheme;

    return Scaffold(
      appBar: AppBar(
        title: const Text('Transaction History'),
      ),
      body: _isLoading
          ? const Center(child: CircularProgressIndicator())
          : _error != null
              ? Center(
                  child: Column(
                    mainAxisAlignment: MainAxisAlignment.center,
                    children: [
                      Icon(Icons.error, size: 64, color: colorScheme.error),
                      const SizedBox(height: 16),
                      Text(_error!, style: TextStyle(color: colorScheme.error)),
                      const SizedBox(height: 16),
                      ElevatedButton(
                        onPressed: () {
                          setState(() {
                            _error = null;
                            _isLoading = true;
                          });
                          _loadTransactionHistory();
                        },
                        child: const Text('Retry'),
                      ),
                    ],
                  ),
                )
              : _transactions.isEmpty
                  ? Center(
                      child: Column(
                        mainAxisAlignment: MainAxisAlignment.center,
                        children: [
                          Icon(Icons.receipt_long,
                              size: 64,
                              color:
                                  colorScheme.onSurface.withValues(alpha: 0.4)),
                          const SizedBox(height: 16),
                          Text(
                            'No transactions yet',
                            style: TextStyle(
                                fontSize: 18,
                                color: colorScheme.onSurface
                                    .withValues(alpha: 0.6)),
                          ),
                          const SizedBox(height: 8),
                          Text(
                            'Your transaction history will appear here',
                            style: TextStyle(
                                color: colorScheme.onSurface
                                    .withValues(alpha: 0.5)),
                          ),
                        ],
                      ),
                    )
                  : RefreshIndicator(
                      onRefresh: _loadTransactionHistory,
                      child: ListView.builder(
                        padding: const EdgeInsets.all(16),
                        itemCount: _transactions.length + 1,
                        itemBuilder: (context, index) {
                          if (index == 0) {
                            // Header summary
                            return Card(
                              child: Padding(
                                padding: const EdgeInsets.all(16),
                                child: Column(
                                  crossAxisAlignment: CrossAxisAlignment.start,
                                  children: [
                                    Text(
                                      'Total Transactions: ${_transactions.length}',
                                      style: const TextStyle(
                                        fontSize: 16,
                                        fontWeight: FontWeight.bold,
                                      ),
                                    ),
                                    const SizedBox(height: 4),
                                    Text(
                                      'Wallet: ${_truncateAddress(_address ?? "")}',
                                      style: TextStyle(
                                          color: colorScheme.onSurface
                                              .withValues(alpha: 0.6)),
                                    ),
                                  ],
                                ),
                              ),
                            );
                          }

                          final tx = _transactions[index - 1];
                          final color = _getTransactionColor(tx);
                          final icon = _getTransactionIcon(tx);
                          final title = _getTransactionTitle(tx);
                          final subtitle = _getTransactionSubtitle(tx);

                          return Card(
                            child: ListTile(
                              onTap: () {
                                Navigator.push(
                                  context,
                                  MaterialPageRoute(
                                    builder: (_) => TransactionDetailScreen(
                                      transaction: tx,
                                      currentAddress: _address,
                                    ),
                                  ),
                                );
                              },
                              leading: Icon(icon, color: color),
                              title: Row(
                                children: [
                                  Text(
                                    title,
                                    style: const TextStyle(
                                      fontWeight: FontWeight.bold,
                                    ),
                                  ),
                                  const Spacer(),
                                  Text(
                                    '${tx.amountDisplay} LOS',
                                    style: TextStyle(
                                      fontWeight: FontWeight.bold,
                                      color: color,
                                    ),
                                  ),
                                ],
                              ),
                              subtitle: Column(
                                crossAxisAlignment: CrossAxisAlignment.start,
                                children: [
                                  Text(subtitle),
                                  if (tx.memo != null &&
                                      tx.memo!.isNotEmpty) ...[
                                    const SizedBox(height: 2),
                                    Row(
                                      children: [
                                        Icon(Icons.note,
                                            size: 12,
                                            color: colorScheme.primary),
                                        const SizedBox(width: 4),
                                        Expanded(
                                          child: Text(
                                            tx.memo!,
                                            style: TextStyle(
                                              fontSize: 11,
                                              color: colorScheme.primary,
                                              fontStyle: FontStyle.italic,
                                            ),
                                            maxLines: 1,
                                            overflow: TextOverflow.ellipsis,
                                          ),
                                        ),
                                      ],
                                    ),
                                  ],
                                  const SizedBox(height: 2),
                                  Text(
                                    tx.timestamp > 0
                                        ? DateFormat('MMM dd, yyyy HH:mm')
                                            .format(DateTime
                                                .fromMillisecondsSinceEpoch(
                                                    tx.timestamp * 1000))
                                        : 'Pending',
                                    style: TextStyle(
                                      fontSize: 11,
                                      color: colorScheme.onSurface
                                          .withValues(alpha: 0.5),
                                    ),
                                  ),
                                ],
                              ),
                              trailing: Icon(
                                Icons.arrow_forward_ios,
                                size: 16,
                                color: colorScheme.onSurface
                                    .withValues(alpha: 0.3),
                              ),
                            ),
                          );
                        },
                      ),
                    ),
    );
  }
}
