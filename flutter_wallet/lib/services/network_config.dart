import '../utils/log.dart';
import 'dart:convert';
import 'package:flutter/services.dart' show rootBundle;

/// Loads bootstrap node addresses from assets/network_config.json.
///
/// This is the SINGLE SOURCE OF TRUTH for .onion addresses.
/// NEVER hardcode .onion addresses in Dart source code.
/// Use scripts/update_network_config.sh to update the config.
class NetworkConfig {
  static Map<String, dynamic>? _config;
  static List<BootstrapNode>? _testnetNodes;
  static List<BootstrapNode>? _mainnetNodes;

  /// Load the config from bundled asset. Safe to call multiple times.
  static Future<void> load() async {
    if (_config != null) return;
    try {
      final raw = await rootBundle.loadString('assets/network_config.json');
      _config = json.decode(raw) as Map<String, dynamic>;

      _testnetNodes = _parseNodes(_config!['testnet']);
      _mainnetNodes = _parseNodes(_config!['mainnet']);

      losLog(
        '🌐 NetworkConfig loaded: '
        '${_testnetNodes!.length} testnet node(s), '
        '${_mainnetNodes!.length} mainnet node(s)',
      );
    } catch (e) {
      losLog('⚠️ NetworkConfig load failed: $e');
      _testnetNodes = [];
      _mainnetNodes = [];
    }
  }

  static List<BootstrapNode> _parseNodes(dynamic networkBlock) {
    if (networkBlock == null) return [];
    final list = networkBlock['bootstrap_nodes'] as List<dynamic>? ?? [];
    return list
        .map((e) => BootstrapNode.fromJson(e as Map<String, dynamic>))
        .toList();
  }

  /// Get the first reachable testnet bootstrap REST URL.
  /// Format: `http://onion-address`  (port 80 is default for HTTP)
  static String get testnetUrl {
    if (_testnetNodes == null || _testnetNodes!.isEmpty) {
      throw StateError(
        'No testnet bootstrap nodes configured. '
        'Run: scripts/update_network_config.sh',
      );
    }
    return _testnetNodes!.first.restUrl;
  }

  /// Get the first reachable mainnet bootstrap REST URL.
  static String get mainnetUrl {
    if (_mainnetNodes == null || _mainnetNodes!.isEmpty) {
      throw StateError(
        'No mainnet bootstrap nodes configured. '
        'Mainnet has not launched yet.',
      );
    }
    return _mainnetNodes!.first.restUrl;
  }

  /// Get all testnet bootstrap nodes (for multi-node fallback).
  static List<BootstrapNode> get testnetNodes => _testnetNodes ?? [];

  /// Get all mainnet bootstrap nodes.
  static List<BootstrapNode> get mainnetNodes => _mainnetNodes ?? [];
}

class BootstrapNode {
  final String name;

  /// Clearnet address (IP or domain). null = onion-only node.
  final String? host;

  /// .onion address for Tor connections.
  final String onion;

  /// External REST port (via Tor hidden service, typically 80).
  final int restPort;

  /// External P2P port (via Tor, typically 4001).
  final int p2pPort;

  /// Local REST port for clearnet access (e.g. 7030 for dev testnet).
  /// Falls back to [restPort] when not set.
  final int? localRestPort;

  /// Local P2P port for clearnet access (e.g. 7041 for dev testnet).
  final int? localP2pPort;

  const BootstrapNode({
    required this.name,
    this.host,
    required this.onion,
    this.restPort = 80,
    this.p2pPort = 4001,
    this.localRestPort,
    this.localP2pPort,
  });

  factory BootstrapNode.fromJson(Map<String, dynamic> json) {
    return BootstrapNode(
      name: json['name'] as String? ?? 'unknown',
      host: json['host'] as String?,
      onion: json['onion'] as String? ?? '',
      restPort: json['rest_port'] as int? ?? 80,
      p2pPort: json['p2p_port'] as int? ?? 4001,
      localRestPort: json['local_rest_port'] as int?,
      localP2pPort: json['local_p2p_port'] as int?,
    );
  }

  /// Primary REST URL — clearnet if available, otherwise .onion.
  /// Clearnet is preferred for dev/testnet (faster, no Tor dependency).
  /// Mainnet security (M-06) filters non-.onion URLs in ApiService.
  String get restUrl {
    if (host != null && host!.isNotEmpty) {
      final port = localRestPort ?? restPort;
      return port == 80 ? 'http://$host' : 'http://$host:$port';
    }
    return restPort == 80 ? 'http://$onion' : 'http://$onion:$restPort';
  }

  /// .onion REST URL (null if no onion address configured).
  String? get onionRestUrl {
    if (onion.isEmpty) return null;
    return restPort == 80 ? 'http://$onion' : 'http://$onion:$restPort';
  }

  /// All REST URLs for this node — clearnet first (faster), then onion (private).
  /// Used by _loadBootstrapUrls to populate the full failover list.
  List<String> get allRestUrls {
    final urls = <String>[];
    if (host != null && host!.isNotEmpty) {
      final port = localRestPort ?? restPort;
      urls.add(port == 80 ? 'http://$host' : 'http://$host:$port');
    }
    if (onion.isNotEmpty) {
      final onionUrl =
          restPort == 80 ? 'http://$onion' : 'http://$onion:$restPort';
      // Avoid duplicate if host IS the onion address
      if (!urls.contains(onionUrl)) urls.add(onionUrl);
    }
    return urls;
  }

  /// P2P address for libp2p connections.
  String get p2pAddress => '$onion:$p2pPort';
}
