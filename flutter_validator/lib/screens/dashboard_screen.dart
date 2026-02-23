import '../utils/log.dart';
import 'package:flutter/material.dart';
import 'dart:async';
import 'package:provider/provider.dart';
import 'package:intl/intl.dart';
import '../services/api_service.dart';
import '../services/network_status_service.dart';
import '../services/node_process_service.dart';
import '../services/wallet_service.dart';
import '../models/account.dart';
import '../constants/blockchain.dart';
import '../widgets/network_status_bar.dart';
import '../widgets/voting_power_card.dart';
import '../widgets/uptime_card.dart';
import '../widgets/network_tokens_card.dart';

class DashboardScreen extends StatefulWidget {
  /// When embedded in NodeControlScreen's IndexedStack, hide appbar.
  final bool embedded;

  const DashboardScreen({super.key, this.embedded = false});

  @override
  State<DashboardScreen> createState() => _DashboardScreenState();
}

class _DashboardScreenState extends State<DashboardScreen> {
  Map<String, dynamic>? _nodeInfo;
  Map<String, dynamic>? _health;
  List<ValidatorInfo> _validators = [];
  List<BlockInfo> _recentBlocks = [];
  List<String> _peers = [];
  bool _isLoading = true;
  bool _isFirstLoad = true;
  String? _error;
  String? _myAddress;

  // Reward countdown state
  Map<String, dynamic>? _rewardInfo;
  Map<String, dynamic>? _miningInfo; // PoW mining stats from local node
  Map<String, dynamic>? _slashingInfo; // Slashing status for this validator
  Map<String, dynamic>? _consensusInfo; // Consensus state
  Map<String, dynamic>? _syncStatus; // Sync progress
  Map<String, dynamic>? _supplyInfo; // Total supply stats
  int _epochRemainingSecs = 0;
  Timer? _countdownTimer;
  Timer? _autoRefreshTimer;
  bool _isDashboardLoading = false; // Prevent concurrent _loadDashboard calls
  DateTime? _lastMonitoringLoad; // Throttle monitoring API calls

  @override
  void initState() {
    super.initState();
    _loadMyAddress();
    _loadDashboard();
    _startCountdownTimer();
    // Auto-refresh dashboard every 30s — but skip when node is stopped
    // Also skip if a load is already in progress (Tor latency guard)
    _autoRefreshTimer = Timer.periodic(const Duration(seconds: 30), (_) {
      if (!mounted) return;
      final nodeService = context.read<NodeProcessService>();
      if (nodeService.isRunning && !_isDashboardLoading) _loadDashboard();
    });
  }

  @override
  void dispose() {
    _countdownTimer?.cancel();
    _autoRefreshTimer?.cancel();
    super.dispose();
  }

  /// Start a 1-second timer that decrements the countdown locally.
  /// When it reaches 0, auto-refresh from the API to pick up the new epoch.
  /// Pauses when node is stopped (no API to query).
  ///
  /// ANTI-STALL design:
  /// - If `_epochRemainingSecs` sits at 0 for too long (API returned 0 or
  ///   the `_loadDashboard` call failed), we force a re-fetch every 15s
  ///   to recover the countdown automatically.
  /// - We NEVER fire a re-fetch while a previous `_loadDashboard` is still
  ///   in-flight, to avoid piling up concurrent API requests through the
  ///   Tor SOCKS5 proxy (which saturates and causes 45s timeout cascades).
  int _zeroTickCount = 0;

  void _startCountdownTimer() {
    losLog(
        '📊 [DashboardScreen._startCountdownTimer] Starting countdown timer');
    _countdownTimer = Timer.periodic(const Duration(seconds: 1), (_) {
      if (!mounted) return;

      // Pause countdown when node is not running
      final nodeService = context.read<NodeProcessService>();
      if (!nodeService.isRunning) return;

      if (_epochRemainingSecs > 1) {
        _zeroTickCount = 0; // Reset stall counter
        setState(() => _epochRemainingSecs--);
      } else if (_epochRemainingSecs == 1) {
        // Epoch just ended — set to 0 then auto-refresh from API
        // to get the new epoch's remaining_secs.
        _zeroTickCount = 0;
        setState(() => _epochRemainingSecs = 0);
        if (!_isDashboardLoading) _loadDashboard();
      } else {
        // _epochRemainingSecs == 0: waiting for _loadDashboard to set a new value.
        // ANTI-STALL: if stuck at 0 for 15+ seconds AND no load is in-flight,
        // force a re-fetch.
        // This handles: (1) API returned 0 during epoch boundary,
        //               (2) _loadDashboard() failed/timed out,
        //               (3) first load before any API response.
        _zeroTickCount++;
        if (_zeroTickCount >= 15 && !_isDashboardLoading) {
          _zeroTickCount = 0;
          losLog(
              '📊 [DashboardScreen] Countdown stuck at 0 — force re-fetching epoch data');
          _loadDashboard();
        }
      }
    });
  }

  Future<void> _loadMyAddress() async {
    losLog('📊 [DashboardScreen._loadMyAddress] Loading address...');
    final walletService = context.read<WalletService>();
    final wallet = await walletService.getCurrentWallet();
    if (wallet != null && mounted) {
      setState(() => _myAddress = wallet['address']);
      losLog(
          '📊 [DashboardScreen._loadMyAddress] Address: ${wallet['address']}');
    }
  }

  Future<void> _loadDashboard() async {
    // Prevent concurrent loads — critical over Tor where each call takes 1-45s.
    // Without this guard, the anti-stall timer + auto-refresh + epoch-end all
    // pile up concurrent `Future.wait` batches that saturate the SOCKS5 proxy.
    if (_isDashboardLoading) return;
    _isDashboardLoading = true;

    losLog('📊 [DashboardScreen._loadDashboard] Loading dashboard...');
    // Only show full-screen spinner on first load.
    // Subsequent refreshes update data silently in the background
    // to avoid annoying loading indicators.
    if (_isFirstLoad) {
      setState(() => _isLoading = true);
    }

    try {
      final apiService = context.read<ApiService>();
      final nodeService = context.read<NodeProcessService>();

      // PHASE 1: Load core data (required for dashboard render)
      // These 7 calls are essential — dashboard can't render without them.
      final results = await Future.wait([
        apiService.getNodeInfo(),
        apiService.getHealth(),
        apiService.getValidators(),
        apiService.getRecentBlocks(),
        apiService.getPeers(),
        apiService.getRewardInfo().catchError((_) => <String, dynamic>{}),
        apiService
            .getMiningInfo(localUrl: nodeService.localApiUrl)
            .catchError((_) => <String, dynamic>{}),
      ]);

      if (!mounted) return;
      final rewardData = results[5] as Map<String, dynamic>;
      final miningData = results[6] as Map<String, dynamic>;
      setState(() {
        _nodeInfo = results[0] as Map<String, dynamic>;
        _health = results[1] as Map<String, dynamic>;
        _validators = results[2] as List<ValidatorInfo>;
        _recentBlocks = results[3] as List<BlockInfo>;
        _peers = results[4] as List<String>;
        _rewardInfo = rewardData.isNotEmpty ? rewardData : null;
        _miningInfo = miningData.isNotEmpty ? miningData : null;
        if (_rewardInfo != null && _rewardInfo!['epoch'] != null) {
          final remaining =
              (_rewardInfo!['epoch']['epoch_remaining_secs'] as num?)
                      ?.toInt() ??
                  0;
          if (remaining > 0) {
            // API returned a positive value — use it directly.
            _epochRemainingSecs = remaining;
            _zeroTickCount = 0; // Reset stall counter on good data
          } else if (_epochRemainingSecs == 0) {
            // API returned 0 AND countdown is already at 0 — epoch boundary.
            // Use epoch_duration_secs as fallback so countdown recovers.
            final duration =
                (_rewardInfo!['epoch']['epoch_duration_secs'] as num?)
                        ?.toInt() ??
                    0;
            if (duration > 0) {
              _epochRemainingSecs = duration;
              _zeroTickCount = 0;
              losLog(
                  '📊 [DashboardScreen] Countdown stuck at 0 — reset to epoch duration: ${duration}s');
            }
          }
          // else: remaining == 0 but countdown is still counting down — skip
        }
        _error = null;
        _isLoading = false;
        _isFirstLoad = false;
      });
      losLog(
          '📊 [DashboardScreen._loadDashboard] Success: validators=${_validators.length}, block_height=${_nodeInfo?['block_height']}, peers=${_peers.length}');

      // PHASE 2: Load monitoring data asynchronously (non-blocking)
      // These endpoints may not exist on all nodes or return empty — don't
      // block the dashboard spinner for them. Throttled to every 2 minutes
      // to reduce Tor SOCKS5 proxy load (4 extra API calls each time).
      final now = DateTime.now();
      final shouldLoadMonitoring = _lastMonitoringLoad == null ||
          now.difference(_lastMonitoringLoad!).inSeconds >= 120;
      if (shouldLoadMonitoring) {
        _lastMonitoringLoad = now;
        _loadMonitoringData(apiService);
      }
    } catch (e) {
      if (!mounted) {
        _isDashboardLoading = false;
        return;
      }
      setState(() {
        // Only show error on first load. On subsequent refreshes,
        // keep showing the last known data instead of replacing with error screen.
        if (_isFirstLoad) {
          _error = e.toString();
        }
        _isLoading = false;
        _isFirstLoad = false;
      });
    } finally {
      _isDashboardLoading = false;
    }
  }

  /// Load monitoring cards data asynchronously — does NOT block the dashboard.
  /// These endpoints (sync, consensus, supply, slashing) may not exist on all
  /// nodes or may return empty `{}`. We fire-and-forget and update the UI
  /// when results arrive.
  Future<void> _loadMonitoringData(ApiService apiService) async {
    try {
      final monitorResults = await Future.wait([
        apiService.getConsensusInfo().catchError((_) => <String, dynamic>{}),
        apiService.getSyncStatus().catchError((_) => <String, dynamic>{}),
        apiService.getSupply().catchError((_) => <String, dynamic>{}),
        (_myAddress != null
                ? apiService.getSlashingForAddress(_myAddress!)
                : Future.value(<String, dynamic>{}))
            .catchError((_) => <String, dynamic>{}),
      ]);
      if (!mounted) return;
      final consensusData = monitorResults[0];
      final syncData = monitorResults[1];
      final supplyData = monitorResults[2];
      final slashingData = monitorResults[3];
      setState(() {
        _consensusInfo = consensusData.isNotEmpty ? consensusData : null;
        _syncStatus = syncData.isNotEmpty ? syncData : null;
        _supplyInfo = supplyData.isNotEmpty ? supplyData : null;
        _slashingInfo = slashingData.isNotEmpty ? slashingData : null;
      });
    } catch (e) {
      losLog('📊 [DashboardScreen] Monitoring data load failed: $e');
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: widget.embedded
          ? null
          : AppBar(
              title: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  const Text('LOS Validator'),
                  const SizedBox(width: 8),
                  _NetworkBadge(),
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
                  onPressed: _loadDashboard,
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
                        child: Column(
                          mainAxisAlignment: MainAxisAlignment.center,
                          children: [
                            const Icon(Icons.error_outline,
                                size: 64, color: Colors.red),
                            const SizedBox(height: 16),
                            Text(
                              'Error: $_error',
                              style: const TextStyle(color: Colors.red),
                            ),
                            const SizedBox(height: 16),
                            ElevatedButton(
                              onPressed: _loadDashboard,
                              child: const Text('RETRY'),
                            ),
                          ],
                        ),
                      )
                    : RefreshIndicator(
                        onRefresh: _loadDashboard,
                        child: ListView(
                          padding: const EdgeInsets.all(16),
                          children: [
                            // Network Warning Banner (testnet only)
                            if (const String.fromEnvironment('NETWORK',
                                    defaultValue: 'mainnet') !=
                                'mainnet')
                              Container(
                                width: double.infinity,
                                padding: const EdgeInsets.all(12),
                                margin: const EdgeInsets.only(bottom: 16),
                                decoration: BoxDecoration(
                                  color: Colors.orange.withValues(alpha: 0.1),
                                  border: Border.all(
                                      color: Colors.orange, width: 1),
                                  borderRadius: BorderRadius.circular(8),
                                ),
                                child: const Row(
                                  children: [
                                    Icon(Icons.science,
                                        color: Colors.orange, size: 20),
                                    SizedBox(width: 12),
                                    Expanded(
                                      child: Text(
                                        '⚠️ TESTNET — This is a testing network. '
                                        'Tokens have no real value.',
                                        style: TextStyle(
                                          color: Colors.orange,
                                          fontSize: 12,
                                          fontWeight: FontWeight.w500,
                                        ),
                                      ),
                                    ),
                                  ],
                                ),
                              ),
                            // Local-only fallback warning (shows when Tor is down)
                            Consumer<ApiService>(
                              builder: (context, api, _) {
                                if (!api.isUsingLocalFallback) {
                                  return const SizedBox.shrink();
                                }
                                return Container(
                                  width: double.infinity,
                                  padding: const EdgeInsets.all(12),
                                  margin: const EdgeInsets.only(bottom: 16),
                                  decoration: BoxDecoration(
                                    color: Colors.deepOrange
                                        .withValues(alpha: 0.12),
                                    border: Border.all(
                                        color: Colors.deepOrange, width: 1),
                                    borderRadius: BorderRadius.circular(8),
                                  ),
                                  child: const Row(
                                    children: [
                                      Icon(Icons.wifi_off,
                                          color: Colors.deepOrange, size: 20),
                                      SizedBox(width: 12),
                                      Expanded(
                                        child: Text(
                                          '🏠 LOCAL DATA ONLY — Tor is reconnecting. '
                                          'Data shown is from your local node. '
                                          'External verification will resume automatically.',
                                          style: TextStyle(
                                            color: Colors.deepOrange,
                                            fontSize: 12,
                                            fontWeight: FontWeight.w500,
                                          ),
                                        ),
                                      ),
                                    ],
                                  ),
                                );
                              },
                            ),
                            // Node Info Card
                            Card(
                              child: Padding(
                                padding: const EdgeInsets.all(16.0),
                                child: Column(
                                  crossAxisAlignment: CrossAxisAlignment.start,
                                  children: [
                                    const Row(
                                      children: [
                                        Icon(Icons.dns, size: 24),
                                        SizedBox(width: 8),
                                        Text(
                                          'Node Information',
                                          style: TextStyle(
                                            fontSize: 18,
                                            fontWeight: FontWeight.bold,
                                          ),
                                        ),
                                      ],
                                    ),
                                    const Divider(),
                                    _buildInfoRow(
                                      'Network',
                                      _nodeInfo?['network'] ??
                                          _nodeInfo?['chain_id'] ??
                                          'N/A',
                                    ),
                                    _buildInfoRow(
                                      'Version',
                                      _nodeInfo?['version'] ?? 'N/A',
                                    ),
                                    _buildInfoRow(
                                      'Block Height',
                                      '${_nodeInfo?['block_height'] ?? 0}',
                                    ),
                                    _buildInfoRow(
                                      'Validators',
                                      '${_nodeInfo?['validator_count'] ?? 0}',
                                    ),
                                    _buildInfoRow(
                                      'Peers',
                                      '${_nodeInfo?['peer_count'] ?? 0}',
                                    ),
                                  ],
                                ),
                              ),
                            ),

                            const SizedBox(height: 16),

                            // Health Status Card
                            Card(
                              child: Padding(
                                padding: const EdgeInsets.all(16.0),
                                child: Column(
                                  crossAxisAlignment: CrossAxisAlignment.start,
                                  children: [
                                    Row(
                                      children: [
                                        Icon(
                                          Icons.favorite,
                                          size: 24,
                                          color:
                                              _health?['status']?.toString() ==
                                                      'healthy'
                                                  ? Colors.green
                                                  : Colors.red,
                                        ),
                                        const SizedBox(width: 8),
                                        const Text(
                                          'Health Status',
                                          style: TextStyle(
                                            fontSize: 18,
                                            fontWeight: FontWeight.bold,
                                          ),
                                        ),
                                      ],
                                    ),
                                    const Divider(),
                                    _buildInfoRow(
                                      'Status',
                                      _health?['status']
                                              ?.toString()
                                              .toUpperCase() ??
                                          'UNKNOWN',
                                    ),
                                    _buildInfoRow(
                                      'Uptime',
                                      _formatUptime(_health?['uptime_seconds']),
                                    ),
                                  ],
                                ),
                              ),
                            ),

                            const SizedBox(height: 16),

                            // Reward Countdown Card
                            if (_rewardInfo != null)
                              _buildRewardCountdownCard(),

                            const SizedBox(height: 16),

                            // PoW Mining Card
                            Consumer<NodeProcessService>(
                              builder: (ctx, node, _) {
                                if (_miningInfo != null) {
                                  return _buildMiningInfoCard(node);
                                }
                                if (node.isRunning && node.enableMining) {
                                  return Card(
                                    child: Padding(
                                      padding: const EdgeInsets.all(16),
                                      child: Row(children: [
                                        const SizedBox(
                                            width: 20,
                                            height: 20,
                                            child: CircularProgressIndicator(
                                                strokeWidth: 2)),
                                        const SizedBox(width: 12),
                                        const Text('Loading mining stats...'),
                                      ]),
                                    ),
                                  );
                                }
                                return const SizedBox.shrink();
                              },
                            ),

                            const SizedBox(height: 16),

                            // Validators Card
                            Card(
                              child: Padding(
                                padding: const EdgeInsets.all(16.0),
                                child: Column(
                                  crossAxisAlignment: CrossAxisAlignment.start,
                                  children: [
                                    const Row(
                                      children: [
                                        Icon(Icons.verified_user, size: 24),
                                        SizedBox(width: 8),
                                        Text(
                                          'Active Validators',
                                          style: TextStyle(
                                            fontSize: 18,
                                            fontWeight: FontWeight.bold,
                                          ),
                                        ),
                                      ],
                                    ),
                                    const Divider(),
                                    ..._validators.map(
                                      (v) {
                                        final isYou = _myAddress != null &&
                                            v.address == _myAddress;
                                        return ListTile(
                                          leading: Icon(
                                            isYou
                                                ? Icons.star
                                                : Icons.check_circle,
                                            color: isYou
                                                ? Colors.amberAccent
                                                : v.isActive
                                                    ? Colors.green
                                                    : Colors.grey,
                                          ),
                                          title: Row(
                                            children: [
                                              Expanded(
                                                child: Text(
                                                  v.address,
                                                  style: TextStyle(
                                                    fontSize: 12,
                                                    fontFamily: 'monospace',
                                                    color: isYou
                                                        ? Colors.amberAccent
                                                        : null,
                                                  ),
                                                  overflow:
                                                      TextOverflow.ellipsis,
                                                ),
                                              ),
                                              if (isYou)
                                                Container(
                                                  margin: const EdgeInsets.only(
                                                      left: 6),
                                                  padding: const EdgeInsets
                                                      .symmetric(
                                                      horizontal: 6,
                                                      vertical: 2),
                                                  decoration: BoxDecoration(
                                                    color: Colors.amberAccent
                                                        .withValues(alpha: 0.2),
                                                    borderRadius:
                                                        BorderRadius.circular(
                                                            8),
                                                    border: Border.all(
                                                        color:
                                                            Colors.amberAccent,
                                                        width: 1),
                                                  ),
                                                  child: const Text(
                                                    'YOU',
                                                    style: TextStyle(
                                                        color:
                                                            Colors.amberAccent,
                                                        fontSize: 9,
                                                        fontWeight:
                                                            FontWeight.bold),
                                                  ),
                                                ),
                                            ],
                                          ),
                                          subtitle: Text(
                                            'Stake: ${v.stakeDisplay} LOS',
                                          ),
                                          trailing: Text(
                                            v.isActive ? 'ACTIVE' : 'INACTIVE',
                                            style: TextStyle(
                                              color: v.isActive
                                                  ? Colors.green
                                                  : Colors.grey,
                                              fontWeight: FontWeight.bold,
                                            ),
                                          ),
                                        );
                                      },
                                    ),
                                  ],
                                ),
                              ),
                            ),

                            const SizedBox(height: 16),

                            // Recent Blocks Card
                            Card(
                              child: Padding(
                                padding: const EdgeInsets.all(16.0),
                                child: Column(
                                  crossAxisAlignment: CrossAxisAlignment.start,
                                  children: [
                                    const Row(
                                      children: [
                                        Icon(Icons.view_module, size: 24),
                                        SizedBox(width: 8),
                                        Text(
                                          'Recent Blocks',
                                          style: TextStyle(
                                            fontSize: 18,
                                            fontWeight: FontWeight.bold,
                                          ),
                                        ),
                                      ],
                                    ),
                                    const Divider(),
                                    ..._recentBlocks.map(
                                      (block) => ListTile(
                                        leading: CircleAvatar(
                                          child: Text('${block.height}'),
                                        ),
                                        title: Text(
                                          block.hash.length >= 16
                                              ? '${block.hash.substring(0, 16)}...'
                                              : block.hash,
                                          style: const TextStyle(
                                            fontFamily: 'monospace',
                                            fontSize: 12,
                                          ),
                                        ),
                                        subtitle: Text(
                                          DateFormat('MMM dd, yyyy HH:mm:ss')
                                              .format(
                                            DateTime.fromMillisecondsSinceEpoch(
                                              block.timestamp * 1000,
                                            ),
                                          ),
                                        ),
                                        trailing: Text('${block.txCount} TXs'),
                                      ),
                                    ),
                                  ],
                                ),
                              ),
                            ),

                            const SizedBox(height: 16),

                            // Peers Card
                            Card(
                              child: Padding(
                                padding: const EdgeInsets.all(16.0),
                                child: Column(
                                  crossAxisAlignment: CrossAxisAlignment.start,
                                  children: [
                                    Row(
                                      children: [
                                        const Icon(
                                          Icons.connect_without_contact,
                                          size: 24,
                                        ),
                                        const SizedBox(width: 8),
                                        Text(
                                          'Connected Peers (${_peers.length})',
                                          style: const TextStyle(
                                            fontSize: 18,
                                            fontWeight: FontWeight.bold,
                                          ),
                                        ),
                                      ],
                                    ),
                                    const Divider(),
                                    ..._peers.map(
                                      (peer) {
                                        final isPeerYou = _myAddress != null &&
                                            peer == _myAddress;
                                        return ListTile(
                                          leading: Icon(
                                            isPeerYou
                                                ? Icons.star
                                                : Icons.router,
                                            size: 20,
                                            color:
                                                isPeerYou ? Colors.amber : null,
                                          ),
                                          title: Text(
                                            peer,
                                            style: TextStyle(
                                              fontSize: 12,
                                              fontFamily: 'monospace',
                                              color: isPeerYou
                                                  ? Colors.amber
                                                  : null,
                                            ),
                                          ),
                                          trailing: isPeerYou
                                              ? Container(
                                                  padding: const EdgeInsets
                                                      .symmetric(
                                                    horizontal: 8,
                                                    vertical: 2,
                                                  ),
                                                  decoration: BoxDecoration(
                                                    color: Colors.amber,
                                                    borderRadius:
                                                        BorderRadius.circular(
                                                            12),
                                                  ),
                                                  child: const Text(
                                                    'YOU',
                                                    style: TextStyle(
                                                      color: Colors.black,
                                                      fontSize: 10,
                                                      fontWeight:
                                                          FontWeight.bold,
                                                    ),
                                                  ),
                                                )
                                              : null,
                                        );
                                      },
                                    ),
                                  ],
                                ),
                              ),
                            ),

                            // Voting Power Card (if validators available)
                            if (_validators.isNotEmpty) ...[
                              const SizedBox(height: 16),
                              Builder(builder: (ctx) {
                                final myValidator = _myAddress != null
                                    ? _validators
                                        .cast<ValidatorInfo?>()
                                        .firstWhere(
                                          (v) => v!.address == _myAddress,
                                          orElse: () => null,
                                        )
                                    : null;
                                return VotingPowerCard(
                                  validatorInfo:
                                      myValidator ?? _validators.first,
                                  allValidators: _validators,
                                );
                              }),
                            ],

                            // Uptime Card (if validators available)
                            if (_validators.isNotEmpty) ...[
                              const SizedBox(height: 16),
                              Builder(builder: (ctx) {
                                final myValidator = _myAddress != null
                                    ? _validators
                                        .cast<ValidatorInfo?>()
                                        .firstWhere(
                                          (v) => v!.address == _myAddress,
                                          orElse: () => null,
                                        )
                                    : null;
                                return UptimeCard(
                                    validatorInfo:
                                        myValidator ?? _validators.first);
                              }),
                            ],

                            // Network Tokens & DEX Overview
                            const SizedBox(height: 16),
                            const NetworkTokensCard(),

                            // ──── Monitoring Section ────────────────────

                            // Slashing Status
                            if (_slashingInfo != null &&
                                _slashingInfo!.isNotEmpty) ...[
                              const SizedBox(height: 16),
                              _buildSlashingCard(),
                            ],

                            // Consensus State
                            if (_consensusInfo != null &&
                                _consensusInfo!.isNotEmpty) ...[
                              const SizedBox(height: 16),
                              _buildConsensusCard(),
                            ],

                            // Supply Info
                            if (_supplyInfo != null &&
                                _supplyInfo!.isNotEmpty) ...[
                              const SizedBox(height: 16),
                              _buildSupplyCard(),
                            ],

                            // Sync Status — always show when node info available
                            if (_nodeInfo != null) ...[
                              const SizedBox(height: 16),
                              _buildSyncCard(),
                            ],
                          ],
                        ),
                      ),
          ),
        ],
      ),
    );
  }

  Widget _buildMiningInfoCard(NodeProcessService node) {
    final m = _miningInfo!;
    final epoch = (m['epoch'] as num?)?.toInt() ?? 0;
    final difficultyBits = (m['difficulty_bits'] as num?)?.toInt() ?? 0;
    final rewardLos = (m['reward_per_epoch_los'] as num?)?.toInt() ?? 0;
    final minersThisEpoch = (m['miners_this_epoch'] as num?)?.toInt() ?? 0;
    final epochRemainingSecs =
        (m['epoch_remaining_secs'] as num?)?.toInt() ?? 0;
    final remainingSupplyLos =
        (m['remaining_supply_los'] as num?)?.toInt() ?? 0;

    // Parse remaining_supply_cil (sent as string by Rust due to u128 size)
    final remainingSupplyCil =
        int.tryParse(m['remaining_supply_cil']?.toString() ?? '0') ?? 0;
    final remainingDisplay = remainingSupplyCil > 0
        ? BlockchainConstants.cilToLosString(remainingSupplyCil)
        : '$remainingSupplyLos';

    final chainId = m['chain_id']?.toString() ?? 'unknown';
    final isMining = node.enableMining;

    return Card(
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(12),
        side: BorderSide(
          color: isMining ? Colors.orange.withValues(alpha: 0.5) : Colors.grey,
          width: 1,
        ),
      ),
      child: Padding(
        padding: const EdgeInsets.all(16.0),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Icon(
                  Icons.hardware,
                  size: 24,
                  color: isMining ? Colors.orange : Colors.grey,
                ),
                const SizedBox(width: 8),
                const Text(
                  'PoW Mining',
                  style: TextStyle(fontSize: 18, fontWeight: FontWeight.bold),
                ),
                const Spacer(),
                Container(
                  padding:
                      const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
                  decoration: BoxDecoration(
                    color: isMining
                        ? Colors.orange.withValues(alpha: 0.15)
                        : Colors.grey.withValues(alpha: 0.1),
                    borderRadius: BorderRadius.circular(12),
                    border: Border.all(
                        color: isMining ? Colors.orange : Colors.grey,
                        width: 1),
                  ),
                  child: Text(
                    isMining ? '⛏ MINING' : 'INACTIVE',
                    style: TextStyle(
                      color: isMining ? Colors.orange : Colors.grey,
                      fontSize: 11,
                      fontWeight: FontWeight.bold,
                    ),
                  ),
                ),
              ],
            ),
            const Divider(),
            _buildInfoRow('Epoch', '#$epoch'),
            _buildInfoRow(
                'Next Reward In', _formatCountdown(epochRemainingSecs)),
            _buildInfoRow('Difficulty', '$difficultyBits leading zero bits'),
            _buildInfoRow('Reward/Epoch', '$rewardLos LOS'),
            _buildInfoRow('Miners This Epoch', '$minersThisEpoch'),
            _buildInfoRow('Mining Threads', '${node.miningThreads}'),
            _buildInfoRow('Chain', chainId),
            _buildInfoRow('Remaining Supply', '$remainingDisplay LOS'),
          ],
        ),
      ),
    );
  }

  Widget _buildRewardCountdownCard() {
    final epoch = _rewardInfo?['epoch'] as Map<String, dynamic>? ?? {};
    final pool = _rewardInfo?['pool'] as Map<String, dynamic>? ?? {};
    final validatorsInfo =
        _rewardInfo?['validators'] as Map<String, dynamic>? ?? {};
    final currentEpoch = (epoch['current_epoch'] as num?)?.toInt() ?? 0;
    final epochDuration = (epoch['epoch_duration_secs'] as num?)?.toInt() ?? 0;
    // Use _cil fields for exact display (the _los fields are JSON strings with f64 drift)
    final rewardRateCil = _safeInt(epoch['epoch_reward_rate_cil']);
    final rewardRateDisplay = rewardRateCil > 0
        ? BlockchainConstants.cilToLosString(rewardRateCil)
        : epoch['epoch_reward_rate_los']?.toString() ?? '0';
    final eligibleCount = (validatorsInfo['eligible'] as num?)?.toInt() ?? 0;
    final remainingCil = _safeInt(pool['remaining_cil']);
    final remainingDisplay = remainingCil > 0
        ? BlockchainConstants.cilToLosString(remainingCil)
        : pool['remaining_los']?.toString() ?? '0';

    // Check node running state
    final nodeService = context.read<NodeProcessService>();
    final isNodeRunning = nodeService.isRunning;

    // Calculate progress (0.0 to 1.0)
    final elapsed =
        epochDuration > 0 ? (epochDuration - _epochRemainingSecs) : 0;
    final progress =
        epochDuration > 0 ? (elapsed / epochDuration).clamp(0.0, 1.0) : 0.0;

    // Check if my validator is eligible
    String myRewardStatus = 'Not registered';
    bool myEligible = false;
    if (_myAddress != null && _rewardInfo?['validators']?['details'] != null) {
      final details = _rewardInfo!['validators']['details'] as List<dynamic>;
      for (final v in details) {
        if (v['address'] == _myAddress) {
          if (v['eligible'] == true) {
            myRewardStatus = 'Eligible ✓';
            myEligible = true;
          } else {
            final uptime = (v['uptime_pct'] as num?)?.toInt() ?? 0;
            if (uptime < 95) {
              myRewardStatus = 'Low uptime ($uptime%)';
            } else {
              myRewardStatus = 'Probation period';
            }
          }
          break;
        }
      }
    }

    // Grey out countdown when user is NOT eligible for rewards
    final isEligibleForReward = myEligible && eligibleCount > 0;
    // Active color only when eligible; otherwise grey to avoid false hope
    final timerColor = !isNodeRunning || !isEligibleForReward
        ? Colors.grey
        : _epochRemainingSecs <= 30
            ? Colors.green
            : _epochRemainingSecs <= 60
                ? Colors.orange
                : Colors.white;
    final barColor = !isNodeRunning || !isEligibleForReward
        ? Colors.grey
        : _epochRemainingSecs <= 30
            ? Colors.green
            : Colors.blue;

    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16.0),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Icon(
                  Icons.timer,
                  size: 24,
                  color: barColor,
                ),
                const SizedBox(width: 8),
                const Text(
                  'Reward Countdown',
                  style: TextStyle(
                    fontSize: 18,
                    fontWeight: FontWeight.bold,
                  ),
                ),
                const Spacer(),
                Container(
                  padding:
                      const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
                  decoration: BoxDecoration(
                    color: Colors.blue.withValues(alpha: 0.1),
                    borderRadius: BorderRadius.circular(12),
                    border: Border.all(color: Colors.blue, width: 1),
                  ),
                  child: Text(
                    'Epoch $currentEpoch',
                    style: const TextStyle(
                      color: Colors.blue,
                      fontSize: 11,
                      fontWeight: FontWeight.bold,
                    ),
                  ),
                ),
              ],
            ),
            const Divider(),

            // Big countdown timer — greyed out when not eligible
            Center(
              child: Padding(
                padding: const EdgeInsets.symmetric(vertical: 12),
                child: !isNodeRunning
                    ? const Column(
                        children: [
                          Icon(Icons.pause_circle_outline,
                              size: 36, color: Colors.grey),
                          SizedBox(height: 4),
                          Text(
                            'PAUSED',
                            style: TextStyle(
                              fontSize: 18,
                              fontWeight: FontWeight.bold,
                              color: Colors.grey,
                              letterSpacing: 2,
                            ),
                          ),
                          Text(
                            'Start node to resume countdown',
                            style: TextStyle(color: Colors.grey, fontSize: 11),
                          ),
                        ],
                      )
                    : Column(
                        children: [
                          Text(
                            _formatCountdown(_epochRemainingSecs),
                            style: TextStyle(
                              fontSize: 36,
                              fontWeight: FontWeight.bold,
                              fontFamily: 'monospace',
                              color: timerColor,
                            ),
                          ),
                          if (!isEligibleForReward) ...[
                            const SizedBox(height: 4),
                            const Text(
                              'NOT ELIGIBLE — no reward this epoch',
                              style: TextStyle(
                                color: Colors.grey,
                                fontSize: 11,
                                fontWeight: FontWeight.w500,
                              ),
                            ),
                          ],
                        ],
                      ),
              ),
            ),

            // Progress bar — greyed out when not eligible
            ClipRRect(
              borderRadius: BorderRadius.circular(8),
              child: LinearProgressIndicator(
                value: isNodeRunning ? progress : 0.0,
                minHeight: 8,
                backgroundColor: Colors.grey.withValues(alpha: 0.2),
                valueColor: AlwaysStoppedAnimation<Color>(barColor),
              ),
            ),
            const SizedBox(height: 12),

            _buildInfoRow('Epoch Duration', _formatCountdown(epochDuration)),
            _buildInfoRow('Reward/Epoch', '$rewardRateDisplay LOS'),
            _buildInfoRow('Eligible Validators', '$eligibleCount'),
            _buildInfoRow('Pool Remaining', '$remainingDisplay LOS'),
            _buildInfoRow('Your Status', myRewardStatus),

            // Warning: no rewards when 0 eligible validators
            if (eligibleCount == 0 && isNodeRunning) ...[
              const SizedBox(height: 8),
              Container(
                padding: const EdgeInsets.all(8),
                decoration: BoxDecoration(
                  color: Colors.orange.withValues(alpha: 0.1),
                  borderRadius: BorderRadius.circular(8),
                  border: Border.all(
                      color: Colors.orange.withValues(alpha: 0.5), width: 1),
                ),
                child: const Row(
                  children: [
                    Icon(Icons.info_outline, color: Colors.orange, size: 16),
                    SizedBox(width: 8),
                    Expanded(
                      child: Text(
                        'No eligible validators — rewards are NOT being distributed this epoch. '
                        'Requires: min 1,000 LOS stake + ≥95% uptime.',
                        style: TextStyle(color: Colors.orange, fontSize: 11),
                      ),
                    ),
                  ],
                ),
              ),
            ],

            // Hint for not-registered status
            if (myRewardStatus == 'Not registered' && isNodeRunning) ...[
              const SizedBox(height: 6),
              const Text(
                'Register as validator (min 1 LOS). Earn rewards with ≥1,000 LOS + ≥95% uptime.',
                style: TextStyle(color: Colors.grey, fontSize: 11),
              ),
            ],
          ],
        ),
      ),
    );
  }

  // ─── Monitoring Cards ──────────────────────────────────────────

  Widget _buildSlashingCard() {
    final s = _slashingInfo!;
    final slashed = s['slashed'] == true;
    final reason = (s['reason'] ?? s['msg'] ?? 'None').toString();
    final penalty = (s['penalty_cil'] ?? s['penalty'] ?? 0).toString();

    return Card(
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(12),
        side: BorderSide(
          color: slashed
              ? Colors.red.withValues(alpha: 0.7)
              : Colors.green.withValues(alpha: 0.3),
          width: 1,
        ),
      ),
      child: Padding(
        padding: const EdgeInsets.all(16.0),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(children: [
              Icon(Icons.gavel,
                  size: 24, color: slashed ? Colors.red : Colors.green),
              const SizedBox(width: 8),
              const Text('Slashing Status',
                  style: TextStyle(fontSize: 18, fontWeight: FontWeight.bold)),
              const Spacer(),
              Container(
                padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
                decoration: BoxDecoration(
                  color: (slashed ? Colors.red : Colors.green)
                      .withValues(alpha: 0.15),
                  borderRadius: BorderRadius.circular(12),
                  border: Border.all(
                      color: slashed ? Colors.red : Colors.green, width: 1),
                ),
                child: Text(
                  slashed ? 'SLASHED' : 'CLEAN',
                  style: TextStyle(
                    color: slashed ? Colors.red : Colors.green,
                    fontSize: 11,
                    fontWeight: FontWeight.bold,
                  ),
                ),
              ),
            ]),
            const Divider(),
            if (slashed) ...[
              _buildInfoRow('Reason', reason),
              _buildInfoRow('Penalty', '$penalty CIL'),
            ] else
              _buildInfoRow('Status', 'No slashing events'),
          ],
        ),
      ),
    );
  }

  Widget _buildConsensusCard() {
    final c = _consensusInfo!;
    final round = (c['round'] ?? c['current_round'] ?? '-').toString();
    final phase = (c['phase'] ?? c['state'] ?? 'unknown').toString();
    final quorum = (c['quorum'] ?? c['quorum_met'] ?? '-').toString();
    final vc =
        (c['validator_count'] ?? c['active_validators'] ?? '-').toString();

    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16.0),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const Row(children: [
              Icon(Icons.how_to_vote, size: 24, color: Colors.purple),
              SizedBox(width: 8),
              Text('Consensus',
                  style: TextStyle(fontSize: 18, fontWeight: FontWeight.bold)),
            ]),
            const Divider(),
            _buildInfoRow('Round', round),
            _buildInfoRow('Phase', phase),
            _buildInfoRow('Quorum', quorum),
            _buildInfoRow('Active Validators', vc),
          ],
        ),
      ),
    );
  }

  Widget _buildSupplyCard() {
    final s = _supplyInfo!;
    // Parse CIL values for precise display
    final totalCil = int.tryParse(
            (s['total_supply_cil'] ?? s['total_cil'] ?? '0').toString()) ??
        0;
    final circulatingCil = int.tryParse(
            (s['circulating_supply_cil'] ?? s['circulating_cil'] ?? '0')
                .toString()) ??
        0;
    final stakedCil = int.tryParse(
            (s['staked_supply_cil'] ?? s['staked_cil'] ?? '0').toString()) ??
        0;
    final miningPoolCil = int.tryParse(
            (s['mining_pool_cil'] ?? s['remaining_mining_cil'] ?? '0')
                .toString()) ??
        0;

    String fmt(int cil) =>
        cil > 0 ? '${BlockchainConstants.cilToLosString(cil)} LOS' : '-';

    // Fallback to _los string fields if _cil parsing returned 0
    final totalDisplay = totalCil > 0
        ? fmt(totalCil)
        : '${s['total_supply_los'] ?? s['total_los'] ?? '21,936,236'} LOS';
    final circDisplay = circulatingCil > 0
        ? fmt(circulatingCil)
        : '${s['circulating_supply_los'] ?? '-'} LOS';
    final stakeDisplay =
        stakedCil > 0 ? fmt(stakedCil) : '${s['staked_supply_los'] ?? '-'} LOS';
    final miningDisplay = miningPoolCil > 0
        ? fmt(miningPoolCil)
        : '${s['mining_pool_los'] ?? '-'} LOS';

    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16.0),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const Row(children: [
              Icon(Icons.pie_chart, size: 24, color: Colors.teal),
              SizedBox(width: 8),
              Text('Supply Overview',
                  style: TextStyle(fontSize: 18, fontWeight: FontWeight.bold)),
            ]),
            const Divider(),
            _buildInfoRow('Total Supply', totalDisplay),
            _buildInfoRow('Circulating', circDisplay),
            _buildInfoRow('Staked', stakeDisplay),
            _buildInfoRow('Mining Pool', miningDisplay),
          ],
        ),
      ),
    );
  }

  Widget _buildSyncCard() {
    // The /sync endpoint returns peer state-sync data — NOT sync status.
    // Use _nodeInfo (from /node-info) which has the real block_height,
    // and _syncStatus (from /sync) for peer sync state if available.
    final s = _syncStatus ?? {};
    final blockHeight = _nodeInfo?['block_height'] ?? 0;
    final peerSyncStatus = s['status']?.toString() ?? 'unknown';
    // Node is "synced" if it has blocks AND the peer-sync status says up_to_date,
    // OR if block_height > 0 (node has processed blocks from the network).
    final synced =
        blockHeight > 0 && (peerSyncStatus == 'up_to_date' || s.isEmpty);
    final peerCount = _peers.length;

    return Card(
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(12),
        side: BorderSide(
          color: synced
              ? Colors.green.withValues(alpha: 0.3)
              : Colors.blue.withValues(alpha: 0.5),
          width: 1,
        ),
      ),
      child: Padding(
        padding: const EdgeInsets.all(16.0),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(children: [
              Icon(Icons.sync,
                  size: 24, color: synced ? Colors.green : Colors.blue),
              const SizedBox(width: 8),
              const Text('Sync Status',
                  style: TextStyle(fontSize: 18, fontWeight: FontWeight.bold)),
              const Spacer(),
              Container(
                padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
                decoration: BoxDecoration(
                  color: (synced ? Colors.green : Colors.blue)
                      .withValues(alpha: 0.15),
                  borderRadius: BorderRadius.circular(12),
                  border: Border.all(
                      color: synced ? Colors.green : Colors.blue, width: 1),
                ),
                child: Text(
                  synced ? 'SYNCED' : 'SYNCING',
                  style: TextStyle(
                    color: synced ? Colors.green : Colors.blue,
                    fontSize: 11,
                    fontWeight: FontWeight.bold,
                  ),
                ),
              ),
            ]),
            const Divider(),
            _buildInfoRow('Block Height', '$blockHeight'),
            _buildInfoRow('Connected Peers', '$peerCount'),
            if (!synced && blockHeight == 0) ...[
              const SizedBox(height: 8),
              const Text(
                'Waiting for blocks from the network...',
                style: TextStyle(color: Colors.grey, fontSize: 12),
              ),
              const SizedBox(height: 8),
              ClipRRect(
                borderRadius: BorderRadius.circular(8),
                child: const LinearProgressIndicator(
                  value: null, // indeterminate
                  minHeight: 6,
                  backgroundColor: Color(0x33888888),
                  valueColor: AlwaysStoppedAnimation<Color>(Colors.blue),
                ),
              ),
            ],
          ],
        ),
      ),
    );
  }

  /// Safely parse a JSON value (int, double, or string) to Dart int.
  static int _safeInt(dynamic v, [int fallback = 0]) {
    if (v == null) return fallback;
    if (v is int) return v;
    if (v is double) return v.toInt();
    return int.tryParse(v.toString()) ?? fallback;
  }

  /// Format seconds into HH:MM:SS or Dd HH:MM:SS countdown string
  String _formatCountdown(int totalSecs) {
    if (totalSecs <= 0) return '00:00';
    final days = totalSecs ~/ 86400;
    final hours = (totalSecs % 86400) ~/ 3600;
    final mins = (totalSecs % 3600) ~/ 60;
    final secs = totalSecs % 60;
    if (days > 0) {
      return '${days}d ${hours.toString().padLeft(2, '0')}:${mins.toString().padLeft(2, '0')}:${secs.toString().padLeft(2, '0')}';
    }
    if (hours > 0) {
      return '${hours.toString().padLeft(2, '0')}:${mins.toString().padLeft(2, '0')}:${secs.toString().padLeft(2, '0')}';
    }
    return '${mins.toString().padLeft(2, '0')}:${secs.toString().padLeft(2, '0')}';
  }

  Widget _buildInfoRow(String label, String value) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4.0),
      child: Row(
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        children: [
          Text(label, style: const TextStyle(color: Colors.grey)),
          Flexible(
            child: Text(
              value,
              style: const TextStyle(fontWeight: FontWeight.bold),
              textAlign: TextAlign.end,
            ),
          ),
        ],
      ),
    );
  }

  /// Format uptime from Unix epoch timestamp to human-readable duration
  String _formatUptime(dynamic uptimeSeconds) {
    if (uptimeSeconds == null) return 'N/A';
    // Backend sends epoch timestamp, not actual uptime — calculate difference
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final up = uptimeSeconds is int
        ? uptimeSeconds
        : int.tryParse(uptimeSeconds.toString()) ?? 0;
    // If the value looks like an epoch timestamp (> year 2020), calculate real uptime
    final seconds = up > 1577836800 ? (now - up).abs() : up;
    if (seconds < 60) return '${seconds}s';
    if (seconds < 3600) return '${seconds ~/ 60}m ${seconds % 60}s';
    if (seconds < 86400) {
      return '${seconds ~/ 3600}h ${(seconds % 3600) ~/ 60}m';
    }
    return '${seconds ~/ 86400}d ${(seconds % 86400) ~/ 3600}h';
  }
}

/// Network badge widget — shows TESTNET (orange) or MAINNET (green)
/// based on the build-time NETWORK dart-define flag.
class _NetworkBadge extends StatelessWidget {
  @override
  Widget build(BuildContext context) {
    final apiService = context.read<ApiService>();
    final isMainnet = apiService.environment == NetworkEnvironment.mainnet;
    final label = isMainnet ? 'MAINNET' : 'TESTNET';
    final color = isMainnet ? Colors.green : Colors.orange;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
      decoration: BoxDecoration(
        color: color.withValues(alpha: 0.15),
        border: Border.all(color: color, width: 1),
        borderRadius: BorderRadius.circular(12),
      ),
      child: Text(
        label,
        style: TextStyle(
          color: color,
          fontSize: 10,
          fontWeight: FontWeight.bold,
        ),
      ),
    );
  }
}
