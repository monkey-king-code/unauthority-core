import '../utils/log.dart';
import 'dart:convert';
import 'dart:ffi';
import 'dart:io';
import 'package:flutter/foundation.dart';
import 'package:ffi/ffi.dart';

/// Dilithium5 Post-Quantum cryptography service via native Rust FFI.
///
/// Provides real CRYSTALS-Dilithium5 operations matching the los-crypto backend:
/// - Keypair generation (NIST Level 5, 2592-byte PK, 4864-byte SK)
/// - Message signing (detached signatures)
/// - Signature verification
/// - LOS address derivation (Base58Check, identical to backend)
/// - Address validation
///
/// Falls back to a "not available" state if the native library isn't compiled.
/// Use [DilithiumService.isAvailable] to check before calling crypto functions.
class DilithiumService {
  static DilithiumService? _instance;
  static bool _initialized = false;
  static bool _available = false;
  static DynamicLibrary? _lib;

  // FFI function typedefs
  static late int Function() _losPublicKeyBytes;
  static late int Function() _losSecretKeyBytes;
  static late int Function() _losSignatureBytes;
  static late int Function() _losMaxAddressBytes;
  static late int Function(Pointer<Uint8>, int, Pointer<Uint8>, int)
      _losGenerateKeypair;
  static late int Function(
          Pointer<Uint8>, int, Pointer<Uint8>, int, Pointer<Uint8>, int)
      _losGenerateKeypairFromSeed;
  static late int Function(
      Pointer<Uint8>, int, Pointer<Uint8>, int, Pointer<Uint8>, int) _losSign;
  static late int Function(
      Pointer<Uint8>, int, Pointer<Uint8>, int, Pointer<Uint8>, int) _losVerify;
  static late int Function(Pointer<Uint8>, int, Pointer<Uint8>, int)
      _losPublicKeyToAddress;
  static late int Function(Pointer<Uint8>, int) _losValidateAddress;
  static late int Function(Pointer<Uint8>, int, Pointer<Uint8>, int)
      _losBytesToHex;
  static late int Function(Pointer<Uint8>, int, Pointer<Uint8>, int)
      _losHexToBytes;
  static late int Function(Pointer<Uint8>, int, int, int, int, Pointer<Uint64>,
      Pointer<Uint8>, int) _losMinePow;

  // Cached sizes
  static int _pkBytes = 0;
  static int _skBytes = 0;
  static int _sigBytes = 0;
  static int _maxAddrBytes = 0;

  DilithiumService._();

  static DilithiumService get instance {
    _instance ??= DilithiumService._();
    return _instance!;
  }

  /// Whether the native Dilithium5 library is loaded and available.
  static bool get isAvailable => _available;

  /// Public key size in bytes (2592 for Dilithium5)
  static int get publicKeyBytes => _pkBytes;

  /// Secret key size in bytes (4864 for Dilithium5)
  static int get secretKeyBytes => _skBytes;

  /// Signature size in bytes
  static int get signatureBytes => _sigBytes;

  /// Initialize the Dilithium5 native library.
  /// Call once at app startup. If the library is not found, [isAvailable] will be false.
  static Future<void> initialize() async {
    if (_initialized) return;
    _initialized = true;

    try {
      _lib = _loadNativeLibrary();
      if (_lib == null) {
        losLog(
            '⚠️  Dilithium5 native library not found — using SHA256 fallback');
        _available = false;
        return;
      }

      // Bind FFI functions
      _losPublicKeyBytes = _lib!
          .lookupFunction<Int32 Function(), int Function()>(
              'los_public_key_bytes');
      _losSecretKeyBytes = _lib!
          .lookupFunction<Int32 Function(), int Function()>(
              'los_secret_key_bytes');
      _losSignatureBytes = _lib!
          .lookupFunction<Int32 Function(), int Function()>(
              'los_signature_bytes');
      _losMaxAddressBytes = _lib!
          .lookupFunction<Int32 Function(), int Function()>(
              'los_max_address_bytes');

      _losGenerateKeypair = _lib!.lookupFunction<
          Int32 Function(Pointer<Uint8>, Int32, Pointer<Uint8>, Int32),
          int Function(Pointer<Uint8>, int, Pointer<Uint8>,
              int)>('los_generate_keypair');

      _losGenerateKeypairFromSeed = _lib!.lookupFunction<
          Int32 Function(Pointer<Uint8>, Int32, Pointer<Uint8>, Int32,
              Pointer<Uint8>, Int32),
          int Function(Pointer<Uint8>, int, Pointer<Uint8>, int, Pointer<Uint8>,
              int)>('los_generate_keypair_from_seed');

      _losSign = _lib!.lookupFunction<
          Int32 Function(Pointer<Uint8>, Int32, Pointer<Uint8>, Int32,
              Pointer<Uint8>, Int32),
          int Function(Pointer<Uint8>, int, Pointer<Uint8>, int, Pointer<Uint8>,
              int)>('los_sign');

      _losVerify = _lib!.lookupFunction<
          Int32 Function(Pointer<Uint8>, Int32, Pointer<Uint8>, Int32,
              Pointer<Uint8>, Int32),
          int Function(Pointer<Uint8>, int, Pointer<Uint8>, int, Pointer<Uint8>,
              int)>('los_verify');

      _losPublicKeyToAddress = _lib!.lookupFunction<
          Int32 Function(Pointer<Uint8>, Int32, Pointer<Uint8>, Int32),
          int Function(Pointer<Uint8>, int, Pointer<Uint8>,
              int)>('los_public_key_to_address');

      _losValidateAddress = _lib!.lookupFunction<
          Int32 Function(Pointer<Uint8>, Int32),
          int Function(Pointer<Uint8>, int)>('los_validate_address');

      _losBytesToHex = _lib!.lookupFunction<
          Int32 Function(Pointer<Uint8>, Int32, Pointer<Uint8>, Int32),
          int Function(
              Pointer<Uint8>, int, Pointer<Uint8>, int)>('los_bytes_to_hex');

      _losHexToBytes = _lib!.lookupFunction<
          Int32 Function(Pointer<Uint8>, Int32, Pointer<Uint8>, Int32),
          int Function(
              Pointer<Uint8>, int, Pointer<Uint8>, int)>('los_hex_to_bytes');

      _losMinePow = _lib!.lookupFunction<
          Int32 Function(Pointer<Uint8>, Int32, Int32, Uint32, Uint64,
              Pointer<Uint64>, Pointer<Uint8>, Int32),
          int Function(Pointer<Uint8>, int, int, int, int, Pointer<Uint64>,
              Pointer<Uint8>, int)>('los_mine_pow');

      // Query sizes
      _pkBytes = _losPublicKeyBytes();
      _skBytes = _losSecretKeyBytes();
      _sigBytes = _losSignatureBytes();
      _maxAddrBytes = _losMaxAddressBytes();

      _available = true;
      losLog('✅ Dilithium5 native library loaded');
      losLog(
          '   PK: $_pkBytes bytes, SK: $_skBytes bytes, Sig: $_sigBytes bytes');
    } catch (e) {
      losLog('⚠️  Failed to load Dilithium5 native library: $e');
      _available = false;
    }
  }

  /// Load the native library from platform-specific locations.
  static DynamicLibrary? _loadNativeLibrary() {
    final String libName;
    if (Platform.isMacOS) {
      libName = 'liblos_crypto_ffi.dylib';
    } else if (Platform.isLinux) {
      libName = 'liblos_crypto_ffi.so';
    } else if (Platform.isWindows) {
      libName = 'los_crypto_ffi.dll';
    } else {
      return null;
    }

    // Try multiple locations in order of priority
    // SECURITY FIX K-01: In release builds, only search bundled app locations.
    // Development paths are restricted to debug mode to prevent library hijacking.
    final sep = Platform.pathSeparator;
    final execDir =
        Platform.resolvedExecutable.replaceAll(RegExp(r'[/\\][^/\\]+$'), '');
    final searchPaths = <String>[
      // 1. macOS app bundle (release): .app/Contents/Frameworks/
      '$execDir$sep..${sep}Frameworks$sep$libName',
      // 2. Linux bundle: bundle/lib/
      '$execDir${sep}lib$sep$libName',
      // 3. Windows/Linux: next to executable
      '$execDir$sep$libName',
    ];

    // Development paths — only in debug/profile builds
    if (kDebugMode) {
      searchPaths.addAll([
        // 4. Development: relative to flutter_wallet/
        'native${sep}los_crypto_ffi${sep}target${sep}release$sep$libName',
        // 5. Development: from workspace root
        '${Directory.current.path}${sep}native${sep}los_crypto_ffi${sep}target${sep}release$sep$libName',
        // 6. macOS Runner copy
        '${Directory.current.path}${sep}macos${sep}Runner$sep$libName',
        // 7. System library path (fallback)
        libName,
      ]);
    }

    for (final path in searchPaths) {
      try {
        final lib = DynamicLibrary.open(path);
        losLog('✅ Loaded native library from: $path');
        return lib;
      } catch (_) {
        // Try next path
      }
    }
    return null;
  }

  // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  // PUBLIC API — Dart-friendly wrappers around FFI calls
  // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

  /// Generate a new Dilithium5 keypair.
  /// Returns {publicKey: Uint8List, secretKey: Uint8List}
  static DilithiumKeypair generateKeypair() {
    losLog('🔑 [DilithiumService.generateKeypair] Generating keypair...');
    if (!_available) {
      throw StateError('Dilithium5 native library not available');
    }

    final pkPtr = calloc<Uint8>(_pkBytes);
    final skPtr = calloc<Uint8>(_skBytes);

    try {
      final result = _losGenerateKeypair(pkPtr, _pkBytes, skPtr, _skBytes);
      if (result != 0) {
        throw StateError('Keypair generation failed: error $result');
      }

      losLog(
          '🔑 [DilithiumService.generateKeypair] Generated keypair (PK: $_pkBytes bytes, SK: $_skBytes bytes)');
      return DilithiumKeypair(
        publicKey: Uint8List.fromList(pkPtr.asTypedList(_pkBytes)),
        secretKey: Uint8List.fromList(skPtr.asTypedList(_skBytes)),
      );
    } finally {
      // FIX C12-06: Zero secret key memory before freeing
      skPtr.asTypedList(_skBytes).fillRange(0, _skBytes, 0);
      calloc.free(pkPtr);
      calloc.free(skPtr);
    }
  }

  /// Generate a deterministic Dilithium5 keypair from BIP39 seed.
  ///
  /// Same seed always produces the same keypair, enabling wallet recovery
  /// from mnemonic alone. Uses domain-separated SHA-256 → ChaCha20 DRBG.
  static DilithiumKeypair generateKeypairFromSeed(List<int> seed) {
    losLog(
        '🔑 [DilithiumService.generateKeypairFromSeed] Generating from seed (${seed.length} bytes)...');
    if (!_available) {
      throw StateError('Dilithium5 native library not available');
    }
    if (seed.length < 32) {
      throw ArgumentError('Seed must be at least 32 bytes');
    }

    final seedPtr = calloc<Uint8>(seed.length);
    final pkPtr = calloc<Uint8>(_pkBytes);
    final skPtr = calloc<Uint8>(_skBytes);

    try {
      seedPtr.asTypedList(seed.length).setAll(0, seed);

      final result = _losGenerateKeypairFromSeed(
        seedPtr,
        seed.length,
        pkPtr,
        _pkBytes,
        skPtr,
        _skBytes,
      );
      if (result != 0) {
        throw StateError('Seeded keypair generation failed: error $result');
      }

      losLog(
          '🔑 [DilithiumService.generateKeypairFromSeed] Deterministic keypair generated (PK: $_pkBytes bytes, SK: $_skBytes bytes)');
      return DilithiumKeypair(
        publicKey: Uint8List.fromList(pkPtr.asTypedList(_pkBytes)),
        secretKey: Uint8List.fromList(skPtr.asTypedList(_skBytes)),
      );
    } finally {
      // Zero the seed memory before freeing
      seedPtr.asTypedList(seed.length).fillRange(0, seed.length, 0);
      // FIX C12-06: Zero secret key memory before freeing
      skPtr.asTypedList(_skBytes).fillRange(0, _skBytes, 0);
      calloc.free(seedPtr);
      calloc.free(pkPtr);
      calloc.free(skPtr);
    }
  }

  /// Sign a message with a Dilithium5 secret key.
  /// Returns the signature as Uint8List.
  static Uint8List sign(Uint8List message, Uint8List secretKey) {
    losLog(
        '🔑 [DilithiumService.sign] Signing message (${message.length} bytes)...');
    if (!_available) {
      throw StateError('Dilithium5 native library not available');
    }

    final msgPtr = calloc<Uint8>(message.length);
    final skPtr = calloc<Uint8>(secretKey.length);
    final sigPtr = calloc<Uint8>(_sigBytes);

    try {
      msgPtr.asTypedList(message.length).setAll(0, message);
      skPtr.asTypedList(secretKey.length).setAll(0, secretKey);

      final sigLen = _losSign(
        msgPtr,
        message.length,
        skPtr,
        secretKey.length,
        sigPtr,
        _sigBytes,
      );
      if (sigLen < 0) {
        throw StateError('Signing failed: error $sigLen');
      }

      losLog('🔑 [DilithiumService.sign] Signed (sig: $sigLen bytes)');
      return Uint8List.fromList(sigPtr.asTypedList(sigLen));
    } finally {
      // SECURITY FIX S3: Zero secret key memory before freeing to prevent leak
      skPtr.asTypedList(secretKey.length).fillRange(0, secretKey.length, 0);
      calloc.free(msgPtr);
      calloc.free(skPtr);
      calloc.free(sigPtr);
    }
  }

  /// Verify a Dilithium5 signature.
  static bool verify(
      Uint8List message, Uint8List signature, Uint8List publicKey) {
    losLog(
        '🔑 [DilithiumService.verify] Verifying (msg: ${message.length} bytes, sig: ${signature.length} bytes)...');
    if (!_available) return false;

    final msgPtr = calloc<Uint8>(message.length);
    final sigPtr = calloc<Uint8>(signature.length);
    final pkPtr = calloc<Uint8>(publicKey.length);

    try {
      msgPtr.asTypedList(message.length).setAll(0, message);
      sigPtr.asTypedList(signature.length).setAll(0, signature);
      pkPtr.asTypedList(publicKey.length).setAll(0, publicKey);

      final result = _losVerify(
        msgPtr,
        message.length,
        sigPtr,
        signature.length,
        pkPtr,
        publicKey.length,
      );
      final verified = result == 1;
      losLog('🔑 [DilithiumService.verify] Verify result: $verified');
      return verified;
    } finally {
      calloc.free(msgPtr);
      calloc.free(sigPtr);
      calloc.free(pkPtr);
    }
  }

  /// Derive LOS address from Dilithium5 public key.
  /// Returns a string like "LOSHjvLcaLZp..." (Base58Check format).
  static String publicKeyToAddress(Uint8List publicKey) {
    losLog(
        '🔑 [DilithiumService.publicKeyToAddress] Deriving address from PK (${publicKey.length} bytes)...');
    if (!_available) {
      throw StateError('Dilithium5 native library not available');
    }

    final pkPtr = calloc<Uint8>(publicKey.length);
    final addrPtr = calloc<Uint8>(_maxAddrBytes);

    try {
      pkPtr.asTypedList(publicKey.length).setAll(0, publicKey);

      final addrLen = _losPublicKeyToAddress(
        pkPtr,
        publicKey.length,
        addrPtr,
        _maxAddrBytes,
      );
      if (addrLen < 0) {
        throw StateError('Address derivation failed: error $addrLen');
      }

      final bytes = addrPtr.asTypedList(addrLen);
      final address = String.fromCharCodes(bytes);
      losLog(
          '🔑 [DilithiumService.publicKeyToAddress] Address: ${address.substring(0, 8)}...${address.substring(address.length - 4)}');
      return address;
    } finally {
      calloc.free(pkPtr);
      calloc.free(addrPtr);
    }
  }

  /// Validate a LOS address (checksum + format).
  static bool validateAddress(String address) {
    losLog(
        '🔑 [DilithiumService.validateAddress] Validating address: ${address.length > 12 ? '${address.substring(0, 8)}...${address.substring(address.length - 4)}' : address}');
    if (!_available) return false;

    final addrBytes = address.codeUnits;
    final addrPtr = calloc<Uint8>(addrBytes.length);

    try {
      addrPtr.asTypedList(addrBytes.length).setAll(0, addrBytes);
      final valid = _losValidateAddress(addrPtr, addrBytes.length) == 1;
      losLog('🔑 [DilithiumService.validateAddress] Result: $valid');
      return valid;
    } finally {
      calloc.free(addrPtr);
    }
  }

  /// Convert raw bytes to hex string (via native lib).
  static String bytesToHex(Uint8List bytes) {
    if (!_available) {
      return bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join('');
    }

    final inPtr = calloc<Uint8>(bytes.length);
    final outPtr = calloc<Uint8>(bytes.length * 2 + 1);

    try {
      inPtr.asTypedList(bytes.length).setAll(0, bytes);
      final hexLen =
          _losBytesToHex(inPtr, bytes.length, outPtr, bytes.length * 2 + 1);
      if (hexLen < 0) {
        return bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join('');
      }
      return String.fromCharCodes(outPtr.asTypedList(hexLen));
    } finally {
      calloc.free(inPtr);
      calloc.free(outPtr);
    }
  }

  /// Decode hex string to raw bytes.
  static Uint8List hexToBytes(String hex) {
    if (!_available) {
      final result = Uint8List(hex.length ~/ 2);
      for (var i = 0; i < hex.length; i += 2) {
        result[i ~/ 2] = int.parse(hex.substring(i, i + 2), radix: 16);
      }
      return result;
    }

    final hexBytes = hex.codeUnits;
    final inPtr = calloc<Uint8>(hexBytes.length);
    final outPtr = calloc<Uint8>(hexBytes.length); // hex/2, but over-allocate

    try {
      inPtr.asTypedList(hexBytes.length).setAll(0, hexBytes);
      final bytesLen =
          _losHexToBytes(inPtr, hexBytes.length, outPtr, hexBytes.length);
      if (bytesLen < 0) {
        throw StateError('Hex decode failed: $bytesLen');
      }
      return Uint8List.fromList(outPtr.asTypedList(bytesLen));
    } finally {
      calloc.free(inPtr);
      calloc.free(outPtr);
    }
  }

  /// Mine Proof-of-Work using native SHA3-256 (NIST FIPS 202, 100-1000x faster than Dart).
  ///
  /// [buffer] is the pre-built signing_hash input with a placeholder 8-byte
  /// work field at [workOffset]. This function iterates nonces in that field
  /// until it finds one where SHA3-256(buffer) has [difficultyBits] leading
  /// zero bits.
  ///
  /// Returns `{'work': nonce, 'hash': hexHash}` on success, null on failure.
  static Map<String, dynamic>? minePow({
    required Uint8List buffer,
    required int workOffset,
    required int difficultyBits,
    int maxIterations = 50000000,
  }) {
    losLog(
        '⛏️ [DilithiumService.minePow] Mining PoW (difficulty: $difficultyBits bits, max: $maxIterations iterations)...');
    if (!_available) return null;

    final bufPtr = calloc<Uint8>(buffer.length);
    final noncePtr = calloc<Uint64>(1);
    final hashPtr = calloc<Uint8>(64);

    try {
      // Copy buffer to native memory (Rust will mutate the work field)
      bufPtr.asTypedList(buffer.length).setAll(0, buffer);

      final result = _losMinePow(
        bufPtr,
        buffer.length,
        workOffset,
        difficultyBits,
        maxIterations,
        noncePtr,
        hashPtr,
        64,
      );

      if (result < 0) {
        losLog('⚠️ Native PoW failed with error: $result');
        return null;
      }

      final nonce = noncePtr.value;
      final hashHex = String.fromCharCodes(hashPtr.asTypedList(result));

      losLog('⛏️ [DilithiumService.minePow] Found nonce: $nonce');
      return {'work': nonce, 'hash': hashHex};
    } finally {
      calloc.free(bufPtr);
      calloc.free(noncePtr);
      calloc.free(hashPtr);
    }
  }
}

/// Dilithium5 keypair container
class DilithiumKeypair {
  final Uint8List publicKey;
  final Uint8List secretKey;

  const DilithiumKeypair({required this.publicKey, required this.secretKey});

  /// Public key as hex string
  String get publicKeyHex => DilithiumService.bytesToHex(publicKey);

  /// SECURITY FIX A-02: secretKeyHex removed — creating a 9728-char immutable
  /// Dart String that cannot be wiped from memory is a security risk.
  /// Use secretKeyBase64 for storage (shorter, same security), or pass
  /// the Uint8List directly to avoid creating any extra copies.
  String get secretKeyBase64 {
    // base64 creates a shorter string than hex and is equally supported
    // by FlutterSecureStorage. Still an immutable Dart String, but
    // significantly smaller (6488 chars vs 9728 chars for SK).
    return base64Encode(secretKey);
  }
}
