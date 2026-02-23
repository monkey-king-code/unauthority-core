import '../utils/log.dart';
import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:http/http.dart' as http;
import 'package:http/io_client.dart';
import 'package:socks5_proxy/socks_client.dart';
import '../models/account.dart';
import '../models/network_tokens.dart';
import '../constants/blockchain.dart';
import 'tor_service.dart';
import 'network_config.dart';
import 'peer_discovery_service.dart';
import 'wallet_service.dart';

enum NetworkEnvironment { testnet, mainnet }

/// Tracks per-node health metrics for latency-based selection and cooldown.
class _NodeHealth {
  /// Last measured round-trip time in milliseconds. null = never probed.
  int? latencyMs;

  /// Timestamp of last successful response from this node.
  DateTime? lastSuccess;

  /// Timestamp of last failure. Used for cooldown logic.
  DateTime? lastFailure;

  /// Consecutive failure count. Reset on success.
  int consecutiveFailures = 0;

  /// Whether this node is currently in cooldown (recently failed).
  bool get isInCooldown {
    if (lastFailure == null || consecutiveFailures == 0) return false;
    // Exponential backoff: 10s × 2^(failures-1), capped at 5 minutes
    final cooldownMs =
        (10000 * (1 << (consecutiveFailures - 1).clamp(0, 5))).clamp(0, 300000);
    return DateTime.now().difference(lastFailure!).inMilliseconds < cooldownMs;
  }

  void recordSuccess(int rttMs) {
    latencyMs = rttMs;
    lastSuccess = DateTime.now();
    consecutiveFailures = 0;
  }

  void recordFailure() {
    lastFailure = DateTime.now();
    consecutiveFailures++;
  }
}

class ApiService {
  // Bootstrap node addresses are loaded from assets/network_config.json
  // via NetworkConfig. NEVER hardcode .onion addresses here.
  // Use: scripts/update_network_config.sh to update addresses.

  /// Default timeout for API calls
  static const Duration _defaultTimeout = Duration(seconds: 30);

  /// Longer timeout for Tor connections (45s — .onion routing can be slow on first circuit)
  static const Duration _torTimeout = Duration(seconds: 45);

  /// Timeout for latency probes (short — just checking reachability)
  static const Duration _probeTimeout = Duration(seconds: 15);

  /// Tor-specific probe timeout (Tor circuits need more time)
  static const Duration _torProbeTimeout = Duration(seconds: 30);

  /// Interval between periodic peer re-discovery runs.
  /// 10min is relaxed: bootstrap nodes rarely change.
  static const Duration _rediscoveryInterval = Duration(minutes: 10);

  /// Interval between current-host health checks.
  /// Only pings the CURRENT host. If 3+ consecutive failures → switch.
  static const Duration _healthCheckInterval = Duration(minutes: 5);

  /// Maximum number of saved peers to load from storage.
  static const int _maxSavedPeers = 200;

  late String baseUrl;
  http.Client _client = http.Client();
  NetworkEnvironment environment;
  final TorService _torService;

  /// All available bootstrap URLs for failover
  List<String> _bootstrapUrls = [];

  /// Index of the currently active bootstrap node
  int _currentNodeIndex = 0;

  /// Per-node health tracking: URL → _NodeHealth
  final Map<String, _NodeHealth> _nodeHealthMap = {};

  /// Track client initialization so callers can await readiness
  late Future<void> _clientReady;

  /// Whether we've completed the initial peer discovery + latency probe
  bool _initialDiscoveryDone = false;

  /// Whether this instance has been disposed (client closed).
  /// Prevents fire-and-forget tasks from using a closed client.
  bool _disposed = false;

  /// Whether the HTTP client can reach .onion addresses (Tor SOCKS5 configured).
  /// When false, failover skips .onion URLs entirely — they'll never work.
  bool _hasTor = false;

  /// Periodic timers for background maintenance
  Timer? _rediscoveryTimer;
  Timer? _healthCheckTimer;

  /// Callback for external health monitor integration (e.g. NetworkStatusService).
  /// When set, this is called whenever a proactive failover occurs.
  void Function(String newBaseUrl)? onNodeSwitched;

  /// The local validator's own .onion address.
  /// When set, this node is EXCLUDED from the peer list to prevent
  /// self-connection (spec: "flutter_validator MUST NOT use its own
  /// local onion address for API consumption").
  String? _excludedOnionUrl;

  // ══════════════════════════════════════════════════════════════════
  //  LOCAL NODE FALLBACK — FIX: Dashboard usable when Tor is down
  // ══════════════════════════════════════════════════════════════════

  /// Local node URL (e.g. http://127.0.0.1:3035) — set when bundled
  /// los-node is running. Used as highest-priority fallback so the
  /// Dashboard works even when Tor SOCKS proxy is unavailable.
  String? _localNodeUrl;

  /// Direct HTTP client (no SOCKS proxy) for localhost requests.
  final http.Client _directClient = http.Client();

  /// Whether the last successful request went through _localNodeUrl
  /// (as fallback) instead of an external .onion peer.
  /// UI can show a warning like "Local data only — external verification pending".
  bool _usingLocalFallback = false;
  bool get isUsingLocalFallback => _usingLocalFallback;

  /// Whether a Tor SOCKS recovery is already in progress.
  bool _torRecoveryInProgress = false;

  /// Build-time flag: --dart-define=NETWORK=testnet to override
  static const _networkMode =
      String.fromEnvironment('NETWORK', defaultValue: 'mainnet');
  static NetworkEnvironment get _defaultEnvironment => _networkMode == 'mainnet'
      ? NetworkEnvironment.mainnet
      : NetworkEnvironment.testnet;

  ApiService({
    String? customUrl,
    NetworkEnvironment? environment,
    TorService? torService,
    String? excludeOwnOnion,
  })  : environment = environment ?? _defaultEnvironment,
        _torService = torService ?? TorService() {
    _excludedOnionUrl = excludeOwnOnion;
    _loadBootstrapUrls(this.environment);
    if (customUrl != null) {
      baseUrl = customUrl;
    } else {
      baseUrl = _bootstrapUrls.isNotEmpty
          ? _bootstrapUrls.first
          : _getBaseUrl(this.environment);
    }
    _clientReady = _initializeClient();

    // FIX B-03: When TorService restarts (e.g. upgrading from SOCKS-only to
    // hidden service), the SOCKS port may change. Re-create our HTTP client
    // so requests don't go to a dead proxy port for 30-120s.
    _torService.onSocksPortChanged = () {
      losLog(
          '🔄 [ApiService] Tor SOCKS port changed — recreating HTTP client...');
      _clientReady = _reinitializeTorClient();
    };

    losLog('🔗 LOS Validator ApiService initialized with baseUrl: $baseUrl '
        '(${_bootstrapUrls.length} bootstrap nodes available)');
  }

  /// Await Tor/HTTP client initialization before first request.
  Future<void> ensureReady() => _clientReady;

  /// Set the excluded onion URL at runtime (e.g., after hidden service is generated).
  /// Removes ALL occurrences from the bootstrap list (may have been added by
  /// _loadSavedPeers before this was known).
  void setExcludedOnion(String onionUrl) {
    _excludedOnionUrl = onionUrl;
    _bootstrapUrls.removeWhere((url) => url == onionUrl);
    if (baseUrl == onionUrl) {
      _switchToNextNode();
    }
    losLog('🔗 Excluded own onion from peer list: $onionUrl');
  }

  /// Set the local node URL when the bundled los-node is running.
  /// This enables a direct (non-SOCKS) fallback path so the Dashboard
  /// works even when Tor is unavailable (dead SOCKS proxy, no internet, etc.)
  ///
  /// This does NOT violate the spec: we still prefer external .onion peers
  /// for cross-verification; local is a fallback only.
  void setLocalNodeUrl(String url) {
    _localNodeUrl = url;
    losLog('🔗 Local node URL set: $url (fallback enabled)');
  }

  /// Clear the local node URL when the bundled los-node stops.
  void clearLocalNodeUrl() {
    _localNodeUrl = null;
    _usingLocalFallback = false;
    losLog('🔗 Local node URL cleared');
  }

  /// Load all bootstrap URLs for the given environment.
  /// Filters out the validator's own .onion address if excluded.
  /// SECURITY FIX M-06: On mainnet, only .onion URLs are permitted.
  void _loadBootstrapUrls(NetworkEnvironment env) {
    final nodes = env == NetworkEnvironment.testnet
        ? NetworkConfig.testnetNodes
        : NetworkConfig.mainnetNodes;
    _bootstrapUrls = nodes
        .map((n) => n.restUrl)
        .where((url) => url != _excludedOnionUrl)
        .where((url) {
      // SECURITY: Mainnet requires .onion-only connections (Tor network)
      if (env == NetworkEnvironment.mainnet && !url.contains('.onion')) {
        losLog('🚫 Rejected non-.onion URL for mainnet: $url');
        return false;
      }
      return true;
    }).toList();
    _currentNodeIndex = 0;
  }

  /// Async initialization: load saved peers, prepend to bootstrap list,
  /// then run initial latency probes to select the best node.
  Future<void> _loadSavedPeers() async {
    final savedPeers = await PeerDiscoveryService.loadSavedPeers();
    if (savedPeers.isNotEmpty) {
      // Collect hostnames already present in bootstrap list to prevent
      // adding port-less duplicates (e.g. "http://x.onion" when
      // "http://x.onion:3030" already exists from NetworkConfig).
      final knownHostnames = _bootstrapUrls
          .map((url) => Uri.tryParse(url)?.host ?? '')
          .where((h) => h.isNotEmpty)
          .toSet();

      final newPeers = savedPeers
          .where((p) {
            if (p == _excludedOnionUrl) return false;
            if (_bootstrapUrls.contains(p)) return false;
            // Hostname dedup: skip if same .onion hostname already known
            final host = Uri.tryParse(p)?.host ?? '';
            if (host.isNotEmpty && knownHostnames.contains(host)) return false;
            return true;
          })
          .take(_maxSavedPeers)
          .toList();
      if (newPeers.isNotEmpty) {
        _bootstrapUrls = [...newPeers, ..._bootstrapUrls];
        losLog('🔗 PeerDiscovery: added ${newPeers.length} saved peer(s) '
            '(total: ${_bootstrapUrls.length} endpoints)');
      }
    }
  }

  String _getBaseUrl(NetworkEnvironment env) {
    // Return first bootstrap URL if available, empty string if config not loaded yet.
    // Avoids throwing StateError during lazy initialization in widget tests
    // or when NetworkConfig.load() hasn't completed.
    switch (env) {
      case NetworkEnvironment.testnet:
        final nodes = NetworkConfig.testnetNodes;
        return nodes.isNotEmpty ? nodes.first.restUrl : '';
      case NetworkEnvironment.mainnet:
        final nodes = NetworkConfig.mainnetNodes;
        return nodes.isNotEmpty ? nodes.first.restUrl : '';
    }
  }

  // ══════════════════════════════════════════════════════════════════
  //  HOST SELECTION & HEALTH
  // ══════════════════════════════════════════════════════════════════

  /// Check ONLY the current host's health. Called periodically.
  ///
  /// Strategy: "Stick with what works."
  /// - If current host responds OK and fast enough → do nothing.
  /// - If current host is down/error/slow → trigger full probe to find replacement.
  ///
  /// This avoids probing ALL nodes every few minutes (which wastes 30s+ per
  /// dead Tor node and causes unnecessary host switching).
  Future<void> _checkCurrentHostHealth() async {
    if (_disposed || baseUrl.isEmpty) return;

    try {
      final timeout =
          baseUrl.contains('.onion') ? _torProbeTimeout : _probeTimeout;
      final sw = Stopwatch()..start();
      final response =
          await _client.get(Uri.parse('$baseUrl/health')).timeout(timeout);
      sw.stop();
      final rtt = sw.elapsedMilliseconds;

      if (response.statusCode >= 200 && response.statusCode < 300) {
        _getHealth(baseUrl).recordSuccess(rtt);
        // Host responded — stay on it regardless of latency.
        // Slow is fine, dead is not.
        losLog('🔌 [HealthCheck] OK (${rtt}ms) ✓');
      } else {
        // HTTP error (4xx/5xx) → host unhealthy but don't switch yet
        _getHealth(baseUrl).recordFailure();
        final failures = _getHealth(baseUrl).consecutiveFailures;
        losLog('🔌 [HealthCheck] HTTP ${response.statusCode} '
            '(failure $failures/3)');
        // Only switch after 3+ consecutive failures (Tor is unreliable)
        if (failures >= 3) {
          losLog('🔌 [HealthCheck] 3 consecutive failures — switching node');
          _switchToNextNode();
        }
      }
    } catch (e) {
      // Timeout or connection error → host may be DOWN
      _getHealth(baseUrl).recordFailure();
      final failures = _getHealth(baseUrl).consecutiveFailures;
      losLog('🔌 [HealthCheck] UNREACHABLE '
          '(failure $failures/3)');
      // Only switch after 3+ consecutive failures
      if (failures >= 3) {
        losLog('🔌 [HealthCheck] 3 consecutive failures — switching node');
        _switchToNextNode();
      }
    }
  }

  /// Probe all known peers for latency, then select the fastest responsive one.
  /// Called ONLY for manual diagnostics or explicit user action.
  /// NOT called automatically — we stick with the current working node.
  Future<void> probeAndSelectBestNode() async {
    if (_disposed || _bootstrapUrls.isEmpty) return;
    losLog(
        '📡 [Probe] Searching for best host across ${_bootstrapUrls.length} node(s)...');

    final results = <String, int>{};

    // FIX I-01: Limit concurrent probes to avoid saturating SOCKS5 proxy.
    // Previously all nodes probed in parallel via Future.wait — each opens a
    // separate Tor circuit through the same SOCKS5 port, causing timeouts.
    const maxConcurrent = 2;
    final nodesToProbe = _bootstrapUrls.where((url) {
      // Never probe our own hidden service (spec: validator must use external peers)
      if (url == _excludedOnionUrl) return false;
      final health = _nodeHealthMap[url];
      if (health != null && health.isInCooldown) {
        losLog(
            '📡 [Probe] $url — skipped (cooldown, ${health.consecutiveFailures} failures)');
        return false;
      }
      return true;
    }).toList();

    // Probe in batches of maxConcurrent
    for (var i = 0; i < nodesToProbe.length; i += maxConcurrent) {
      final batch = nodesToProbe.skip(i).take(maxConcurrent);
      final futures = batch.map((url) async {
        try {
          final timeout =
              url.contains('.onion') ? _torProbeTimeout : _probeTimeout;
          final sw = Stopwatch()..start();
          final response =
              await _client.get(Uri.parse('$url/health')).timeout(timeout);
          sw.stop();

          if (response.statusCode >= 200 && response.statusCode < 300) {
            results[url] = sw.elapsedMilliseconds;
            _getHealth(url).recordSuccess(sw.elapsedMilliseconds);
            losLog('📡 [Probe] $url — ${sw.elapsedMilliseconds}ms ✓');
          } else {
            _getHealth(url).recordFailure();
            losLog('📡 [Probe] $url — HTTP ${response.statusCode} ✗');
          }
        } catch (e) {
          _getHealth(url).recordFailure();
          losLog('📡 [Probe] $url — unreachable ($e) ✗');
        }
      });
      await Future.wait(futures);
    }

    if (results.isEmpty) {
      losLog(
          '📡 [Probe] No responsive nodes found — keeping current: $baseUrl');
      return;
    }

    // Sort by latency ascending, pick the fastest
    final sorted = results.entries.toList()
      ..sort((a, b) => a.value.compareTo(b.value));

    final bestUrl = sorted.first.key;
    final bestLatency = sorted.first.value;

    if (bestUrl != baseUrl) {
      final oldUrl = baseUrl;
      baseUrl = bestUrl;
      _currentNodeIndex =
          _bootstrapUrls.indexOf(bestUrl).clamp(0, _bootstrapUrls.length - 1);
      losLog('🏆 [Probe] Switched to $bestUrl (${bestLatency}ms) from $oldUrl');
      onNodeSwitched?.call(baseUrl);
    } else {
      losLog('🏆 [Probe] Best node unchanged: $baseUrl (${bestLatency}ms) — '
          '${sorted.length}/${_bootstrapUrls.length} responsive');
    }
  }

  /// Get or create health tracker for a URL.
  _NodeHealth _getHealth(String url) {
    return _nodeHealthMap.putIfAbsent(url, () => _NodeHealth());
  }

  // ══════════════════════════════════════════════════════════════════
  //  FAILOVER
  // ══════════════════════════════════════════════════════════════════

  /// Switch to the next available node, skipping nodes in cooldown,
  /// .onion URLs when Tor is unavailable, and validator's own .onion.
  bool _switchToNextNode() {
    if (_bootstrapUrls.length <= 1) return false;
    final startIndex = _currentNodeIndex;
    do {
      _currentNodeIndex = (_currentNodeIndex + 1) % _bootstrapUrls.length;
      final candidate = _bootstrapUrls[_currentNodeIndex];
      if (candidate == _excludedOnionUrl) continue;
      if (!_hasTor && candidate.contains('.onion')) continue;
      if (_getHealth(candidate).isInCooldown) continue;
      if (candidate != baseUrl) {
        baseUrl = candidate;
        losLog(
            '🔄 Failover: switched to node ${_currentNodeIndex + 1}/${_bootstrapUrls.length}: $baseUrl');
        onNodeSwitched?.call(baseUrl);
        return true;
      }
    } while (_currentNodeIndex != startIndex);
    // All nodes in cooldown — reset cooldowns and try round-robin
    if (_allNodesInCooldown()) {
      losLog('⚠️ All nodes in cooldown — resetting cooldowns for fresh retry');
      for (final h in _nodeHealthMap.values) {
        h.consecutiveFailures = 0;
      }
      return _switchToNextNodeNoCooldown();
    }
    return false;
  }

  bool _allNodesInCooldown() {
    for (final url in _bootstrapUrls) {
      if (url == _excludedOnionUrl) continue;
      if (!_hasTor && url.contains('.onion')) continue;
      if (!_getHealth(url).isInCooldown) return false;
    }
    return true;
  }

  bool _switchToNextNodeNoCooldown() {
    if (_bootstrapUrls.length <= 1) return false;
    final startIndex = _currentNodeIndex;
    do {
      _currentNodeIndex = (_currentNodeIndex + 1) % _bootstrapUrls.length;
      final candidate = _bootstrapUrls[_currentNodeIndex];
      if (candidate == _excludedOnionUrl) continue;
      if (!_hasTor && candidate.contains('.onion')) continue;
      if (candidate != baseUrl) {
        baseUrl = candidate;
        onNodeSwitched?.call(baseUrl);
        return true;
      }
    } while (_currentNodeIndex != startIndex);
    return false;
  }

  /// Execute an HTTP request with intelligent failover.
  ///
  /// STRATEGY: "Local first, external as background upgrade"
  /// When we have a local node running → use it IMMEDIATELY (< 100ms).
  /// Don't waste 45-225s trying .onion nodes through Tor SOCKS first.
  ///
  /// Order:
  /// 1. Local node (direct HTTP, no SOCKS) — instant response
  /// 2. External .onion peer via Tor SOCKS (sticky + retry)
  /// 3. Failover to other .onion nodes
  Future<http.Response> _requestWithFailover(
    Future<http.Response> Function(String url) requestFn,
    String endpoint,
  ) async {
    await ensureReady();

    // ── Phase 0: Local node FIRST (instant, no Tor needed) ──
    // The node is running at 127.0.0.1:3035 — response in < 100ms.
    // Don't make the user wait 45-225s for Tor when local data is available.
    if (_localNodeUrl != null) {
      try {
        final response = await _directClient
            .get(Uri.parse('$_localNodeUrl$endpoint'))
            .timeout(const Duration(seconds: 5));

        if (response.statusCode < 500) {
          if (!_usingLocalFallback) {
            _usingLocalFallback = true;
            losLog('🏠 Using local node (instant) — '
                'Dashboard data from local node');
          }
          // Trigger initial discovery (once) so we learn about external peers
          if (!_initialDiscoveryDone) {
            _initialDiscoveryDone = true;
            Future.microtask(() => _runInitialDiscovery());
          }
          return response;
        }
      } on Exception catch (_) {
        // Local node not responding — fall through to external nodes
      }
    }

    // ── Phase 1: Try current external node with a retry ──
    bool socksDead = false;
    for (var retry = 0; retry < 2; retry++) {
      try {
        final sw = Stopwatch()..start();
        final response = await requestFn(baseUrl).timeout(_timeout);
        sw.stop();

        _getHealth(baseUrl).recordSuccess(sw.elapsedMilliseconds);

        if (response.statusCode >= 500) {
          _getHealth(baseUrl).recordFailure();
          break;
        }

        // Success via external peer — clear local fallback
        if (_usingLocalFallback) {
          _usingLocalFallback = false;
          losLog('🌐 Restored external .onion connectivity');
        }

        if (!_initialDiscoveryDone) {
          _initialDiscoveryDone = true;
          Future.microtask(() => _runInitialDiscovery());
        }

        return response;
      } on Exception catch (e) {
        _getHealth(baseUrl).recordFailure();
        if (retry == 0) {
          final errStr = e.toString();
          if (errStr.contains('Connection refused') ||
              errStr.contains('SOCKS') ||
              errStr.contains('Proxy')) {
            _triggerTorRecovery();
            socksDead = true;
            break;
          }
          continue;
        }
      }
    }

    // ── Phase 1b: Try other nodes (skip if SOCKS dead) ──
    if (!socksDead) {
      final otherAttempts = (_bootstrapUrls.length - 1).clamp(0, 2);
      for (var i = 0; i < otherAttempts; i++) {
        if (!_switchToNextNode()) break;
        try {
          final sw = Stopwatch()..start();
          final response = await requestFn(baseUrl).timeout(_timeout);
          sw.stop();

          _getHealth(baseUrl).recordSuccess(sw.elapsedMilliseconds);

          if (response.statusCode >= 500) {
            _getHealth(baseUrl).recordFailure();
            continue;
          }

          if (_usingLocalFallback) {
            _usingLocalFallback = false;
            losLog('🌐 Restored external .onion via failover');
          }

          if (!_initialDiscoveryDone) {
            _initialDiscoveryDone = true;
            Future.microtask(() => _runInitialDiscovery());
          }

          return response;
        } on Exception catch (e) {
          _getHealth(baseUrl).recordFailure();
          losLog(
              '⚠️ Failover node ${_currentNodeIndex + 1} failed for $endpoint: $e');
        }
      }
    }

    // ── Phase 2: Everything failed ──
    throw Exception('All nodes unreachable for $endpoint');
  }

  /// Trigger Tor SOCKS proxy recovery in the background.
  /// Non-blocking: fires and forgets. Only one recovery attempt at a time.
  void _triggerTorRecovery() {
    if (_torRecoveryInProgress || _disposed) return;
    // Use a non-final field via closure
    losLog('🔄 [ApiService] Triggering Tor SOCKS recovery...');
    _torRecoveryInProgress = true;
    Future(() async {
      try {
        final started = await _torService.start();
        if (started) {
          losLog('✅ [ApiService] Tor recovered — recreating HTTP client');
          await _reinitializeTorClient();
        } else {
          losLog('⚠️ [ApiService] Tor recovery failed — local fallback active');
        }
      } catch (e) {
        losLog('❌ [ApiService] Tor recovery error: $e');
      } finally {
        _torRecoveryInProgress = false;
      }
    });
  }

  /// Runs once after first successful API response:
  /// 1. Discover peers from network (save for future failover)
  /// 2. Start periodic background timers
  ///
  /// NOTE: We do NOT probe/switch nodes here. The current node
  /// already responded successfully — stick with it.
  Future<void> _runInitialDiscovery() async {
    if (_disposed) return;
    try {
      await discoverAndSavePeers();
    } catch (e) {
      losLog('⚠️ Initial discovery failed (non-critical): $e');
    }
    _startBackgroundTimers();
  }

  /// Start recurring background tasks:
  /// - Re-discover peers every 10 minutes (bootstrap list rarely changes)
  /// - Health-check current host every 5 minutes (NOT full probe)
  void _startBackgroundTimers() {
    _rediscoveryTimer?.cancel();
    _healthCheckTimer?.cancel();

    _rediscoveryTimer = Timer.periodic(_rediscoveryInterval, (_) {
      if (!_disposed) discoverAndSavePeers();
    });

    _healthCheckTimer = Timer.periodic(_healthCheckInterval, (_) {
      if (!_disposed) _checkCurrentHostHealth();
    });

    losLog('⏰ Background timers started: '
        'discovery every ${_rediscoveryInterval.inMinutes}m, '
        'current-host health check every ${_healthCheckInterval.inMinutes}m');
  }

  /// Called by NetworkStatusService when health check detects degradation.
  /// Instead of probing all nodes, just try the NEXT one.
  /// Rule: don't waste time probing — just rotate once.
  void onHealthDegraded() {
    if (_disposed) return;
    _getHealth(baseUrl).recordFailure();
    if (_getHealth(baseUrl).consecutiveFailures >= 3) {
      losLog('🔌 Health degraded (3+ failures) — switching to next node');
      _switchToNextNode();
    } else {
      losLog(
          '🔌 Health degraded (${_getHealth(baseUrl).consecutiveFailures}/3) — staying on current node');
    }
  }

  /// Get appropriate timeout based on whether using Tor
  Duration get _timeout =>
      baseUrl.contains('.onion') ? _torTimeout : _defaultTimeout;

  /// Initialize HTTP client — ALWAYS attempts Tor first (even for localhost),
  /// so we can reach .onion bootstrap peers during failover.
  /// FIX B-02: Previously only created Tor client if initial baseUrl was .onion,
  /// which meant starting on localhost = no Tor ever = .onion peers unreachable.
  Future<void> _initializeClient() async {
    // Always try Tor — we need SOCKS5 to reach .onion peers even when
    // the initial connection target is localhost.
    try {
      _client = await _createTorClient();
      if (_hasTor) {
        losLog('✅ Tor SOCKS5 client ready (can reach .onion peers)');
      }
    } catch (e) {
      losLog('⚠️ Tor init failed ($e) — falling back to direct HTTP');
      _client = http.Client();
      _hasTor = false;
    }
    if (!_hasTor && !baseUrl.contains('.onion')) {
      losLog('✅ Direct HTTP client for $baseUrl (Tor unavailable)');
    }
    // After client is ready, load saved peers into bootstrap list
    PeerDiscoveryService.setNetwork(environment.name);
    await _loadSavedPeers();
  }

  /// Create Tor-enabled HTTP client.
  /// Uses the shared TorService.start() so _isRunning is properly synced.
  ///
  /// If Tor is unavailable or broken, returns a plain HTTP client
  /// with _hasTor=false so failover can skip .onion URLs gracefully.
  Future<http.Client> _createTorClient() async {
    // Always go through start() — it detects existing SOCKS5 proxy internally
    // AND sets _isRunning=true, which startWithHiddenService() depends on.
    final started = await _torService.start();
    if (!started) {
      losLog('⚠️ Tor unavailable — no SOCKS5 proxy detected on any port. '
          'Falling back to direct HTTP (cannot reach .onion addresses).');
      _hasTor = false;
      return http.Client();
    }

    final socksPort = _torService.activeSocksPort;
    final httpClient = HttpClient();

    SocksTCPClient.assignToHttpClient(
      httpClient,
      [ProxySettings(InternetAddress.loopbackIPv4, socksPort)],
    );

    httpClient.connectionTimeout = const Duration(seconds: 30);
    httpClient.idleTimeout = const Duration(seconds: 30);

    _hasTor = true;
    losLog('✅ Tor SOCKS5 proxy configured (localhost:$socksPort)');
    return IOClient(httpClient);
  }

  /// FIX B-03: Recreate the HTTP client after Tor restarts on a new SOCKS port.
  /// Called when TorService fires onSocksPortChanged after hidden service startup.
  Future<void> _reinitializeTorClient() async {
    try {
      _client = await _createTorClient();
      if (_hasTor) {
        losLog('✅ [ApiService] HTTP client recreated on new SOCKS port '
            '${_torService.activeSocksPort}');
      }
    } catch (e) {
      losLog('❌ [ApiService] Failed to recreate Tor client: $e');
    }
  }

  // Switch network environment
  void switchEnvironment(NetworkEnvironment newEnv) {
    _loadBootstrapUrls(newEnv);

    // Mainnet guard: refuse switch if no mainnet nodes are configured
    if (newEnv == NetworkEnvironment.mainnet && _bootstrapUrls.isEmpty) {
      losLog('🚫 Cannot switch to mainnet: no bootstrap nodes configured');
      _loadBootstrapUrls(NetworkEnvironment.testnet);
      throw StateError(
        'Mainnet has not launched yet. No bootstrap nodes available.',
      );
    }

    environment = newEnv;
    // SECURITY FIX F2: Sync mainnet mode to WalletService so Ed25519
    // fallback crypto is refused on mainnet.
    WalletService.mainnetMode = (newEnv == NetworkEnvironment.mainnet);
    _nodeHealthMap.clear();
    _initialDiscoveryDone = false;
    _rediscoveryTimer?.cancel();
    _healthCheckTimer?.cancel();
    baseUrl =
        _bootstrapUrls.isNotEmpty ? _bootstrapUrls.first : _getBaseUrl(newEnv);
    _clientReady = _initializeClient();
    losLog('🔄 Switched to ${newEnv.name.toUpperCase()}: $baseUrl '
        '(${_bootstrapUrls.length} nodes)');
  }

  // Node Info
  Future<Map<String, dynamic>> getNodeInfo() async {
    losLog('🌐 [ApiService.getNodeInfo] Fetching node info...');
    try {
      final response = await _requestWithFailover(
        (url) => _client.get(Uri.parse('$url/node-info')),
        '/node-info',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        losLog(
            '🌐 [ApiService.getNodeInfo] Success: block_height=${data['block_height']}');
        return data;
      }
      throw Exception('Failed to get node info: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getNodeInfo error: $e');
      rethrow;
    }
  }

  /// Fetch node-info from a specific URL (used by NodeControlScreen for local node).
  /// NOTE: This method uses bare http.get() (no Tor SOCKS5 proxy) intentionally
  /// because it is ONLY called with localhost URLs (127.0.0.1) to check the
  /// local node status. .onion URLs would fail here — use _requestWithFailover
  /// via getNodeInfo() for Tor-routed requests.
  Future<Map<String, dynamic>?> getNodeInfoFromUrl(String url) async {
    assert(!url.contains('.onion'),
        'getNodeInfoFromUrl is localhost-only. Use getNodeInfo() for .onion URLs.');
    losLog('🌐 [ApiService.getNodeInfoFromUrl] url: $url');
    try {
      final response = await http
          .get(Uri.parse('$url/node-info'))
          .timeout(const Duration(seconds: 5));
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        losLog('🌐 [ApiService.getNodeInfoFromUrl] Success');
        return data;
      }
    } catch (e) {
      losLog('⚠️ getNodeInfoFromUrl($url) error: $e');
    }
    return null;
  }

  // Health Check
  Future<Map<String, dynamic>> getHealth() async {
    losLog('🌐 [ApiService.getHealth] Fetching health...');
    try {
      final response = await _requestWithFailover(
        (url) => _client.get(Uri.parse('$url/health')),
        '/health',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        losLog('🌐 [ApiService.getHealth] Success');
        return data;
      }
      throw Exception('Failed to get health: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getHealth error: $e');
      rethrow;
    }
  }

  /// Parse an int from a value that may be int, String, double, or null.
  static int _safeInt(dynamic v, [int fallback = 0]) {
    if (v == null) return fallback;
    if (v is int) return v;
    if (v is double) return v.toInt();
    return int.tryParse(v.toString()) ?? fallback;
  }

  // Get Balance
  // Backend: GET /balance/:address → {balance, balance_los: string, balance_cil: u128-int}
  // Backend: GET /bal/:address     → {balance_los: string, balance_cil: u128-int}
  Future<Account> getBalance(String address) async {
    losLog('🌐 [ApiService.getBalance] address: $address');
    try {
      final response = await _requestWithFailover(
        (url) => _client.get(Uri.parse('$url/balance/$address')),
        '/balance/$address',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        // Prefer balance_cil_str (string) over balance_cil (number) for
        // JSON precision safety: numbers > 2^53 may lose precision in parsing.
        // balance_cil is the canonical CIL amount (u128).
        // balance_los / balance are formatted LOS strings ("1000.00000000000").
        int balanceVoid;
        if (data['balance_cil_str'] != null) {
          balanceVoid = int.tryParse(data['balance_cil_str'].toString()) ?? 0;
        } else if (data['balance_cil'] != null) {
          balanceVoid = _safeInt(data['balance_cil']);
        } else if (data['balance_los'] != null) {
          final val = data['balance_los'];
          if (val is int) {
            balanceVoid = val;
          } else if (val is String) {
            // FIX C12-03: balance_los is a formatted decimal string like "1000.00000000000"
            // int.tryParse fails on decimal strings. Use losStringToCil for proper conversion.
            balanceVoid = BlockchainConstants.losStringToCil(val);
          } else {
            balanceVoid = 0;
          }
        } else if (data['balance'] != null) {
          final val = data['balance'];
          if (val is int) {
            balanceVoid = val;
          } else if (val is String) {
            balanceVoid = BlockchainConstants.losStringToCil(val);
          } else {
            balanceVoid = 0;
          }
        } else {
          balanceVoid = 0;
        }
        final account = Account(
          address: address,
          balance: balanceVoid,
          cilBalance: 0,
          history: [],
        );
        losLog('🌐 [ApiService.getBalance] Success: balance=$balanceVoid CILD');
        return account;
      }
      throw Exception('Failed to get balance: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getBalance error: $e');
      rethrow;
    }
  }

  // Get Account (with history)
  Future<Account> getAccount(String address) async {
    losLog('🌐 [ApiService.getAccount] address: $address');
    try {
      final response = await _requestWithFailover(
        (url) => _client.get(Uri.parse('$url/account/$address')),
        '/account/$address',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        losLog('🌐 [ApiService.getAccount] Success');
        return Account.fromJson(data);
      }
      throw Exception('Failed to get account: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getAccount error: $e');
      rethrow;
    }
  }

  // Request Faucet
  Future<Map<String, dynamic>> requestFaucet(String address) async {
    losLog('🌐 [ApiService.requestFaucet] address: $address');
    try {
      final response = await _requestWithFailover(
        (url) => _client.post(
          Uri.parse('$url/faucet'),
          headers: {'Content-Type': 'application/json'},
          body: json.encode({'address': address}),
        ),
        '/faucet',
      );

      final data = json.decode(response.body);

      // Critical: Check BOTH status code AND response body status
      // Backend returns 200 on success, >= 400 on errors (400 for validation, 429 for rate limit)
      if (response.statusCode >= 400 || data['status'] == 'error') {
        throw Exception(data['msg'] ?? 'Faucet request failed');
      }

      losLog('🌐 [ApiService.requestFaucet] Success');
      return data;
    } catch (e) {
      losLog('❌ requestFaucet error: $e');
      rethrow;
    }
  }

  // Send Transaction
  // Backend POST /send requires: {from, target, amount (LOS int), signature, public_key}
  Future<Map<String, dynamic>> sendTransaction({
    required String from,
    required String to,
    required int amount,
    required String signature,
    required String publicKey,
  }) async {
    losLog(
        '🌐 [ApiService.sendTransaction] from: $from, to: $to, amount: $amount');
    try {
      final response = await _requestWithFailover(
        (url) => _client.post(
          Uri.parse('$url/send'),
          headers: {'Content-Type': 'application/json'},
          body: json.encode({
            'from': from,
            'target': to,
            'amount': amount,
            'signature': signature,
            'public_key': publicKey,
          }),
        ),
        '/send',
      );

      final data = json.decode(response.body);

      // Critical: Check BOTH status code AND response body status
      if (response.statusCode >= 400 || data['status'] == 'error') {
        throw Exception(data['msg'] ?? 'Transaction failed');
      }

      losLog(
          '🌐 [ApiService.sendTransaction] Success: txid=${data['txid'] ?? data['tx_id'] ?? 'N/A'}');
      return data;
    } catch (e) {
      losLog('❌ sendTransaction error: $e');
      rethrow;
    }
  }

  // Burn request
  Future<Map<String, dynamic>> submitBurn({
    required String losAddress,
    required String btcTxid,
    required String ethTxid,
    required int amount,
  }) async {
    try {
      final response = await _requestWithFailover(
        (url) => _client.post(
          Uri.parse('$url/burn'),
          headers: {'Content-Type': 'application/json'},
          body: json.encode({
            'los_address': losAddress,
            'btc_txid': btcTxid,
            'eth_txid': ethTxid,
            'amount': amount,
          }),
        ),
        '/burn',
      );

      final data = json.decode(response.body);

      // Critical: Check BOTH status code AND response body status
      if (response.statusCode >= 400 || data['status'] == 'error') {
        throw Exception(data['msg'] ?? 'Burn submission failed');
      }

      return data;
    } catch (e) {
      losLog('❌ submitBurn error: $e');
      rethrow;
    }
  }

  // Get Validators
  // FIX C-01: Backend wraps in {"validators": [...]}, not bare array
  Future<List<ValidatorInfo>> getValidators() async {
    losLog('🛡️ [ApiService.getValidators] Fetching validators...');
    try {
      final response = await _requestWithFailover(
        (url) => _client.get(Uri.parse('$url/validators')),
        '/validators',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final decoded = json.decode(response.body);
        // Handle both {"validators": [...]} (backend) and bare [...] (future)
        final List<dynamic> data = decoded is List
            ? decoded
            : (decoded['validators'] as List<dynamic>?) ?? [];
        final validators = data.map((v) => ValidatorInfo.fromJson(v)).toList();
        losLog(
            '🛡️ [ApiService.getValidators] Success: ${validators.length} validators');
        return validators;
      }
      throw Exception('Failed to get validators: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getValidators error: $e');
      rethrow;
    }
  }

  /// Check if an address is an active genesis bootstrap validator.
  /// Returns true if the address is found in /validators with is_genesis=true and is_active=true.
  Future<bool> isActiveGenesisValidator(String address) async {
    losLog('🛡️ [ApiService.isActiveGenesisValidator] address: $address');
    try {
      final validators = await getValidators();
      final result = validators.any(
        (v) => v.address == address && v.isGenesis && v.isActive,
      );
      losLog('🛡️ [ApiService.isActiveGenesisValidator] Result: $result');
      return result;
    } catch (e) {
      losLog('⚠️ isActiveGenesisValidator check failed: $e');
      return false;
    }
  }

  // Get Latest Block
  Future<BlockInfo> getLatestBlock() async {
    losLog('🌐 [ApiService.getLatestBlock] Fetching latest block...');
    try {
      final response = await _requestWithFailover(
        (url) => _client.get(Uri.parse('$url/block')),
        '/block',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        losLog(
            '🌐 [ApiService.getLatestBlock] Success: height=${data['height']}');
        return BlockInfo.fromJson(data);
      }
      throw Exception('Failed to get latest block: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getLatestBlock error: $e');
      rethrow;
    }
  }

  // Get Recent Blocks
  Future<List<BlockInfo>> getRecentBlocks() async {
    losLog('🌐 [ApiService.getRecentBlocks] Fetching recent blocks...');
    try {
      final response = await _requestWithFailover(
        (url) => _client.get(Uri.parse('$url/blocks/recent')),
        '/blocks/recent',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final decoded = json.decode(response.body);
        // FIX C11-07: Handle both bare array and wrapped {"blocks": [...]}
        final List<dynamic> data = decoded is List
            ? decoded
            : (decoded['blocks'] as List<dynamic>?) ?? [];
        final blocks = data.map((b) => BlockInfo.fromJson(b)).toList();
        losLog(
            '🌐 [ApiService.getRecentBlocks] Success: ${blocks.length} blocks');
        return blocks;
      }
      throw Exception('Failed to get recent blocks: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getRecentBlocks error: $e');
      rethrow;
    }
  }

  // Get Peers
  // Backend returns {"peers": [{"address":..., "is_validator":..., ...}], "peer_count": N, ...}
  Future<List<String>> getPeers() async {
    losLog('📡 [ApiService.getPeers] Fetching peers...');
    try {
      final response = await _requestWithFailover(
        (url) => _client.get(Uri.parse('$url/peers')),
        '/peers',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final decoded = json.decode(response.body);
        if (decoded is Map) {
          // New format: {"peers": [{"address": "...", ...}], "peer_count": N}
          if (decoded.containsKey('peers') && decoded['peers'] is List) {
            final peers = (decoded['peers'] as List)
                .map((p) =>
                    p is Map ? (p['address'] ?? '').toString() : p.toString())
                .where((s) => s.isNotEmpty)
                .toList();
            losLog('📡 [ApiService.getPeers] Success: ${peers.length} peers');
            return peers;
          }
          // Legacy fallback: flat HashMap<String, String>
          return decoded.keys.cast<String>().toList();
        } else if (decoded is List) {
          return decoded.whereType<String>().toList();
        }
        return [];
      }
      throw Exception('Failed to get peers: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getPeers error: $e');
      rethrow;
    }
  }

  /// Register this node as an active validator on the local los-node.
  /// Requires Dilithium5 signature proof of key ownership.
  /// The node will broadcast the registration to all peers via gossipsub.
  Future<Map<String, dynamic>> registerValidator({
    required String address,
    required String publicKey,
    required String signature,
    required int timestamp,
    String? onionAddress,
  }) async {
    losLog('🛡️ [ApiService.registerValidator] address: $address');
    try {
      final body = <String, dynamic>{
        'address': address,
        'public_key': publicKey,
        'signature': signature,
        'timestamp': timestamp,
      };
      // Include our own .onion address so the receiving node broadcasts it
      // correctly (instead of broadcasting its own onion address).
      if (onionAddress != null && onionAddress.isNotEmpty) {
        body['onion_address'] = onionAddress;
      }
      final response = await _requestWithFailover(
        (url) => _client.post(
          Uri.parse('$url/register-validator'),
          headers: {'Content-Type': 'application/json'},
          body: json.encode(body),
        ),
        '/register-validator',
      );

      final data = json.decode(response.body);

      if (response.statusCode >= 400 || data['status'] == 'error') {
        throw Exception(data['msg'] ?? 'Validator registration failed');
      }

      losLog('🛡️ [ApiService.registerValidator] Success');
      return data;
    } catch (e) {
      losLog('❌ registerValidator error: $e');
      rethrow;
    }
  }

  /// Get the name of the currently connected bootstrap node (e.g. "validator-1")
  String get connectedNodeName {
    final nodes = environment == NetworkEnvironment.testnet
        ? NetworkConfig.testnetNodes
        : NetworkConfig.mainnetNodes;
    if (_currentNodeIndex < nodes.length) {
      return nodes[_currentNodeIndex].name;
    }
    // Discovered peer — show short onion address
    if (_currentNodeIndex < _bootstrapUrls.length) {
      final url = _bootstrapUrls[_currentNodeIndex];
      final onion = url.replaceAll('http://', '');
      return onion.length > 16 ? '${onion.substring(0, 12)}...' : onion;
    }
    return 'unknown';
  }

  /// Get the current connected node index and total count
  String get connectionInfo =>
      'Node ${_currentNodeIndex + 1}/${_bootstrapUrls.length}';

  /// Fetch validator reward pool status from GET /reward-info.
  Future<Map<String, dynamic>> getRewardInfo() async {
    losLog('🌐 [ApiService.getRewardInfo] Fetching reward info...');
    try {
      final response = await _requestWithFailover(
        (url) => _client.get(Uri.parse('$url/reward-info')),
        '/reward-info',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        losLog('🌐 [ApiService.getRewardInfo] Success');
        return data;
      }
      throw Exception('Failed to get reward info: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getRewardInfo error: $e');
      rethrow;
    }
  }

  /// Discover new validator endpoints from the network and save locally.
  /// Called periodically (every 5 minutes) to maintain an up-to-date peer table.
  /// Filters out the validator's own onion address if set.
  Future<void> discoverAndSavePeers() async {
    if (_disposed) return;
    try {
      final response = await _requestWithFailover(
        (url) => _client.get(Uri.parse('$url/network/peers')),
        '/network/peers',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        final endpoints = (data['endpoints'] as List<dynamic>?)
                ?.map((e) => e as Map<String, dynamic>)
                .toList() ??
            [];
        if (endpoints.isNotEmpty) {
          await PeerDiscoveryService.savePeers(endpoints);
          for (final ep in endpoints) {
            final onion = ep['onion_address']?.toString() ?? '';
            if (onion.isNotEmpty && onion.endsWith('.onion')) {
              // Extract bare hostname (strip any embedded port/scheme)
              final hostname =
                  onion.contains(':') ? onion.split(':').first : onion;

              // DEDUP FIX: Check if this onion hostname already exists in
              // _bootstrapUrls (which have correct ports from NetworkConfig).
              // Prevents adding "http://x.onion" when "http://x.onion:3030"
              // is already present — the root cause of port-less duplicates.
              final alreadyKnown = _bootstrapUrls.any((existing) {
                final uri = Uri.tryParse(existing);
                return uri?.host == hostname;
              });
              if (alreadyKnown) continue;

              // Build URL with rest_port if provided by the API
              final restPort = ep['rest_port'] as int?;
              final url = (restPort != null && restPort != 80)
                  ? 'http://$hostname:$restPort'
                  : 'http://$hostname';

              // Exclude own onion (validator self-connection prevention)
              if (url == _excludedOnionUrl) continue;
              if (!_bootstrapUrls.contains(url)) {
                _bootstrapUrls.add(url);
              }
            }
          }
          losLog('🌐 Discovery: ${endpoints.length} endpoint(s), '
              'total URLs: ${_bootstrapUrls.length}');
        }
      }
    } catch (e) {
      losLog('⚠️ Peer discovery failed (non-critical): $e');
    }
  }

  /// Expose current node health for UI display
  Map<String, Map<String, dynamic>> get nodeHealthSummary {
    final summary = <String, Map<String, dynamic>>{};
    for (final url in _bootstrapUrls) {
      final h = _nodeHealthMap[url];
      summary[url] = {
        'latency_ms': h?.latencyMs,
        'consecutive_failures': h?.consecutiveFailures ?? 0,
        'in_cooldown': h?.isInCooldown ?? false,
        'is_current': url == baseUrl,
      };
    }
    return summary;
  }

  // ─── USP-01 Token Read-Only (Validator Dashboard) ───────────────

  /// Fetch all registered USP-01 tokens on the network.
  Future<List<Token>> getTokens() async {
    losLog('🪙 [API] getTokens...');
    try {
      final response = await _requestWithFailover(
        (url) => _client.get(Uri.parse('$url/tokens')),
        '/tokens',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        if (data is List) {
          final tokens = data
              .map((j) => Token.fromJson(j as Map<String, dynamic>))
              .toList();
          losLog('🪙 [API] getTokens: ${tokens.length} tokens');
          return tokens;
        }
      }
      return [];
    } catch (e) {
      losLog('❌ getTokens error: $e');
      rethrow;
    }
  }

  // ─── DEX Read-Only (Validator Dashboard) ────────────────────────

  /// Fetch all DEX liquidity pools.
  Future<List<DexPool>> getDexPools() async {
    losLog('📊 [API] getDexPools...');
    try {
      final response = await _requestWithFailover(
        (url) => _client.get(Uri.parse('$url/dex/pools')),
        '/dex/pools',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        if (data is List) {
          final pools = data
              .map((j) => DexPool.fromJson(j as Map<String, dynamic>))
              .toList();
          losLog('📊 [API] getDexPools: ${pools.length} pools');
          return pools;
        }
      }
      return [];
    } catch (e) {
      losLog('❌ getDexPools error: $e');
      rethrow;
    }
  }

  /// Release HTTP client resources and cancel background timers.
  void dispose() {
    losLog('🌐 [ApiService.dispose] Disposed');
    _disposed = true;
    _rediscoveryTimer?.cancel();
    _healthCheckTimer?.cancel();
    _client.close();
    _directClient.close();
  }
}
