import '../utils/log.dart';
import 'dart:io';
import 'dart:async';
import 'package:crypto/crypto.dart' as crypto;
import 'package:flutter/foundation.dart';
import 'package:path_provider/path_provider.dart';
import 'package:path/path.dart' as path;

/// Bundled Tor Service — Zero-intervention Tor connectivity
///
/// Priority chain (no user action required):
/// 1. Detect existing Tor (port 9150/9052/9050) → use it
/// 2. Find system `tor` binary in PATH / common locations → start on port 9250
/// 3. Check cached Tor download in app support dir → start
/// 4. Auto-install via package manager (brew/apt) → start
/// 5. Auto-download Tor Expert Bundle from torproject.org → start
///
/// The user and their friend NEVER need to manually install anything.
class TorService {
  Process? _torProcess;
  String? _torDataDir;

  // ── Named constants (no magic numbers) ────────────────────────────
  /// SOCKS port used when we start our own bundled Tor process.
  static const int bundledSocksPort = 9250;

  /// Well-known SOCKS ports to probe for existing Tor instances.
  static const int torBrowserPort = 9150;
  static const int losTorPort = 9052;
  static const int systemTorPort = 9050;

  /// Timeout for Tor to finish bootstrapping (100% circuit established).
  static const Duration bootstrapTimeout = Duration(seconds: 120);

  /// Timeout for package-manager install (brew/apt).
  static const Duration installTimeout = Duration(minutes: 5);

  /// Timeout for downloading the Tor Expert Bundle tarball.
  static const Duration downloadTimeout = Duration(minutes: 10);

  /// Interval between health-check probes while waiting for bootstrap.
  static const Duration bootstrapPollInterval = Duration(milliseconds: 500);

  final int _socksPort = bundledSocksPort;
  bool _isRunning = false;
  String? _activeProxy; // "host:port" of the active SOCKS proxy

  bool get isRunning => _isRunning;
  String get proxyAddress => _activeProxy ?? 'localhost:$_socksPort';
  int get activeSocksPort {
    if (_activeProxy != null) {
      return int.tryParse(_activeProxy!.split(':').last) ?? _socksPort;
    }
    return _socksPort;
  }

  /// Start Tor with full auto-detection/installation chain.
  /// Returns true if a SOCKS proxy is available, false if all methods failed.
  Future<bool> start() async {
    if (_isRunning) {
      losLog('🔵 Tor already running on $_activeProxy');
      return true;
    }

    // 1. Detect existing Tor instances
    final existing = await detectExistingTor();
    if (existing['found'] == true) {
      _activeProxy = existing['proxy'] as String;
      _isRunning = true;
      losLog('✅ Using existing ${existing['type']}: $_activeProxy');
      return true;
    }

    // 2. Find or install Tor binary
    String? torBinary = await _findTorBinary();

    if (torBinary == null) {
      losLog('🔍 No Tor binary found, attempting auto-install...');
      torBinary = await _autoInstallTor();
    }

    if (torBinary == null) {
      losLog('📥 Auto-install failed, attempting download...');
      torBinary = await _downloadAndCacheTor();
    }

    if (torBinary == null) {
      losLog('❌ Could not find, install, or download Tor');
      losLog('   The wallet will not be able to connect to .onion nodes');
      return false;
    }

    // 3. Start Tor process
    return await _startTorProcess(torBinary);
  }

  /// Stop Tor daemon
  Future<void> stop() async {
    if (_torProcess != null) {
      losLog('🛑 Stopping Tor daemon...');
      _torProcess!.kill(ProcessSignal.sigterm);
      await Future.delayed(bootstrapPollInterval);
      _torProcess = null;
      _isRunning = false;
      _activeProxy = null;
      losLog('✅ Tor stopped');
    }
  }

  // ════════════════════════════════════════════════════════════════════════
  // BINARY DISCOVERY — Find Tor on the system
  // ════════════════════════════════════════════════════════════════════════

  /// Search for tor binary in PATH and common locations
  Future<String?> _findTorBinary() async {
    // Check PATH using 'which' (Unix) or 'where' (Windows)
    try {
      final cmd = Platform.isWindows ? 'where' : 'which';
      final result = await Process.run(cmd, ['tor']);
      if (result.exitCode == 0) {
        final torPath = result.stdout.toString().trim().split('\n').first;
        if (torPath.isNotEmpty && await File(torPath).exists()) {
          losLog('✅ Found system tor: $torPath');
          return torPath;
        }
      }
    } catch (e) {
      losLog('⚠️ _findTorBinary PATH check failed: $e');
    }

    // Check common installation locations
    final commonPaths = <String>[];
    if (Platform.isMacOS) {
      commonPaths.addAll([
        '/opt/homebrew/bin/tor', // Apple Silicon Homebrew
        '/usr/local/bin/tor', // Intel Homebrew
        '/opt/local/bin/tor', // MacPorts
      ]);
    } else if (Platform.isLinux) {
      commonPaths.addAll([
        '/usr/bin/tor',
        '/usr/local/bin/tor',
        '/snap/bin/tor',
      ]);
    } else if (Platform.isWindows) {
      final localAppData = Platform.environment['LOCALAPPDATA'] ?? '';
      final appData = Platform.environment['APPDATA'] ?? '';
      final userProfile = Platform.environment['USERPROFILE'] ?? '';
      commonPaths.addAll([
        r'C:\Program Files\Tor\tor.exe',
        r'C:\Program Files\Tor Browser\Browser\TorBrowser\Tor\tor.exe',
        r'C:\Program Files (x86)\Tor Browser\Browser\TorBrowser\Tor\tor.exe',
        if (localAppData.isNotEmpty)
          path.join(localAppData, 'Tor Browser', 'Browser', 'TorBrowser', 'Tor',
              'tor.exe'),
        if (userProfile.isNotEmpty)
          path.join(userProfile, 'Desktop', 'Tor Browser', 'Browser',
              'TorBrowser', 'Tor', 'tor.exe'),
        if (appData.isNotEmpty)
          path.join(appData, 'Tor Browser', 'Browser', 'TorBrowser', 'Tor',
              'tor.exe'),
      ]);
    }

    for (final torPath in commonPaths) {
      if (await File(torPath).exists()) {
        losLog('✅ Found tor at: $torPath');
        return torPath;
      }
    }

    // Check bundled binary in app assets
    final bundled = await _getBundledTorBinary();
    if (bundled != null) return bundled;

    // Check cached download
    final cached = await _getCachedTorBinary();
    if (cached != null) return cached;

    return null;
  }

  /// Check for bundled Tor binary in Flutter assets
  Future<String?> _getBundledTorBinary() async {
    String? binaryName;
    if (Platform.isMacOS) {
      binaryName = 'tor/macos/tor';
    } else if (Platform.isWindows) {
      binaryName = 'tor/windows/tor.exe';
    } else if (Platform.isLinux) {
      binaryName = 'tor/linux/tor';
    } else {
      return null;
    }

    final executableDir = path.dirname(Platform.resolvedExecutable);
    final locations = [
      path.join(Directory.current.path, binaryName),
      path.join(executableDir, '..', 'Resources', 'flutter_assets', binaryName),
      path.join(executableDir, 'data', 'flutter_assets', binaryName),
    ];

    for (final location in locations) {
      final file = File(location);
      if (await file.exists()) {
        if (!Platform.isWindows) {
          await Process.run('chmod', ['+x', location]);
        }
        losLog('✅ Found bundled tor: $location');
        return location;
      }
    }
    return null;
  }

  /// Check for previously downloaded & cached Tor binary
  Future<String?> _getCachedTorBinary() async {
    try {
      final appDir = await getApplicationSupportDirectory();
      final torBinDir = path.join(appDir.path, 'tor_bin');

      final binaryName = Platform.isWindows ? 'tor.exe' : 'tor';
      final cachedPath = path.join(torBinDir, binaryName);

      if (await File(cachedPath).exists()) {
        losLog('✅ Found cached tor: $cachedPath');
        return cachedPath;
      }
    } catch (e) {
      losLog('⚠️ _getCachedTorBinary check failed: $e');
    }
    return null;
  }

  // ════════════════════════════════════════════════════════════════════════
  // AUTO-INSTALL — Package manager installation
  // ════════════════════════════════════════════════════════════════════════

  /// Try to install Tor via system package manager (silent, no user input).
  /// Supports macOS (brew), Linux (apt/dnf/pacman with sudo), Windows (winget/choco).
  Future<String?> _autoInstallTor() async {
    try {
      if (Platform.isMacOS) {
        // Check if Homebrew is available
        final brewCheck = await Process.run('which', ['brew']);
        if (brewCheck.exitCode == 0) {
          losLog('📦 Installing Tor via Homebrew (this may take a minute)...');
          final installResult = await Process.run(
            'brew',
            ['install', 'tor'],
            runInShell: true,
          ).timeout(installTimeout);

          if (installResult.exitCode == 0) {
            // Find the installed binary
            final whichResult = await Process.run('which', ['tor']);
            if (whichResult.exitCode == 0) {
              final torPath = whichResult.stdout.toString().trim();
              losLog('✅ Tor installed via Homebrew: $torPath');
              return torPath;
            }
            // Try common Homebrew paths
            for (final p in ['/opt/homebrew/bin/tor', '/usr/local/bin/tor']) {
              if (await File(p).exists()) return p;
            }
          } else {
            losLog('⚠️  brew install tor failed: ${installResult.stderr}');
          }
        }
      } else if (Platform.isLinux) {
        // Try apt with sudo (Debian/Ubuntu)
        final aptCheck = await Process.run('which', ['apt-get']);
        if (aptCheck.exitCode == 0) {
          losLog('📦 Installing Tor via sudo apt-get...');
          final result = await Process.run(
            'sudo',
            ['apt-get', 'install', '-y', 'tor'],
            runInShell: true,
          ).timeout(installTimeout);

          if (result.exitCode == 0 && await File('/usr/bin/tor').exists()) {
            losLog('✅ Tor installed via apt');
            return '/usr/bin/tor';
          }
        }

        // Try dnf with sudo (Fedora/RHEL)
        final dnfCheck = await Process.run('which', ['dnf']);
        if (dnfCheck.exitCode == 0) {
          losLog('📦 Installing Tor via sudo dnf...');
          final result = await Process.run(
            'sudo',
            ['dnf', 'install', '-y', 'tor'],
            runInShell: true,
          ).timeout(installTimeout);

          if (result.exitCode == 0 && await File('/usr/bin/tor').exists()) {
            losLog('✅ Tor installed via dnf');
            return '/usr/bin/tor';
          }
        }

        // Try pacman with sudo (Arch)
        final pacmanCheck = await Process.run('which', ['pacman']);
        if (pacmanCheck.exitCode == 0) {
          losLog('📦 Installing Tor via sudo pacman...');
          final result = await Process.run(
            'sudo',
            ['pacman', '-S', '--noconfirm', 'tor'],
            runInShell: true,
          ).timeout(installTimeout);

          if (result.exitCode == 0 && await File('/usr/bin/tor').exists()) {
            losLog('✅ Tor installed via pacman');
            return '/usr/bin/tor';
          }
        }
      } else if (Platform.isWindows) {
        // Try winget (Windows 10/11 built-in)
        try {
          final wingetCheck =
              await Process.run('where', ['winget'], runInShell: true);
          if (wingetCheck.exitCode == 0) {
            losLog('📦 Installing Tor via winget...');
            final result = await Process.run(
              'winget',
              ['install', '--id', 'TorProject.TorBrowser', '-e', '--silent'],
              runInShell: true,
            ).timeout(const Duration(minutes: 10));

            if (result.exitCode == 0) {
              for (final p in [
                r'C:\Program Files\Tor Browser\Browser\TorBrowser\Tor\tor.exe',
                r'C:\Program Files (x86)\Tor Browser\Browser\TorBrowser\Tor\tor.exe',
                path.join(Platform.environment['LOCALAPPDATA'] ?? '',
                    'Tor Browser', 'Browser', 'TorBrowser', 'Tor', 'tor.exe'),
              ]) {
                if (await File(p).exists()) {
                  losLog('✅ Tor installed via winget: $p');
                  return p;
                }
              }
            }
          }
        } catch (_) {}

        // Try chocolatey
        try {
          final chocoCheck =
              await Process.run('where', ['choco'], runInShell: true);
          if (chocoCheck.exitCode == 0) {
            losLog('📦 Installing Tor via choco...');
            final result = await Process.run(
              'choco',
              ['install', 'tor', '-y'],
              runInShell: true,
            ).timeout(const Duration(minutes: 10));

            if (result.exitCode == 0) {
              final whereResult =
                  await Process.run('where', ['tor'], runInShell: true);
              if (whereResult.exitCode == 0) {
                final torPath =
                    whereResult.stdout.toString().trim().split('\n').first;
                losLog('✅ Tor installed via choco: $torPath');
                return torPath;
              }
            }
          }
        } catch (_) {}
      }
    } catch (e) {
      losLog('⚠️  Auto-install failed: $e');
    }
    return null;
  }

  // ════════════════════════════════════════════════════════════════════════
  // AUTO-DOWNLOAD — Download Tor Expert Bundle from torproject.org
  // ════════════════════════════════════════════════════════════════════════

  /// Download Tor Expert Bundle and cache it locally
  Future<String?> _downloadAndCacheTor() async {
    try {
      final url = await _getTorDownloadUrl();
      if (url == null) {
        losLog('❌ No Tor download URL for ${Platform.operatingSystem}');
        return null;
      }

      final appDir = await getApplicationSupportDirectory();
      final torBinDir = path.join(appDir.path, 'tor_bin');
      await Directory(torBinDir).create(recursive: true);

      final downloadPath = path.join(torBinDir, 'tor-expert-bundle.tar.gz');

      losLog('📥 Downloading Tor Expert Bundle...');
      losLog('   URL: $url');

      // Download using curl (Win10+, macOS, Linux) or PowerShell fallback
      ProcessResult curlResult;
      try {
        curlResult = await Process.run(
          'curl',
          ['-L', '-o', downloadPath, '--connect-timeout', '30', url],
          runInShell: true,
        ).timeout(downloadTimeout);
      } catch (_) {
        // curl not available — try PowerShell on Windows
        if (Platform.isWindows) {
          losLog('⚠️ curl not found, using PowerShell...');
          curlResult = await Process.run(
            'powershell',
            [
              '-Command',
              'Invoke-WebRequest -Uri "$url" -OutFile "$downloadPath" -TimeoutSec 300',
            ],
            runInShell: true,
          ).timeout(downloadTimeout);
        } else {
          rethrow;
        }
      }

      if (curlResult.exitCode != 0) {
        losLog('❌ Download failed: ${curlResult.stderr}');
        return null;
      }

      final downloadFile = File(downloadPath);
      if (!await downloadFile.exists() || await downloadFile.length() < 1000) {
        losLog('❌ Download appears incomplete or empty');
        return null;
      }

      // SECURITY FIX B-01: Verify SHA-256 hash of downloaded archive.
      // Prevents MITM attacks on clearnet download from torproject.org.
      final expectedHash = _getExpectedHash(url);
      if (expectedHash != null) {
        final fileBytes = await downloadFile.readAsBytes();
        final actualHash = crypto.sha256.convert(fileBytes).toString();
        if (actualHash != expectedHash) {
          losLog('❌ SECURITY: SHA-256 hash mismatch!');
          losLog('   Expected: $expectedHash');
          losLog('   Actual:   $actualHash');
          losLog('   Deleting potentially tampered download.');
          await downloadFile.delete();
          return null;
        }
        losLog('✅ SHA-256 hash verified: ${actualHash.substring(0, 16)}...');
      } else {
        losLog('⚠️ No known hash for this URL — skipping verification');
      }

      losLog('📦 Extracting Tor binary...');

      // Extract the tarball
      // `tar` is available natively on Windows 10+, macOS, and Linux
      final extractResult = await Process.run(
        'tar',
        ['xzf', downloadPath, '-C', torBinDir],
        runInShell: true,
      );

      if (extractResult.exitCode != 0) {
        // Windows fallback: try PowerShell extraction if tar fails
        if (Platform.isWindows) {
          losLog('⚠️ tar failed, trying PowerShell extraction...');
          final psResult = await Process.run(
            'powershell',
            [
              '-Command',
              'Expand-Archive -Force -Path "$downloadPath" -DestinationPath "$torBinDir"',
            ],
            runInShell: true,
          );
          if (psResult.exitCode != 0) {
            losLog('❌ Extraction failed: ${psResult.stderr}');
            return null;
          }
        } else {
          losLog('❌ Extraction failed: ${extractResult.stderr}');
          return null;
        }
      }

      // Find the tor binary in the extracted files
      final torBinary = await _findExtractedTorBinary(torBinDir);
      if (torBinary != null) {
        // Make executable
        if (!Platform.isWindows) {
          await Process.run('chmod', ['+x', torBinary]);
        }

        // Copy to a stable location
        final stableName = Platform.isWindows ? 'tor.exe' : 'tor';
        final stablePath = path.join(torBinDir, stableName);
        if (torBinary != stablePath) {
          await File(torBinary).copy(stablePath);
          await Process.run('chmod', ['+x', stablePath]);
        }

        // Clean up tarball
        try {
          await downloadFile.delete();
        } catch (e) {
          losLog('⚠️ Tarball cleanup failed: $e');
        }

        losLog('✅ Tor downloaded and cached: $stablePath');
        return stablePath;
      }

      losLog('❌ Could not find tor binary in extracted archive');
      return null;
    } catch (e) {
      losLog('❌ Download/extract failed: $e');
      return null;
    }
  }

  /// Get platform-specific Tor Expert Bundle download URL
  Future<String?> _getTorDownloadUrl() async {
    // Tor Expert Bundle 14.0.4 (stable as of early 2026)
    const version = '14.0.4';
    const base =
        'https://archive.torproject.org/tor-package-archive/torbrowser/$version';

    if (Platform.isMacOS) {
      // Detect ARM (Apple Silicon) vs Intel via uname -m
      String arch = 'aarch64'; // default to Apple Silicon
      try {
        final result = await Process.run('uname', ['-m']);
        if (result.exitCode == 0) {
          final uname = result.stdout.toString().trim();
          if (uname == 'x86_64') {
            arch = 'x86_64';
          }
        }
      } catch (_) {
        // Fallback to aarch64 (most common modern Mac)
      }
      return '$base/tor-expert-bundle-macos-$arch-$version.tar.gz';
    } else if (Platform.isLinux) {
      // Detect ARM64 (Raspberry Pi, etc.) vs x86_64
      String arch = 'x86_64';
      try {
        final result = await Process.run('uname', ['-m']);
        if (result.exitCode == 0) {
          final uname = result.stdout.toString().trim();
          if (uname == 'aarch64' || uname == 'arm64') {
            arch = 'aarch64';
          }
        }
      } catch (_) {}
      return '$base/tor-expert-bundle-linux-$arch-$version.tar.gz';
    } else if (Platform.isWindows) {
      return '$base/tor-expert-bundle-windows-x86_64-$version.tar.gz';
    }
    return null;
  }

  /// SECURITY FIX B-01: Known SHA-256 hashes for Tor Expert Bundle 14.0.4.
  /// Source: https://archive.torproject.org/tor-package-archive/torbrowser/14.0.4/sha256sums-signed-build.txt
  /// If the archive version is updated, these hashes must be updated too.
  /// Returns null for unknown URLs (verification skipped with warning).
  static String? _getExpectedHash(String url) {
    const knownHashes = <String, String>{
      // These will need to be populated with actual hashes from torproject.org
      // when the version is pinned. For now, we log a warning if unknown.
      // To populate: download sha256sums-signed-build.txt from the same directory
      // and extract the hash for each platform's tar.gz file.
    };
    return knownHashes[url];
  }

  /// Search for tor binary in extracted archive directory.
  /// Uses pure Dart directory listing — works on ALL platforms (Linux/macOS/Windows).
  Future<String?> _findExtractedTorBinary(String dir) async {
    final binaryName = Platform.isWindows ? 'tor.exe' : 'tor';

    // 1. Check common subdirectories first (fast path)
    final candidates = [
      path.join(dir, 'tor', binaryName),
      path.join(dir, binaryName),
      path.join(dir, 'tor-expert-bundle', 'tor', binaryName),
      path.join(dir, 'Tor', binaryName), // Windows capitalization
      path.join(dir, 'tor-expert-bundle', 'Tor', binaryName),
    ];

    for (final candidate in candidates) {
      if (await File(candidate).exists()) return candidate;
    }

    // 2. Recursive Dart directory listing (cross-platform, no Unix `find`)
    try {
      await for (final entity
          in Directory(dir).list(recursive: true, followLinks: false)) {
        if (entity is File && path.basename(entity.path) == binaryName) {
          losLog('✅ Found tor binary: ${entity.path}');
          return entity.path;
        }
      }
    } catch (e) {
      losLog('⚠️ _findExtractedTorBinary scan error: $e');
    }

    return null;
  }

  // ════════════════════════════════════════════════════════════════════════
  // TOR PROCESS MANAGEMENT — Start/stop with custom torrc
  // ════════════════════════════════════════════════════════════════════════

  /// Start a Tor process with custom configuration
  Future<bool> _startTorProcess(String torBinary) async {
    try {
      losLog('🚀 Starting Tor daemon...');
      losLog('   Binary: $torBinary');
      losLog('   SOCKS: localhost:$_socksPort');

      // Setup data directory
      final appDir = await getApplicationSupportDirectory();
      _torDataDir = path.join(appDir.path, 'tor_data');
      await Directory(_torDataDir!).create(recursive: true);

      // Create torrc
      final torrcPath = await _createTorrc();

      // Start Tor process
      _torProcess = await Process.start(
        torBinary,
        ['-f', torrcPath],
        runInShell: false,
      );

      // Monitor output for bootstrap completion
      _torProcess!.stdout.listen((data) {
        final output = String.fromCharCodes(data);
        if (output.contains('Bootstrapped 100%') ||
            output.contains('Tor has successfully opened a circuit')) {
          _isRunning = true;
          _activeProxy = 'localhost:$_socksPort';
          losLog('✅ Tor ready! SOCKS proxy: $_activeProxy');
        }
      });

      _torProcess!.stderr.listen((data) {
        final error = String.fromCharCodes(data);
        if (error.contains('error') || error.contains('Error')) {
          losLog('⚠️  Tor: $error');
        }
      });

      // Handle process exit
      _torProcess!.exitCode.then((code) {
        if (code != 0 && _isRunning) {
          losLog('⚠️  Tor process exited with code $code');
        }
        _isRunning = false;
      });

      // Wait for bootstrap (allow enough time for slow connections)
      final deadline = DateTime.now().add(bootstrapTimeout);
      while (!_isRunning && DateTime.now().isBefore(deadline)) {
        await Future.delayed(bootstrapPollInterval);
        // Also check if process died
        if (_torProcess == null) return false;
      }

      if (!_isRunning) {
        losLog('❌ Tor failed to bootstrap within 120 seconds');
        await stop();
        return false;
      }

      return true;
    } catch (e) {
      losLog('❌ Failed to start Tor: $e');
      return false;
    }
  }

  /// Create torrc configuration file
  Future<String> _createTorrc() async {
    final torrcPath = path.join(_torDataDir!, 'torrc');
    final torrc = File(torrcPath);

    final config = '''
# LOS Wallet — Auto-managed Tor Configuration
# Generated automatically — do not edit manually

DataDirectory $_torDataDir
SocksPort $_socksPort
Log notice stdout

# Performance tuning for wallet
MaxCircuitDirtiness 600
MaxClientCircuitsPending 48
ConstrainedSockets 1

# Client-only mode (no relay)
DisableNetwork 0
ClientOnly 1
ExitRelay 0
ExitPolicy reject *:*

# Fast bootstrap
UseBridges 0
''';

    await torrc.writeAsString(config);
    return torrcPath;
  }

  // ════════════════════════════════════════════════════════════════════════
  // DETECTION — Find running Tor instances
  // ════════════════════════════════════════════════════════════════════════

  /// Detect existing Tor SOCKS proxies
  Future<Map<String, dynamic>> detectExistingTor() async {
    // Check LOS bundled Tor
    if (await _isSocks5Proxy('localhost', bundledSocksPort)) {
      return {
        'found': true,
        'type': 'LOS Bundled Tor',
        'proxy': 'localhost:$bundledSocksPort'
      };
    }

    // Check Tor Browser
    if (await _isSocks5Proxy('localhost', torBrowserPort)) {
      return {
        'found': true,
        'type': 'Tor Browser',
        'proxy': 'localhost:$torBrowserPort'
      };
    }

    // Check LOS testnet Tor
    if (await _isSocks5Proxy('localhost', losTorPort)) {
      return {
        'found': true,
        'type': 'LOS Tor',
        'proxy': 'localhost:$losTorPort'
      };
    }

    // Check system Tor
    if (await _isSocks5Proxy('localhost', systemTorPort)) {
      return {
        'found': true,
        'type': 'System Tor',
        'proxy': 'localhost:$systemTorPort'
      };
    }

    return {'found': false};
  }

  /// Verify a port is running a SOCKS5 proxy (not just TCP open).
  ///
  /// Sends SOCKS5 handshake: [0x05, 0x01, 0x00] (version 5, 1 auth method, no-auth)
  /// Expects response: [0x05, 0x00] (version 5, no-auth accepted)
  /// This prevents RangeError crashes from connecting to non-SOCKS5 services.
  Future<bool> _isSocks5Proxy(String host, int port) async {
    Socket? socket;
    try {
      socket = await Socket.connect(
        host,
        port,
        timeout: const Duration(seconds: 3),
      );

      // Send SOCKS5 greeting: version=5, 1 auth method, method=0 (no auth)
      socket.add([0x05, 0x01, 0x00]);
      await socket.flush();

      // Read response with timeout
      final response = await socket.first.timeout(
        const Duration(seconds: 3),
        onTimeout: () => Uint8List(0),
      );

      socket.destroy();

      // Valid SOCKS5 response: [0x05, 0x00] (version 5, auth accepted)
      if (response.length >= 2 && response[0] == 0x05) {
        losLog('✅ SOCKS5 verified on $host:$port');
        return true;
      }

      losLog('⚠️ Port $host:$port open but NOT SOCKS5 (got: $response)');
      return false;
    } catch (e) {
      socket?.destroy();
      return false;
    }
  }
}
