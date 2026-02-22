// DEX Pools Screen — browse liquidity pools and LP positions.
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/api_service.dart';
import '../services/wallet_service.dart';
import '../models/dex_pool.dart';
import '../utils/log.dart';
import 'dex_swap_screen.dart';

class DexScreen extends StatefulWidget {
  const DexScreen({super.key});

  @override
  State<DexScreen> createState() => _DexScreenState();
}

class _DexScreenState extends State<DexScreen>
    with SingleTickerProviderStateMixin {
  late TabController _tabController;
  List<DexPool> _pools = [];
  List<LpPosition> _positions = [];
  bool _isLoading = true;
  String? _error;

  @override
  void initState() {
    super.initState();
    _tabController = TabController(length: 2, vsync: this);
    _loadData();
  }

  @override
  void dispose() {
    _tabController.dispose();
    super.dispose();
  }

  Future<void> _loadData() async {
    setState(() {
      _isLoading = true;
      _error = null;
    });
    try {
      final api = context.read<ApiService>();
      final wallet = context.read<WalletService>();
      final pools = await api.getDexPools();

      // Load LP positions for current wallet
      final walletInfo = await wallet.getCurrentWallet();
      final myAddr = walletInfo?['address'];
      final positions = <LpPosition>[];
      if (myAddr != null) {
        for (final pool in pools) {
          try {
            final pos = await api.getDexPosition(
                pool.contractAddress, pool.poolId, myAddr);
            if (pos.lpShares != '0') {
              positions.add(pos);
            }
          } catch (_) {
            // Pool may have no position for this user — ignore
          }
        }
      }

      if (!mounted) return;
      setState(() {
        _pools = pools;
        _positions = positions;
        _isLoading = false;
      });
    } catch (e) {
      losLog('❌ DEX load error: $e');
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
        title: const Text('DEX'),
        bottom: TabBar(
          controller: _tabController,
          tabs: const [
            Tab(text: 'Pools', icon: Icon(Icons.water_drop)),
            Tab(text: 'My Positions', icon: Icon(Icons.account_balance_wallet)),
          ],
        ),
      ),
      body: _isLoading
          ? const Center(child: CircularProgressIndicator())
          : _error != null
              ? Center(
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Text('Error: $_error',
                          style: const TextStyle(color: Colors.red)),
                      const SizedBox(height: 8),
                      ElevatedButton(
                          onPressed: _loadData, child: const Text('Retry')),
                    ],
                  ),
                )
              : RefreshIndicator(
                  onRefresh: _loadData,
                  child: TabBarView(
                    controller: _tabController,
                    children: [
                      _buildPoolsTab(),
                      _buildPositionsTab(),
                    ],
                  ),
                ),
    );
  }

  Widget _buildPoolsTab() {
    if (_pools.isEmpty) {
      return const Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(Icons.water_drop_outlined, size: 64, color: Colors.grey),
            SizedBox(height: 16),
            Text('No liquidity pools yet',
                style: TextStyle(color: Colors.grey)),
          ],
        ),
      );
    }

    return ListView.builder(
      padding: const EdgeInsets.all(12),
      itemCount: _pools.length,
      itemBuilder: (context, i) => _PoolCard(
        pool: _pools[i],
        onSwap: () => Navigator.push(
          context,
          MaterialPageRoute(builder: (_) => DexSwapScreen(pool: _pools[i])),
        ).then((_) => _loadData()),
      ),
    );
  }

  Widget _buildPositionsTab() {
    if (_positions.isEmpty) {
      return const Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(Icons.account_balance_wallet_outlined,
                size: 64, color: Colors.grey),
            SizedBox(height: 16),
            Text('No LP positions', style: TextStyle(color: Colors.grey)),
          ],
        ),
      );
    }

    return ListView.builder(
      padding: const EdgeInsets.all(12),
      itemCount: _positions.length,
      itemBuilder: (context, i) {
        final pos = _positions[i];
        return Card(
          child: ListTile(
            leading: const CircleAvatar(child: Icon(Icons.pool)),
            title: Text('Pool ${pos.poolId}'),
            subtitle: Text('LP Shares: ${pos.lpShares}'),
            trailing: const Icon(Icons.chevron_right),
          ),
        );
      },
    );
  }
}

class _PoolCard extends StatelessWidget {
  final DexPool pool;
  final VoidCallback onSwap;

  const _PoolCard({required this.pool, required this.onSwap});

  @override
  Widget build(BuildContext context) {
    return Card(
      margin: const EdgeInsets.only(bottom: 12),
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Expanded(
                  child: Text(
                    pool.pairLabel,
                    style: const TextStyle(
                        fontSize: 18, fontWeight: FontWeight.bold),
                  ),
                ),
                Container(
                  padding:
                      const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
                  decoration: BoxDecoration(
                    color: Colors.blue.withValues(alpha: 0.15),
                    borderRadius: BorderRadius.circular(8),
                  ),
                  child: Text(
                    'Fee: ${pool.feeDisplay}',
                    style: TextStyle(fontSize: 11, color: Colors.blue.shade300),
                  ),
                ),
              ],
            ),
            const SizedBox(height: 12),
            Row(
              children: [
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      const Text('Reserve A',
                          style: TextStyle(fontSize: 11, color: Colors.grey)),
                      Text(pool.reserveA,
                          style: const TextStyle(fontWeight: FontWeight.w500)),
                    ],
                  ),
                ),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      const Text('Reserve B',
                          style: TextStyle(fontSize: 11, color: Colors.grey)),
                      Text(pool.reserveB,
                          style: const TextStyle(fontWeight: FontWeight.w500)),
                    ],
                  ),
                ),
              ],
            ),
            const SizedBox(height: 8),
            Text('Total LP: ${pool.totalLp}',
                style: const TextStyle(fontSize: 12, color: Colors.grey)),
            const SizedBox(height: 12),
            SizedBox(
              width: double.infinity,
              child: ElevatedButton.icon(
                onPressed: onSwap,
                icon: const Icon(Icons.swap_horiz),
                label: const Text('Swap'),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
