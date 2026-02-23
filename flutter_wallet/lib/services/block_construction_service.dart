import '../utils/log.dart';
import 'dart:convert';
import 'dart:isolate';
import 'package:flutter/foundation.dart';
import 'package:pointycastle/digests/sha3.dart';
import 'api_service.dart';
import 'dilithium_service.dart';
import 'wallet_service.dart';
import '../constants/blockchain.dart';

/// Client-side block-lattice block construction for LOS.
///
/// Matches the backend's Block struct and signing_hash() exactly:
/// - SHA3-256 with CHAIN_ID domain separation (via pointycastle)
/// - PoW anti-spam: 16 leading zero bits
/// - Dilithium5 signature over signing_hash
///
/// This enables fully sovereign transactions — the node only verifies,
/// it never touches the user's secret key.
///
/// The backend's POST /send handler accepts client-provided `timestamp`
/// and `fee` fields when a signature is present, preserving the
/// signing_hash integrity for client-signed blocks.
class BlockConstructionService {
  final ApiService _api;
  final WalletService _wallet;

  /// Testnet CHAIN_ID = 2. Mainnet = 1.
  /// Must match los_core::CHAIN_ID in the backend.
  static const int chainIdTestnet = 2;
  static const int chainIdMainnet = 1;

  /// Current chain ID — configurable at runtime.
  /// Defaults to mainnet (1). Override with setChainId() for testnet.
  int chainId;

  /// PoW difficulty: fetched from /node-info (fallback 16 if cached)
  int _powDifficultyBits = 16;

  /// Base fee in CIL — fetched from /node-info (single source of truth).
  /// null until first fetch; sendTransaction will fail if node unreachable.
  int? _baseFeeCil;

  /// Whether protocol params have been fetched from the node.
  bool _protocolFetched = false;

  /// Default base fee in CIL (0.001 LOS) — used as last-resort fallback
  /// when both /node-info and /fee-estimate are unreachable.
  static const int defaultBaseFeeCil = 100000000;

  /// Maximum PoW iterations before giving up
  static const int maxPowIterations = 50000000;
  static const int blockTypeSend = 0;
  static const int blockTypeReceive = 1;
  static const int blockTypeChange = 2;
  static const int blockTypeMint = 3;
  static const int blockTypeSlash = 4;

  /// 1 LOS = 10^11 CIL — redirects to the single source of truth.
  /// DO NOT define a separate constant here; always use BlockchainConstants.
  static int get cilPerLos => BlockchainConstants.cilPerLos;

  BlockConstructionService({
    required ApiService api,
    required WalletService wallet,
    this.chainId = chainIdMainnet,
  })  : _api = api,
        _wallet = wallet;

  /// Update the chain ID at runtime (e.g., switching between testnet ↔ mainnet).
  void setChainId(int id) {
    assert(id == chainIdTestnet || id == chainIdMainnet,
        'Invalid chainId: must be $chainIdTestnet or $chainIdMainnet');
    chainId = id;
  }

  /// Fetch protocol parameters from the node's /node-info endpoint.
  /// Single source of truth — no hardcoded fee values in the wallet.
  /// Cached after first successful fetch.
  Future<void> _ensureProtocolParams() async {
    if (_protocolFetched) return;
    try {
      await _api.ensureReady();
      final info = await _api.getNodeInfo();
      final protocol = info['protocol'] as Map<String, dynamic>?;
      if (protocol != null) {
        _baseFeeCil =
            (protocol['base_fee_cil'] as num?)?.toInt() ?? defaultBaseFeeCil;
        _powDifficultyBits =
            (protocol['pow_difficulty_bits'] as num?)?.toInt() ?? 16;
        final nodeChainId =
            (protocol['chain_id_numeric'] as num?)?.toInt() ?? chainId;
        if (nodeChainId != chainId) {
          losLog(
              '⚠️ Chain ID mismatch: wallet=$chainId, node=$nodeChainId — updating to node value');
          chainId = nodeChainId;
        }
        _protocolFetched = true;
        losLog(
            '✅ Protocol params from node: base_fee=$_baseFeeCil CIL, pow=$_powDifficultyBits bits, chain_id=$chainId');
      } else {
        throw Exception(
            '/node-info missing "protocol" field — node upgrade required');
      }
    } catch (e) {
      losLog('⚠️ Failed to fetch protocol params: $e');
      if (_baseFeeCil == null) {
        throw Exception(
            'Cannot send: protocol parameters unavailable from node. '
            'Ensure the node is reachable.');
      }
      // If we have cached values from a previous fetch, continue with those
    }
  }

  /// Invalidate cached protocol params (e.g., after network switch).
  void invalidateProtocolCache() {
    _protocolFetched = false;
    _baseFeeCil = null;
  }

  /// Send LOS with full client-side block construction.
  ///
  /// 1. Fetch sender's frontier (head block hash) from node
  /// 2. Construct Block with all fields
  /// 3. Mine PoW (16 zero bits anti-spam)
  /// 4. Sign with Dilithium5 (SHA3-256 signing_hash)
  /// 5. Submit pre-signed to POST /send
  ///
  /// Returns the transaction result from the node.
  Future<Map<String, dynamic>> sendTransaction({
    required String to,
    required String amountLosStr,
  }) async {
    losLog(
        '📦 [BlockConstruction.sendTransaction] from=pending, to=$to, amount=$amountLosStr LOS');
    // 0. Fetch protocol params from node (base_fee, pow_difficulty, chain_id)
    await _ensureProtocolParams();
    final powBits = _powDifficultyBits;

    // 1. Get wallet info
    final walletInfo = await _wallet.getCurrentWallet();
    if (walletInfo == null) throw Exception('No wallet found');

    final address = walletInfo['address']!;
    losLog('📦 [BlockConstruction.sendTransaction] from=$address');
    final publicKeyHex = walletInfo['public_key'];
    if (publicKeyHex == null) {
      throw Exception(
          'No public key available — wallet must have Dilithium5 keypair');
    }

    // 1b. Fetch fee from node (flat BASE_FEE_CIL)
    Map<String, dynamic> feeData;
    try {
      feeData = await _api.getFeeEstimate(address);
    } catch (e) {
      losLog('⚠️ Fee endpoint unreachable, using last-resort base fee: $e');
      // Only as a last resort — callers should be aware this is a fallback
      feeData = {
        'base_fee_cil': _baseFeeCil ?? defaultBaseFeeCil,
        'estimated_fee_cil': _baseFeeCil ?? defaultBaseFeeCil,
        'fee_multiplier': 1,
        'tx_count_in_window': 0,
        'is_fallback': true,
      };
    }
    final fee = (feeData['estimated_fee_cil'] as num).toInt();
    final multiplier = (feeData['fee_multiplier'] as num).toInt();
    losLog(
        '📦 [BlockConstruction.sendTransaction] Fee: $fee CIL (multiplier: ${multiplier}x)');
    if (multiplier > 1) {
      losLog('⚠️ Fee multiplier active: $multiplier× base fee ($fee CIL)');
    }

    // 2. Fetch account state (frontier)
    final account = await _api.getAccount(address);
    final previous = account.headBlock ?? '0';

    // 3. Convert amount to CIL using integer-only math (no f64 precision loss).
    final amountCil =
        BigInt.from(BlockchainConstants.losStringToCil(amountLosStr));
    // Integer LOS for API backward compat — only included if > 0.
    // Sub-LOS amounts (e.g. 0.5 LOS) rely solely on amount_cil.
    final amountLos =
        (amountCil ~/ BigInt.from(BlockchainConstants.cilPerLos)).toInt();

    // 4. Current timestamp
    final timestamp = DateTime.now().millisecondsSinceEpoch ~/ 1000;

    losLog('⛏️ [Send] Mining PoW ($powBits-bit difficulty)...');
    final powStart = DateTime.now();

    // 5. Mine PoW in a background isolate (FIX U1: prevents UI freeze)
    final powResult = await _minePoWInIsolate(
      chainId: chainId,
      account: address,
      previous: previous,
      blockType: blockTypeSend,
      amount: amountCil,
      link: to,
      publicKey: publicKeyHex,
      timestamp: timestamp,
      fee: fee,
    );

    final powMs = DateTime.now().difference(powStart).inMilliseconds;
    losLog('⛏️ [Send] PoW completed in ${powMs}ms');

    if (powResult == null) {
      throw Exception(
          'PoW failed after $maxPowIterations iterations. Try again.');
    }

    final work = powResult['work'] as int;
    final signingHash = powResult['hash'] as String;
    losLog('🔏 [Send] Signing with Dilithium5...');

    // 6. Sign the signing_hash with Dilithium5
    final signStart = DateTime.now();
    final signature = await _wallet.signTransaction(signingHash);
    final signMs = DateTime.now().difference(signStart).inMilliseconds;
    losLog('🔏 [Send] Signature done in ${signMs}ms');

    // 7. Submit pre-signed block to node
    losLog('📡 [Send] Submitting to node...');

    // 7. Submit pre-signed block to node
    // Pass amount_cil so backend uses exact CIL amount (supports sub-LOS precision)
    final txResult = await _api.sendTransaction(
      from: address,
      to: to,
      amount: amountLos,
      signature: signature,
      publicKey: publicKeyHex,
      previous: previous,
      work: work,
      timestamp: timestamp,
      fee: fee,
      amountCil: amountCil.toInt(),
    );
    losLog(
        '📦 [BlockConstruction.sendTransaction] SUCCESS txid=${txResult['tx_hash'] ?? txResult['txid']}');
    return txResult;
  }

  /// Compute the signing_hash — delegates to static method for isolate compatibility.
  /// This is kept as a convenience entry point for non-PoW uses.
  ///
  /// SHA3-256 of:
  ///   chain_id (u64 LE) || account || previous || block_type (1 byte) ||
  ///   amount (u128 LE) || link || public_key || work (u64 LE) ||
  ///   timestamp (u64 LE) || fee (u128 LE)
  static String computeSigningHash({
    required int chainId,
    required String account,
    required String previous,
    required int blockType,
    required BigInt amount,
    required String link,
    required String publicKey,
    required int work,
    required int timestamp,
    required int fee,
  }) {
    return _computeSigningHashStatic(
      chainId: chainId,
      account: account,
      previous: previous,
      blockType: blockType,
      amount: amount,
      link: link,
      publicKey: publicKey,
      work: work,
      timestamp: timestamp,
      fee: fee,
    );
  }

  /// FIX U1: Mine PoW in a background isolate to prevent UI thread freezing.
  /// The heavy nonce loop runs off the main thread via Isolate.run().
  ///
  /// OPTIMIZATION: If the native Rust FFI library is available (DilithiumService),
  /// uses native SHA3-256 for 100-1000x speedup. Falls back to pure Dart
  /// if the native library is not loaded.
  Future<Map<String, dynamic>?> _minePoWInIsolate({
    required int chainId,
    required String account,
    required String previous,
    required int blockType,
    required BigInt amount,
    required String link,
    required String publicKey,
    required int timestamp,
    required int fee,
  }) async {
    // Build the signing hash input buffer (same layout as backend Block::signing_hash())
    final preData = <int>[];
    // chain_id (u64 LE)
    final cidData = ByteData(8);
    cidData.setUint64(0, chainId, Endian.little);
    preData.addAll(cidData.buffer.asUint8List());
    // account
    preData.addAll(utf8.encode(account));
    // previous
    preData.addAll(utf8.encode(previous));
    // block_type
    preData.add(blockType);
    // amount (u128 LE)
    preData.addAll(_u128ToLeBytesStatic(amount));
    // link
    preData.addAll(utf8.encode(link));
    // public_key
    preData.addAll(utf8.encode(publicKey));

    // Record the offset where WORK starts
    final workOffset = preData.length;

    // Placeholder WORK (8 bytes, will be overwritten by miner)
    preData.addAll(List.filled(8, 0));

    // timestamp (u64 LE)
    final tData = ByteData(8);
    tData.setUint64(0, timestamp, Endian.little);
    preData.addAll(tData.buffer.asUint8List());
    // fee (u128 LE)
    preData.addAll(_u128ToLeBytesStatic(BigInt.from(fee)));

    final buffer = Uint8List.fromList(preData);

    // Try native Rust PoW first (100-1000x faster)
    if (DilithiumService.isAvailable) {
      losLog('⚡ [PoW] Using native Rust SHA3-256');
      final result = DilithiumService.minePow(
        buffer: buffer,
        workOffset: workOffset,
        difficultyBits: _powDifficultyBits,
        maxIterations: maxPowIterations,
      );
      if (result != null) {
        losLog('⚡ [PoW] Native: nonce=${result['work']}');
        return result;
      }
      losLog('⚠️ [PoW] Native mining failed, falling back to Dart');
    } else {
      losLog('⚠️ [PoW] Native library not available, using pure Dart');
    }

    // Fallback: pure Dart PoW in isolate
    final params = {
      'chainId': chainId,
      'account': account,
      'previous': previous,
      'blockType': blockType,
      'amount': amount.toString(),
      'link': link,
      'publicKey': publicKey,
      'timestamp': timestamp,
      'fee': fee,
      'maxIter': maxPowIterations,
      'diffBits': _powDifficultyBits,
    };

    // FIX F1: Wrap Isolate.run in try-catch to handle "Computation ended without result"
    // which can occur when the isolate is killed (e.g., during hot restart).
    try {
      final result = await Isolate.run(() => _minePoWSync(params));
      return result;
    } catch (e) {
      losLog('⚠️ [PoW] Isolate.run failed: $e');
      losLog('⚠️ [PoW] Falling back to main-thread PoW (will block UI)...');
      // Last-resort fallback: run in main thread rather than lose the transaction
      return _minePoWSync(params);
    }
  }

  /// Static synchronous PoW mining — runs inside an isolate.
  /// Must be static/top-level for Isolate.run() compatibility.
  ///
  /// OPTIMIZED: Precomputes all static fields once, only mutates the 8-byte
  /// work nonce in a fixed buffer position each iteration. This avoids
  /// rebuilding the entire input buffer and re-encoding strings 50M times.
  static Map<String, dynamic>? _minePoWSync(Map<String, dynamic> params) {
    final chainId = params['chainId'] as int;
    final account = params['account'] as String;
    final previous = params['previous'] as String;
    final blockType = params['blockType'] as int;
    final amount = BigInt.parse(params['amount'] as String);
    final link = params['link'] as String;
    final publicKey = params['publicKey'] as String;
    final timestamp = params['timestamp'] as int;
    final fee = params['fee'] as int;
    final maxIter = params['maxIter'] as int;
    final diffBits = params['diffBits'] as int;

    // Precompute all static parts of the signing hash input.
    // Layout: [chainId(8)] [account] [previous] [blockType(1)] [amount(16)]
    //         [link] [publicKey] [WORK(8)] [timestamp(8)] [fee(16)]
    //
    // Only WORK changes each iteration — we record its byte offset.

    final preData = <int>[];
    // chain_id (u64 LE)
    final cidData = ByteData(8);
    cidData.setUint64(0, chainId, Endian.little);
    preData.addAll(cidData.buffer.asUint8List());
    // account
    preData.addAll(utf8.encode(account));
    // previous
    preData.addAll(utf8.encode(previous));
    // block_type
    preData.add(blockType);
    // amount (u128 LE)
    preData.addAll(_u128ToLeBytesStatic(amount));
    // link
    preData.addAll(utf8.encode(link));
    // public_key
    preData.addAll(utf8.encode(publicKey));

    // Record the offset where WORK starts
    final workOffset = preData.length;

    // Placeholder WORK (8 bytes, will be overwritten)
    preData.addAll(List.filled(8, 0));

    // timestamp (u64 LE)
    final tData = ByteData(8);
    tData.setUint64(0, timestamp, Endian.little);
    preData.addAll(tData.buffer.asUint8List());
    // fee (u128 LE)
    preData.addAll(_u128ToLeBytesStatic(BigInt.from(fee)));

    // Convert to mutable Uint8List for in-place nonce updates
    final buffer = Uint8List.fromList(preData);
    final workBytes = ByteData.sublistView(buffer, workOffset, workOffset + 8);

    final sw = Stopwatch()..start();
    final sha3 = SHA3Digest(256);
    final requiredZeroBytes = diffBits ~/ 8;
    final remainingBits = diffBits % 8;
    final mask = remainingBits > 0 ? (0xFF << (8 - remainingBits)) & 0xFF : 0;

    for (int nonce = 0; nonce < maxIter; nonce++) {
      // Update only the 8-byte work field in-place
      workBytes.setUint64(0, nonce, Endian.little);

      // Hash with reusable SHA3Digest instance
      sha3.reset();
      final output = sha3.process(buffer);

      // Fast leading-zero-bits check (byte-level, no hex conversion)
      bool valid = true;
      for (int i = 0; i < requiredZeroBytes; i++) {
        if (output[i] != 0) {
          valid = false;
          break;
        }
      }
      if (valid && remainingBits > 0) {
        if ((output[requiredZeroBytes] & mask) != 0) valid = false;
      }

      if (valid) {
        final elapsed = sw.elapsedMilliseconds;
        final hashHex =
            output.map((b) => b.toRadixString(16).padLeft(2, '0')).join('');
        losLog(
            '⛏️ PoW found! nonce=$nonce, ${elapsed}ms, ${(nonce / (elapsed / 1000)).round()} H/s');
        return {'work': nonce, 'hash': hashHex};
      }
    }
    losLog(
        '❌ PoW FAILED after $maxIter iterations (${sw.elapsedMilliseconds}ms)');
    return null; // Failed to find valid nonce
  }

  /// Static version of _computeSigningHash for isolate use.
  static String _computeSigningHashStatic({
    required int chainId,
    required String account,
    required String previous,
    required int blockType,
    required BigInt amount,
    required String link,
    required String publicKey,
    required int work,
    required int timestamp,
    required int fee,
  }) {
    final data = <int>[];
    // chain_id (u64 LE)
    final cidData = ByteData(8);
    cidData.setUint64(0, chainId, Endian.little);
    data.addAll(cidData.buffer.asUint8List());
    // account bytes
    data.addAll(utf8.encode(account));
    // previous
    data.addAll(utf8.encode(previous));
    // block_type (1 byte)
    data.add(blockType);
    // amount (u128 LE)
    data.addAll(_u128ToLeBytesStatic(amount));
    // link
    data.addAll(utf8.encode(link));
    // public_key
    data.addAll(utf8.encode(publicKey));
    // work (u64 LE)
    final wData = ByteData(8);
    wData.setUint64(0, work, Endian.little);
    data.addAll(wData.buffer.asUint8List());
    // timestamp (u64 LE)
    final tData = ByteData(8);
    tData.setUint64(0, timestamp, Endian.little);
    data.addAll(tData.buffer.asUint8List());
    // fee (u128 LE)
    data.addAll(_u128ToLeBytesStatic(BigInt.from(fee)));

    return _sha3_256Static(Uint8List.fromList(data));
  }

  /// Static u128 LE for isolate use.
  static Uint8List _u128ToLeBytesStatic(BigInt value) {
    final bytes = Uint8List(16);
    var v = value;
    for (int i = 0; i < 16; i++) {
      bytes[i] = (v & BigInt.from(0xFF)).toInt();
      v = v >> 8;
    }
    return bytes;
  }

  // ════════════════════════════════════════════════════════════════════
  // SHA3-256 — via pointycastle (NIST FIPS 202 SHA-3, matches sha3::Sha3_256)
  // ════════════════════════════════════════════════════════════════

  /// SHA3-256 using pointycastle's verified implementation.
  /// Works in isolates (pure Dart, no FFI).
  static String _sha3_256Static(Uint8List input) {
    final digest = SHA3Digest(256);
    final output = digest.process(input);
    return output.map((b) => b.toRadixString(16).padLeft(2, '0')).join('');
  }
}
