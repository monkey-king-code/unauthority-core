import '../utils/log.dart';
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/api_service.dart';
import '../constants/blockchain.dart';

/// Network information screen showing supply, consensus, validators, and rewards.
/// Gives wallet users transparency into the blockchain's health and economics.
class NetworkInfoScreen extends StatefulWidget {
  const NetworkInfoScreen({super.key});

  @override
  State<NetworkInfoScreen> createState() => _NetworkInfoScreenState();
}

class _NetworkInfoScreenState extends State<NetworkInfoScreen> {
  Map<String, dynamic>? _supply;
  Map<String, dynamic>? _consensus;
  Map<String, dynamic>? _rewardInfo;
  Map<String, dynamic>? _nodeInfo;
  bool _isLoading = true;
  String? _error;

  @override
  void initState() {
    super.initState();
    _loadAll();
  }

  Future<void> _loadAll() async {
    losLog('🌐 [NetworkInfo] Loading all network data...');
    setState(() {
      _isLoading = true;
      _error = null;
    });

    try {
      final api = context.read<ApiService>();
      final results = await Future.wait([
        api.getSupply().catchError((e) {
          losLog('⚠️ [NetworkInfo] getSupply failed: $e');
          return <String, dynamic>{};
        }),
        api.getConsensus().catchError((e) {
          losLog('⚠️ [NetworkInfo] getConsensus failed: $e');
          return <String, dynamic>{};
        }),
        api.getRewardInfo().catchError((e) {
          losLog('⚠️ [NetworkInfo] getRewardInfo failed: $e');
          return <String, dynamic>{};
        }),
        api.getNodeInfo().catchError((e) {
          losLog('⚠️ [NetworkInfo] getNodeInfo failed: $e');
          return <String, dynamic>{};
        }),
      ]);

      if (!mounted) return;
      losLog('🌐 [NetworkInfo] All data loaded successfully');
      setState(() {
        _supply = results[0];
        _consensus = results[1];
        _rewardInfo = results[2];
        _nodeInfo = results[3];
        _isLoading = false;
      });
    } catch (e) {
      losLog('❌ [NetworkInfo] _loadAll error: $e');
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
        title: const Text('Network Info'),
        centerTitle: true,
        actions: [
          IconButton(
            icon: const Icon(Icons.refresh),
            onPressed: _loadAll,
          ),
        ],
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
                      const SizedBox(height: 16),
                      ElevatedButton(
                        onPressed: _loadAll,
                        child: const Text('Retry'),
                      ),
                    ],
                  ),
                )
              : RefreshIndicator(
                  onRefresh: _loadAll,
                  child: ListView(
                    padding: const EdgeInsets.all(16),
                    children: [
                      _buildNodeInfoCard(),
                      const SizedBox(height: 12),
                      _buildSupplyCard(),
                      const SizedBox(height: 12),
                      _buildConsensusCard(),
                      const SizedBox(height: 12),
                      _buildRewardsCard(),
                    ],
                  ),
                ),
    );
  }

  Widget _buildNodeInfoCard() {
    if (_nodeInfo == null || _nodeInfo!.isEmpty) return const SizedBox.shrink();
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const Row(
              children: [
                Icon(Icons.dns, color: Colors.blue),
                SizedBox(width: 8),
                Text('Node Info',
                    style:
                        TextStyle(fontSize: 18, fontWeight: FontWeight.bold)),
              ],
            ),
            const Divider(),
            _infoRow('Network', _nodeInfo!['network']?.toString() ?? 'N/A'),
            _infoRow('Version', _nodeInfo!['version']?.toString() ?? 'N/A'),
            _infoRow(
                'Block Height', _nodeInfo!['block_height']?.toString() ?? '0'),
            _infoRow(
                'Validators', _nodeInfo!['validator_count']?.toString() ?? '0'),
            _infoRow('Peers', _nodeInfo!['peer_count']?.toString() ?? '0'),
            _infoRow('Chain ID', _nodeInfo!['chain_id']?.toString() ?? 'N/A'),
          ],
        ),
      ),
    );
  }

  /// Safely convert a CIL value from JSON to a display LOS string.
  /// JSON numbers > 2^53 may lose precision as double; we prefer int parsing.
  /// Falls back to the raw _los string field if CIL is unavailable.
  String _cilToDisplay(dynamic cilValue, [String? losStringFallback]) {
    if (cilValue != null) {
      int? parsed;
      if (cilValue is int) {
        parsed = cilValue;
      } else {
        parsed = int.tryParse(cilValue.toString());
      }
      if (parsed != null) {
        return BlockchainConstants.cilToLosString(parsed);
      }
    }
    // Fallback: use the _los string from backend (may have f64 drift)
    return losStringFallback ?? 'N/A';
  }

  Widget _buildSupplyCard() {
    if (_supply == null || _supply!.isEmpty) return const SizedBox.shrink();

    final remainingLos = _cilToDisplay(_supply!['remaining_supply_cil'],
        _supply!['remaining_supply']?.toString());

    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const Row(
              children: [
                Icon(Icons.pie_chart, color: Colors.green),
                SizedBox(width: 8),
                Text('Token Supply',
                    style:
                        TextStyle(fontSize: 18, fontWeight: FontWeight.bold)),
              ],
            ),
            const Divider(),
            _infoRow('Total Supply',
                '${BlockchainConstants.totalSupply.toString()} LOS'),
            _infoRow('Remaining (PoW Mining)', '$remainingLos LOS'),
            _infoRow('Total Burned (USD)',
                '\$${_supply!['total_burned_usd']?.toString() ?? '0'}'),
          ],
        ),
      ),
    );
  }

  Widget _buildConsensusCard() {
    if (_consensus == null || _consensus!.isEmpty) {
      return const SizedBox.shrink();
    }

    final safety = _consensus!['safety'] as Map<String, dynamic>? ?? {};
    final finality = _consensus!['finality'] as Map<String, dynamic>? ?? {};

    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const Row(
              children: [
                Icon(Icons.shield, color: Colors.orange),
                SizedBox(width: 8),
                Text('Consensus',
                    style:
                        TextStyle(fontSize: 18, fontWeight: FontWeight.bold)),
              ],
            ),
            const Divider(),
            _infoRow('Protocol', _consensus!['protocol']?.toString() ?? 'aBFT'),
            _infoRow('Active Validators',
                safety['active_validators']?.toString() ?? '0'),
            _infoRow(
              'Byzantine Safe',
              (safety['byzantine_safe'] == true) ? 'Yes' : 'No',
              valueColor: (safety['byzantine_safe'] == true)
                  ? Colors.green
                  : Colors.red,
            ),
            if (finality['average_ms'] != null)
              _infoRow('Avg Finality', '${finality['average_ms']}ms'),
          ],
        ),
      ),
    );
  }

  Widget _buildRewardsCard() {
    if (_rewardInfo == null || _rewardInfo!.isEmpty) {
      return const SizedBox.shrink();
    }

    final epoch = _rewardInfo!['epoch'] as Map<String, dynamic>? ?? {};
    final pool = _rewardInfo!['pool'] as Map<String, dynamic>? ?? {};
    final validators =
        _rewardInfo!['validators'] as Map<String, dynamic>? ?? {};

    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const Row(
              children: [
                Icon(Icons.emoji_events, color: Colors.amber),
                SizedBox(width: 8),
                Text('Reward Pool',
                    style:
                        TextStyle(fontSize: 18, fontWeight: FontWeight.bold)),
              ],
            ),
            const Divider(),
            _infoRow(
                'Current Epoch', epoch['current_epoch']?.toString() ?? '0'),
            _infoRow('Epoch Reward Rate',
                '${_cilToDisplay(epoch['epoch_reward_rate_cil'], epoch['epoch_reward_rate_los']?.toString())} LOS/epoch'),
            _infoRow('Pool Remaining',
                '${_cilToDisplay(pool['remaining_cil'], pool['remaining_los']?.toString())} LOS'),
            _infoRow('Total Distributed',
                '${_cilToDisplay(pool['total_distributed_cil'], pool['total_distributed_los']?.toString())} LOS'),
            _infoRow('Eligible Validators',
                '${validators['eligible'] ?? 0}/${validators['total'] ?? 0}'),
          ],
        ),
      ),
    );
  }

  Widget _infoRow(String label, String value, {Color? valueColor}) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Row(
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        children: [
          Text(label, style: const TextStyle(color: Colors.grey)),
          Text(
            value,
            style: TextStyle(
              fontWeight: FontWeight.w600,
              color: valueColor,
            ),
          ),
        ],
      ),
    );
  }
}
