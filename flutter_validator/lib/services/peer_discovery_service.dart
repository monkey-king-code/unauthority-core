import '../utils/log.dart';
import 'dart:convert';
import 'package:shared_preferences/shared_preferences.dart';

/// Persistent Peer Discovery Service — "Memory" for discovered nodes.
///
/// Saves validator .onion endpoints discovered from the network into
/// SharedPreferences so the app can reconnect to known-good peers
/// even when all 4 bootstrap nodes are offline.
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
  /// Returns a list of onion URLs with port: ["http://xyz.onion:3030", ...]
  static Future<List<String>> loadSavedPeers() async {
    try {
      final prefs = await SharedPreferences.getInstance();
      final raw = prefs.getString(_storageKey);
      if (raw == null || raw.isEmpty) return [];

      final List<dynamic> decoded = json.decode(raw);
      final peers = decoded
          .whereType<Map<String, dynamic>>()
          .where((p) =>
              p['onion_address'] != null &&
              (p['onion_address'] as String).endsWith('.onion'))
          .map((p) {
        final onion = p['onion_address'] as String;
        final restPort = p['rest_port'] as int?;
        // Build REST URL with port if available
        if (onion.contains('://')) return onion;
        if (restPort != null && restPort != 80) {
          return 'http://$onion:$restPort';
        }
        return 'http://$onion';
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
  ///
  /// [endpoints] should be the "endpoints" array from the API response:
  /// [{"address": "LOS...", "onion_address": "xyz.onion", "stake_los": 1000, "reachable": true}, ...]
  static Future<void> savePeers(List<Map<String, dynamic>> endpoints) async {
    try {
      if (endpoints.isEmpty) return;

      final prefs = await SharedPreferences.getInstance();

      // Load existing peers and merge (dedup by onion_address)
      final existing = await _loadRawPeers(prefs);
      final merged = <String, Map<String, dynamic>>{};

      // Existing peers first (lower priority)
      for (final p in existing) {
        final onion = p['onion_address']?.toString() ?? '';
        if (onion.isNotEmpty) merged[onion] = p;
      }

      // New peers overwrite (higher priority — fresher data)
      for (final p in endpoints) {
        final onion = p['onion_address']?.toString() ?? '';
        if (onion.isNotEmpty && onion.endsWith('.onion')) {
          merged[onion] = {
            'address': p['address'],
            'onion_address': onion,
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
}
