import '../utils/log.dart';
import '../constants/colors.dart';
import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:provider/provider.dart';
import '../services/wallet_service.dart';
import '../services/api_service.dart';
import '../services/network_config.dart';
import '../services/network_status_service.dart';
import '../services/node_process_service.dart';
import '../services/tor_service.dart';
import '../constants/blockchain.dart';
import '../widgets/network_status_bar.dart';
import '../main.dart';
import 'dashboard_screen.dart';

/// Main screen after wallet registration.
/// Three tabs: Node (process controls), Dashboard (network monitoring), Settings.
class NodeControlScreen extends StatefulWidget {
  const NodeControlScreen({super.key});

  @override
  State<NodeControlScreen> createState() => _NodeControlScreenState();
}

class _NodeControlScreenState extends State<NodeControlScreen>
    with SingleTickerProviderStateMixin {
  late TabController _tabController;
  String? _walletAddress;
  int? _balanceCil; // Balance in CIL (smallest unit) — integer precision
  bool _showLogs = false;
  bool _isMonitorMode = false; // Genesis bootstrap validator → dashboard only
  bool _isStartingNode = false; // Debounce: prevent double-click race

  @override
  void initState() {
    super.initState();
    _tabController = TabController(length: 3, vsync: this);
    _loadWalletInfo();
  }

  Future<void> _loadWalletInfo() async {
    losLog('🖥️ [NodeControlScreen._loadWalletInfo] Loading wallet info...');
    final walletService = context.read<WalletService>();
    final wallet = await walletService.getCurrentWallet();
    final monitorMode = await walletService.isMonitorMode();
    if (wallet != null && mounted) {
      setState(() {
        _walletAddress = wallet['address'];
        _isMonitorMode = monitorMode;
      });
      losLog(
          '🖥️ [NodeControlScreen._loadWalletInfo] Address: ${wallet['address']}, monitorMode: $monitorMode');
      _refreshBalance();
      // Auto-register as validator on the bootstrap node if not already registered.
      // This ensures the network knows about this validator even if the setup wizard
      // registration was skipped (e.g. node was already running, or Tor was down).
      _ensureValidatorRegistered(walletService, wallet);
    }
  }

  /// Register this wallet as a validator on the bootstrap node if not yet registered.
  /// Uses the same Dilithium5 signed proof of ownership — fully mainnet-ready.
  Future<void> _ensureValidatorRegistered(
      WalletService walletService, Map<String, String> wallet) async {
    // Capture context-dependent services before any async gap
    final apiService = context.read<ApiService>();
    final nodeService = context.read<NodeProcessService>();
    final torService = context.read<TorService>();
    try {
      final isAddressOnly = await walletService.isAddressOnlyImport();
      if (isAddressOnly) return; // Can't sign without keys

      final address = wallet['address'];
      final publicKey = wallet['public_key'];
      if (address == null || publicKey == null) return;

      // Check if already registered on bootstrap node
      final validators = await apiService.getValidators();
      final alreadyRegistered = validators.any((v) => v.address == address);
      if (alreadyRegistered) {
        losLog('✅ Validator already registered on bootstrap node');
        return;
      }

      // Not registered — sign and register
      final timestamp = DateTime.now().millisecondsSinceEpoch ~/ 1000;
      final message = 'REGISTER_VALIDATOR:$address:$timestamp';
      final signature = await walletService.signTransaction(message);

      // Include our .onion address so peers know how to reach us
      final myOnion = torService.onionAddress;

      // Register on bootstrap node (shared ApiService points to .onion)
      await apiService.ensureReady();
      final result = await apiService.registerValidator(
        address: address,
        publicKey: publicKey,
        signature: signature,
        timestamp: timestamp,
        onionAddress: myOnion,
      );
      losLog('✅ Auto-registered on bootstrap: ${result['msg']}');

      // Also register on local node if running
      if (nodeService.isRunning) {
        final localApi = ApiService(
          customUrl: 'http://127.0.0.1:${nodeService.apiPort}',
        );
        await localApi.ensureReady();
        try {
          await localApi.registerValidator(
            address: address,
            publicKey: publicKey,
            signature: signature,
            timestamp: timestamp,
            onionAddress: myOnion,
          );
          losLog('✅ Auto-registered on local node');
        } catch (e) {
          losLog('⚠️ Local registration: $e');
        } finally {
          localApi.dispose();
        }
      }
    } catch (e) {
      losLog('⚠️ Auto-registration deferred: $e');
    }
  }

  Future<void> _refreshBalance() async {
    losLog('💰 [NodeControlScreen._refreshBalance] Refreshing balance...');
    if (_walletAddress == null) return;
    try {
      final apiService = context.read<ApiService>();
      final account = await apiService.getBalance(_walletAddress!);
      if (mounted) setState(() => _balanceCil = account.balance);
      losLog(
          '💰 [NodeControlScreen._refreshBalance] Balance: ${account.balance} CIL');
    } catch (e) {
      losLog('Balance refresh error: $e');
    }
  }

  @override
  void dispose() {
    _tabController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Row(children: [
          Icon(Icons.verified_user, size: 24),
          SizedBox(width: 8),
          Text('LOS Validator'),
        ]),
        actions: [
          if (_isMonitorMode)
            Container(
              margin: const EdgeInsets.only(right: 4),
              padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
              decoration: BoxDecoration(
                  color: ValidatorColors.accent.withValues(alpha: 0.2),
                  borderRadius: BorderRadius.circular(12),
                  border: Border.all(
                      color: ValidatorColors.accent.withValues(alpha: 0.5))),
              child: const Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Icon(Icons.monitor_heart,
                      size: 12, color: ValidatorColors.accent),
                  SizedBox(width: 4),
                  Text('MONITOR',
                      style: TextStyle(
                          fontSize: 10,
                          fontWeight: FontWeight.bold,
                          color: ValidatorColors.accent)),
                ],
              ),
            ),
          _NetworkBadge(),
        ],
        bottom: TabBar(
          controller: _tabController,
          tabs: const [
            Tab(icon: Icon(Icons.dns), text: 'Node'),
            Tab(icon: Icon(Icons.dashboard), text: 'Dashboard'),
            Tab(icon: Icon(Icons.settings), text: 'Settings'),
          ],
        ),
      ),
      body: Column(children: [
        const NetworkStatusBar(),
        Expanded(
            child: TabBarView(
          controller: _tabController,
          children: [
            _buildNodeTab(),
            const DashboardScreen(),
            _buildSettingsTab(),
          ],
        )),
      ]),
    );
  }

  // ================================================================
  // TAB 1: NODE CONTROL
  // ================================================================

  Widget _buildNodeTab() {
    // Monitor mode: no local node process — show info card instead of controls
    if (_isMonitorMode) {
      return _buildMonitorModeTab();
    }
    return Consumer<NodeProcessService>(
      builder: (context, node, _) {
        return SingleChildScrollView(
          padding: const EdgeInsets.all(16),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              _buildNodeStatusCard(node),
              const SizedBox(height: 16),
              _buildNodeInfoCard(node),
              const SizedBox(height: 16),
              _buildControlButtons(node),
              const SizedBox(height: 16),
              _buildLogSection(node),
            ],
          ),
        );
      },
    );
  }

  Widget _buildMonitorModeTab() {
    return SingleChildScrollView(
      padding: const EdgeInsets.all(16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          // Status card — monitor mode
          Card(
            child: Padding(
              padding: const EdgeInsets.all(20),
              child: Column(children: [
                const Icon(Icons.monitor_heart,
                    size: 56, color: ValidatorColors.accent),
                const SizedBox(height: 12),
                const Text('Monitor Mode',
                    style: TextStyle(
                        fontSize: 24,
                        fontWeight: FontWeight.bold,
                        color: ValidatorColors.accent)),
                const SizedBox(height: 4),
                Text('Viewing genesis bootstrap validator dashboard',
                    style: TextStyle(color: Colors.grey[400], fontSize: 12)),
              ]),
            ),
          ),
          const SizedBox(height: 16),
          // Info card
          Card(
            child: Padding(
              padding: const EdgeInsets.all(16),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const Row(children: [
                    Icon(Icons.shield, color: Colors.amber),
                    SizedBox(width: 8),
                    Text('Genesis Bootstrap Node',
                        style: TextStyle(
                            fontSize: 16, fontWeight: FontWeight.bold)),
                  ]),
                  const Divider(),
                  _infoRow('Mode', 'Monitor Only (Read-Only)'),
                  _infoRow(
                      'Address',
                      _walletAddress != null
                          ? _shortAddr(_walletAddress!)
                          : 'Loading...'),
                  _infoRow(
                      'Balance',
                      _balanceCil != null
                          ? '${BlockchainConstants.cilToLosString(_balanceCil!)} LOS'
                          : 'Loading...'),
                  // Show connected bootstrap node's .onion host
                  Builder(builder: (ctx) {
                    final url = context.read<ApiService>().baseUrl;
                    final uri = Uri.tryParse(url);
                    final host = uri?.host ?? url;
                    if (host.endsWith('.onion')) {
                      return _infoTapRow('Onion Host', _shortOnion(host), host);
                    }
                    return _infoRow('Host', host);
                  }),
                  _infoRow('Local Node', 'Managed by CLI (not this app)'),
                ],
              ),
            ),
          ),
          const SizedBox(height: 16),
          // Explanation card
          Container(
            padding: const EdgeInsets.all(16),
            decoration: BoxDecoration(
                color: Colors.amber.withValues(alpha: 0.1),
                borderRadius: BorderRadius.circular(8),
                border: Border.all(color: Colors.amber.withValues(alpha: 0.3))),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                const Row(children: [
                  Icon(Icons.info_outline, color: Colors.amber, size: 20),
                  SizedBox(width: 8),
                  Text('Why Monitor Mode?',
                      style: TextStyle(
                          fontWeight: FontWeight.bold,
                          color: Colors.amber,
                          fontSize: 14)),
                ]),
                const SizedBox(height: 8),
                Text(
                    'Your wallet belongs to a genesis bootstrap validator that is already '
                    'running as a CLI node.\n\n'
                    'To prevent equivocation (double-signing), this app will NOT:\n'
                    '• Spawn a new los-node process\n'
                    '• Create a new Tor hidden service\n'
                    '• Restart or interfere with the running bootstrap node\n\n'
                    'You can safely view the Dashboard tab to monitor network health, '
                    'validators, blocks, and peers.',
                    style: TextStyle(fontSize: 13, color: Colors.grey[300])),
              ],
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildNodeStatusCard(NodeProcessService node) {
    final (statusColor, statusIcon, statusText) = switch (node.status) {
      NodeStatus.stopped => (Colors.grey, Icons.stop_circle, 'Stopped'),
      NodeStatus.starting => (Colors.amber, Icons.hourglass_top, 'Starting...'),
      NodeStatus.syncing => (Colors.blue, Icons.sync, 'Syncing'),
      NodeStatus.running => (Colors.green, Icons.check_circle, 'Running'),
      NodeStatus.stopping => (Colors.orange, Icons.pause_circle, 'Stopping...'),
      NodeStatus.error => (Colors.red, Icons.error, 'Error'),
    };

    return Card(
      child: Padding(
        padding: const EdgeInsets.all(20),
        child: Column(children: [
          Icon(statusIcon, size: 56, color: statusColor),
          const SizedBox(height: 12),
          Text(statusText,
              style: TextStyle(
                  fontSize: 24,
                  fontWeight: FontWeight.bold,
                  color: statusColor)),
          const SizedBox(height: 4),
          if (node.status == NodeStatus.running)
            Text('Port ${node.apiPort} | PID active',
                style: TextStyle(color: Colors.grey[400], fontSize: 12)),
          if (node.status == NodeStatus.error && node.errorMessage != null)
            Padding(
                padding: const EdgeInsets.only(top: 8),
                child: Container(
                    padding: const EdgeInsets.all(8),
                    decoration: BoxDecoration(
                        color: Colors.red.withValues(alpha: 0.1),
                        borderRadius: BorderRadius.circular(8)),
                    child: Text(node.errorMessage!,
                        style: const TextStyle(color: Colors.red, fontSize: 12),
                        textAlign: TextAlign.center))),
        ]),
      ),
    );
  }

  Widget _buildNodeInfoCard(NodeProcessService node) {
    final torService = context.read<TorService>();
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const Row(children: [
              Icon(Icons.info_outline, color: ValidatorColors.accent),
              SizedBox(width: 8),
              Text('Node Details',
                  style: TextStyle(fontSize: 16, fontWeight: FontWeight.bold)),
            ]),
            const Divider(),
            _infoRow('Status', node.status.name.toUpperCase()),
            _infoRow('API Port', node.apiPort.toString()),
            _infoRow('Local API', node.localApiUrl),
            if (node.nodeAddress != null)
              _infoTapRow('Node Address', _shortAddr(node.nodeAddress!),
                  node.nodeAddress!),
            if (node.onionAddress != null || torService.onionAddress != null)
              _infoTapRow(
                '.onion Address',
                _shortOnion(node.onionAddress ?? torService.onionAddress!),
                node.onionAddress ?? torService.onionAddress!,
              ),
            if (node.dataDir != null) _infoRow('Data Dir', node.dataDir!),
          ],
        ),
      ),
    );
  }

  Widget _buildControlButtons(NodeProcessService node) {
    final isRunning = node.isRunning;
    final isStopped = node.isStopped;
    final canStart = isStopped && !_isStartingNode; // Debounce double-click

    return Row(children: [
      Expanded(
        flex: 2,
        child: ElevatedButton.icon(
          onPressed: canStart
              ? () => _startNode(node)
              : (isRunning ? () => _stopNode(node) : null),
          icon: Icon(canStart ? Icons.play_arrow : Icons.stop),
          label: Text(
              _isStartingNode
                  ? 'STARTING...'
                  : (isStopped ? 'START NODE' : 'STOP NODE'),
              style: const TextStyle(fontWeight: FontWeight.bold)),
          style: ElevatedButton.styleFrom(
              backgroundColor: isStopped ? Colors.green : Colors.red,
              foregroundColor: Colors.white,
              padding: const EdgeInsets.symmetric(vertical: 16),
              shape: RoundedRectangleBorder(
                  borderRadius: BorderRadius.circular(12))),
        ),
      ),
      const SizedBox(width: 12),
      Expanded(
        child: ElevatedButton.icon(
          onPressed: isRunning ? () => _restartNode(node) : null,
          icon: const Icon(Icons.refresh),
          label: const Text('RESTART'),
          style: ElevatedButton.styleFrom(
              backgroundColor: ValidatorColors.accent,
              foregroundColor: Colors.white,
              padding: const EdgeInsets.symmetric(vertical: 16),
              shape: RoundedRectangleBorder(
                  borderRadius: BorderRadius.circular(12))),
        ),
      ),
    ]);
  }

  Widget _buildLogSection(NodeProcessService node) {
    return Card(
      child: Column(children: [
        ListTile(
          leading: const Icon(Icons.terminal, color: Colors.green),
          title: const Text('Node Logs',
              style: TextStyle(fontWeight: FontWeight.bold)),
          trailing: Row(mainAxisSize: MainAxisSize.min, children: [
            Text('${node.logs.length} lines',
                style: TextStyle(fontSize: 12, color: Colors.grey[400])),
            const SizedBox(width: 8),
            IconButton(
                icon: Icon(_showLogs ? Icons.expand_less : Icons.expand_more),
                onPressed: () => setState(() => _showLogs = !_showLogs)),
          ]),
        ),
        if (_showLogs)
          Container(
            height: 300,
            margin: const EdgeInsets.fromLTRB(16, 0, 16, 16),
            decoration: BoxDecoration(
                color: Colors.black, borderRadius: BorderRadius.circular(8)),
            child: node.logs.isEmpty
                ? const Center(
                    child: Text('No logs yet',
                        style: TextStyle(
                            color: Colors.grey, fontFamily: 'monospace')))
                : ListView.builder(
                    itemCount: node.logs.length,
                    reverse: true,
                    padding: const EdgeInsets.all(8),
                    itemBuilder: (_, i) {
                      final line = node.logs[node.logs.length - 1 - i];
                      Color color = Colors.grey[300]!;
                      if (line.contains('ERR')) {
                        color = Colors.red;
                      } else if (line.contains('ready') ||
                          line.contains('running')) {
                        color = Colors.green;
                      } else if (line.contains('restart') ||
                          line.contains('warning')) {
                        color = Colors.amber;
                      }
                      return Padding(
                          padding: const EdgeInsets.symmetric(vertical: 1),
                          child: Text(line,
                              style: TextStyle(
                                  fontFamily: 'monospace',
                                  fontSize: 11,
                                  color: color)));
                    }),
          ),
      ]),
    );
  }

  Future<void> _startNode(NodeProcessService node) async {
    losLog('🖥️ [NodeControlScreen._startNode] Starting node...');
    if (_isMonitorMode) return; // Monitor mode — CLI manages the node
    if (_isStartingNode) return; // Already starting — prevent double-click

    // DEBOUNCE: Disable button immediately before any async work.
    // Without this, rapid clicks can both enter _startNode() because
    // the async gap before node.start() leaves the button enabled.
    setState(() => _isStartingNode = true);

    try {
      final torService = context.read<TorService>();
      final apiService = context.read<ApiService>();
      final walletService = context.read<WalletService>();
      String? onion = torService.onionAddress;

      if (onion == null && !torService.isRunning) {
        onion = await torService.startWithHiddenService(
          localPort: node.apiPort,
          onionPort: 80,
        );
      }

      // CRITICAL: Exclude own .onion from API failover peer list.
      // Spec: "flutter_validator MUST NOT use its own local onion
      // address for API consumption".
      if (onion != null) {
        apiService.setExcludedOnion('http://$onion');
      }

      // Retrieve mnemonic so los-node can derive the same keypair
      final wallet =
          await walletService.getCurrentWallet(includeMnemonic: true);
      final mnemonic = wallet?['mnemonic'];

      // Build bootstrap nodes for P2P discovery.
      // MAINNET PARITY: ALWAYS use .onion P2P addresses.
      // No localhost/127.0.0.1 — Tor onion routing is mandatory.
      const networkMode =
          String.fromEnvironment('NETWORK', defaultValue: 'mainnet');
      final activeNodes = networkMode == 'mainnet'
          ? NetworkConfig.mainnetNodes
          : NetworkConfig.testnetNodes;
      String? bootstrapNodes;
      if (activeNodes.isNotEmpty) {
        bootstrapNodes = activeNodes.map((n) => n.p2pAddress).join(',');
        losLog('\ud83c\udf10 Bootstrap nodes (.onion): $bootstrapNodes');
      }

      // P2P port: auto-derived from API port + 1000 (matches los-node dynamic port)
      final p2pPort = node.apiPort + 1000;
      losLog('📡 P2P port: $p2pPort');

      // Tor SOCKS5 proxy: MANDATORY for dialing .onion bootstrap peers.
      // MAINNET PARITY: Without SOCKS5, los-node cannot reach any peer.
      if (!torService.isRunning) {
        const isMainnet =
            String.fromEnvironment('NETWORK', defaultValue: 'mainnet') ==
                'mainnet';
        if (isMainnet) {
          // MAINNET SAFETY (M-6): Refuse to start without Tor
          if (mounted) {
            ScaffoldMessenger.of(context).showSnackBar(
              const SnackBar(
                content:
                    Text('❌ Mainnet requires Tor. Please start Tor first.'),
                backgroundColor: Colors.red,
              ),
            );
          }
          setState(() => _isStartingNode = false);
          return;
        }
        losLog(
            '⚠️ Tor not running — node will start without SOCKS5 (standalone mode)');
      }
      final torSocks5 = torService.isRunning
          ? '127.0.0.1:${torService.activeSocksPort}'
          : null;
      losLog('🧅 Tor SOCKS5: $torSocks5');

      final started = await node.start(
        port: node.apiPort,
        onionAddress: onion,
        seedPhrase: mnemonic,
        bootstrapNodes: bootstrapNodes,
        p2pPort: p2pPort,
        torSocks5: torSocks5,
      );

      // FIX: Enable local node fallback in ApiService so Dashboard works
      // even when Tor SOCKS proxy is unavailable. The local node's REST API
      // at http://127.0.0.1:<port> is always reachable without Tor.
      if (started) {
        apiService.setLocalNodeUrl(node.localApiUrl);
      }

      losLog(
          '🖥️ [NodeControlScreen._startNode] ${started ? 'Success' : 'Failed'}');
    } finally {
      if (mounted) setState(() => _isStartingNode = false);
    }
  }

  Future<void> _stopNode(NodeProcessService node) async {
    losLog('🖥️ [NodeControlScreen._stopNode] Stopping node...');
    // Capture ApiService BEFORE async gap to satisfy use_build_context_synchronously
    final apiService = context.read<ApiService>();
    await node.stop();
    // Clear local fallback — node is gone, localhost won't respond.
    apiService.clearLocalNodeUrl();
    losLog('🖥️ [NodeControlScreen._stopNode] Node stopped');
  }

  Future<void> _restartNode(NodeProcessService node) async {
    losLog('🖥️ [NodeControlScreen._restartNode] Restarting node...');
    await node.restart();
    losLog('🖥️ [NodeControlScreen._restartNode] Node restarted');
  }

  // ================================================================
  // TAB 3: SETTINGS
  // ================================================================

  Widget _buildSettingsTab() {
    return SingleChildScrollView(
      padding: const EdgeInsets.all(16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _buildWalletCard(),
          const SizedBox(height: 16),
          _buildNetworkInfoCard(),
          const SizedBox(height: 16),
          Container(
            padding: const EdgeInsets.all(12),
            decoration: BoxDecoration(
                color: Colors.blue.withValues(alpha: 0.1),
                borderRadius: BorderRadius.circular(8)),
            child: const Row(children: [
              Icon(Icons.info_outlined, color: Colors.blue, size: 20),
              SizedBox(width: 8),
              Expanded(
                  child: Text(
                      'To send, receive, or burn LOS, use the LOS Wallet app. '
                      'This validator controls your node and monitors network health.',
                      style: TextStyle(fontSize: 12, color: Colors.blue))),
            ]),
          ),
          const SizedBox(height: 16),
          OutlinedButton.icon(
              onPressed: _confirmLogout,
              icon: Icon(_isMonitorMode ? Icons.logout : Icons.logout,
                  color: Colors.red),
              label: Text(
                  _isMonitorMode
                      ? 'Disconnect Monitor'
                      : 'Unregister Validator',
                  style: const TextStyle(color: Colors.red)),
              style: OutlinedButton.styleFrom(
                  padding: const EdgeInsets.all(16),
                  side: const BorderSide(color: Colors.red))),
        ],
      ),
    );
  }

  Widget _buildWalletCard() {
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const Row(children: [
              Icon(Icons.account_balance_wallet, color: ValidatorColors.accent),
              SizedBox(width: 8),
              Text('Validator Wallet',
                  style: TextStyle(fontSize: 18, fontWeight: FontWeight.bold)),
            ]),
            const Divider(),
            const SizedBox(height: 8),
            const Text('Address',
                style: TextStyle(fontSize: 12, color: Colors.grey)),
            const SizedBox(height: 4),
            Row(children: [
              Expanded(
                  child: Text(_walletAddress ?? 'Loading...',
                      style: const TextStyle(
                          fontFamily: 'monospace', fontSize: 12))),
              if (_walletAddress != null)
                IconButton(
                    icon: const Icon(Icons.copy, size: 18),
                    onPressed: () {
                      Clipboard.setData(ClipboardData(text: _walletAddress!));
                      ScaffoldMessenger.of(context).showSnackBar(const SnackBar(
                          content: Text('Address copied'),
                          duration: Duration(seconds: 2)));
                    }),
            ]),
            const SizedBox(height: 12),
            const Text('Balance',
                style: TextStyle(fontSize: 12, color: Colors.grey)),
            const SizedBox(height: 4),
            Row(children: [
              Text(
                  _balanceCil != null
                      ? '${BlockchainConstants.cilToLosString(_balanceCil!)} LOS'
                      : 'Loading...',
                  style: const TextStyle(
                      fontSize: 20, fontWeight: FontWeight.bold)),
              const Spacer(),
              IconButton(
                  icon: const Icon(Icons.refresh, size: 20),
                  onPressed: _refreshBalance),
            ]),
            const SizedBox(height: 8),
            if (_balanceCil != null)
              Container(
                  padding:
                      const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
                  decoration: BoxDecoration(
                      // Compare in CIL: 1000 LOS = 1000 * cilPerLos CIL
                      // Integer comparison — no f64 precision loss
                      color:
                          _balanceCil! >= 1000 * BlockchainConstants.cilPerLos
                              ? Colors.green.withValues(alpha: 0.1)
                              : Colors.red.withValues(alpha: 0.1),
                      borderRadius: BorderRadius.circular(8)),
                  child: Row(mainAxisSize: MainAxisSize.min, children: [
                    Icon(
                        _balanceCil! >= 1000 * BlockchainConstants.cilPerLos
                            ? Icons.check_circle
                            : Icons.warning,
                        size: 16,
                        color:
                            _balanceCil! >= 1000 * BlockchainConstants.cilPerLos
                                ? Colors.green
                                : Colors.red),
                    const SizedBox(width: 6),
                    Text(
                        _balanceCil! >= 1000 * BlockchainConstants.cilPerLos
                            ? 'Active Validator (Stake >= 1,000 LOS)'
                            : 'Insufficient Stake',
                        style: TextStyle(
                            fontSize: 12,
                            fontWeight: FontWeight.w600,
                            color: _balanceCil! >=
                                    1000 * BlockchainConstants.cilPerLos
                                ? Colors.green
                                : Colors.red)),
                  ])),
          ],
        ),
      ),
    );
  }

  Widget _buildNetworkInfoCard() {
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const Row(children: [
              Icon(Icons.public, color: Colors.blue),
              SizedBox(width: 8),
              Text('Network',
                  style: TextStyle(fontSize: 18, fontWeight: FontWeight.bold)),
            ]),
            const Divider(),
            Consumer<NetworkStatusService>(
                builder: (context, net, _) => Column(children: [
                      _infoRow('Status',
                          net.isConnected ? 'Connected' : 'Disconnected'),
                      _infoRow('Block Height', '${net.blockHeight}'),
                      _infoRow('Peers', '${net.peerCount}'),
                      _infoRow('Version', net.nodeVersion),
                    ])),
          ],
        ),
      ),
    );
  }

  // ================================================================
  // HELPERS
  // ================================================================

  Widget _infoRow(String label, String value) {
    return Padding(
        padding: const EdgeInsets.symmetric(vertical: 6),
        child:
            Row(mainAxisAlignment: MainAxisAlignment.spaceBetween, children: [
          Text(label, style: TextStyle(color: Colors.grey[400])),
          Flexible(
              child: Text(value,
                  style: const TextStyle(fontWeight: FontWeight.w600),
                  overflow: TextOverflow.ellipsis)),
        ]));
  }

  Widget _infoTapRow(String label, String displayValue, String fullValue) {
    return Padding(
        padding: const EdgeInsets.symmetric(vertical: 6),
        child:
            Row(mainAxisAlignment: MainAxisAlignment.spaceBetween, children: [
          Text(label, style: TextStyle(color: Colors.grey[400])),
          Row(mainAxisSize: MainAxisSize.min, children: [
            Text(displayValue,
                style: const TextStyle(
                    fontWeight: FontWeight.w600,
                    fontFamily: 'monospace',
                    fontSize: 12)),
            const SizedBox(width: 4),
            GestureDetector(
                onTap: () {
                  Clipboard.setData(ClipboardData(text: fullValue));
                  ScaffoldMessenger.of(context).showSnackBar(SnackBar(
                      content: Text('$label copied'),
                      duration: const Duration(seconds: 2)));
                },
                child: const Icon(Icons.copy, size: 14, color: Colors.grey)),
          ]),
        ]));
  }

  String _shortAddr(String addr) {
    if (addr.length <= 20) return addr;
    return '${addr.substring(0, 10)}...${addr.substring(addr.length - 8)}';
  }

  String _shortOnion(String onion) {
    if (onion.length <= 20) return onion;
    return '${onion.substring(0, 12)}...onion';
  }

  void _confirmLogout() {
    losLog('⚙️ [NodeControlScreen._confirmLogout] Showing logout dialog...');
    showDialog(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text(
            _isMonitorMode ? 'Exit Monitor Mode?' : 'Unregister Validator?'),
        content: Text(_isMonitorMode
            ? 'This will disconnect from the bootstrap node dashboard and remove '
                'your wallet from this app.\n\n'
                'The bootstrap CLI node will continue running undisturbed.\n'
                'You can reconnect anytime with the same wallet.'
            : 'This will stop the node and remove your wallet from this app. '
                'Your funds are safe on the blockchain.\n\n'
                'You can re-register anytime with the same seed phrase.'),
        actions: [
          TextButton(
              onPressed: () {
                losLog('⚙️ [NodeControlScreen._confirmLogout] Cancelled');
                Navigator.pop(ctx);
              },
              child: const Text('CANCEL')),
          TextButton(
              onPressed: () async {
                losLog('⚙️ [NodeControlScreen._confirmLogout] Confirmed');
                Navigator.pop(ctx);

                // Capture context-dependent services before async gap
                final walletService = context.read<WalletService>();
                NodeProcessService? nodeRef;
                TorService? torRef;
                if (!_isMonitorMode) {
                  try {
                    nodeRef = context.read<NodeProcessService>();
                  } catch (_) {}
                  try {
                    torRef = context.read<TorService>();
                  } catch (_) {}
                }

                // 1. Delete wallet FIRST (most important)
                try {
                  await walletService.deleteWallet();
                } catch (e) {
                  losLog('⚠️ deleteWallet error: $e');
                }

                // 2. Navigate back to setup wizard IMMEDIATELY
                //    (don't wait for node/tor stop — user sees result instantly)
                if (mounted) {
                  MyApp.resetToSetup(context);
                }

                // 3. Stop node & Tor in background (fire-and-forget)
                //    Skip for monitor mode — CLI node is not managed by this app
                if (!_isMonitorMode) {
                  if (nodeRef != null && nodeRef.isRunning) {
                    unawaited(nodeRef.stop());
                  }
                  if (torRef != null) unawaited(torRef.stop());
                }
              },
              child: Text(_isMonitorMode ? 'DISCONNECT' : 'UNREGISTER',
                  style: const TextStyle(color: Colors.red))),
        ],
      ),
    );
  }
}

/// Network badge widget — reads from ApiService so it updates on runtime switch
class _NetworkBadge extends StatelessWidget {
  @override
  Widget build(BuildContext context) {
    final apiService = context.read<ApiService>();
    final isMainnet = apiService.environment == NetworkEnvironment.mainnet;
    final label = isMainnet ? 'MAINNET' : 'TESTNET';
    final color = isMainnet ? Colors.green : Colors.orange;
    return Container(
      margin: const EdgeInsets.only(right: 8),
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
      decoration: BoxDecoration(
        color: color.withValues(alpha: 0.2),
        borderRadius: BorderRadius.circular(12),
        border: Border.all(color: color.withValues(alpha: 0.5)),
      ),
      child: Text(label,
          style: TextStyle(
              fontSize: 10, fontWeight: FontWeight.bold, color: color)),
    );
  }
}
