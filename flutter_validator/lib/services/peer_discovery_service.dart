import '../utils/log.dart';
import 'dart:convert';
import 'package:http/http.dart' as http;
import 'package:shared_preferences/shared_preferences.dart';

/// Persistent Peer Discovery Service — "Memory" for discovered nodes.
///
/// Saves validator endpoints (clearnet and/or .onion) discovered from the
/// network into SharedPreferences so the app can reconnect to known-good
/// peers even when all 4 bootstrap nodes are offline.
///
/// Flow:
///   1. App starts → loadSavedPeers() → get any previously-discovered peers
///   2. App connects to a node → discoverPeers(apiResponse) → merge new peers
///   3. On next startup → saved peers tried BEFORE bootstrap nodes
///
/// This is the LOS equivalent of Bitcoin's "peers.dat" — once you've
/// connected to the network once, you can find it again without bootstrap.
class PeerDiscoveryService {
  /// Network-aware storage key prevents testnet peers leaking into mainnet.
  /// Call [setNetwork] before any load/save to select the correct bucket.
  static String _network = 'mainnet';
  static String get _storageKey => 'los_discovered_peers_${_network}_v1';
  static const int _maxSavedPeers = 200;

  /// Set the active network ('mainnet' or 'testnet').
  /// Must be called once during ApiService init, before loadSavedPeers().
  static void setNetwork(String network) {
    _network = network;
    losLog('🔑 PeerDiscovery: storage bucket set to "$_storageKey"');
  }

  /// Load previously discovered validator endpoints from local storage.
  /// Returns a list of URLs: ["http://xyz.onion:3030", "http://1.2.3.4:7030", ...]
  /// Supports both .onion (Tor) and clearnet (IP/domain) endpoints.
  static Future<List<String>> loadSavedPeers() async {
    try {
      final prefs = await SharedPreferences.getInstance();
      final raw = prefs.getString(_storageKey);
      if (raw == null || raw.isEmpty) return [];

      final List<dynamic> decoded = json.decode(raw);
      final peers = decoded.whereType<Map<String, dynamic>>().where((p) {
        // Accept peers with either host_address or onion_address
        final host = p['host_address']?.toString() ?? '';
        final onion = p['onion_address']?.toString() ?? '';
        return host.isNotEmpty || onion.isNotEmpty;
      }).map((p) {
        // Prefer host_address (universal), fall back to onion_address (legacy)
        final host = (p['host_address']?.toString() ?? '').isNotEmpty
            ? p['host_address'] as String
            : p['onion_address'] as String;
        final restPort = p['rest_port'] as int?;
        // Already a full URL — return as-is
        if (host.contains('://')) return host;
        // Extract hostname (strip embedded port for URL building)
        final hostname = host.contains(':') ? host.split(':').first : host;
        // Use rest_port from saved data, or extract from host string, or default 80
        final port = restPort ??
            (host.contains(':')
                ? int.tryParse(host.split(':').last) ?? 80
                : 80);
        if (port != 80) {
          return 'http://$hostname:$port';
        }
        return 'http://$hostname';
      }).toList();

      losLog(
          '📚 PeerDiscovery: loaded ${peers.length} saved peer(s) from storage'
          ' (key: $_storageKey)');
      return peers;
    } catch (e) {
      losLog('⚠️ PeerDiscovery: failed to load saved peers: $e');
      return [];
    }
  }

  /// Save discovered validator endpoints to local storage.
  /// Called after a successful GET /network/peers response.
  /// Supports both clearnet and onion endpoints.
  ///
  /// [endpoints] should be the "endpoints" array from the API response:
  /// [{"address": "LOS...", "host_address": "1.2.3.4:7030", "onion_address": "xyz.onion",
  ///   "transport": "clearnet", "stake_los": 1000, "reachable": true}, ...]
  static Future<void> savePeers(List<Map<String, dynamic>> endpoints) async {
    try {
      if (endpoints.isEmpty) return;

      final prefs = await SharedPreferences.getInstance();

      // Load existing peers and merge (dedup by host_address)
      final existing = await _loadRawPeers(prefs);
      final merged = <String, Map<String, dynamic>>{};

      // Existing peers first (lower priority)
      for (final p in existing) {
        final key = (p['host_address']?.toString() ?? '').isNotEmpty
            ? p['host_address'].toString()
            : p['onion_address']?.toString() ?? '';
        if (key.isNotEmpty) merged[key] = p;
      }

      // New peers overwrite (higher priority — fresher data)
      for (final p in endpoints) {
        // Prefer host_address (works for both clearnet and onion)
        final hostAddr = p['host_address']?.toString() ?? '';
        final onionAddr = p['onion_address']?.toString() ?? '';
        final key = hostAddr.isNotEmpty ? hostAddr : onionAddr;
        if (key.isNotEmpty) {
          merged[key] = {
            'address': p['address'],
            'host_address': hostAddr.isNotEmpty ? hostAddr : onionAddr,
            'onion_address': onionAddr.isNotEmpty ? onionAddr : hostAddr,
            'transport': p['transport'] ??
                (key.contains('.onion') ? 'onion' : 'clearnet'),
            'stake_los': p['stake_los'] ?? 0,
            if (p['rest_port'] != null) 'rest_port': p['rest_port'],
            'last_seen': DateTime.now().millisecondsSinceEpoch,
          };
        }
      }

      // Cap at max peers (keep most-recently-seen)
      final sorted = merged.values.toList()
        ..sort((a, b) => (b['last_seen'] as int? ?? 0)
            .compareTo(a['last_seen'] as int? ?? 0));
      final capped = sorted.take(_maxSavedPeers).toList();

      await prefs.setString(_storageKey, json.encode(capped));
      losLog('💾 PeerDiscovery: saved ${capped.length} peer(s) to storage');
    } catch (e) {
      losLog('⚠️ PeerDiscovery: failed to save peers: $e');
    }
  }

  /// Get the count of saved peers (for UI display).
  static Future<int> getSavedPeerCount() async {
    try {
      final prefs = await SharedPreferences.getInstance();
      final raw = prefs.getString(_storageKey);
      if (raw == null || raw.isEmpty) return 0;
      final List<dynamic> decoded = json.decode(raw);
      return decoded.length;
    } catch (e) {
      losLog('⚠️ PeerDiscovery: getSavedPeerCount failed: $e');
      return 0;
    }
  }

  /// Clear all saved peers (for debugging/reset).
  static Future<void> clearSavedPeers() async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.remove(_storageKey);
    losLog('🗑️ PeerDiscovery: cleared all saved peers');
  }

  // ══════════════════════════════════════════════════════════════════
  //  CUSTOM PEER MANAGEMENT — "Add Custom Node" Feature
  // ══════════════════════════════════════════════════════════════════
  //
  // Custom peers are user-provided URLs stored separately from
  // auto-discovered peers. They survive app restarts AND take highest
  // priority during failover (loaded before both saved and bootstrap).
  //
  // This is the key to blockchain survival without bootstrap nodes:
  // if all 4 bootstrap nodes are offline, a new user can manually
  // add any known validator URL to join the network.
  // ══════════════════════════════════════════════════════════════════

  static String get _customPeerKey => 'los_custom_peers_${_network}_v1';

  /// Add a user-provided custom node URL.
  /// Returns true if added successfully, false if invalid or duplicate.
  static Future<bool> addCustomPeer(String rawUrl) async {
    try {
      // Normalize the URL
      String url = rawUrl.trim();
      if (url.isEmpty) return false;

      // Add http:// if no scheme provided
      if (!url.startsWith('http://') && !url.startsWith('https://')) {
        url = 'http://$url';
      }

      // Validate URL structure
      final uri = Uri.tryParse(url);
      if (uri == null || uri.host.isEmpty) return false;

      final prefs = await SharedPreferences.getInstance();
      final existing = await _loadCustomPeerList(prefs);

      // Dedup by hostname
      final existingHosts =
          existing.map((u) => Uri.tryParse(u)?.host ?? '').toSet();
      if (existingHosts.contains(uri.host)) {
        losLog('⚠️ PeerDiscovery: custom peer ${uri.host} already exists');
        return false;
      }

      existing.add(url);
      await prefs.setString(_customPeerKey, json.encode(existing));
      losLog('✅ PeerDiscovery: added custom peer: $url');
      return true;
    } catch (e) {
      losLog('⚠️ PeerDiscovery: failed to add custom peer: $e');
      return false;
    }
  }

  /// Remove a custom peer by URL.
  static Future<void> removeCustomPeer(String url) async {
    try {
      final prefs = await SharedPreferences.getInstance();
      final existing = await _loadCustomPeerList(prefs);
      existing.remove(url);
      await prefs.setString(_customPeerKey, json.encode(existing));
      losLog('🗑️ PeerDiscovery: removed custom peer: $url');
    } catch (e) {
      losLog('⚠️ PeerDiscovery: failed to remove custom peer: $e');
    }
  }

  /// Load all user-provided custom peer URLs.
  static Future<List<String>> loadCustomPeers() async {
    try {
      final prefs = await SharedPreferences.getInstance();
      return await _loadCustomPeerList(prefs);
    } catch (e) {
      losLog('⚠️ PeerDiscovery: failed to load custom peers: $e');
      return [];
    }
  }

  static Future<List<String>> _loadCustomPeerList(
      SharedPreferences prefs) async {
    final raw = prefs.getString(_customPeerKey);
    if (raw == null || raw.isEmpty) return [];
    try {
      final List<dynamic> decoded = json.decode(raw);
      return decoded.whereType<String>().toList();
    } catch (e) {
      return [];
    }
  }

  static Future<List<Map<String, dynamic>>> _loadRawPeers(
      SharedPreferences prefs) async {
    final raw = prefs.getString(_storageKey);
    if (raw == null || raw.isEmpty) return [];
    try {
      final List<dynamic> decoded = json.decode(raw);
      return decoded.whereType<Map<String, dynamic>>().toList();
    } catch (_) {
      return [];
    }
  }

  // ══════════════════════════════════════════════════════════════════
  //  PEER DIRECTORY — Embedded in Every Validator Node
  // ══════════════════════════════════════════════════════════════════
  //
  // Every LOS validator node serves a built-in peer directory at
  // /directory/api/active — listing all known active validators.
  // No separate server needed. Any validator .onion = peer directory.
  //
  // When bootstrap nodes are unreachable, the app can try fetching
  // from ANY known validator's /directory/api/active endpoint.
  // ══════════════════════════════════════════════════════════════════

  /// Fetch active peers from a validator's embedded peer directory.
  /// Every LOS validator serves GET /directory/api/active.
  /// Returns a list of REST URLs for currently active validators.
  ///
  /// [directoryUrl] — Any validator base URL (e.g., "http://xyz.onion:3030")
  /// [client]       — Optional HTTP client (use Tor SOCKS5 client for .onion)
  /// [timeout]      — Request timeout (default: 30s, generous for Tor)
  static Future<List<String>> fetchPeerDirectory({
    required String directoryUrl,
    http.Client? client,
    Duration timeout = const Duration(seconds: 30),
  }) async {
    try {
      final url = directoryUrl.endsWith('/')
          ? '${directoryUrl}directory/api/active'
          : '$directoryUrl/directory/api/active';

      losLog('\ud83d\udcda PeerDirectory: fetching from $url');

      final httpClient = client ?? http.Client();
      final response = await httpClient.get(Uri.parse(url)).timeout(timeout);

      if (response.statusCode != 200) {
        losLog('\u26a0\ufe0f PeerDirectory: HTTP ${response.statusCode}');
        return [];
      }

      final data = json.decode(response.body) as Map<String, dynamic>;
      final peers = (data['peers'] as List<dynamic>? ?? [])
          .whereType<Map<String, dynamic>>()
          .map((p) {
            final host = p['host']?.toString() ?? '';
            if (host.isEmpty) return '';
            // Ensure it's a full URL
            if (host.contains('://')) return host;
            return 'http://$host';
          })
          .where((url) => url.isNotEmpty)
          .toList();

      losLog('\u2705 PeerDirectory: found ${peers.length} active peer(s)');

      // Auto-save discovered peers for future use
      if (peers.isNotEmpty) {
        final endpoints = peers.map((url) {
          final uri = Uri.tryParse(url);
          final hostPort = uri != null
              ? (uri.port != 80 ? '${uri.host}:${uri.port}' : uri.host)
              : url.replaceFirst('http://', '');
          return <String, dynamic>{
            'host_address': hostPort,
            'address': '',
            'transport': hostPort.contains('.onion') ? 'onion' : 'clearnet',
            'rest_port': uri?.port ?? 80,
          };
        }).toList();
        await savePeers(endpoints);
      }

      return peers;
    } catch (e) {
      losLog('\u26a0\ufe0f PeerDirectory: fetch failed: $e');
      return [];
    }
  }
}
