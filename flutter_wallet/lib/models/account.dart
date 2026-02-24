import '../constants/blockchain.dart';

class Account {
  final String address;
  final int balance; // In CIL (smallest unit)
  final int cilBalance; // Staked/locked CIL
  final List<Transaction> history;
  final String? headBlock; // Latest block hash (frontier) — from /account/:addr
  final int blockCount; // Number of blocks in this account's chain

  Account({
    required this.address,
    required this.balance,
    required this.cilBalance,
    required this.history,
    this.headBlock,
    this.blockCount = 0,
  });

  /// Parse a dynamic value (int, String, double, null) into int safely.
  static int _parseIntField(dynamic value) {
    if (value == null) return 0;
    if (value is int) return value;
    if (value is double) return value.toInt();
    if (value is String) {
      // Try direct int parse first, then LOS string → CIL conversion
      return int.tryParse(value) ?? BlockchainConstants.losStringToCil(value);
    }
    return 0;
  }

  factory Account.fromJson(Map<String, dynamic> json) {
    // Use containsKey instead of != 0 so real zero balances
    // are not skipped. A zero balance from balance_cil is still valid data.
    // Removed duplicate containsKey check that made json['balance'] unreachable.
    final int parsedBalance = json.containsKey('balance_cil')
        ? _parseIntField(json['balance_cil'])
        : _parseIntField(json['balance']);

    return Account(
      address: json['address'] ?? '',
      balance: parsedBalance,
      cilBalance: _parseIntField(json['cil_balance']),
      headBlock: json['head_block'],
      blockCount: json['block_count'] ?? 0,
      history: (json['transactions'] as List?)
              ?.map((tx) => Transaction.fromJson(tx))
              .toList() ??
          (json['history'] as List?)
              ?.map((tx) => Transaction.fromJson(tx))
              .toList() ??
          [],
    );
  }

  /// Balance formatted as LOS display string (integer-only math).
  String get balanceDisplay => BlockchainConstants.formatCilAsLos(balance);

  /// CIL balance formatted as LOS display string.
  String get cilBalanceDisplay =>
      BlockchainConstants.formatCilAsLos(cilBalance);
}

class Transaction {
  final String txid;
  final String from;
  final String to;
  final int amount; // In CIL (smallest unit) for internal consistency
  final int timestamp;
  final String type;
  final String? memo;
  final String? signature;
  final int fee; // Fee in CIL

  Transaction({
    required this.txid,
    required this.from,
    required this.to,
    required this.amount,
    required this.timestamp,
    required this.type,
    this.memo,
    this.signature,
    this.fee = 0,
  });

  /// Parse amount from backend which may be:
  /// - int (from /account endpoint: LOS integer, needs ×CIL_PER_LOS)
  /// - double (rare but possible — convert via string to avoid f64 precision loss)
  /// - String like "10.00000000000" (from /history endpoint: formatted LOS)
  /// Returns value in CIL for internal consistency.
  static int _parseAmount(dynamic value) {
    if (value == null) return 0;
    if (value is int) {
      // Guard: If the value exceeds total LOS supply (21,936,236), it's already
      // in CIL (e.g. from amount_cil field parsed as dynamic). Don't multiply again.
      if (value > BlockchainConstants.totalSupply) return value;
      // /account endpoint returns amount as LOS integer (block.amount / CIL_PER_LOS)
      // Convert to CIL for consistent internal representation
      return value * BlockchainConstants.cilPerLos;
    }
    if (value is double) {
      // Convert via string to use integer-only math (avoids f64 off-by-1 CIL errors)
      return BlockchainConstants.losStringToCil(value.toString());
    }
    if (value is String) {
      // /history endpoint returns "10.00000000000" (formatted LOS string)
      return BlockchainConstants.losStringToCil(value);
    }
    return 0;
  }

  factory Transaction.fromJson(Map<String, dynamic> json) {
    // Determine amount in CIL:
    // - /transaction/{hash} and /block/{hash} return 'amount_cil' (raw CIL integer)
    // - /history and /account return 'amount' (formatted LOS string like "10.00000000000")
    final int parsedAmount;
    if (json.containsKey('amount_cil') && json['amount_cil'] != null) {
      // Raw CIL integer — use directly (no conversion needed)
      parsedAmount = Account._parseIntField(json['amount_cil']);
    } else {
      // Formatted LOS string or legacy integer — convert via _parseAmount
      parsedAmount = _parseAmount(json['amount']);
    }

    return Transaction(
      // Backend returns "hash" not "txid" — map both
      txid: json['txid'] ?? json['hash'] ?? '',
      from: json['from'] ?? json['account'] ?? '',
      to: json['to'] ?? json['link'] ?? '',
      amount: parsedAmount,
      timestamp: json['timestamp'] ?? 0,
      type: (json['type'] ?? 'transfer').toString().toLowerCase(),
      memo: json['memo'],
      signature: json['signature'],
      fee: (json['fee'] is int)
          ? json['fee']
          : (json['fee'] is String ? int.tryParse(json['fee']) ?? 0 : 0),
    );
  }

  /// Amount formatted as LOS display string (integer-only math).
  String get amountDisplay => BlockchainConstants.formatCilAsLos(amount);

  /// Fee formatted as LOS display string (integer-only math).
  String get feeDisplay => BlockchainConstants.formatCilAsLos(fee);
}

class BlockInfo {
  final int height;
  final String hash;
  final int timestamp;
  final int txCount;

  BlockInfo({
    required this.height,
    required this.hash,
    required this.timestamp,
    required this.txCount,
  });

  factory BlockInfo.fromJson(Map<String, dynamic> json) {
    return BlockInfo(
      height: json['height'] ?? 0,
      hash: json['hash'] ?? '',
      timestamp: json['timestamp'] ?? 0,
      // Backend /blocks/recent returns "transactions_count", legacy used "tx_count"
      txCount: json['transactions_count'] ?? json['tx_count'] ?? 0,
    );
  }
}

class ValidatorInfo {
  final String address;
  final int stake;
  final bool isActive;
  final bool connected;
  final bool isGenesis;
  final int uptimeBps; // Basis points: 10000 = 100.00%, integer-only
  final bool hasMinStake;
  final String? onionAddress;

  ValidatorInfo({
    required this.address,
    required this.stake,
    required this.isActive,
    this.connected = false,
    this.isGenesis = false,
    this.uptimeBps = 0,
    this.hasMinStake = false,
    this.onionAddress,
  });

  /// Display-only uptime string: "99.50%"
  String get uptimeDisplay =>
      '${uptimeBps ~/ 100}.${(uptimeBps % 100).toString().padLeft(2, '0')}%';

  factory ValidatorInfo.fromJson(Map<String, dynamic> json) {
    // Parse uptime: backend sends bps (int) or legacy percentage (double)
    final rawUptime = json['uptime_percentage'] ?? json['uptime_bps'] ?? 0;
    final int bps;
    if (rawUptime is double) {
      bps = (rawUptime * 100).round(); // legacy: 99.5 → 9950 bps
    } else if (rawUptime is int) {
      // If value ≤ 100, it's a percentage — convert to bps
      bps = rawUptime <= 100 ? rawUptime * 100 : rawUptime;
    } else {
      bps = 0;
    }
    return ValidatorInfo(
      address: json['address'] ?? '',
      stake: json['stake'] ?? 0,
      // Backend sends both "is_active" and "active" — prefer "active" (computed field)
      isActive: json['active'] ?? json['is_active'] ?? false,
      connected: json['connected'] ?? false,
      isGenesis: json['is_genesis'] ?? false,
      uptimeBps: bps,
      hasMinStake: json['has_min_stake'] ?? false,
      onionAddress: json['onion_address']?.toString(),
    );
  }

  /// Stake in LOS — backend already sends stake as LOS integer.
  /// Returns display string without f64 conversion.
  String get stakeDisplay => '$stake';
}
