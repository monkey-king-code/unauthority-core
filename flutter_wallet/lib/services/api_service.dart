import '../utils/log.dart';
import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:http/http.dart' as http;
import '../models/token.dart';
import '../models/dex_pool.dart';
import 'package:http/io_client.dart';
import 'package:socks5_proxy/socks_client.dart';
import '../models/account.dart';
import 'tor_service.dart';
import 'network_config.dart';
import 'peer_discovery_service.dart';
import 'wallet_service.dart';
import '../config/testnet_config.dart';

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

  /// Default timeout for clearnet API calls.
  /// 8s is generous for clearnet (localhost responds <100ms, remote <2s).
  /// Previously 30s — caused 30s+ waits when a clearnet node was down.
  static const Duration _defaultTimeout = Duration(seconds: 8);

  /// Longer timeout for Tor connections (45s — .onion routing can be slow on first circuit)
  static const Duration _torTimeout = Duration(seconds: 45);

  /// Timeout for latency probes (short — just checking reachability)
  static const Duration _probeTimeout = Duration(seconds: 5);

  /// Tor-specific probe timeout (Tor circuits need more time)
  static const Duration _torProbeTimeout = Duration(seconds: 30);

  /// Max retry attempts across bootstrap nodes before giving up
  static const int _maxRetries = 4;

  /// Max saved peers to prepend to bootstrap URLs (avoid 200+ dead .onion bloat)
  static const int _maxSavedPeers = 10;

  /// Interval between periodic peer re-discovery runs
  static const Duration _rediscoveryInterval = Duration(minutes: 5);

  /// Interval between current-host health checks.
  /// Only pings the CURRENT host — does NOT probe all nodes.
  /// If current host is down/error/slow → triggers full probe to find replacement.
  static const Duration _healthCheckInterval = Duration(minutes: 2);

  late String baseUrl;
  // Initialize with safe default to prevent LateInitializationError.
  // _initializeClient() replaces with Tor client asynchronously when needed.
  http.Client _client = http.Client();

  /// Direct HTTP client (no SOCKS5 proxy) for clearnet URLs.
  /// Clearnet requests through SOCKS5 fail with SocksClientConnectionCommandFailedException.
  final http.Client _directClient = http.Client();

  NetworkEnvironment environment;
  final TorService _torService;

  /// All available bootstrap URLs for failover
  List<String> _bootstrapUrls = [];

  /// Index of the currently active bootstrap node
  int _currentNodeIndex = 0;

  /// Per-node health tracking: URL → _NodeHealth
  final Map<String, _NodeHealth> _nodeHealthMap = {};

  /// Track client initialization future so callers can await
  /// readiness before making requests. Prevents DNS leaks from using
  /// the default http.Client on .onion URLs before Tor is ready.
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

  /// Optional: the local validator's own .onion address.
  /// When set, this node is EXCLUDED from the peer list to prevent
  /// self-connection (spec: "flutter_validator MUST NOT use its own
  /// local onion address for API consumption").
  String? _excludedOnionUrl;

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
    losLog('🔗 LOS ApiService initialized with baseUrl: $baseUrl '
        '(${_bootstrapUrls.length} bootstrap nodes available)');
  }

  /// Await Tor/HTTP client initialization before first request.
  /// Safe to call multiple times — resolves immediately after first init.
  Future<void> ensureReady() => _clientReady;

  /// Load all bootstrap URLs for the given environment.
  /// Filters out the validator's own .onion address if excluded.
  /// On mainnet, only .onion URLs are permitted.
  void _loadBootstrapUrls(NetworkEnvironment env) {
    final nodes = env == NetworkEnvironment.testnet
        ? NetworkConfig.testnetNodes
        : NetworkConfig.mainnetNodes;
    _bootstrapUrls = nodes
        .expand((n) => n.allRestUrls)
        .where((url) => url != _excludedOnionUrl)
        .where((url) {
          // SECURITY: Mainnet requires .onion-only connections (Tor network)
          if (env == NetworkEnvironment.mainnet && !url.contains('.onion')) {
            losLog('🚫 Rejected non-.onion URL for mainnet: $url');
            return false;
          }
          return true;
        })
        .toSet()
        .toList();
    // Sort clearnet URLs first, .onion last.
    // Prevents wasting 30-45s per .onion timeout when clearnet is available.
    // Critical for testnet dev where .onion addresses may not exist on Tor.
    _bootstrapUrls.sort((a, b) {
      final aIsOnion = a.contains('.onion') ? 1 : 0;
      final bIsOnion = b.contains('.onion') ? 1 : 0;
      return aIsOnion.compareTo(bIsOnion);
    });
    _currentNodeIndex = 0;
  }

  /// Async initialization: load custom + saved peers, prepend to bootstrap list,
  /// then run initial latency probes to select the best node.
  /// Priority: custom peers > saved peers > bootstrap nodes.
  /// Caps saved peers to _maxSavedPeers to avoid 200+ dead .onion bloat.
  Future<void> _loadSavedPeers() async {
    // 1. Load user-added custom peers (highest priority)
    final customPeers = await PeerDiscoveryService.loadCustomPeers();
    if (customPeers.isNotEmpty) {
      final newCustom = customPeers
          .where((p) => p != _excludedOnionUrl && !_bootstrapUrls.contains(p))
          .toList();
      if (newCustom.isNotEmpty) {
        _bootstrapUrls = [...newCustom, ..._bootstrapUrls];
        losLog('🎯 PeerDiscovery: added ${newCustom.length} custom peer(s) '
            '(total: ${_bootstrapUrls.length} endpoints)');
      }
    }

    // 2. Load auto-discovered saved peers (medium priority)
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
            final uri = Uri.tryParse(p);
            final host = uri?.host ?? '';
            if (host.isNotEmpty && knownHostnames.contains(host)) return false;
            // GHOST PEER FILTER: Skip .onion URLs with default port 80.
            // These are almost always P2P addresses (libp2p gossip) saved by
            // mistake — the REST API runs on explicit ports (3030-3033).
            // Port 80 .onion peers cause 30s timeouts and never respond.
            if (host.endsWith('.onion') && (uri?.port ?? 80) == 80) {
              losLog('🚫 Skipping ghost peer (port 80 .onion): $p');
              return false;
            }
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

  // ══════════════════════════════════════════════════════════════════
  //  ADD CUSTOM NODE — For Bootstrap Independence
  // ══════════════════════════════════════════════════════════════════

  /// Add a custom node URL provided by the user.
  /// Saves to persistent storage and immediately adds to the failover list.
  /// If all bootstrap nodes are dead, a user can add any known validator
  /// URL to join the network — this is the key to blockchain survival
  /// without the original bootstrap nodes.
  ///
  /// Returns a status message for UI display.
  Future<String> addCustomNode(String rawUrl) async {
    final added = await PeerDiscoveryService.addCustomPeer(rawUrl);
    if (!added) {
      return 'Invalid URL or already added.';
    }

    // Normalize URL (same logic as PeerDiscoveryService)
    String url = rawUrl.trim();
    if (!url.startsWith('http://') && !url.startsWith('https://')) {
      url = 'http://$url';
    }

    // Immediately inject into live bootstrap list (highest priority)
    if (!_bootstrapUrls.contains(url)) {
      _bootstrapUrls = [url, ..._bootstrapUrls];
      _currentNodeIndex = 0; // Reset to try custom peer first
    }

    // Try to connect to it immediately
    try {
      final client = _clientFor(url);
      final timeout = url.contains('.onion') ? _torProbeTimeout : _probeTimeout;
      final response =
          await client.get(Uri.parse('$url/health')).timeout(timeout);
      if (response.statusCode >= 200 && response.statusCode < 300) {
        baseUrl = url;
        _getHealth(url).recordSuccess(0);
        return 'Connected to $url successfully!';
      } else {
        return 'Node added but returned status ${response.statusCode}. Will retry during failover.';
      }
    } catch (e) {
      return 'Node saved for failover. Could not connect now: ${e.toString().split('\n').first}';
    }
  }

  /// Get the list of user-added custom nodes (for UI display).
  Future<List<String>> getCustomNodes() async {
    return PeerDiscoveryService.loadCustomPeers();
  }

  /// Remove a custom node.
  Future<void> removeCustomNode(String url) async {
    await PeerDiscoveryService.removeCustomPeer(url);
    _bootstrapUrls.remove(url);
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
  /// - If current host responds OK → do nothing (even if slow — Tor is slow).
  /// - If current host fails 3+ times consecutively → switch to next node.
  ///
  /// NEVER triggers full probe (probes ALL nodes, each 30s+ on Tor).
  /// That was causing the non-stop "finding host" bug.
  Future<void> _checkCurrentHostHealth() async {
    if (_disposed || baseUrl.isEmpty) return;

    try {
      final timeout =
          baseUrl.contains('.onion') ? _torProbeTimeout : _probeTimeout;
      final sw = Stopwatch()..start();
      final response = await _clientFor(baseUrl)
          .get(Uri.parse('$baseUrl/health'))
          .timeout(timeout);
      sw.stop();
      final rtt = sw.elapsedMilliseconds;

      if (response.statusCode >= 200 && response.statusCode < 300) {
        _getHealth(baseUrl).recordSuccess(rtt);
        // Host responded OK — stay on it regardless of latency.
        // Slow is fine over Tor, dead is not.
        losLog('🔌 [HealthCheck] OK (${rtt}ms) ✓');
      } else {
        // HTTP error (4xx/5xx) → host unhealthy but don't switch yet
        _getHealth(baseUrl).recordFailure();
        final failures = _getHealth(baseUrl).consecutiveFailures;
        losLog('🔌 [HealthCheck] HTTP ${response.statusCode} '
            '(failure $failures/3)');
        // Only switch after 3+ consecutive failures (Tor can be unreliable)
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
  /// Called on startup (initial discovery) and when current host is degraded.
  /// NOT called periodically — only triggered when needed.
  Future<void> probeAndSelectBestNode() async {
    if (_disposed || _bootstrapUrls.isEmpty) return;
    losLog(
        '📡 [Probe] Searching for best host across ${_bootstrapUrls.length} node(s)...');

    final results = <String, int>{};

    // Probe in batches. 4 concurrent Tor circuits is safe for SOCKS5.
    // Previously 2 — caused 120s+ waits with 8 nodes.
    const maxConcurrent = 4;
    final nodesToProbe = _bootstrapUrls.where((url) {
      final health = _nodeHealthMap[url];
      if (health != null && health.isInCooldown) {
        losLog(
            '📡 [Probe] $url — skipped (cooldown, ${health.consecutiveFailures} failures)');
        return false;
      }
      return true;
    }).toList();

    for (var i = 0; i < nodesToProbe.length; i += maxConcurrent) {
      final batch = nodesToProbe.skip(i).take(maxConcurrent);
      final futures = batch.map((url) async {
        try {
          final timeout =
              url.contains('.onion') ? _torProbeTimeout : _probeTimeout;
          final sw = Stopwatch()..start();
          final response = await _clientFor(url)
              .get(Uri.parse('$url/health'))
              .timeout(timeout);
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

  /// Select the appropriate HTTP client for a given URL.
  /// .onion URLs → _client (Tor SOCKS5 proxy)
  /// Clearnet URLs → _directClient (plain HTTP, no proxy)
  /// Sending clearnet requests through SOCKS5 causes
  /// SocksClientConnectionCommandFailedException.
  http.Client _clientFor(String url) {
    return url.contains('.onion') ? _client : _directClient;
  }

  // ══════════════════════════════════════════════════════════════════
  //  FAILOVER
  // ══════════════════════════════════════════════════════════════════

  /// Switch to the next available node, skipping nodes in cooldown
  /// and .onion URLs when Tor is unavailable.
  bool _switchToNextNode() {
    if (_bootstrapUrls.length <= 1) return false;
    final startIndex = _currentNodeIndex;
    // First pass — try clearnet nodes (fast, no Tor dependency).
    // Second pass — try .onion nodes (slow, 30-45s timeout each).
    // This prevents wasting minutes on unreachable .onion before trying localhost.
    for (final preferClearnet in [true, false]) {
      _currentNodeIndex = startIndex;
      do {
        _currentNodeIndex = (_currentNodeIndex + 1) % _bootstrapUrls.length;
        final candidate = _bootstrapUrls[_currentNodeIndex];
        // Skip .onion URLs if we don't have a Tor client
        if (!_hasTor && candidate.contains('.onion')) continue;
        // Skip nodes in cooldown (recently failed, exponential backoff)
        if (_getHealth(candidate).isInCooldown) continue;
        final isOnion = candidate.contains('.onion');
        if (preferClearnet && isOnion) continue;
        if (!preferClearnet && !isOnion) continue;
        if (candidate != baseUrl) {
          baseUrl = candidate;
          losLog(
              '🔄 Failover: switched to node ${_currentNodeIndex + 1}/${_bootstrapUrls.length}: $baseUrl');
          onNodeSwitched?.call(baseUrl);
          return true;
        }
      } while (_currentNodeIndex != startIndex);
    }
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

  /// Check if all candidate nodes are in cooldown.
  bool _allNodesInCooldown() {
    for (final url in _bootstrapUrls) {
      if (!_hasTor && url.contains('.onion')) continue;
      if (!_getHealth(url).isInCooldown) return false;
    }
    return true;
  }

  /// Simple round-robin switch — no cooldown checks. Used as last resort.
  bool _switchToNextNodeNoCooldown() {
    if (_bootstrapUrls.length <= 1) return false;
    final startIndex = _currentNodeIndex;
    do {
      _currentNodeIndex = (_currentNodeIndex + 1) % _bootstrapUrls.length;
      final candidate = _bootstrapUrls[_currentNodeIndex];
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
  /// Handles: timeouts, connection errors, AND HTTP 5xx server errors.
  Future<http.Response> _requestWithFailover(
    Future<http.Response> Function(String url) requestFn,
    String endpoint,
  ) async {
    await ensureReady();
    int attempts = 0;
    final maxAttempts = _bootstrapUrls.length.clamp(1, _maxRetries);
    while (attempts < maxAttempts) {
      try {
        final sw = Stopwatch()..start();
        final response = await requestFn(baseUrl).timeout(_timeout);
        sw.stop();

        // Record successful RTT for this node
        _getHealth(baseUrl).recordSuccess(sw.elapsedMilliseconds);

        // HTTP 5xx = server error → treat as node failure, try next
        if (response.statusCode >= 500) {
          losLog(
              '⚠️ Node ${_currentNodeIndex + 1} returned HTTP ${response.statusCode} for $endpoint');
          _getHealth(baseUrl).recordFailure();
          final isLastAttempt = attempts >= maxAttempts - 1;
          if (isLastAttempt) return response; // Return as-is, let caller handle
          _switchToNextNode();
          attempts++;
          continue;
        }

        // Trigger initial discovery + latency probes (once)
        if (!_initialDiscoveryDone) {
          _initialDiscoveryDone = true;
          Future.microtask(() => _runInitialDiscovery());
        }

        return response;
      } catch (e) {
        // Catch Error too (e.g. RangeError from SOCKS5 .onion failures)
        _getHealth(baseUrl).recordFailure();
        final isLastAttempt = attempts >= maxAttempts - 1;
        losLog('⚠️ Node ${_currentNodeIndex + 1} failed for $endpoint: $e');
        if (isLastAttempt) {
          losLog('❌ All ${attempts + 1} bootstrap nodes failed for $endpoint');
          rethrow;
        }
        _switchToNextNode();
        attempts++;
        losLog('🔄 Retrying $endpoint on node ${_currentNodeIndex + 1}...');
      }
    }
    throw Exception('All bootstrap nodes unreachable for $endpoint');
  }

  /// Runs once after first successful API response:
  /// 1. Discover peers from network
  /// 2. Probe all nodes for latency
  /// 3. Select the best (fastest) node
  /// 4. Start periodic background timers
  Future<void> _runInitialDiscovery() async {
    if (_disposed) return;
    try {
      await discoverAndSavePeers();
      await probeAndSelectBestNode();
    } catch (e) {
      losLog('⚠️ Initial discovery/probe failed (non-critical): $e');
    }
    // Start periodic background timers
    _startBackgroundTimers();
  }

  /// Start recurring background tasks:
  /// - Re-discover peers every 5 minutes
  /// - Health-check current host every 2 minutes (NOT full probe)
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
  /// Instead of probing all nodes, just try the NEXT one after 3+ failures.
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

  /// Get appropriate timeout for the current baseUrl.
  /// Clearnet (localhost/IP) = 8s, .onion = 45s.
  Duration get _timeout =>
      baseUrl.contains('.onion') ? _torTimeout : _defaultTimeout;

  /// Initialize HTTP client — ALWAYS attempts Tor first (even for localhost),
  /// so we can reach .onion bootstrap peers during failover.
  /// Previously only created Tor client if initial baseUrl was .onion,
  /// which meant starting on localhost = no Tor ever = .onion peers unreachable.
  Future<void> _initializeClient() async {
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

    // Race all bootstrap nodes in parallel to find the first live one.
    // This turns O(N × 30s) sequential failover into O(min_latency).
    await _raceForFirstNode();
  }

  /// Race ALL bootstrap nodes in parallel for /health.
  /// First node to respond with HTTP 2xx wins → becomes baseUrl.
  /// This eliminates the 30-45s × N sequential timeout cascade
  /// that occurs when most nodes are offline.
  ///
  /// Example: 100 nodes, 1 alive → old: up to 99 × 30s = 49 min.
  ///          With race: ~3-30s (time for the 1 alive node to respond).
  Future<void> _raceForFirstNode() async {
    final candidates = _bootstrapUrls.where((url) {
      if (!_hasTor && url.contains('.onion')) return false;
      if (_getHealth(url).isInCooldown) return false;
      return true;
    }).toList();

    if (candidates.isEmpty) return;
    if (candidates.length == 1) {
      baseUrl = candidates.first;
      return;
    }

    losLog('🏁 [Race] Racing ${candidates.length} node(s) for /health...');

    final completer = Completer<String?>();
    int failCount = 0;

    for (final url in candidates) {
      // Fire all probes simultaneously — each runs independently
      () async {
        try {
          final timeout =
              url.contains('.onion') ? _torProbeTimeout : _probeTimeout;
          final sw = Stopwatch()..start();
          final response = await _clientFor(url)
              .get(Uri.parse('$url/health'))
              .timeout(timeout);
          sw.stop();

          if (response.statusCode >= 200 &&
              response.statusCode < 300 &&
              !completer.isCompleted) {
            _getHealth(url).recordSuccess(sw.elapsedMilliseconds);
            completer.complete(url);
            losLog(
                '🏁 [Race] Winner: $url (${sw.elapsedMilliseconds}ms)');
          } else {
            _getHealth(url).recordFailure();
            failCount++;
            if (failCount >= candidates.length && !completer.isCompleted) {
              completer.complete(null);
            }
          }
        } catch (e) {
          _getHealth(url).recordFailure();
          failCount++;
          if (failCount >= candidates.length && !completer.isCompleted) {
            completer.complete(null);
          }
        }
      }();
    }

    // Global timeout — don't block startup forever
    Future.delayed(const Duration(seconds: 60), () {
      if (!completer.isCompleted) {
        losLog('🏁 [Race] Global timeout — no nodes responded in 60s');
        completer.complete(null);
      }
    });

    final winner = await completer.future;
    if (winner != null) {
      baseUrl = winner;
      _currentNodeIndex =
          _bootstrapUrls.indexOf(winner).clamp(0, _bootstrapUrls.length - 1);
      losLog('🏁 [Race] baseUrl set to $winner');
    } else {
      losLog('🏁 [Race] No responsive nodes — keeping default: $baseUrl');
    }
  }

  /// Create Tor-enabled HTTP client.
  /// Uses the shared TorService.start() so _isRunning is properly synced.
  ///
  /// If Tor is unavailable or broken, returns a plain HTTP client
  /// with _hasTor=false so failover can skip .onion URLs gracefully.
  Future<http.Client> _createTorClient() async {
    // Always go through start() — it detects existing SOCKS5 proxy internally
    // AND sets _isRunning=true, which is critical for shared TorService state.
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
      [
        ProxySettings(InternetAddress.loopbackIPv4, socksPort),
      ],
    );

    httpClient.connectionTimeout = const Duration(seconds: 30);
    httpClient.idleTimeout = const Duration(seconds: 30);

    _hasTor = true;
    losLog('✅ Tor SOCKS5 proxy configured (localhost:$socksPort)');
    return IOClient(httpClient);
  }

  // Switch network environment
  void switchEnvironment(NetworkEnvironment newEnv) {
    _loadBootstrapUrls(newEnv);

    // Mainnet guard: refuse switch if no mainnet nodes are configured
    if (newEnv == NetworkEnvironment.mainnet && _bootstrapUrls.isEmpty) {
      losLog('🚫 Cannot switch to mainnet: no bootstrap nodes configured');
      // Revert to testnet
      _loadBootstrapUrls(NetworkEnvironment.testnet);
      throw StateError(
        'Mainnet has not launched yet. No bootstrap nodes available.',
      );
    }

    environment = newEnv;
    // Sync badge/UI config so NetworkBadge reflects the runtime choice
    if (newEnv == NetworkEnvironment.mainnet) {
      WalletConfig.useMainnet();
    } else {
      WalletConfig.useFunctionalTestnet();
    }
    // Sync mainnet mode to WalletService so Ed25519
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
        (url) => _clientFor(url).get(Uri.parse('$url/node-info')),
        '/node-info',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        losLog(
            '🌐 [ApiService.getNodeInfo] SUCCESS block_height=${data['block_height'] ?? data['protocol']?['block_height'] ?? 'N/A'}');
        return data;
      }
      throw Exception('Failed to get node info: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getNodeInfo error: $e');
      rethrow;
    }
  }

  /// Fetch fee estimate for the NEXT transaction from [address].
  /// Returns the estimated fee in CIL (flat BASE_FEE_CIL).
  /// Wallet MUST call this before constructing a signed block.
  Future<Map<String, dynamic>> getFeeEstimate(String address) async {
    losLog(
        '🌐 [ApiService.getFeeEstimate] Fetching fee estimate for $address...');
    try {
      final response = await _requestWithFailover(
        (url) => _clientFor(url).get(Uri.parse('$url/fee-estimate/$address')),
        '/fee-estimate/$address',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        losLog(
            '🌐 [ApiService.getFeeEstimate] SUCCESS fee=${data['estimated_fee_cil']} CIL');
        return data;
      }
      throw Exception('Failed to get fee estimate: ${response.statusCode}');
    } catch (e) {
      losLog('⚠️ getFeeEstimate error: $e');
      // Re-throw so callers can show the user a proper error
      // instead of silently using a stale hardcoded fee.
      rethrow;
    }
  }

  // Health Check
  Future<Map<String, dynamic>> getHealth() async {
    losLog('🌐 [ApiService.getHealth] Fetching health...');
    try {
      final response = await _requestWithFailover(
        (url) => _clientFor(url).get(Uri.parse('$url/health')),
        '/health',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        losLog('🌐 [ApiService.getHealth] SUCCESS');
        return data;
      }
      throw Exception('Failed to get health: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getHealth error: $e');
      rethrow;
    }
  }

  // Get Balance
  Future<Account> getBalance(String address) async {
    losLog('🌐 [ApiService.getBalance] Fetching balance for $address...');
    try {
      final response = await _requestWithFailover(
        (url) => _clientFor(url).get(Uri.parse('$url/bal/$address')),
        '/bal/$address',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        losLog(
            '🌐 [ApiService.getBalance] SUCCESS balance=${data['balance_cil'] ?? 0} CIL');
        // Prefer balance_cil_str (string) over balance_cil (number) for
        // JSON precision safety: numbers > 2^53 may lose precision in JSON parsing.
        int balanceCil;
        if (data['balance_cil_str'] != null) {
          balanceCil = int.tryParse(data['balance_cil_str'].toString()) ?? 0;
        } else {
          balanceCil = data['balance_cil'] ?? 0;
        }
        return Account(
          address: address,
          balance: balanceCil,
          cilBalance: 0,
          history: [],
          headBlock: data['head']?.toString(),
          blockCount: data['block_count'] ?? 0,
        );
      }
      throw Exception('Failed to get balance: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getBalance error: $e');
      rethrow;
    }
  }

  // Get Account (with history)
  Future<Account> getAccount(String address) async {
    losLog('🌐 [ApiService.getAccount] Fetching account for $address...');
    try {
      final response = await _requestWithFailover(
        (url) => _clientFor(url).get(Uri.parse('$url/account/$address')),
        '/account/$address',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        // Backend /account/:address may not include 'address' field — inject it
        if (data is Map<String, dynamic> && !data.containsKey('address')) {
          data['address'] = address;
        }
        losLog('🌐 [ApiService.getAccount] SUCCESS for $address');
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
    losLog('🚠 [API] requestFaucet -> $baseUrl/faucet  address=$address');
    try {
      final response = await _requestWithFailover(
        (url) => _clientFor(url).post(
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
        losLog('🚠 [API] faucet FAILED: ${data['msg']}');
        throw Exception(data['msg'] ?? 'Faucet request failed');
      }

      losLog('🚠 [API] faucet SUCCESS: $data');
      return data;
    } catch (e) {
      losLog('❌ requestFaucet error: $e');
      rethrow;
    }
  }

  // Send Transaction
  // Supports optional Dilithium5 signature + public_key for L2+/mainnet
  // Supports optional previous (frontier) + work (PoW nonce) for client-signed blocks
  Future<Map<String, dynamic>> sendTransaction({
    required String from,
    required String to,
    required int amount,
    String? signature,
    String? publicKey,
    String? previous,
    int? work,
    int? timestamp,
    int? fee,
    int?
        amountCil, // Amount already in CIL (for sub-LOS precision). Backend expects u128 integer.
  }) async {
    losLog(
        '💸 [API] sendTransaction -> $baseUrl/send  from=$from to=$to amount=$amount sig=${signature != null}');
    try {
      final body = <String, dynamic>{
        'from': from,
        'target': to,
        'amount': amount,
      };
      // If amount_cil is provided, send as integer.
      // Backend deserializes as u128 — Dart int (2^63-1 max) is sufficient
      // since total supply CIL = 2.19e18 < 9.22e18 (i64 max).
      if (amountCil != null) {
        body['amount_cil'] = amountCil;
      }
      // Attach Dilithium5 signature + public key if available (L2+/mainnet)
      if (signature != null && publicKey != null) {
        body['signature'] = signature;
        body['public_key'] = publicKey;
      }
      // Attach frontier hash + PoW nonce if client-constructed
      if (previous != null) {
        body['previous'] = previous;
      }
      if (work != null) {
        body['work'] = work;
      }
      // Attach timestamp and fee for client-signed blocks (part of signing_hash)
      if (timestamp != null) {
        body['timestamp'] = timestamp;
      }
      if (fee != null) {
        body['fee'] = fee;
      }

      final response = await _requestWithFailover(
        (url) => _clientFor(url).post(
          Uri.parse('$url/send'),
          headers: {'Content-Type': 'application/json'},
          body: json.encode(body),
        ),
        '/send',
      );

      final data = json.decode(response.body);

      // Critical: Check BOTH status code AND response body status
      // Backend returns 200 on success, >= 400 on errors
      if (response.statusCode >= 400 || data['status'] == 'error') {
        losLog('💸 [API] send FAILED: ${data['msg']}');
        throw Exception(data['msg'] ?? 'Transaction failed');
      }

      losLog('💸 [API] send SUCCESS: ${data['tx_hash'] ?? data['txid']}');
      return data;
    } catch (e) {
      losLog('❌ sendTransaction error: $e');
      rethrow;
    }
  }

  // Get Validators
  Future<List<ValidatorInfo>> getValidators() async {
    losLog('🌐 [ApiService.getValidators] Fetching validators...');
    try {
      final response = await _requestWithFailover(
        (url) => _clientFor(url).get(Uri.parse('$url/validators')),
        '/validators',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final decoded = json.decode(response.body);
        // Handle both {"validators": [...]} wrapper and bare [...]
        final List<dynamic> data = decoded is List
            ? decoded
            : (decoded['validators'] as List<dynamic>?) ?? [];
        final validators = data.map((v) => ValidatorInfo.fromJson(v)).toList();
        losLog(
            '🌐 [ApiService.getValidators] SUCCESS count=${validators.length}');
        return validators;
      }
      throw Exception('Failed to get validators: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getValidators error: $e');
      rethrow;
    }
  }

  // Get Latest Block — uses /blocks/recent endpoint which returns timestamp
  Future<BlockInfo> getLatestBlock() async {
    losLog('🌐 [ApiService.getLatestBlock] Fetching latest block...');
    try {
      final response = await _requestWithFailover(
        (url) => _clientFor(url).get(Uri.parse('$url/blocks/recent')),
        '/blocks/recent',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final decoded = json.decode(response.body);
        final List<dynamic> blocks = decoded is List
            ? decoded
            : (decoded['blocks'] as List<dynamic>?) ?? [];
        if (blocks.isNotEmpty) {
          final block = BlockInfo.fromJson(blocks[0] as Map<String, dynamic>);
          losLog(
              '🌐 [ApiService.getLatestBlock] SUCCESS height=${block.height}');
          return block;
        }
        // No blocks yet — return empty sentinel
        return BlockInfo(height: 0, hash: '', timestamp: 0, txCount: 0);
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
        (url) => _clientFor(url).get(Uri.parse('$url/blocks/recent')),
        '/blocks/recent',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final decoded = json.decode(response.body);
        // Handle both {"blocks": [...]} wrapper and bare [...]
        final List<dynamic> data = decoded is List
            ? decoded
            : (decoded['blocks'] as List<dynamic>?) ?? [];
        final blocks = data.map((b) => BlockInfo.fromJson(b)).toList();
        losLog(
            '🌐 [ApiService.getRecentBlocks] SUCCESS count=${blocks.length}');
        return blocks;
      }
      throw Exception('Failed to get recent blocks: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getRecentBlocks error: $e');
      rethrow;
    }
  }

  // Get Peers
  // Backend returns {"peers": [{...}], "peer_count": N, ...}
  Future<List<String>> getPeers() async {
    losLog('🌐 [ApiService.getPeers] Fetching peers...');
    try {
      final response = await _requestWithFailover(
        (url) => _clientFor(url).get(Uri.parse('$url/peers')),
        '/peers',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final decoded = json.decode(response.body);
        List<String> peers;
        if (decoded is Map) {
          // New format: {"peers": [{"address": "...", ...}], "peer_count": N}
          if (decoded.containsKey('peers') && decoded['peers'] is List) {
            peers = (decoded['peers'] as List)
                .map((p) =>
                    p is Map ? (p['address'] ?? '').toString() : p.toString())
                .where((s) => s.isNotEmpty)
                .toList();
          } else {
            // Legacy fallback: flat HashMap<String, String>
            peers = decoded.keys.cast<String>().toList();
          }
        } else if (decoded is List) {
          peers = decoded.whereType<String>().toList();
        } else {
          peers = [];
        }
        losLog('🌐 [ApiService.getPeers] SUCCESS count=${peers.length}');
        return peers;
      }
      throw Exception('Failed to get peers: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getPeers error: $e');
      rethrow;
    }
  }

  // Get Transaction History for address
  Future<List<Transaction>> getHistory(String address) async {
    losLog('🌐 [ApiService.getHistory] Fetching history for $address...');
    try {
      final response = await _requestWithFailover(
        (url) => _clientFor(url).get(Uri.parse('$url/history/$address')),
        '/history/$address',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        // Backend returns {"transactions": [...]} wrapper,
        // not a bare array. Handle both formats for resilience.
        final decoded = json.decode(response.body);
        final List<dynamic> data = decoded is List
            ? decoded
            : (decoded['transactions'] as List<dynamic>?) ?? [];
        final txList = data.map((tx) => Transaction.fromJson(tx)).toList();
        losLog('🌐 [ApiService.getHistory] SUCCESS count=${txList.length}');
        return txList;
      }
      throw Exception('Failed to get history: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getHistory error: $e');
      rethrow;
    }
  }

  // ══════════════════════════════════════════════════════════════════
  //  ADDITIONAL API ENDPOINTS
  // ══════════════════════════════════════════════════════════════════

  /// Get supply information: remaining supply, circulating supply.
  Future<Map<String, dynamic>> getSupply() async {
    losLog('🌐 [ApiService.getSupply] Fetching supply info...');
    try {
      final response = await _requestWithFailover(
        (url) => _clientFor(url).get(Uri.parse('$url/supply')),
        '/supply',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        losLog('🌐 [ApiService.getSupply] SUCCESS');
        return data;
      }
      throw Exception('Failed to get supply: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getSupply error: $e');
      rethrow;
    }
  }

  /// Look up a specific transaction by its hash.
  /// Returns: { status, transaction: { hash, from, to, type, amount, amount_cil, timestamp, signature, confirmed } }
  Future<Map<String, dynamic>> getTransaction(String hash) async {
    losLog('🌐 [ApiService.getTransaction] Fetching transaction $hash...');
    try {
      final response = await _requestWithFailover(
        (url) => _clientFor(url).get(Uri.parse('$url/transaction/$hash')),
        '/transaction/$hash',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        losLog('🌐 [ApiService.getTransaction] SUCCESS');
        return data;
      }
      throw Exception('Failed to get transaction: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getTransaction error: $e');
      rethrow;
    }
  }

  /// Look up a specific block by its hash.
  /// Returns: { status, block: { hash, account, previous, type, amount, amount_cil, ... } }
  Future<Map<String, dynamic>> getBlock(String hash) async {
    losLog('🌐 [ApiService.getBlock] Fetching block $hash...');
    try {
      final response = await _requestWithFailover(
        (url) => _clientFor(url).get(Uri.parse('$url/block/$hash')),
        '/block/$hash',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        losLog('🌐 [ApiService.getBlock] SUCCESS');
        return data;
      }
      throw Exception('Failed to get block: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getBlock error: $e');
      rethrow;
    }
  }

  /// Search for addresses, transactions, or blocks by query string.
  /// Returns: { query, results: [{ type, address, balance, block_count }], count }
  Future<Map<String, dynamic>> search(String query) async {
    losLog('🌐 [ApiService.search] Searching for "$query"...');
    try {
      final response = await _requestWithFailover(
        (url) => _clientFor(url).get(Uri.parse('$url/search/$query')),
        '/search/$query',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        losLog('🌐 [ApiService.search] SUCCESS count=${data['count'] ?? 0}');
        return data;
      }
      throw Exception('Failed to search: ${response.statusCode}');
    } catch (e) {
      losLog('❌ search error: $e');
      rethrow;
    }
  }

  /// Get network consensus status: protocol info, safety metrics, finality times.
  Future<Map<String, dynamic>> getConsensus() async {
    losLog('🌐 [ApiService.getConsensus] Fetching consensus...');
    try {
      final response = await _requestWithFailover(
        (url) => _clientFor(url).get(Uri.parse('$url/consensus')),
        '/consensus',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        losLog('🌐 [ApiService.getConsensus] SUCCESS');
        return data;
      }
      throw Exception('Failed to get consensus: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getConsensus error: $e');
      rethrow;
    }
  }

  /// Get reward pool information: epoch, distribution, validator eligibility.
  Future<Map<String, dynamic>> getRewardInfo() async {
    losLog('🌐 [ApiService.getRewardInfo] Fetching reward info...');
    try {
      final response = await _requestWithFailover(
        (url) => _clientFor(url).get(Uri.parse('$url/reward-info')),
        '/reward-info',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        losLog('🌐 [ApiService.getRewardInfo] SUCCESS');
        return data;
      }
      throw Exception('Failed to get reward info: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getRewardInfo error: $e');
      rethrow;
    }
  }

  // ====================================================================
  // USP-01 TOKEN API
  // ====================================================================

  /// List all deployed USP-01 tokens
  Future<List<Token>> getTokens() async {
    losLog('🪙 [API] getTokens...');
    try {
      final response = await _requestWithFailover(
        (url) => _clientFor(url).get(Uri.parse('$url/tokens')),
        '/tokens',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        final tokens = (data['tokens'] as List?)
                ?.map((t) => Token.fromJson(t as Map<String, dynamic>))
                .toList() ??
            [];
        losLog('🪙 [API] getTokens: ${tokens.length} tokens');
        return tokens;
      }
      throw Exception('Failed to get tokens: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getTokens error: $e');
      rethrow;
    }
  }

  /// Get USP-01 token metadata
  Future<Token> getToken(String contractAddress) async {
    losLog('🪙 [API] getToken: $contractAddress');
    try {
      final response = await _requestWithFailover(
        (url) => _clientFor(url).get(Uri.parse('$url/token/$contractAddress')),
        '/token/$contractAddress',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        return Token.fromJson(data['token'] as Map<String, dynamic>);
      }
      throw Exception('Failed to get token: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getToken error: $e');
      rethrow;
    }
  }

  /// Get USP-01 token balance for a holder
  Future<TokenBalance> getTokenBalance(
      String contractAddress, String holder) async {
    losLog('🪙 [API] getTokenBalance: $contractAddress / $holder');
    try {
      final response = await _requestWithFailover(
        (url) => _clientFor(url)
            .get(Uri.parse('$url/token/$contractAddress/balance/$holder')),
        '/token/$contractAddress/balance/$holder',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        return TokenBalance.fromJson(data as Map<String, dynamic>);
      }
      throw Exception('Failed to get token balance: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getTokenBalance error: $e');
      rethrow;
    }
  }

  /// Get USP-01 token allowance
  Future<TokenAllowance> getTokenAllowance(
      String contractAddress, String owner, String spender) async {
    losLog('🪙 [API] getTokenAllowance: $contractAddress / $owner → $spender');
    try {
      final response = await _requestWithFailover(
        (url) => _clientFor(url).get(
            Uri.parse('$url/token/$contractAddress/allowance/$owner/$spender')),
        '/token/$contractAddress/allowance/$owner/$spender',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        return TokenAllowance.fromJson(data as Map<String, dynamic>);
      }
      throw Exception('Failed to get allowance: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getTokenAllowance error: $e');
      rethrow;
    }
  }

  /// Call a smart contract function (USP-01 or DEX).
  /// Used for: transfer, approve, burn, swap, create_pool, add/remove liquidity.
  Future<Map<String, dynamic>> callContract({
    required String contractAddress,
    required String function,
    required List<String> args,
    String? caller,
    int? gasLimit,
    int? amountCil,
    String? signature,
    String? publicKey,
    String? previous,
    int? work,
    int? timestamp,
    int? fee,
  }) async {
    losLog(
        '📝 [API] callContract: $contractAddress.$function(${args.join(", ")})');
    try {
      final body = <String, dynamic>{
        'contract_address': contractAddress,
        'function': function,
        'args': args,
      };
      if (caller != null) body['caller'] = caller;
      if (gasLimit != null) body['gas_limit'] = gasLimit;
      if (amountCil != null) body['amount_cil'] = amountCil;
      if (signature != null) body['signature'] = signature;
      if (publicKey != null) body['public_key'] = publicKey;
      if (previous != null) body['previous'] = previous;
      if (work != null) body['work'] = work;
      if (timestamp != null) body['timestamp'] = timestamp;
      if (fee != null) body['fee'] = fee;

      final response = await _requestWithFailover(
        (url) => _clientFor(url).post(
          Uri.parse('$url/call-contract'),
          headers: {'Content-Type': 'application/json'},
          body: json.encode(body),
        ),
        '/call-contract',
      );
      final data = json.decode(response.body) as Map<String, dynamic>;
      if (response.statusCode >= 400 || data['status'] == 'error') {
        throw Exception(data['msg'] ?? data['error'] ?? 'Contract call failed');
      }
      losLog('📝 [API] callContract SUCCESS');
      return data;
    } catch (e) {
      losLog('❌ callContract error: $e');
      rethrow;
    }
  }

  /// Deploy a WASM smart contract
  Future<Map<String, dynamic>> deployContract({
    required String owner,
    required String bytecode,
    Map<String, dynamic>? initialState,
    int? amountCil,
    String? signature,
    String? publicKey,
    String? previous,
    int? work,
    int? timestamp,
    int? fee,
  }) async {
    losLog('🚀 [API] deployContract: owner=$owner');
    try {
      final body = <String, dynamic>{
        'owner': owner,
        'bytecode': bytecode,
      };
      if (initialState != null) body['initial_state'] = initialState;
      if (amountCil != null) body['amount_cil'] = amountCil;
      if (signature != null) body['signature'] = signature;
      if (publicKey != null) body['public_key'] = publicKey;
      if (previous != null) body['previous'] = previous;
      if (work != null) body['work'] = work;
      if (timestamp != null) body['timestamp'] = timestamp;
      if (fee != null) body['fee'] = fee;

      final response = await _requestWithFailover(
        (url) => _clientFor(url).post(
          Uri.parse('$url/deploy-contract'),
          headers: {'Content-Type': 'application/json'},
          body: json.encode(body),
        ),
        '/deploy-contract',
      );
      final data = json.decode(response.body) as Map<String, dynamic>;
      if (response.statusCode >= 400 || data['status'] == 'error') {
        throw Exception(data['msg'] ?? data['error'] ?? 'Deploy failed');
      }
      losLog('🚀 [API] deployContract SUCCESS: ${data['contract_address']}');
      return data;
    } catch (e) {
      losLog('❌ deployContract error: $e');
      rethrow;
    }
  }

  // ====================================================================
  // DEX AMM API
  // ====================================================================

  /// List all DEX pools across all contracts
  Future<List<DexPool>> getDexPools() async {
    losLog('📊 [API] getDexPools...');
    try {
      final response = await _requestWithFailover(
        (url) => _clientFor(url).get(Uri.parse('$url/dex/pools')),
        '/dex/pools',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        final pools = (data['pools'] as List?)
                ?.map((p) => DexPool.fromJson(p as Map<String, dynamic>))
                .toList() ??
            [];
        losLog('📊 [API] getDexPools: ${pools.length} pools');
        return pools;
      }
      throw Exception('Failed to get DEX pools: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getDexPools error: $e');
      rethrow;
    }
  }

  /// Get specific pool info
  Future<DexPool> getDexPool(String contractAddress, String poolId) async {
    losLog('📊 [API] getDexPool: $contractAddress/$poolId');
    try {
      final response = await _requestWithFailover(
        (url) => _clientFor(url)
            .get(Uri.parse('$url/dex/pool/$contractAddress/$poolId')),
        '/dex/pool/$contractAddress/$poolId',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        return DexPool.fromJson(
          data['pool'] as Map<String, dynamic>,
          contract: contractAddress,
        );
      }
      throw Exception('Failed to get pool: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getDexPool error: $e');
      rethrow;
    }
  }

  /// Get swap quote (estimated output, no execution)
  Future<DexQuote> getDexQuote(String contractAddress, String poolId,
      String tokenIn, String amountIn) async {
    losLog('📊 [API] getDexQuote: $contractAddress/$poolId/$tokenIn/$amountIn');
    try {
      final response = await _requestWithFailover(
        (url) => _clientFor(url).get(Uri.parse(
            '$url/dex/quote/$contractAddress/$poolId/$tokenIn/$amountIn')),
        '/dex/quote/$contractAddress/$poolId/$tokenIn/$amountIn',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        return DexQuote.fromJson(data['quote'] as Map<String, dynamic>);
      }
      throw Exception('Failed to get quote: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getDexQuote error: $e');
      rethrow;
    }
  }

  /// Get user's LP position in a pool
  Future<LpPosition> getDexPosition(
      String contractAddress, String poolId, String user) async {
    losLog('📊 [API] getDexPosition: $contractAddress/$poolId/$user');
    try {
      final response = await _requestWithFailover(
        (url) => _clientFor(url)
            .get(Uri.parse('$url/dex/position/$contractAddress/$poolId/$user')),
        '/dex/position/$contractAddress/$poolId/$user',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        return LpPosition.fromJson(data as Map<String, dynamic>);
      }
      throw Exception('Failed to get position: ${response.statusCode}');
    } catch (e) {
      losLog('❌ getDexPosition error: $e');
      rethrow;
    }
  }

  // ─── Smart Contract Browsing ─────────────────────────────────────

  /// Get details of a deployed smart contract by address.
  Future<Map<String, dynamic>> getContract(String address) async {
    losLog('📜 [API] getContract: $address');
    try {
      final response = await _requestWithFailover(
        (url) => _clientFor(url).get(Uri.parse('$url/contract/$address')),
        '/contract/$address',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        return json.decode(response.body) as Map<String, dynamic>;
      }
      return {};
    } catch (e) {
      losLog('📜 getContract error: $e');
      return {};
    }
  }

  /// List all deployed contracts on the network.
  Future<List<Map<String, dynamic>>> getContracts() async {
    losLog('📜 [API] getContracts...');
    try {
      final response = await _requestWithFailover(
        (url) => _clientFor(url).get(Uri.parse('$url/contracts')),
        '/contracts',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final data = json.decode(response.body);
        if (data is List) {
          return data.cast<Map<String, dynamic>>();
        }
      }
      return [];
    } catch (e) {
      losLog('📜 getContracts error: $e');
      return [];
    }
  }

  /// Get mempool statistics.
  Future<Map<String, dynamic>> getMempoolStats() async {
    losLog('📊 [API] getMempoolStats...');
    try {
      final response = await _requestWithFailover(
        (url) => _clientFor(url).get(Uri.parse('$url/mempool/stats')),
        '/mempool/stats',
      );
      if (response.statusCode >= 200 && response.statusCode < 300) {
        return json.decode(response.body) as Map<String, dynamic>;
      }
      return {};
    } catch (e) {
      losLog('📊 getMempoolStats error: $e');
      return {};
    }
  }

  /// Cleanup: stop bundled Tor and cancel background timers
  Future<void> dispose() async {
    losLog('🌐 [ApiService.dispose] Disposing...');
    _disposed = true;
    _rediscoveryTimer?.cancel();
    _healthCheckTimer?.cancel();
    _client.close();
    _directClient.close();
    await _torService.stop();
    losLog('🌐 [ApiService.dispose] Disposed');
  }

  /// Discover new validator endpoints from the network and save locally.
  /// Called periodically (every 5 minutes) to maintain an up-to-date peer table.
  /// Filters out the validator's own onion address if set.
  Future<void> discoverAndSavePeers() async {
    if (_disposed) return; // Client already closed — skip silently
    try {
      final response = await _requestWithFailover(
        (url) => _clientFor(url).get(Uri.parse('$url/network/peers')),
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
            // Use host_address (new field) with onion_address as fallback
            final host = (ep['host_address']?.toString() ?? '').isNotEmpty
                ? ep['host_address'].toString()
                : ep['onion_address']?.toString() ?? '';
            if (host.isEmpty) continue;

            final isOnion = host.contains('.onion');

            // Extract bare hostname (strip any embedded port/scheme)
            final hostname = host.contains(':') ? host.split(':').first : host;

            // DEDUP FIX: Check if this hostname already exists in
            // _bootstrapUrls (which have correct ports from NetworkConfig).
            // Prevents adding "http://x.onion" when "http://x.onion:3030"
            // is already present — the root cause of port-less duplicates.
            final alreadyKnown = _bootstrapUrls.any((existing) {
              final uri = Uri.tryParse(existing);
              return uri?.host == hostname;
            });
            if (alreadyKnown) continue;

            // Skip .onion URLs when Tor is not available
            if (isOnion && !_hasTor) continue;

            // Build URL with rest_port if provided by the API
            final restPort = ep['rest_port'] as int?;

            // GHOST PEER FILTER: For .onion, require explicit rest_port.
            // Without it, we'd default to port 80 which is almost always
            // wrong — it's a P2P address from libp2p, not a REST endpoint.
            // This was causing ghost .onion entries that timeout for 30s each.
            if (isOnion && restPort == null) {
              losLog('🚫 Skipping .onion peer without rest_port: $hostname');
              continue;
            }

            // Skip P2P-only addresses (port 4xxx = libp2p gossip)
            final rawPort = host.contains(':')
                ? int.tryParse(host.split(':').last)
                : null;
            if (rawPort != null && rawPort >= 4000 && rawPort < 5000) {
              losLog('🚫 Skipping P2P-only address: $host');
              continue;
            }

            final port = restPort ?? rawPort;
            final url = (port != null && port != 80)
                ? 'http://$hostname:$port'
                : 'http://$hostname';

            // Exclude own onion (validator self-connection prevention)
            if (url == _excludedOnionUrl) continue;
            if (!_bootstrapUrls.contains(url)) {
              _bootstrapUrls.add(url);
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

  /// Get the name of the currently connected bootstrap node
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
}
