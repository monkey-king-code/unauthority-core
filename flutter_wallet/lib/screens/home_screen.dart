import '../utils/log.dart';
import '../utils/secure_clipboard.dart';
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import 'package:intl/intl.dart';
import '../services/wallet_service.dart';
import '../services/api_service.dart';
import '../services/network_status_service.dart';
import '../models/account.dart';
import '../config/testnet_config.dart';
import '../widgets/network_badge.dart';
import '../widgets/network_status_bar.dart';
import 'send_screen.dart';
import 'settings_screen.dart';
import 'receive_screen.dart';
import 'history_screen.dart';
import 'network_info_screen.dart';
import 'explorer_screen.dart';
import 'tokens_screen.dart';
import 'dex_screen.dart';

class HomeScreen extends StatefulWidget {
  const HomeScreen({super.key});

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends State<HomeScreen> {
  String? _address;
  Account? _account;
  bool _isLoading = true;
  String? _error;

  @override
  void initState() {
    super.initState();
    _loadWallet();
  }

  Future<void> _loadWallet() async {
    losLog('🏠 [Home] Loading wallet...');
    try {
      final walletService = context.read<WalletService>();
      final wallet = await walletService.getCurrentWallet();

      if (!mounted) return;
      if (wallet == null) {
        setState(() => _error = 'No wallet found');
        return;
      }

      setState(() {
        _address = wallet['address'];
        _isLoading = false;
      });
      losLog('🏠 [Home] Wallet loaded: $_address');

      await _refreshBalance();
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _error = e.toString();
        _isLoading = false;
      });
    }
  }

  Future<void> _refreshBalance() async {
    if (_address == null) return;
    losLog('🏠 [Home] Refreshing balance for $_address...');

    try {
      final apiService = context.read<ApiService>();
      final account = await apiService.getAccount(_address!);

      if (!mounted) return;
      setState(() {
        _account = account;
        _error = null;
      });
      losLog('🏠 [Home] Balance: ${account.balanceDisplay} LOS');
    } catch (e) {
      if (!mounted) return;
      setState(() => _error = e.toString());
    }
  }

  Future<void> _requestFaucet() async {
    if (_address == null) return;
    losLog('🚠 [Home] Requesting faucet for $_address...');

    setState(() => _isLoading = true);

    try {
      final apiService = context.read<ApiService>();
      final result = await apiService.requestFaucet(_address!);

      if (!mounted) return;

      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: Text(result['msg'] ?? 'Faucet claimed successfully!'),
          backgroundColor: Colors.green,
        ),
      );
      losLog('🚠 [Home] Faucet SUCCESS: ${result['msg']}');

      await _refreshBalance();
    } catch (e) {
      if (!mounted) return;
      losLog('🚠 [Home] Faucet ERROR: $e');
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text(e.toString()), backgroundColor: Colors.red),
      );
    } finally {
      if (mounted) setState(() => _isLoading = false);
    }
  }

  void _copyAddress() {
    if (_address != null) {
      SecureClipboard.copyPublic(_address!);
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('Address copied to clipboard')),
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: Row(
          mainAxisSize: MainAxisSize.min,
          children: const [
            Text('LOS Wallet'),
            SizedBox(width: 8),
            NetworkBadge(),
          ],
        ),
        centerTitle: true,
        actions: [
          // Online/Offline dot indicator
          Consumer<NetworkStatusService>(
            builder: (context, status, _) => Tooltip(
              message: status.statusText,
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 4),
                child: Icon(
                  Icons.circle,
                  size: 12,
                  color: status.isConnected
                      ? Colors.green
                      : status.isConnecting
                          ? Colors.orange
                          : Colors.red,
                ),
              ),
            ),
          ),
          IconButton(
            icon: const Icon(Icons.refresh),
            onPressed: _refreshBalance,
          ),
          IconButton(
            icon: const Icon(Icons.settings),
            onPressed: () async {
              await Navigator.push(
                context,
                MaterialPageRoute(builder: (_) => const SettingsScreen()),
              );
              // Refresh after returning from settings (network may have changed)
              _refreshBalance();
            },
          ),
        ],
      ),
      body: Column(
        children: [
          const NetworkStatusBar(),
          Expanded(
            child: _isLoading
                ? const Center(child: CircularProgressIndicator())
                : _error != null
                    ? Center(
                        child: Text('Error: $_error',
                            style: const TextStyle(color: Colors.red)))
                    : RefreshIndicator(
                        onRefresh: _refreshBalance,
                        child: ListView(
                          padding: const EdgeInsets.all(16),
                          children: [
                            // Testnet Warning Banner
                            const NetworkWarningBanner(),
                            // Balance Card
                            Card(
                              child: Padding(
                                padding: const EdgeInsets.all(24.0),
                                child: Column(
                                  children: [
                                    const Text('Total Balance',
                                        style: TextStyle(
                                            fontSize: 14, color: Colors.grey)),
                                    const SizedBox(height: 8),
                                    Text(
                                      '${_account?.balanceDisplay ?? '0.00'} LOS',
                                      style: const TextStyle(
                                          fontSize: 36,
                                          fontWeight: FontWeight.bold),
                                    ),
                                    if (_account != null &&
                                        _account!.cilBalance > 0) ...[
                                      const SizedBox(height: 8),
                                      Text(
                                        'CIL: ${_account!.cilBalanceDisplay} LOS',
                                        style: const TextStyle(
                                            fontSize: 14, color: Colors.orange),
                                      ),
                                    ],
                                  ],
                                ),
                              ),
                            ),

                            const SizedBox(height: 16),

                            // Address Card
                            Card(
                              child: ListTile(
                                title: const Text('Your Address',
                                    style: TextStyle(
                                        fontSize: 12, color: Colors.grey)),
                                subtitle: Text(
                                  _address ?? 'N/A',
                                  style: const TextStyle(
                                      fontSize: 14, fontFamily: 'monospace'),
                                ),
                                trailing: IconButton(
                                  icon: const Icon(Icons.copy, size: 20),
                                  onPressed: _copyAddress,
                                ),
                              ),
                            ),

                            const SizedBox(height: 24),

                            // Action Buttons Row 1: SEND / RECEIVE
                            Row(
                              children: [
                                Expanded(
                                  child: ElevatedButton.icon(
                                    onPressed: () async {
                                      await Navigator.push(
                                        context,
                                        MaterialPageRoute(
                                            builder: (_) => const SendScreen()),
                                      );
                                      _refreshBalance();
                                    },
                                    icon: const Icon(Icons.send),
                                    label: const Text('SEND'),
                                    style: ElevatedButton.styleFrom(
                                      padding: const EdgeInsets.all(16),
                                    ),
                                  ),
                                ),
                                const SizedBox(width: 16),
                                Expanded(
                                  child: OutlinedButton.icon(
                                    onPressed: () {
                                      Navigator.push(
                                        context,
                                        MaterialPageRoute(
                                            builder: (_) =>
                                                const ReceiveScreen()),
                                      );
                                    },
                                    icon: const Icon(Icons.qr_code),
                                    label: const Text('RECEIVE'),
                                    style: OutlinedButton.styleFrom(
                                      padding: const EdgeInsets.all(16),
                                    ),
                                  ),
                                ),
                              ],
                            ),

                            const SizedBox(height: 12),

                            // Action Buttons Row 2: HISTORY / NETWORK
                            Row(
                              children: [
                                Expanded(
                                  child: OutlinedButton.icon(
                                    onPressed: () {
                                      Navigator.push(
                                        context,
                                        MaterialPageRoute(
                                            builder: (_) =>
                                                const HistoryScreen()),
                                      );
                                    },
                                    icon: const Icon(Icons.history),
                                    label: const Text('HISTORY'),
                                    style: OutlinedButton.styleFrom(
                                      padding: const EdgeInsets.all(16),
                                    ),
                                  ),
                                ),
                                const SizedBox(width: 16),
                                Expanded(
                                  child: OutlinedButton.icon(
                                    onPressed: () {
                                      Navigator.push(
                                        context,
                                        MaterialPageRoute(
                                            builder: (_) =>
                                                const NetworkInfoScreen()),
                                      );
                                    },
                                    icon: const Icon(Icons.public),
                                    label: const Text('NETWORK'),
                                    style: OutlinedButton.styleFrom(
                                      padding: const EdgeInsets.all(16),
                                    ),
                                  ),
                                ),
                              ],
                            ),

                            const SizedBox(height: 12),

                            // Action Buttons Row 3: EXPLORER / TOKENS
                            Row(
                              children: [
                                Expanded(
                                  child: OutlinedButton.icon(
                                    onPressed: () {
                                      Navigator.push(
                                        context,
                                        MaterialPageRoute(
                                            builder: (_) =>
                                                const ExplorerScreen()),
                                      );
                                    },
                                    icon: const Icon(Icons.explore),
                                    label: const Text('EXPLORER'),
                                    style: OutlinedButton.styleFrom(
                                      padding: const EdgeInsets.all(16),
                                    ),
                                  ),
                                ),
                                const SizedBox(width: 16),
                                Expanded(
                                  child: OutlinedButton.icon(
                                    onPressed: () {
                                      Navigator.push(
                                        context,
                                        MaterialPageRoute(
                                            builder: (_) =>
                                                const TokensScreen()),
                                      );
                                    },
                                    icon: const Icon(Icons.token),
                                    label: const Text('TOKENS'),
                                    style: OutlinedButton.styleFrom(
                                      padding: const EdgeInsets.all(16),
                                    ),
                                  ),
                                ),
                              ],
                            ),

                            const SizedBox(height: 12),

                            // Action Buttons Row 4: DEX
                            Row(
                              children: [
                                Expanded(
                                  child: OutlinedButton.icon(
                                    onPressed: () {
                                      Navigator.push(
                                        context,
                                        MaterialPageRoute(
                                            builder: (_) => const DexScreen()),
                                      );
                                    },
                                    icon: const Icon(Icons.swap_horiz),
                                    label: const Text('DEX'),
                                    style: OutlinedButton.styleFrom(
                                      padding: const EdgeInsets.all(16),
                                    ),
                                  ),
                                ),
                              ],
                            ),

                            const SizedBox(height: 16),

                            // Faucet is only available on testnet, hidden on mainnet
                            if (WalletConfig.current.faucetAvailable)
                              ElevatedButton.icon(
                                onPressed: _requestFaucet,
                                icon: const Icon(Icons.water_drop),
                                label: const Text('REQUEST FAUCET (5,000 LOS)'),
                                style: ElevatedButton.styleFrom(
                                  backgroundColor: Colors.purple,
                                  padding: const EdgeInsets.all(16),
                                ),
                              ),

                            const SizedBox(height: 32),

                            // Transaction History
                            if (_account != null &&
                                _account!.history.isNotEmpty) ...[
                              const Text(
                                'Recent Transactions',
                                style: TextStyle(
                                    fontSize: 18, fontWeight: FontWeight.bold),
                              ),
                              const SizedBox(height: 12),
                              ..._account!.history.map((tx) {
                                // Determine direction: check both from and type
                                final isSent = tx.from == _address ||
                                    (tx.type == 'send' && tx.from.isNotEmpty);
                                final isMint = tx.type == 'mint';
                                final otherParty = isSent ? tx.to : tx.from;
                                // Safe timestamp display
                                final timeStr = tx.timestamp > 0
                                    ? DateFormat('MMM dd, yyyy HH:mm').format(
                                        DateTime.fromMillisecondsSinceEpoch(
                                            tx.timestamp * 1000))
                                    : 'Pending';
                                // Direction label
                                String dirLabel;
                                if (isMint) {
                                  dirLabel = 'Minted';
                                } else if (isSent) {
                                  dirLabel =
                                      'To: ${otherParty.length > 20 ? '${otherParty.substring(0, 8)}...${otherParty.substring(otherParty.length - 8)}' : otherParty}';
                                } else {
                                  dirLabel =
                                      'From: ${otherParty.length > 20 ? '${otherParty.substring(0, 8)}...${otherParty.substring(otherParty.length - 8)}' : otherParty}';
                                }
                                return Card(
                                  child: ListTile(
                                    leading: Icon(
                                      isMint
                                          ? Icons.add_circle
                                          : (isSent
                                              ? Icons.arrow_upward
                                              : Icons.arrow_downward),
                                      color: isMint
                                          ? Colors.blue
                                          : (isSent
                                              ? Colors.red
                                              : Colors.green),
                                    ),
                                    title: Text(
                                      '${tx.amountDisplay} LOS',
                                      style: const TextStyle(
                                          fontWeight: FontWeight.bold),
                                    ),
                                    subtitle: Text(
                                      '$dirLabel\n$timeStr',
                                      style: const TextStyle(fontSize: 11),
                                    ),
                                    trailing: Text(
                                      tx.type.toUpperCase(),
                                      style: const TextStyle(
                                          fontSize: 10, color: Colors.grey),
                                    ),
                                  ),
                                );
                              }),
                            ],
                          ],
                        ),
                      ),
          ),
        ],
      ),
    );
  }
}
