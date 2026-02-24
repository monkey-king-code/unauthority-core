import '../utils/log.dart';
import 'dart:convert';
import 'dart:isolate';
import 'dart:typed_data';
import 'package:pointycastle/digests/sha3.dart';
import 'api_service.dart';
import 'wallet_service.dart';
import '../constants/blockchain.dart';

/// Client-side block-lattice block construction for LOS (Validator Edition).
///
/// Matches the backend's Block struct and signing_hash() exactly:
/// - SHA3-256 with CHAIN_ID domain separation (via pointycastle)
/// - PoW anti-spam: 16 leading zero bits
/// - Dilithium5 signature over signing_hash
///
/// This enables fully sovereign transactions — the node only verifies,
/// it never touches the user's secret key.
///
/// Ported from flutter_wallet's BlockConstructionService.
/// NOTE: Uses pure Dart SHA3 for PoW (no native FFI PoW in validator).
class BlockConstructionService {
  final ApiService _api;
  final WalletService _wallet;

  /// Testnet CHAIN_ID = 2. Mainnet = 1.
  /// Must match los_core::CHAIN_ID in the backend.
  static const int chainIdTestnet = 2;
  static const int chainIdMainnet = 1;

  /// Current chain ID — configurable at runtime.
  int chainId;

  /// PoW difficulty: fetched from /node-info (fallback 16 if cached)
  int _powDifficultyBits = 16;

  /// Base fee in CIL — fetched from /node-info (single source of truth).
  int? _baseFeeCil;

  /// Whether protocol params have been fetched from the node.
  bool _protocolFetched = false;

  /// Default base fee in CIL (0.001 LOS = 100_000 CIL)
  static const int defaultBaseFeeCil = 100000;

  /// Maximum PoW iterations before giving up
  static const int maxPowIterations = 50000000;
  static const int blockTypeSend = 0;
  static const int blockTypeReceive = 1;
  static const int blockTypeChange = 2;
  static const int blockTypeMint = 3;
  static const int blockTypeSlash = 4;

  /// 1 LOS = 10^11 CIL
  static int get cilPerLos => BlockchainConstants.cilPerLos;

  BlockConstructionService({
    required ApiService api,
    required WalletService wallet,
    this.chainId = chainIdMainnet,
  })  : _api = api,
        _wallet = wallet;

  /// Update the chain ID at runtime (e.g., switching testnet ↔ mainnet).
  void setChainId(int id) {
    assert(id == chainIdTestnet || id == chainIdMainnet,
        'Invalid chainId: must be $chainIdTestnet or $chainIdMainnet');
    chainId = id;
  }

  /// Fetch protocol parameters from the node's /node-info endpoint.
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
              '⚠️ Chain ID mismatch: validator=$chainId, node=$nodeChainId — updating');
          chainId = nodeChainId;
        }
        _protocolFetched = true;
        losLog(
            '✅ Protocol params: base_fee=$_baseFeeCil CIL, pow=$_powDifficultyBits bits, chain_id=$chainId');
      } else {
        throw Exception(
            '/node-info missing "protocol" field — node upgrade required');
      }
    } catch (e) {
      losLog('⚠️ Failed to fetch protocol params: $e');
      if (_baseFeeCil == null) {
        throw Exception(
            'Cannot send: protocol parameters unavailable from node.');
      }
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
  Future<Map<String, dynamic>> sendTransaction({
    required String to,
    required String amountLosStr,
  }) async {
    losLog(
        '📦 [BlockConstruction.sendTransaction] to=$to, amount=$amountLosStr LOS');
    // 0. Fetch protocol params from node
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

    // 1b. Fetch fee from node
    int fee;
    try {
      final feeData = await _api.getFeeEstimate(address);
      fee = (feeData['estimated_fee_cil'] as num).toInt();
    } catch (e) {
      losLog('⚠️ Fee endpoint unreachable, using cached base fee: $e');
      fee = _baseFeeCil ?? defaultBaseFeeCil;
    }
    losLog('📦 Fee: $fee CIL');

    // 2. Fetch account state (frontier)
    final account = await _api.getAccount(address);
    final previous = account.headBlock ?? '0';

    // 3. Convert amount to CIL using integer-only math
    final amountCil =
        BigInt.from(BlockchainConstants.losStringToCil(amountLosStr));
    final amountLos =
        (amountCil ~/ BigInt.from(BlockchainConstants.cilPerLos)).toInt();

    // 4. Current timestamp
    final timestamp = DateTime.now().millisecondsSinceEpoch ~/ 1000;

    losLog('⛏️ Mining PoW ($powBits-bit difficulty)...');
    final powStart = DateTime.now();

    // 5. Mine PoW in background isolate (pure Dart SHA3)
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
    losLog('⛏️ PoW completed in ${powMs}ms');

    if (powResult == null) {
      throw Exception(
          'PoW failed after $maxPowIterations iterations. Try again.');
    }

    final work = powResult['work'] as int;
    final signingHash = powResult['hash'] as String;

    losLog('🔏 Signing with Dilithium5...');

    // 6. Sign the signing_hash with Dilithium5
    final signature = await _wallet.signTransaction(signingHash);

    // 7. Submit pre-signed block to node
    losLog(
        '📡 Submitting: from=$address to=$to amount=$amountLos amount_cil=${amountCil.toInt()} fee=$fee');

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
    losLog('📦 SUCCESS txid=${txResult['tx_hash'] ?? txResult['txid']}');
    return txResult;
  }

  /// Compute the signing_hash.
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

  /// Mine PoW in a background isolate (pure Dart SHA3-256).
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

    try {
      final result = await Isolate.run(() => _minePoWSync(params));
      return result;
    } catch (e) {
      losLog('⚠️ Isolate.run failed: $e — falling back to main thread');
      return _minePoWSync(params);
    }
  }

  /// Static synchronous PoW mining — runs inside an isolate.
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

    final preData = <int>[];
    // chain_id (u64 LE)
    final cidData = ByteData(8);
    cidData.setUint64(0, chainId, Endian.little);
    preData.addAll(cidData.buffer.asUint8List());
    preData.addAll(utf8.encode(account));
    preData.addAll(utf8.encode(previous));
    preData.add(blockType);
    preData.addAll(_u128ToLeBytesStatic(amount));
    preData.addAll(utf8.encode(link));
    preData.addAll(utf8.encode(publicKey));

    final workOffset = preData.length;
    preData.addAll(List.filled(8, 0)); // placeholder WORK

    final tData = ByteData(8);
    tData.setUint64(0, timestamp, Endian.little);
    preData.addAll(tData.buffer.asUint8List());
    preData.addAll(_u128ToLeBytesStatic(BigInt.from(fee)));

    final buffer = Uint8List.fromList(preData);
    final workBytes = ByteData.sublistView(buffer, workOffset, workOffset + 8);

    final sha3 = SHA3Digest(256);
    final requiredZeroBytes = diffBits ~/ 8;
    final remainingBits = diffBits % 8;
    final mask = remainingBits > 0 ? (0xFF << (8 - remainingBits)) & 0xFF : 0;

    for (int nonce = 0; nonce < maxIter; nonce++) {
      workBytes.setUint64(0, nonce, Endian.little);
      sha3.reset();
      final output = sha3.process(buffer);

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
        final hashHex =
            output.map((b) => b.toRadixString(16).padLeft(2, '0')).join('');
        return {'work': nonce, 'hash': hashHex};
      }
    }
    return null;
  }

  /// Static signing hash computation matching Rust Block::signing_hash().
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
    final cidData = ByteData(8);
    cidData.setUint64(0, chainId, Endian.little);
    data.addAll(cidData.buffer.asUint8List());
    data.addAll(utf8.encode(account));
    data.addAll(utf8.encode(previous));
    data.add(blockType);
    data.addAll(_u128ToLeBytesStatic(amount));
    data.addAll(utf8.encode(link));
    data.addAll(utf8.encode(publicKey));
    final wData = ByteData(8);
    wData.setUint64(0, work, Endian.little);
    data.addAll(wData.buffer.asUint8List());
    final tData = ByteData(8);
    tData.setUint64(0, timestamp, Endian.little);
    data.addAll(tData.buffer.asUint8List());
    data.addAll(_u128ToLeBytesStatic(BigInt.from(fee)));

    return _sha3_256Static(Uint8List.fromList(data));
  }

  /// u128 little-endian byte encoding.
  static Uint8List _u128ToLeBytesStatic(BigInt value) {
    final bytes = Uint8List(16);
    var v = value;
    for (int i = 0; i < 16; i++) {
      bytes[i] = (v & BigInt.from(0xFF)).toInt();
      v = v >> 8;
    }
    return bytes;
  }

  /// SHA3-256 via pointycastle (NIST FIPS 202, matches sha3::Sha3_256 in Rust).
  static String _sha3_256Static(Uint8List input) {
    final digest = SHA3Digest(256);
    final output = digest.process(input);
    return output.map((b) => b.toRadixString(16).padLeft(2, '0')).join('');
  }
}
