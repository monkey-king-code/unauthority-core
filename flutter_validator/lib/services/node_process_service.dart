import '../utils/log.dart';
import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:flutter/foundation.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:path_provider/path_provider.dart';
import 'package:path/path.dart' as path;

/// Node lifecycle states
enum NodeStatus {
  stopped, // Not running
  starting, // Process spawned, waiting for ready
  syncing, // Connected to peers, syncing blocks
  running, // Fully operational, participating in consensus
  stopping, // Graceful shutdown in progress
  error, // Crashed or failed to start
}

/// Manages the los-node binary process lifecycle.
///
/// Responsibilities:
/// - Find or bundle the los-node binary
/// - Launch with correct CLI args (--port, --data-dir, --json-log, etc.)
/// - Monitor process stdout for structured JSON events
/// - Track node status (starting → syncing → running)
/// - Graceful shutdown / restart
/// - Auto-restart on crash
class NodeProcessService extends ChangeNotifier {
  Process? _process;
  NodeStatus _status = NodeStatus.stopped;
  String? _nodeAddress; // Node's LOSX... address
  String? _onionAddress; // .onion hidden service address
  // SECURITY FIX A-01: Seed phrase is NO LONGER cached in memory.
  // It is re-read from FlutterSecureStorage on demand (auto-restart).
  // This prevents the mnemonic from lingering in process memory.
  static const _secureStorage = FlutterSecureStorage();
  static const _seedStorageKey = 'v_seed_phrase';
  String? _bootstrapNodes; // Saved for auto-restart
  int? _p2pPort; // Saved for auto-restart
  String? _torSocks5; // Saved for auto-restart
  int _apiPort = 3035;
  String? _dataDir;
  String? _errorMessage;
  final List<String> _logs = [];
  static const int _maxLogLines = 500;

  // Auto-restart
  int _crashCount = 0;
  static const int _maxAutoRestarts = 5;
  Timer? _restartTimer;
  String? _lastFatalError; // Track fatal error type for restart decisions

  // Getters
  NodeStatus get status => _status;
  String? get nodeAddress => _nodeAddress;
  String? get onionAddress => _onionAddress;
  int get apiPort => _apiPort;
  String? get dataDir => _dataDir;
  String? get errorMessage => _errorMessage;
  List<String> get logs => List.unmodifiable(_logs);
  bool get isRunning =>
      _status == NodeStatus.running || _status == NodeStatus.syncing;
  bool get isStopped =>
      _status == NodeStatus.stopped || _status == NodeStatus.error;

  String get localApiUrl => 'http://127.0.0.1:$_apiPort';

  /// Kill any orphaned los-node that survived a Flutter hot-reload.
  /// Kills by port AND by node-id to ensure the database lock is released.
  /// Uses SIGKILL because suspended processes (SIGTTIN) ignore SIGTERM.
  /// CRITICAL: Polls to verify all processes are dead before returning,
  /// so the database lock (flock) is fully released by the OS.
  Future<void> _killOrphanedNode(int port) async {
    try {
      final allPids = <int>{};

      // 1. Find by port (lsof)
      final result = await Process.run(
        'lsof',
        ['-ti', 'tcp:$port'],
      );
      for (final pidStr in (result.stdout as String)
          .trim()
          .split('\n')
          .where((s) => s.isNotEmpty)) {
        final pid = int.tryParse(pidStr.trim());
        if (pid != null) allPids.add(pid);
      }

      // 2. Find by node-id (pgrep) — catches orphans on different ports
      //    that still hold the database lock file.
      final pgrepResult = await Process.run(
        'pgrep',
        ['-f', 'los-node.*flutter-validator'],
      );
      for (final pidStr in (pgrepResult.stdout as String)
          .trim()
          .split('\n')
          .where((s) => s.isNotEmpty)) {
        final pid = int.tryParse(pidStr.trim());
        if (pid != null) allPids.add(pid);
      }

      if (allPids.isEmpty) return;

      // Phase 1: SIGTERM — graceful shutdown (sled flushes + releases flock)
      for (final pid in allPids) {
        losLog('🧹 Killing orphaned los-node (PID $pid)...');
        Process.killPid(pid, ProcessSignal.sigterm);
      }
      await Future.delayed(const Duration(seconds: 1));

      // Phase 2: SIGKILL — force kill any survivors
      for (final pid in allPids) {
        try {
          Process.killPid(pid, ProcessSignal.sigkill);
        } catch (_) {}
      }
      // Also use system kill -9 as fallback
      await Process.run('kill', ['-9', ...allPids.map((p) => p.toString())]);

      // Phase 3: POLL until all processes are confirmed dead.
      // The OS needs time to fully release flock() after SIGKILL.
      // Without this, the new los-node races against the dying process
      // for the database lock → crash loop.
      const maxWaitMs = 5000; // 5 seconds max
      const pollIntervalMs = 250;
      var waited = 0;
      while (waited < maxWaitMs) {
        await Future.delayed(const Duration(milliseconds: pollIntervalMs));
        waited += pollIntervalMs;

        // Check if any of the killed PIDs still exist
        var anyAlive = false;
        for (final pid in allPids) {
          // kill(pid, 0) checks if process exists without sending a signal
          final check = await Process.run('kill', ['-0', pid.toString()]);
          if (check.exitCode == 0) {
            anyAlive = true;
            break;
          }
        }
        if (!anyAlive) {
          losLog(
              '✅ All orphaned los-node processes confirmed dead (${waited}ms)');
          // Extra 200ms grace for OS to release file descriptors / flock
          await Future.delayed(const Duration(milliseconds: 200));
          return;
        }
      }
      losLog(
          '⚠️ Some orphaned processes may still be alive after ${maxWaitMs}ms');
    } catch (e) {
      losLog('⚠️ _killOrphanedNode: $e');
    }
  }

  /// If zombie los-node processes survive SIGKILL (macOS UE/uninterruptible
  /// state), they still hold flock on the sled database. Remove the stale
  /// database directory so a fresh node can start and sync from peers.
  ///
  /// KEY INSIGHT: On macOS, UE (Uninterruptible + Exiting) processes ARE still
  /// holding the sled flock, even though they can't be killed. If we try to
  /// start a new los-node with the same data-dir, it will also block on
  /// flock() and become ANOTHER UE zombie — creating a cascade.
  ///
  /// FIX: Also check the PID lockfile written by los-node. If the PID in the
  /// lockfile is dead/zombie/UE, the DB must be nuked.
  Future<void> _clearStaleLockIfNeeded() async {
    try {
      final dataDir = await _getDataDir();

      // ── Strategy 1: PID lockfile check ─────────────────────────────
      // los-node writes .los-node.pid on startup, removes on clean shutdown.
      // If the file exists, either the node is still running or it crashed.
      final pidFile = File('$dataDir/.los-node.pid');
      if (await pidFile.exists()) {
        final pidStr = (await pidFile.readAsString()).trim();
        final pid = int.tryParse(pidStr);
        if (pid != null) {
          // Check if PID is still an active (non-zombie, non-UE) process
          final check = await Process.run('kill', ['-0', pid.toString()]);
          if (check.exitCode != 0) {
            // PID doesn't exist → stale lockfile from crash. Clean up.
            losLog('🗑️ Stale PID lockfile (PID $pid dead) — removing');
            await pidFile.delete();
            // Lock was released when process died — DB should be openable
            return;
          }

          // PID exists — check if it's a zombie/UE
          final stateResult = await Process.run(
            'ps',
            ['-p', pid.toString(), '-o', 'stat='],
          );
          final state = (stateResult.stdout as String).trim();
          if (state.startsWith('U') || state.startsWith('Z')) {
            // UE/Zombie process — it STILL holds the flock but can't release it.
            // MUST nuke the DB so the new node can start fresh.
            losLog('💀 PID $pid is in $state state (unkillable) — nuking DB');
            await _nukeDatabaseDirs(dataDir);
            await pidFile.delete();
            return;
          }

          // Process is alive and healthy — don't nuke
          losLog('⚠️ PID $pid is still running (state: $state) — DB in use');
          return;
        }
      }

      // ── Strategy 2: Fallback pgrep scan ────────────────────────────
      // If no PID lockfile exists, scan for any los-node flutter-validator
      final r =
          await Process.run('pgrep', ['-f', 'los-node.*flutter-validator']);
      final pidLines = (r.stdout as String).trim();
      if (pidLines.isEmpty) return; // No processes at all — clean

      // Check if ANY of the found processes are UE/zombie
      var hasUeZombies = false;
      var hasRealProcesses = false;
      for (final pidStr in pidLines.split('\n').where((s) => s.isNotEmpty)) {
        final pid = pidStr.trim();
        final stateResult = await Process.run(
          'ps',
          ['-p', pid, '-o', 'stat='],
        );
        final state = (stateResult.stdout as String).trim();
        if (state.isEmpty) continue;

        if (state.startsWith('U') || state.startsWith('Z')) {
          hasUeZombies = true;
        } else {
          hasRealProcesses = true;
        }
      }

      if (hasUeZombies) {
        // UE/zombie processes STILL HOLD the flock — we MUST nuke the DB.
        // This is the critical fix: the old code skipped the nuke for UE,
        // but UE processes DO hold flock, causing the next los-node to also
        // enter UE state → cascading zombie chain.
        losLog(
            '💀 Found UE/zombie los-node processes holding DB flock — nuking DB');
        await _nukeDatabaseDirs(dataDir);
        return;
      }

      if (hasRealProcesses) {
        // Real alive processes — DB is actively in use. Don't nuke.
        losLog('⚠️ Active los-node process found — DB in use, skipping nuke');
        return;
      }
    } catch (e) {
      losLog('⚠️ _clearStaleLockIfNeeded: $e');
    }
  }

  /// Remove sled database and checkpoint directories so a fresh node can
  /// start and re-sync from peers. Only called when UE zombies hold the lock.
  Future<void> _nukeDatabaseDirs(String dataDir) async {
    final dbDir = Directory('$dataDir/los_database');
    if (await dbDir.exists()) {
      await dbDir.delete(recursive: true);
      losLog('🗑️ Removed stale los_database (zombie flock detected)');
      _addLog('⚠️ Cleaned stale DB lock from zombie process — will resync');
    }
    final cpDir = Directory('$dataDir/checkpoints');
    if (await cpDir.exists()) {
      await cpDir.delete(recursive: true);
      losLog('🗑️ Removed stale checkpoints DB');
    }
  }

  /// Find an available port starting from [preferred].
  /// Returns the first port that's not already in use.
  static Future<int> findAvailablePort({int preferred = 3035}) async {
    losLog(
        '🖥️ [NodeProcessService.findAvailablePort] Searching from port $preferred...');
    for (int port = preferred; port < preferred + 20; port++) {
      try {
        final socket = await ServerSocket.bind(
          InternetAddress.loopbackIPv4,
          port,
          shared: false,
        );
        await socket.close();
        losLog(
            '🖥️ [NodeProcessService.findAvailablePort] Found available port: $port');
        return port;
      } catch (_) {
        // Port in use, try next
      }
    }
    losLog(
        '🖥️ [NodeProcessService.findAvailablePort] No free port found, fallback: $preferred');
    return preferred; // Fallback
  }

  /// Start the los-node process.
  ///
  /// [port] — REST API port (default 3035, auto-detects if occupied)
  /// [onionAddress] — Pre-configured .onion address (from Tor hidden service)
  /// [bootstrapNodes] — Comma-separated list of bootstrap node addresses
  /// [walletPassword] — Encryption password for the node wallet
  /// [seedPhrase] — BIP-39 mnemonic to derive keypair
  /// [p2pPort] — libp2p listen port (default derives from API port)
  /// [torSocks5] — Tor SOCKS5 proxy address (e.g. '127.0.0.1:9052')
  /// [testnetLevel] — Testnet level: 'functional', 'consensus', or 'production'
  ///                  Defaults to 'consensus' (Level 2) for real multi-node testing.
  ///                  On mainnet builds, this is ignored (los-node forces production).
  ///                  Use 'functional' ONLY for local single-node dev.
  Future<bool> start({
    int port = 3035,
    String? onionAddress,
    String? bootstrapNodes,
    String? walletPassword,
    String? seedPhrase,
    int? p2pPort,
    String? torSocks5,
    String testnetLevel = 'consensus',
  }) async {
    if (_status == NodeStatus.starting || _status == NodeStatus.running) {
      losLog('⚠️ Node already running or starting');
      return false;
    }

    // Cancel any pending auto-restart timer to prevent race condition
    // between manual start and timer-triggered start.
    _restartTimer?.cancel();
    _restartTimer = null;
    _lastFatalError = null; // Reset fatal error for fresh start

    _apiPort = port;
    _status = NodeStatus.starting;
    _errorMessage = null;
    final startTime = DateTime.now();
    notifyListeners();

    try {
      // 0. Kill any orphaned los-node on the target port (survives hot-reload)
      await _killOrphanedNode(port);

      // 0b. If zombies survive SIGKILL (macOS UE state), remove stale DB lock
      //     so the new process can open the database.
      await _clearStaleLockIfNeeded();

      // 1. Find los-node binary
      final binaryPath = await _findNodeBinary();
      if (binaryPath == null) {
        _setError(
            'los-node binary not found. Please build with: cargo build --release -p los-node');
        return false;
      }

      // 2. Setup data directory
      _dataDir = await _getDataDir();
      await Directory(_dataDir!).create(recursive: true);

      // 3. Build environment variables
      // On mainnet builds, LOS_TESTNET_LEVEL is omitted entirely — los-node
      // forces production mode when compiled with --features mainnet.
      // On testnet builds, testnetLevel controls security posture:
      //   'functional' = Level 1 (no consensus, no sig check — dev only)
      //   'consensus'  = Level 2 (real aBFT, real signatures — default)
      //   'production' = Level 3 (identical to mainnet — full security)
      const isMainnetBuild =
          String.fromEnvironment('NETWORK', defaultValue: 'mainnet') ==
              'mainnet';
      final env = <String, String>{
        if (!isMainnetBuild) 'LOS_TESTNET_LEVEL': testnetLevel,
      };
      // SECURITY FIX S-01: Seed phrase is NO LONGER passed via environment variable.
      // It is now sent via stdin pipe (see below) to prevent exposure via
      // /proc/[pid]/environ on Linux.
      // SECURITY FIX A-01: Seed phrase is NOT cached as a field anymore.
      // For auto-restart, it is re-read from FlutterSecureStorage.
      String? effectiveSeed = seedPhrase;
      if (effectiveSeed == null || effectiveSeed.isEmpty) {
        // Re-read from secure storage for auto-restart scenarios
        effectiveSeed = await _secureStorage.read(key: _seedStorageKey);
      }
      if (onionAddress != null) {
        env['LOS_ONION_ADDRESS'] = onionAddress;
        _onionAddress = onionAddress;
      }
      if (bootstrapNodes != null && bootstrapNodes.isNotEmpty) {
        env['LOS_BOOTSTRAP_NODES'] = bootstrapNodes;
        _bootstrapNodes = bootstrapNodes; // Save for auto-restart
      } else if (_bootstrapNodes != null) {
        env['LOS_BOOTSTRAP_NODES'] = _bootstrapNodes!;
      }
      // P2P port for libp2p listener (default 4001 in los-node)
      final effectiveP2pPort = p2pPort ?? _p2pPort;
      if (effectiveP2pPort != null) {
        env['LOS_P2P_PORT'] = effectiveP2pPort.toString();
        _p2pPort = effectiveP2pPort; // Save for auto-restart
      }
      // Tor SOCKS5 proxy for outgoing Tor connections
      final effectiveTorSocks5 = torSocks5 ?? _torSocks5;
      if (effectiveTorSocks5 != null && effectiveTorSocks5.isNotEmpty) {
        env['LOS_TOR_SOCKS5'] = effectiveTorSocks5;
        _torSocks5 = effectiveTorSocks5; // Save for auto-restart
      }
      // SECURITY FIX F5: Wallet password passed via stdin pipe instead of
      // environment variable. Environment variables are readable via
      // /proc/[pid]/environ on Linux. Stdin is not externally observable.
      final bool hasWalletPassword =
          walletPassword != null && walletPassword.isNotEmpty;

      // 4. Build CLI args
      final args = <String>[
        if (isMainnetBuild)
          '--mainnet', // Safety gate: must match compile-time feature
        '--port',
        port.toString(),
        '--data-dir',
        _dataDir!,
        '--node-id',
        'flutter-validator',
        '--json-log',
      ];

      losLog('🚀 Starting los-node: $binaryPath ${args.join(' ')}');
      _addLog('Starting los-node on port $port...');

      // 5. Spawn process
      _process = await Process.start(
        binaryPath,
        args,
        environment: env,
        workingDirectory: await _getWorkingDir(),
      );

      // SECURITY FIX F5+S-01: Write secrets to stdin pipe then close.
      // Protocol: line 1 = wallet_password, line 2 = seed_phrase.
      // This avoids exposing secrets in /proc/[pid]/environ on Linux.
      // Empty lines are sent for missing values (Rust side skips empty lines).
      {
        final stdinSeed = effectiveSeed ?? '';
        final effectivePassword = hasWalletPassword ? walletPassword : '';
        _process!.stdin.writeln(effectivePassword);
        _process!.stdin.writeln(stdinSeed);
        await _process!.stdin.flush();
        await _process!.stdin.close();
        // SECURITY FIX A-01: Do not retain seed reference after passing to stdin
      }

      // 6. Monitor stdout for JSON events + human-readable logs
      _process!.stdout
          .transform(utf8.decoder)
          .transform(const LineSplitter())
          .listen(
            _handleStdout,
            onError: (e) => losLog('⚠️ stdout error: $e'),
          );

      _process!.stderr
          .transform(utf8.decoder)
          .transform(const LineSplitter())
          .listen(
            _handleStderr,
            onError: (e) => losLog('⚠️ stderr error: $e'),
          );

      // 7. Monitor process exit
      _process!.exitCode.then(_handleProcessExit);

      // 8. Wait for node_ready event (max 180s)
      // FIX: 60s was too tight — Dilithium5 keygen + DB init + genesis loading
      // can take 50-70s on some machines, leaving near-zero margin for the
      // node_ready JSON event to reach the Dart event loop before timeout.
      // 180s gives comfortable headroom for slow machines / cold start.
      final ready = await _waitForReady(timeout: const Duration(seconds: 180));
      if (!ready) {
        _setError('Node failed to start within 180 seconds');
        await stop();
        return false;
      }

      _crashCount = 0; // Reset crash counter on successful start
      final elapsed = DateTime.now().difference(startTime).inMilliseconds;
      losLog(
          '🖥️ [NodeProcessService.start] Node started in ${elapsed}ms on port $_apiPort');
      return true;
    } catch (e) {
      _setError('Failed to start node: $e');
      return false;
    }
  }

  /// Graceful shutdown
  Future<void> stop() async {
    if (_process == null) return;

    _restartTimer?.cancel();
    _status = NodeStatus.stopping;
    notifyListeners();
    _addLog('Stopping node...');

    try {
      // Send SIGTERM for graceful shutdown
      _process!.kill(ProcessSignal.sigterm);

      // Wait up to 10s for graceful exit
      final exitCode = await _process!.exitCode.timeout(
        const Duration(seconds: 10),
        onTimeout: () {
          losLog('⚠️ Node did not exit in 10s, sending SIGKILL');
          _process!.kill(ProcessSignal.sigkill);
          return -1;
        },
      );

      losLog('🛑 Node exited with code $exitCode');
      _addLog('Node stopped (exit code: $exitCode)');
    } catch (e) {
      losLog('⚠️ Error stopping node: $e');
      _process?.kill(ProcessSignal.sigkill);
    }

    // Clean up PID lockfile (in case the SIGTERM handler didn't get to it)
    try {
      if (_dataDir != null) {
        final pidFile = File('$_dataDir/.los-node.pid');
        if (await pidFile.exists()) {
          await pidFile.delete();
          losLog('🗑️ Removed PID lockfile');
        }
      }
    } catch (_) {}

    _process = null;
    _status = NodeStatus.stopped;
    notifyListeners();
  }

  /// Force restart
  Future<bool> restart({
    String? onionAddress,
    String? bootstrapNodes,
    String? walletPassword,
    int? p2pPort,
    String? torSocks5,
  }) async {
    await stop();
    await Future.delayed(const Duration(seconds: 2));
    // SECURITY FIX A-01: Seed phrase is re-read from SecureStorage,
    // not cached as a field. Pass null to let start() re-read it.
    return start(
      port: _apiPort,
      onionAddress: onionAddress ?? _onionAddress,
      bootstrapNodes: bootstrapNodes ?? _bootstrapNodes,
      walletPassword: walletPassword,
      seedPhrase: null, // Re-read from SecureStorage inside start()
      p2pPort: p2pPort ?? _p2pPort,
      torSocks5: torSocks5 ?? _torSocks5,
    );
  }

  // ════════════════════════════════════════════════════════════════════════
  // PROCESS OUTPUT HANDLING
  // ════════════════════════════════════════════════════════════════════════

  void _handleStdout(String line) {
    _addLog(line);

    // Try to parse as JSON event
    if (line.startsWith('{')) {
      try {
        final event = json.decode(line) as Map<String, dynamic>;
        _handleJsonEvent(event);
        return;
      } catch (_) {
        // Not JSON, treat as regular log
      }
    }

    // Fallback: detect key messages from human-readable output.
    // Multiple detection patterns for robustness — any of these means
    // the node's REST/gRPC API is up and serving requests.
    if (line.contains('API Server running at') ||
        line.contains('gRPC Server STARTED') ||
        line.contains('UNAUTHORITY (LOS)')) {
      if (_status != NodeStatus.running) {
        _status = NodeStatus.running;
        _addLog('✅ Node is running!');
        notifyListeners();
      }
    }
  }

  void _handleStderr(String line) {
    _addLog('[ERR] $line');
    // CRITICAL FIX: Only treat truly FATAL errors as node failures.
    // Previously `line.contains('❌')` matched non-fatal errors like:
    //   "❌ P2P dial error: Transport(Timeout)" — normal Tor jitter
    //   "❌ Gossip sign error" — transient signing failure
    //   "❌ Auto-Receive signing failed" — non-critical
    // These set NodeStatus.error → killed the running node.
    //
    // New rule: only 'FATAL' keyword (always uppercase in los-node) or
    // specific fatal conditions trigger node ERROR status.
    if (line.contains('FATAL') ||
        line.contains('database_lock_failed') ||
        line.contains('checkpoint_db_lock_failed')) {
      _setError(line);
    }
  }

  void _handleJsonEvent(Map<String, dynamic> event) {
    final type = event['event'] as String?;
    switch (type) {
      case 'init':
        _addLog('📂 Data dir: ${event['data_dir']}');
        break;
      case 'wallet_ready':
        _nodeAddress = event['address'] as String?;
        _addLog('🔑 Node address: $_nodeAddress');
        notifyListeners();
        break;
      case 'node_ready':
        _nodeAddress = event['address'] as String?;
        final onion = event['onion'] as String?;
        if (onion != null && onion != 'none') {
          _onionAddress = onion;
        }
        _status = NodeStatus.running;
        _addLog('✅ Node is running!');
        notifyListeners();
        break;
      case 'fatal':
        // CRITICAL: los-node emits {"event":"fatal","error":"database_lock_failed"}
        // when another instance holds the sled database lock.
        // We MUST set error state immediately so _waitForReady() exits
        // and auto-restart logic knows this is an unrecoverable error.
        final errorCode = event['error'] as String? ?? 'unknown';
        final errorPath = event['path'] as String?;
        _lastFatalError = errorCode;
        String userMsg;
        switch (errorCode) {
          case 'database_lock_failed':
            userMsg =
                'Database locked by another instance. Kill all los-node processes and retry.';
            break;
          case 'checkpoint_db_lock_failed':
            userMsg =
                'Checkpoint database locked. Kill all los-node processes and retry.';
            break;
          default:
            userMsg = 'Fatal error: $errorCode';
        }
        if (errorPath != null) {
          userMsg += '\nPath: $errorPath';
        }
        _setError(userMsg);
        losLog('🛑 Fatal JSON event: $errorCode (path: $errorPath)');
        break;
      default:
        losLog('Unknown JSON event: $type');
    }
  }

  void _handleProcessExit(int exitCode) {
    losLog('💀 los-node exited with code $exitCode');
    _addLog('⚠️ Node process exited (code: $exitCode)');

    if (_status == NodeStatus.stopping || _status == NodeStatus.stopped) {
      // Intentional shutdown
      _status = NodeStatus.stopped;
    } else {
      // Unexpected crash — auto-restart (unless fatal DB lock error)
      _status = NodeStatus.error;
      _crashCount++;

      // CRITICAL: Do NOT auto-restart on database_lock_failed or
      // checkpoint_db_lock_failed — restarting will just hit the same lock.
      // The user must kill the orphan process first.
      final isFatalDbLock = _lastFatalError == 'database_lock_failed' ||
          _lastFatalError == 'checkpoint_db_lock_failed';

      if (isFatalDbLock) {
        _addLog(
            '🛑 Database lock error — auto-restart disabled. Kill orphan processes and restart manually.');
      } else if (_crashCount <= _maxAutoRestarts) {
        final delay =
            Duration(seconds: _crashCount * 5); // Backoff: 5s, 10s, 15s...
        _addLog(
            '🔄 Auto-restart in ${delay.inSeconds}s (attempt $_crashCount/$_maxAutoRestarts)');
        _restartTimer = Timer(delay, () async {
          // Kill any zombie processes before restart to release DB lock
          await _killOrphanedNode(_apiPort);
          _lastFatalError = null; // Reset for next attempt
          // SECURITY FIX A-01: Pass null seedPhrase — start() will
          // re-read from SecureStorage on demand.
          start(
              port: _apiPort,
              onionAddress: _onionAddress,
              seedPhrase: null,
              bootstrapNodes: _bootstrapNodes,
              p2pPort: _p2pPort,
              torSocks5: _torSocks5);
        });
      } else {
        _setError(
            'Node crashed $_crashCount times. Stopped auto-restart. Check logs.');
      }
    }

    _process = null;
    notifyListeners();
  }

  Future<bool> _waitForReady({required Duration timeout}) async {
    final deadline = DateTime.now().add(timeout);
    while (DateTime.now().isBefore(deadline)) {
      if (_status == NodeStatus.running) return true;
      if (_status == NodeStatus.error) return false;
      await Future.delayed(const Duration(milliseconds: 500));
    }
    return false;
  }

  // ════════════════════════════════════════════════════════════════════════
  // BINARY DISCOVERY
  // ════════════════════════════════════════════════════════════════════════

  /// Find los-node binary — checks bundled location, cargo build output, PATH
  /// SECURITY FIX J-02: In release builds, restrict discovery to bundled
  /// locations only. Dev paths (cargo build, PATH) only in debug mode.
  Future<String?> _findNodeBinary() async {
    final binaryName = Platform.isWindows ? 'los-node.exe' : 'los-node';

    // 1. Check bundled in app (for distribution) — always searched
    final execDir = path.dirname(Platform.resolvedExecutable);
    final bundledPaths = [
      path.join(execDir, binaryName), // Linux/Windows: same dir as executable
      path.join(execDir, '..', 'Resources', binaryName), // macOS: Resources/
      path.join(execDir, '..', 'Resources', 'bin',
          binaryName), // macOS: Resources/bin/
      path.join(execDir, '..', 'MacOS', binaryName), // macOS: MacOS/ (alt)
    ];
    for (final p in bundledPaths) {
      if (await File(p).exists()) {
        losLog('✅ Found bundled los-node: $p');
        return p;
      }
    }

    // SECURITY FIX J-02: Only search development paths in debug mode.
    // In release builds, an attacker could place a malicious binary in
    // PATH or cargo output directory to hijack the validator.
    if (!kDebugMode) {
      losLog(
          '⚠️ los-node binary not found in bundled locations (release mode)');
      losLog('   Release builds only search bundled app paths for security.');
      return null;
    }

    // 2. Check cargo build output (development mode only)
    final workDir = await _getWorkingDir();
    final cargoPaths = [
      path.join(workDir, 'target', 'release', binaryName),
      path.join(workDir, 'target', 'debug', binaryName),
    ];
    for (final p in cargoPaths) {
      if (await File(p).exists()) {
        losLog('✅ Found cargo-built los-node: $p');
        return p;
      }
    }

    // 3. Check PATH (development mode only)
    try {
      final cmd = Platform.isWindows ? 'where' : 'which';
      final result = await Process.run(cmd, [binaryName]);
      if (result.exitCode == 0) {
        final p = result.stdout.toString().trim().split('\n').first;
        if (p.isNotEmpty && await File(p).exists()) {
          losLog('✅ Found los-node in PATH: $p');
          return p;
        }
      }
    } catch (_) {}

    losLog('❌ los-node binary not found anywhere');
    return null;
  }

  /// Get the workspace root (for cargo builds in dev mode)
  Future<String> _getWorkingDir() async {
    // Walk up from the executable to find Cargo.toml
    // macOS debug app is deeply nested:
    // flutter_validator/build/macos/Build/Products/Debug/App.app/Contents/MacOS/
    // That's ~9 levels up to the workspace root, so walk up 12 to be safe.
    var dir = Directory(path.dirname(Platform.resolvedExecutable));
    for (var i = 0; i < 12; i++) {
      final cargoToml = File(path.join(dir.path, 'Cargo.toml'));
      if (await cargoToml.exists()) {
        return dir.path;
      }
      dir = dir.parent;
    }

    // Fallback: check common dev paths
    final home = Platform.environment['HOME'] ?? '/tmp';
    final devPaths = [
      path.join(home, 'unauthority-core'),
      path.join(home, 'Documents', 'unauthority-core'),
      Directory.current.path,
    ];

    for (final p in devPaths) {
      if (await File(path.join(p, 'Cargo.toml')).exists()) {
        return p;
      }
    }

    return Directory.current.path;
  }

  /// Get persistent data directory for the node
  Future<String> _getDataDir() async {
    final appDir = await getApplicationSupportDirectory();
    return path.join(appDir.path, 'validator-node');
  }

  // ════════════════════════════════════════════════════════════════════════
  // LOG MANAGEMENT
  // ════════════════════════════════════════════════════════════════════════

  void _addLog(String msg) {
    final timestamp = DateTime.now().toIso8601String().substring(11, 19);
    _logs.add('[$timestamp] $msg');
    while (_logs.length > _maxLogLines) {
      _logs.removeAt(0);
    }
    // Don't notifyListeners() for every log line — batch via Timer
  }

  void _setError(String msg) {
    _errorMessage = msg;
    _status = NodeStatus.error;
    _addLog('❌ $msg');
    notifyListeners();
  }

  @override
  void dispose() {
    _restartTimer?.cancel();
    stop();
    super.dispose();
  }
}
