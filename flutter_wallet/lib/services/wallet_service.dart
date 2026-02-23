import '../utils/log.dart';
import 'dart:convert';
import 'package:flutter/foundation.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:crypto/crypto.dart';
import 'package:bip39/bip39.dart' as bip39;
import 'package:pointycastle/digests/blake2b.dart';
// TESTNET ONLY: Ed25519 is used as a fallback when Dilithium5 native lib is not built.
// On mainnet builds (--dart-define=NETWORK=mainnet), all Ed25519 code paths throw
// before reaching the crypto call. Dart tree-shaker eliminates dead branches in AOT.
import 'package:cryptography/cryptography.dart' as ed_crypto;
import 'dilithium_service.dart';
import '../constants/blockchain.dart';

/// Wallet Service for LOS Blockchain
///
/// SECURITY: All secret material (seed, private key) is stored in
/// platform Keychain/Keystore via flutter_secure_storage.
/// Only the address and crypto_mode are in SharedPreferences (non-sensitive).
///
/// Dilithium5 mode (native library available):
/// - Real CRYSTALS-Dilithium5 keypair (PK: 2592 bytes, SK: 4864 bytes)
/// - Deterministic from BIP39 seed via HMAC-SHA512 DRBG seeding
/// - Real LOS address: "LOS" + Base58Check(BLAKE2b-160(pubkey))
/// - Real post-quantum signatures for transactions
///
/// Fallback mode (native library not compiled):
/// - SHA256-based simplified derivation (testnet L1 compatible)
/// - Deterministic from BIP39 seed
///
/// Both modes use BIP39 24-word mnemonic for wallet creation/backup.
class WalletService {
  // Non-sensitive keys (SharedPreferences — survives app reinstall on some platforms)
  static const String _addressKey = 'wallet_address';
  static const String _importModeKey = 'wallet_import_mode';
  static const String _cryptoModeKey = 'wallet_crypto_mode';

  // Sensitive keys (flutter_secure_storage — Keychain/Keystore)
  static const String _seedKey = 'wallet_seed';
  static const String _publicKeyKey = 'wallet_public_key';
  static const String _secretKeyKey = 'wallet_secret_key';

  /// Encrypted storage backed by platform Keychain (iOS/macOS) or Keystore (Android)
  /// useDataProtectionKeyChain: false = legacy file-based keychain (works with ad-hoc signing)
  /// macOS will ask for login password ONCE → click "Always Allow" → never asks again.
  static const _secureStorage = FlutterSecureStorage(
    aOptions: AndroidOptions(encryptedSharedPreferences: true),
    iOptions: IOSOptions(accessibility: KeychainAccessibility.first_unlock),
    mOptions: MacOsOptions(useDataProtectionKeyChain: false),
  );

  /// SECURITY: When true, only Dilithium5 crypto is permitted.
  /// Ed25519 fallback is refused — mainnet requires post-quantum signatures.
  /// Set by ApiService.switchEnvironment() when connecting to mainnet.
  /// Build-time flag: --dart-define=NETWORK=testnet to override
  static bool mainnetMode =
      const String.fromEnvironment('NETWORK', defaultValue: 'mainnet') ==
          'mainnet';

  /// One-time migration from SharedPreferences → SecureStorage
  /// Called on app startup. Silent if already migrated.
  Future<void> migrateFromSharedPreferences() async {
    final prefs = await SharedPreferences.getInstance();
    final existingSeed = prefs.getString(_seedKey);
    if (existingSeed != null) {
      // Migrate secrets to secure storage
      await _secureStorage.write(key: _seedKey, value: existingSeed);
      final pk = prefs.getString(_publicKeyKey);
      if (pk != null) await _secureStorage.write(key: _publicKeyKey, value: pk);
      final sk = prefs.getString(_secretKeyKey);
      if (sk != null) {
        await _secureStorage.write(key: _secretKeyKey, value: sk);
      }
      // Remove secrets from SharedPreferences
      await prefs.remove(_seedKey);
      await prefs.remove(_publicKeyKey);
      await prefs.remove(_secretKeyKey);
      losLog('🔒 Migrated wallet secrets to secure storage');
    }
  }

  /// Whether this wallet uses real Dilithium5 cryptography
  Future<bool> isDilithium5Wallet() async {
    final prefs = await SharedPreferences.getInstance();
    final result = prefs.getString(_cryptoModeKey) == 'dilithium5';
    losLog('💰 [WalletService.isDilithium5Wallet] Result: $result');
    return result;
  }

  /// Generate new wallet with real Dilithium5 keypair (if available)
  Future<Map<String, String>> generateWallet() async {
    losLog('💰 [WalletService.generateWallet] Generating wallet...');
    final mnemonic = bip39.generateMnemonic(strength: 256);
    String address;
    String cryptoMode;

    if (DilithiumService.isAvailable) {
      // Real Dilithium5 keypair — deterministic from seed
      final seed = bip39.mnemonicToSeed(mnemonic);
      try {
        final keypair = DilithiumService.generateKeypairFromSeed(seed);
        address = DilithiumService.publicKeyToAddress(keypair.publicKey);
        cryptoMode = 'dilithium5';

        // Secrets → Keychain/Keystore
        await _secureStorage.write(key: _seedKey, value: mnemonic);
        await _secureStorage.write(
          key: _publicKeyKey,
          value: keypair.publicKeyHex,
        );
        await _secureStorage.write(
          key: _secretKeyKey,
          value: keypair.secretKeyBase64,
        );

        // Non-sensitive → SharedPreferences
        final prefs = await SharedPreferences.getInstance();
        await prefs.setString(_addressKey, address);
        await prefs.setString(_importModeKey, 'mnemonic');
        await prefs.setString(_cryptoModeKey, cryptoMode);

        losLog('🔐 Dilithium5 wallet created (deterministic from seed)');
        losLog('   Address: $address');
        losLog('   PK: ${keypair.publicKey.length} bytes');
      } finally {
        // Zero BIP39 seed bytes in Dart memory after keypair generation
        seed.fillRange(0, seed.length, 0);
      }
    } else {
      // Refuse Ed25519 fallback on mainnet.
      // Mainnet requires Dilithium5 post-quantum signatures.
      if (mainnetMode) {
        throw Exception(
          'MAINNET SECURITY: Dilithium5 native library required for wallet generation. '
          'Ed25519 fallback is disabled on mainnet. Please build the native library.',
        );
      }

      // Ed25519 + BLAKE2b fallback (TESTNET ONLY — matches los-crypto address format)
      final seed = bip39.mnemonicToSeed(mnemonic);
      try {
        // Also store Ed25519 public key hex so sendTransaction() has it
        final privateSeed = Uint8List.fromList(seed.sublist(0, 32));
        final algorithm = ed_crypto.Ed25519();
        final keyPair =
            await algorithm.newKeyPairFromSeed(privateSeed.toList());
        final pubKey = await keyPair.extractPublicKey();
        final pubKeyHex = pubKey.bytes
            .map((b) => b.toRadixString(16).padLeft(2, '0'))
            .join('');

        address = _deriveAddressFromPublicKey(Uint8List.fromList(pubKey.bytes));
        cryptoMode = 'ed25519';

        await _secureStorage.write(key: _seedKey, value: mnemonic);
        await _secureStorage.write(key: _publicKeyKey, value: pubKeyHex);

        final prefs = await SharedPreferences.getInstance();
        await prefs.setString(_addressKey, address);
        await prefs.setString(_importModeKey, 'mnemonic');
        await prefs.setString(_cryptoModeKey, cryptoMode);

        losLog(
          '⚠️ Ed25519 fallback wallet — TESTNET ONLY (Dilithium5 native lib not loaded)',
        );
      } finally {
        // Zero BIP39 seed bytes in SHA256 fallback path too
        seed.fillRange(0, seed.length, 0);
      }
    }

    losLog(
        '💰 [WalletService.generateWallet] Wallet generated: $address, mode: $cryptoMode');
    return {
      'mnemonic': mnemonic,
      'address': address,
      'crypto_mode': cryptoMode,
    };
  }

  /// Import wallet from mnemonic.
  ///
  /// Dilithium5: Deterministic from seed — same mnemonic = same address.
  /// SHA256 fallback: Also deterministic.
  Future<Map<String, String>> importWallet(String mnemonic) async {
    losLog(
        '💰 [WalletService.importWallet] Importing wallet from mnemonic (${mnemonic.split(' ').length} words)...');
    if (!bip39.validateMnemonic(mnemonic)) {
      throw Exception('Invalid mnemonic phrase');
    }

    String address;
    String cryptoMode;

    if (DilithiumService.isAvailable) {
      // Deterministic keypair from BIP39 seed
      final seed = bip39.mnemonicToSeed(mnemonic);
      try {
        final keypair = DilithiumService.generateKeypairFromSeed(seed);
        address = DilithiumService.publicKeyToAddress(keypair.publicKey);
        cryptoMode = 'dilithium5';

        // Secrets → Keychain/Keystore
        await _secureStorage.write(key: _seedKey, value: mnemonic);
        await _secureStorage.write(
          key: _publicKeyKey,
          value: keypair.publicKeyHex,
        );
        await _secureStorage.write(
          key: _secretKeyKey,
          value: keypair.secretKeyBase64,
        );

        // Non-sensitive → SharedPreferences
        final prefs = await SharedPreferences.getInstance();
        await prefs.setString(_addressKey, address);
        await prefs.setString(_importModeKey, 'mnemonic');
        await prefs.setString(_cryptoModeKey, cryptoMode);

        losLog(
          '🔐 Dilithium5 wallet restored from mnemonic (deterministic)',
        );
        losLog('   Address: $address');
      } finally {
        // Zero BIP39 seed bytes in Dart memory
        seed.fillRange(0, seed.length, 0);
      }
    } else {
      // Refuse Ed25519 fallback on mainnet.
      if (mainnetMode) {
        throw Exception(
          'MAINNET SECURITY: Dilithium5 native library required for wallet import. '
          'Ed25519 fallback is disabled on mainnet.',
        );
      }

      final seed = bip39.mnemonicToSeed(mnemonic);
      try {
        // Also store Ed25519 public key hex so sendTransaction() has it
        final privateSeed = Uint8List.fromList(seed.sublist(0, 32));
        final algorithm = ed_crypto.Ed25519();
        final keyPair =
            await algorithm.newKeyPairFromSeed(privateSeed.toList());
        final pubKey = await keyPair.extractPublicKey();
        final pubKeyHex = pubKey.bytes
            .map((b) => b.toRadixString(16).padLeft(2, '0'))
            .join('');

        address = _deriveAddressFromPublicKey(Uint8List.fromList(pubKey.bytes));
        cryptoMode = 'ed25519';

        await _secureStorage.write(key: _seedKey, value: mnemonic);
        await _secureStorage.write(key: _publicKeyKey, value: pubKeyHex);

        final prefs = await SharedPreferences.getInstance();
        await prefs.setString(_addressKey, address);
        await prefs.setString(_importModeKey, 'mnemonic');
        await prefs.setString(_cryptoModeKey, cryptoMode);
      } finally {
        seed.fillRange(0, seed.length, 0);
      }
    }

    losLog(
        '💰 [WalletService.importWallet] Imported: $address, mode: $cryptoMode');
    return {
      'mnemonic': mnemonic,
      'address': address,
      'crypto_mode': cryptoMode,
    };
  }

  /// Import by address only (testnet genesis accounts)
  Future<Map<String, String>> importByAddress(String address) async {
    losLog(
        '💰 [WalletService.importByAddress] Importing address-only: $address');
    if (!address.startsWith('LOS') || address.length < 30) {
      throw Exception('Invalid LOS address format');
    }

    final prefs = await SharedPreferences.getInstance();
    await prefs.setString(_addressKey, address);
    await prefs.setString(_importModeKey, 'address');
    await prefs.setString(_cryptoModeKey, 'address_only');
    // Clear any secrets (with try-catch for macOS Keychain issues)
    for (final key in [_seedKey, _publicKeyKey, _secretKeyKey]) {
      try {
        await _secureStorage.delete(key: key);
      } catch (e) {
        losLog('⚠️ SecureStorage.delete($key) failed during import: $e');
      }
    }

    losLog('💰 [WalletService.importByAddress] Address-only import success');
    return {'address': address};
  }

  /// Get current wallet info.
  /// Does NOT return mnemonic by default. Use [includeMnemonic]
  /// only when user explicitly requests seed phrase (e.g. settings backup).
  Future<Map<String, String>?> getCurrentWallet({
    bool includeMnemonic = false,
  }) async {
    losLog(
        '💰 [WalletService.getCurrentWallet] includeMnemonic=$includeMnemonic');
    final prefs = await SharedPreferences.getInstance();
    final address = prefs.getString(_addressKey);
    if (address == null) {
      losLog('💰 [WalletService.getCurrentWallet] No wallet found');
      return null;
    }

    final result = <String, String>{'address': address};
    if (includeMnemonic) {
      final mnemonic = await _secureStorage.read(key: _seedKey);
      if (mnemonic != null) result['mnemonic'] = mnemonic;
    }
    final pk = await _secureStorage.read(key: _publicKeyKey);
    if (pk != null) result['public_key'] = pk;
    final mode = prefs.getString(_cryptoModeKey);
    if (mode != null) result['crypto_mode'] = mode;

    losLog('💰 [WalletService.getCurrentWallet] Address: $address');
    return result;
  }

  /// Get hex-encoded public key (for sending with transactions)
  Future<String?> getPublicKeyHex() async {
    losLog('💰 [WalletService.getPublicKeyHex] Fetching public key hex...');
    final pk = await _secureStorage.read(key: _publicKeyKey);
    losLog(
        '💰 [WalletService.getPublicKeyHex] Result: ${pk != null ? '${pk.length} hex chars' : 'null'}');
    return pk;
  }

  /// Delete wallet — wipes all sensitive and non-sensitive data
  Future<void> deleteWallet() async {
    losLog('💰 [WalletService.deleteWallet] Deleting wallet...');
    final prefs = await SharedPreferences.getInstance();
    await prefs.remove(_addressKey);
    await prefs.remove(_importModeKey);
    await prefs.remove(_cryptoModeKey);
    // Wipe secrets from secure storage
    // Wrap each delete in try-catch — macOS Keychain can throw
    // PlatformException -25244 (errSecInvalidOwner) when app bundle ID changes
    // between debug runs. We must not let this silently abort the logout flow.
    for (final key in [_seedKey, _publicKeyKey, _secretKeyKey]) {
      try {
        await _secureStorage.delete(key: key);
      } catch (e) {
        losLog(
            '⚠️ [WalletService.deleteWallet] SecureStorage.delete($key) failed: $e');
        // On macOS, try deleteAll as nuclear fallback
        try {
          await _secureStorage.deleteAll();
          losLog(
              '⚠️ [WalletService.deleteWallet] deleteAll() succeeded as fallback');
          break; // All keys wiped — no need to continue the loop
        } catch (e2) {
          losLog(
              '⚠️ [WalletService.deleteWallet] deleteAll() also failed: $e2');
          // Continue — prefs are cleared, wallet state is reset even if
          // Keychain items are orphaned. They won't be readable without
          // the matching bundle ID anyway.
        }
      }
    }
    losLog('💰 [WalletService.deleteWallet] Wallet deleted');
  }

  /// Clear wallet — alias for deleteWallet
  Future<void> clearWallet() async {
    losLog('💰 [WalletService.clearWallet] Clearing wallet...');
    await deleteWallet();
    losLog('💰 [WalletService.clearWallet] Wallet cleared');
  }

  /// Check if wallet was imported by address only
  Future<bool> isAddressOnlyImport() async {
    final prefs = await SharedPreferences.getInstance();
    final result = prefs.getString(_importModeKey) == 'address';
    losLog('💰 [WalletService.isAddressOnlyImport] Result: $result');
    return result;
  }

  // ══════════════════════════════════════════════════════════════════════
  // SIGNING
  // ══════════════════════════════════════════════════════════════════════

  /// Sign transaction data with Dilithium5 or SHA256 fallback.
  Future<String> signTransaction(String txData) async {
    losLog(
        '💰 [WalletService.signTransaction] Signing tx (${txData.length} bytes)...');
    final wallet = await getCurrentWallet();
    if (wallet == null) throw Exception('No wallet found');

    final isAddressOnly = await isAddressOnlyImport();
    if (isAddressOnly) {
      return 'address-only-no-local-signing';
    }

    final cryptoMode = wallet['crypto_mode'] ?? 'sha256';

    if (cryptoMode == 'dilithium5' && DilithiumService.isAvailable) {
      final skStored = await _secureStorage.read(key: _secretKeyKey);
      if (skStored == null) {
        throw Exception('Secret key not found in secure storage');
      }

      // Secret key is stored as Base64 (see DilithiumKeypair.secretKeyBase64),
      // NOT hex. Decode accordingly. Detect format by checking for Base64 padding
      // or non-hex characters.
      Uint8List secretKey;
      if (skStored.contains('+') ||
          skStored.contains('/') ||
          skStored.endsWith('=') ||
          RegExp(r'[^0-9a-fA-F]').hasMatch(skStored)) {
        // Base64-encoded secret key (current storage format)
        secretKey = Uint8List.fromList(base64Decode(skStored));
      } else {
        // Legacy hex-encoded secret key (backward compatibility)
        secretKey = DilithiumService.hexToBytes(skStored);
      }
      final message = Uint8List.fromList(utf8.encode(txData));
      try {
        final signature = DilithiumService.sign(message, secretKey);
        final sigHex = DilithiumService.bytesToHex(signature);
        losLog(
            '💰 [WalletService.signTransaction] Signed (sig: ${sigHex.length} hex chars), mode: dilithium5');
        return sigHex;
      } finally {
        // Zero secret key in Dart memory after signing.
        // FFI layer zeros its copy, but Dart Uint8List remains until GC.
        secretKey.fillRange(0, secretKey.length, 0);
      }
    } else {
      // Fallback: Ed25519 signing when native Dilithium5 is unavailable.
      // Refuse Ed25519 fallback on mainnet.
      if (mainnetMode) {
        throw Exception(
          'MAINNET SECURITY: Dilithium5 native library required for signing. '
          'Ed25519 fallback is disabled on mainnet.',
        );
      }

      // TESTNET ONLY: Uses the first 32 bytes of BIP39 seed as Ed25519 private key.
      // Ed25519 is a proper signature scheme — verifiable without the private key.
      // However, the node MUST be configured to accept Ed25519 signatures for this
      // to work on a real network. On functional testnet, signature format is lenient.
      final mnemonic = await _secureStorage.read(key: _seedKey);
      if (mnemonic == null) throw Exception('No mnemonic for signing');
      final seed = bip39.mnemonicToSeed(mnemonic);
      final privateKeyBytes = seed.sublist(0, 32);
      try {
        final algorithm = ed_crypto.Ed25519();
        final keyPair = await algorithm.newKeyPairFromSeed(privateKeyBytes);
        final message = utf8.encode(txData);
        final signature = await algorithm.sign(message, keyPair: keyPair);
        // Return hex-encoded signature bytes
        final sigHex = signature.bytes
            .map((b) => b.toRadixString(16).padLeft(2, '0'))
            .join('');
        losLog(
            '💰 [WalletService.signTransaction] Signed (sig: ${sigHex.length} hex chars), mode: ed25519');
        return sigHex;
      } finally {
        // Zero private key material after signing
        privateKeyBytes.fillRange(0, privateKeyBytes.length, 0);
        seed.fillRange(0, seed.length, 0);
      }
    }
  }

  // ══════════════════════════════════════════════════════════════════════
  // ADDRESS DERIVATION — Ed25519 + BLAKE2b-160 + Base58Check
  // Matches los-crypto::public_key_to_address() EXACTLY
  // ══════════════════════════════════════════════════════════════════════

  /// Base58 alphabet (Bitcoin-style)
  static const _base58Chars =
      '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';

  /// Base58 encode bytes (matches bs58 crate in Rust)
  static String _base58Encode(Uint8List input) {
    if (input.isEmpty) return '';

    // Count leading zeros
    int zeros = 0;
    for (final b in input) {
      if (b != 0) break;
      zeros++;
    }

    // Convert to base58
    final encoded = <int>[];
    var num = BigInt.zero;
    for (final b in input) {
      num = (num << 8) | BigInt.from(b);
    }
    while (num > BigInt.zero) {
      final rem = (num % BigInt.from(58)).toInt();
      num = num ~/ BigInt.from(58);
      encoded.insert(0, rem);
    }

    // Add leading '1' for each leading zero byte
    final result = StringBuffer();
    for (int i = 0; i < zeros; i++) {
      result.write('1');
    }
    for (final idx in encoded) {
      result.write(_base58Chars[idx]);
    }
    return result.toString();
  }

  /// Derive LOS address from Ed25519 public key (los-crypto compatible).
  ///
  /// Algorithm (IDENTICAL to Rust los-crypto::public_key_to_address):
  /// 1. BLAKE2b-512(pubkey) → first 20 bytes
  /// 2. Prepend VERSION_BYTE 0x4A
  /// 3. Checksum = SHA256(SHA256(payload))[0:4]
  /// 4. Base58(payload + checksum)
  /// 5. Prepend "LOS"
  static String _deriveAddressFromPublicKey(Uint8List publicKey) {
    const versionByte = BlockchainConstants.addressVersionByte;

    // 1. BLAKE2b-512 hash → first 20 bytes
    final blake2b = Blake2bDigest(digestSize: 64);
    final hash = Uint8List(64);
    blake2b.update(publicKey, 0, publicKey.length);
    blake2b.doFinal(hash, 0);
    final pubkeyHash = hash.sublist(0, 20);

    // 2. Payload: version + hash (21 bytes)
    final payload = Uint8List(21);
    payload[0] = versionByte;
    payload.setRange(1, 21, pubkeyHash);

    // 3. Checksum: SHA256(SHA256(payload)) → first 4 bytes
    final hash1 = sha256.convert(payload);
    final hash2 = sha256.convert(hash1.bytes);
    final checksum = hash2.bytes.sublist(0, 4);

    // 4. Combine: payload + checksum (25 bytes)
    final addressBytes = Uint8List(25);
    addressBytes.setRange(0, 21, payload);
    addressBytes.setRange(21, 25, checksum);

    // 5. Base58 encode + "LOS" prefix
    return 'LOS${_base58Encode(addressBytes)}';
  }

  /// Derive Ed25519 keypair from BIP39 seed and return LOS address.
  /// Uses seed[0:32] as Ed25519 private seed → public key → BLAKE2b address.
  /// This matches los-crypto's address format for Ed25519 keys.
  Future<String> _deriveAddressEd25519(List<int> seed) async {
    losLog(
        '💰 [WalletService._deriveAddressEd25519] Deriving Ed25519 address...');
    final privateSeed = Uint8List.fromList(seed.sublist(0, 32));
    try {
      // Ed25519 keypair from 32-byte seed using cryptography package
      final algorithm = ed_crypto.Ed25519();
      final keyPair = await algorithm.newKeyPairFromSeed(privateSeed.toList());
      final pubKey = await keyPair.extractPublicKey();

      final address =
          _deriveAddressFromPublicKey(Uint8List.fromList(pubKey.bytes));
      losLog('💰 [WalletService._deriveAddressEd25519] Address: $address');
      return address;
    } finally {
      privateSeed.fillRange(0, privateSeed.length, 0);
    }
  }

  /// Derive address from mnemonic without persisting any keys.
  /// Used by AccountManagementScreen to create new sub-accounts
  /// without overwriting the primary wallet's SecureStorage keys.
  ///
  /// MAINNET: Ed25519 derivation is forbidden — Dilithium5 is required.
  /// This method is testnet-only; mainnet callers must use DilithiumService directly.
  Future<String> deriveAddressFromMnemonic(String mnemonic) async {
    // SECURITY: Block Ed25519 address derivation on mainnet builds
    if (mainnetMode) {
      throw Exception(
          'MAINNET SECURITY: Ed25519 address derivation is forbidden on mainnet. '
          'Use DilithiumService for all key operations.');
    }
    losLog(
        '💰 [WalletService.deriveAddressFromMnemonic] Deriving address from mnemonic (${mnemonic.split(' ').length} words)...');
    final seed = bip39.mnemonicToSeed(mnemonic);
    try {
      final address = await _deriveAddressEd25519(seed);
      losLog('💰 [WalletService.deriveAddressFromMnemonic] Address: $address');
      return address;
    } finally {
      seed.fillRange(0, seed.length, 0);
    }
  }
}
