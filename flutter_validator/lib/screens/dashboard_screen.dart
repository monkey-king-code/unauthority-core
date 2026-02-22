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
  int _epochRemainingSecs = 0;
  Timer? _countdownTimer;
  Timer? _autoRefreshTimer;
  bool _isDashboardLoading = false; // Prevent concurrent _loadDashboard calls

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

      // FIX C12-05: Parallelize independent API calls (critical over Tor)
      final results = await Future.wait([
        apiService.getNodeInfo(),
        apiService.getHealth(),
        apiService.getValidators(),
        apiService.getRecentBlocks(),
        apiService.getPeers(),
        apiService.getRewardInfo().catchError((_) => <String, dynamic>{}),
      ]);

      if (!mounted) return;
      final rewardData = results[5] as Map<String, dynamic>;
      setState(() {
        _nodeInfo = results[0] as Map<String, dynamic>;
        _health = results[1] as Map<String, dynamic>;
        _validators = results[2] as List<ValidatorInfo>;
        _recentBlocks = results[3] as List<BlockInfo>;
        _peers = results[4] as List<String>;
        _rewardInfo = rewardData.isNotEmpty ? rewardData : null;
        if (_rewardInfo != null && _rewardInfo!['epoch'] != null) {
          final remaining =
              (_rewardInfo!['epoch']['epoch_remaining_secs'] as num?)
                      ?.toInt() ??
                  0;
          // Only update countdown if API returned a positive value.
          // If the API returns 0 (epoch boundary), keep the current countdown
          // so the anti-stall timer will re-fetch in a few seconds when the
          // new epoch has started and remaining_secs is positive again.
          if (remaining > 0) {
            _epochRemainingSecs = remaining;
            _zeroTickCount = 0; // Reset stall counter on good data
          }
        }
        _error = null;
        _isLoading = false;
        _isFirstLoad = false;
      });
      losLog(
          '📊 [DashboardScreen._loadDashboard] Success: validators=${_validators.length}, block_height=${_nodeInfo?['block_height']}, peers=${_peers.length}');
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
                          ],
                        ),
                      ),
          ),
        ],
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
    if (_myAddress != null && _rewardInfo?['validators']?['details'] != null) {
      final details = _rewardInfo!['validators']['details'] as List<dynamic>;
      for (final v in details) {
        if (v['address'] == _myAddress) {
          if (v['eligible'] == true) {
            myRewardStatus = 'Eligible ✓';
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
                  color: _epochRemainingSecs <= 30 ? Colors.green : Colors.blue,
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

            // Big countdown timer
            Center(
              child: Padding(
                padding: const EdgeInsets.symmetric(vertical: 12),
                child: isNodeRunning
                    ? Text(
                        _formatCountdown(_epochRemainingSecs),
                        style: TextStyle(
                          fontSize: 36,
                          fontWeight: FontWeight.bold,
                          fontFamily: 'monospace',
                          color: _epochRemainingSecs <= 30
                              ? Colors.green
                              : _epochRemainingSecs <= 60
                                  ? Colors.orange
                                  : Colors.white,
                        ),
                      )
                    : const Column(
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
                      ),
              ),
            ),

            // Progress bar
            ClipRRect(
              borderRadius: BorderRadius.circular(8),
              child: LinearProgressIndicator(
                value: isNodeRunning ? progress : 0.0,
                minHeight: 8,
                backgroundColor: Colors.grey.withValues(alpha: 0.2),
                valueColor: AlwaysStoppedAnimation<Color>(
                  !isNodeRunning
                      ? Colors.grey
                      : _epochRemainingSecs <= 30
                          ? Colors.green
                          : Colors.blue,
                ),
              ),
            ),
            const SizedBox(height: 12),

            _buildInfoRow('Epoch Duration', _formatCountdown(epochDuration)),
            _buildInfoRow('Reward/Epoch', '$rewardRateDisplay LOS'),
            _buildInfoRow('Eligible Validators', '$eligibleCount'),
            _buildInfoRow('Pool Remaining', '$remainingDisplay LOS'),
            _buildInfoRow('Your Status', myRewardStatus),
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
