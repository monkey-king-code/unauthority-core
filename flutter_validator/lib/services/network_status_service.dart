import '../utils/log.dart';
// Network Status Service - Monitors blockchain connection and sync status.
// Wired to ApiService: when health degrades, proactively triggers failover
// so the next user request goes to a healthy node.
import 'dart:async';
import 'package:flutter/foundation.dart';
import 'api_service.dart';

enum ConnectionStatus {
  connected,
  disconnected,
  connecting,
  error,
}

class NetworkStatusService extends ChangeNotifier {
  final ApiService _apiService;

  ConnectionStatus _status = ConnectionStatus.connecting;
  String _networkType = 'Unknown';
  int _blockHeight = 0;
  int _peerCount = 0;
  String _nodeVersion = '0.0.0';
  DateTime? _lastSyncTime;
  String? _errorMessage;
  bool _hasConnectedOnce = false;
  String _connectedNodeName = '';

  /// Count of consecutive health check failures. Used to trigger proactive failover.
  int _consecutiveHealthFailures = 0;

  /// Threshold: after this many consecutive failures, trigger proactive failover.
  /// High threshold (5) because Tor is unreliable — don't switch on jitter.
  static const int _failoverThreshold = 5;

  Timer? _statusCheckTimer;

  NetworkStatusService(this._apiService) {
    losLog('🔌 [NetworkStatus] Service created, starting status checks...');
    _apiService.onNodeSwitched = (newUrl) {
      _connectedNodeName = _apiService.connectedNodeName;
      notifyListeners();
    };
    _startStatusChecking();
  }

  // Getters
  ConnectionStatus get status => _status;
  String get networkType => _networkType;
  int get blockHeight => _blockHeight;
  int get peerCount => _peerCount;
  String get nodeVersion => _nodeVersion;
  DateTime? get lastSyncTime => _lastSyncTime;
  String? get errorMessage => _errorMessage;
  String get connectedNodeName => _connectedNodeName;

  bool get isConnected => _status == ConnectionStatus.connected;
  bool get isDisconnected => _status == ConnectionStatus.disconnected;
  bool get isConnecting => _status == ConnectionStatus.connecting;
  bool get hasError => _status == ConnectionStatus.error;

  String get statusText {
    switch (_status) {
      case ConnectionStatus.connected:
        return 'Connected';
      case ConnectionStatus.disconnected:
        return 'Disconnected';
      case ConnectionStatus.connecting:
        return 'Connecting...';
      case ConnectionStatus.error:
        return 'Error';
    }
  }

  void _startStatusChecking() {
    _checkNetworkStatus();

    // FIX I-04: 15s was too aggressive for Tor — each health check opens a
    // Tor circuit (2-5s RTT), and combined with ApiService's 2min + 5min polls,
    // 3 overlapping loops saturated SOCKS5.  60s is sufficient for status display.
    _statusCheckTimer = Timer.periodic(
      const Duration(seconds: 60),
      (_) => _checkNetworkStatus(),
    );
  }

  Future<void> _checkNetworkStatus() async {
    final previousStatus = _status;
    try {
      if (!_hasConnectedOnce) {
        _status = ConnectionStatus.connecting;
        notifyListeners();
      }

      losLog('🔌 [NetworkStatus] Checking health...');

      final health = await _apiService.getHealth();
      losLog('🔌 [NetworkStatus] Health response: ${health['status']}');

      if (health['status'] == 'healthy' || health['status'] == 'degraded') {
        _status = ConnectionStatus.connected;
        _hasConnectedOnce = true;
        _errorMessage = null;
        _lastSyncTime = DateTime.now();
        _consecutiveHealthFailures = 0;
        _connectedNodeName = _apiService.connectedNodeName;

        if (health['chain'] != null) {
          _blockHeight = health['chain']['blocks'] ?? 0;
        }

        try {
          final nodeInfo = await _apiService.getNodeInfo();
          _networkType = _extractNetworkType(nodeInfo['chain_id'] ?? 'unknown');
          _nodeVersion = nodeInfo['version'] ?? '0.0.0';
          _peerCount = nodeInfo['peer_count'] ?? 0;
          _blockHeight = nodeInfo['block_height'] ?? _blockHeight;
          losLog('🔌 [NetworkStatus] Connected to $_connectedNodeName: '
              'v$_nodeVersion, height=$_blockHeight, peers=$_peerCount, net=$_networkType');
        } catch (e) {
          losLog('⚠️ [NetworkStatus] Node info failed: $e');
        }
      } else {
        _status = ConnectionStatus.error;
        _errorMessage = 'Node unhealthy';
        _consecutiveHealthFailures++;
        losLog('🔌 [NetworkStatus] Node unhealthy: ${health['status']}');
        _maybeFailover();
      }

      if (_status != previousStatus || !_hasConnectedOnce) {
        notifyListeners();
      }
    } catch (e) {
      _status = ConnectionStatus.disconnected;
      _errorMessage = 'Connection failed';
      _consecutiveHealthFailures++;
      losLog('🔌 [NetworkStatus] Connection failed: $e');

      _maybeFailover();

      if (_status != previousStatus) {
        notifyListeners();
      }
    }
  }

  void _maybeFailover() {
    if (_consecutiveHealthFailures >= _failoverThreshold) {
      losLog('🔌 [NetworkStatus] $_consecutiveHealthFailures consecutive '
          'failures — triggering proactive failover');
      _apiService.onHealthDegraded();
      _consecutiveHealthFailures = 0;
    }
  }

  String _extractNetworkType(String chainId) {
    if (chainId.contains('mainnet')) {
      return 'Mainnet';
    } else if (chainId.contains('testnet')) {
      return 'Testnet';
    } else {
      return 'Unknown';
    }
  }

  Future<void> refresh() async {
    _hasConnectedOnce = false;
    await _checkNetworkStatus();
  }

  @override
  void dispose() {
    _statusCheckTimer?.cancel();
    super.dispose();
  }
}
