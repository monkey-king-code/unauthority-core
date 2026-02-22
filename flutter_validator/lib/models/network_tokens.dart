/// USP-01 Token Model for Validator Dashboard (read-only).
///
/// Mirrors the wallet's Token model but only used for dashboard display.
/// All numeric values are Strings to avoid f64 in financial logic.
class Token {
  final String contractAddress;
  final String name;
  final String symbol;
  final int decimals;
  final String totalSupply;
  final bool isWrapped;
  final String wrappedOrigin;
  final String maxSupply;
  final String owner;

  Token({
    required this.contractAddress,
    required this.name,
    required this.symbol,
    required this.decimals,
    required this.totalSupply,
    this.isWrapped = false,
    this.wrappedOrigin = '',
    this.maxSupply = '0',
    this.owner = '',
  });

  factory Token.fromJson(Map<String, dynamic> json) {
    return Token(
      contractAddress: json['contract_address'] ?? '',
      name: json['name'] ?? '',
      symbol: json['symbol'] ?? '',
      decimals: json['decimals'] ?? 0,
      totalSupply: '${json['total_supply'] ?? 0}',
      isWrapped: json['is_wrapped'] ?? false,
      wrappedOrigin: json['wrapped_origin'] ?? '',
      maxSupply: '${json['max_supply'] ?? 0}',
      owner: json['owner'] ?? '',
    );
  }

  String get shortAddress => contractAddress.length > 16
      ? '${contractAddress.substring(0, 8)}...${contractAddress.substring(contractAddress.length - 8)}'
      : contractAddress;
}

/// DEX Pool Model for Validator Dashboard (read-only).
class DexPool {
  final String contractAddress;
  final String poolId;
  final String tokenA;
  final String tokenB;
  final String reserveA;
  final String reserveB;
  final String totalLp;
  final int feeBps;

  DexPool({
    required this.contractAddress,
    required this.poolId,
    required this.tokenA,
    required this.tokenB,
    required this.reserveA,
    required this.reserveB,
    required this.totalLp,
    required this.feeBps,
  });

  factory DexPool.fromJson(Map<String, dynamic> json) {
    return DexPool(
      contractAddress: json['contract_address'] ?? '',
      poolId: json['pool_id'] ?? '',
      tokenA: json['token_a'] ?? '',
      tokenB: json['token_b'] ?? '',
      reserveA: '${json['reserve_a'] ?? 0}',
      reserveB: '${json['reserve_b'] ?? 0}',
      totalLp: '${json['total_lp'] ?? 0}',
      feeBps: json['fee_bps'] ?? 0,
    );
  }

  String get pairLabel => '$tokenA / $tokenB';
  String get feeDisplay {
    final whole = feeBps ~/ 100;
    final frac = feeBps % 100;
    return frac == 0 ? '$whole%' : '$whole.${frac.toString().padLeft(2, '0')}%';
  }
}
