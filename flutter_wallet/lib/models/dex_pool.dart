/// DEX AMM Models - Liquidity Pool & Swap Types
///
/// All amounts are integer strings to avoid f64 precision loss.
/// Matches the DEX AMM WASM contract and los-node REST API.
library;

class DexPool {
  final String contractAddress;
  final String poolId;
  final String tokenA; // Contract address or "LOS" for native
  final String tokenB;
  final String reserveA; // Integer string
  final String reserveB;
  final String totalLp; // Total LP shares
  final int feeBps; // Fee in basis points (e.g. 30 = 0.3%)
  final String creator;
  final int lastTrade; // Unix timestamp
  final String spotPriceScaled; // Scaled by 10^12

  DexPool({
    required this.contractAddress,
    required this.poolId,
    required this.tokenA,
    required this.tokenB,
    required this.reserveA,
    required this.reserveB,
    required this.totalLp,
    required this.feeBps,
    required this.creator,
    this.lastTrade = 0,
    this.spotPriceScaled = '0',
  });

  factory DexPool.fromJson(Map<String, dynamic> json, {String contract = ''}) {
    return DexPool(
      contractAddress:
          contract.isNotEmpty ? contract : (json['contract'] ?? '').toString(),
      poolId: (json['pool_id'] ?? '0').toString(),
      tokenA: (json['token_a'] ?? '').toString(),
      tokenB: (json['token_b'] ?? '').toString(),
      reserveA: (json['reserve_a'] ?? '0').toString(),
      reserveB: (json['reserve_b'] ?? '0').toString(),
      totalLp: (json['total_lp'] ?? '0').toString(),
      feeBps: (json['fee_bps'] is int)
          ? json['fee_bps']
          : int.tryParse(json['fee_bps']?.toString() ?? '30') ?? 30,
      creator: (json['creator'] ?? '').toString(),
      lastTrade: (json['last_trade'] is int) ? json['last_trade'] : 0,
      spotPriceScaled: (json['spot_price_scaled'] ?? '0').toString(),
    );
  }

  /// Fee as display percentage (e.g. "0.30%")
  String get feeDisplay {
    final whole = feeBps ~/ 100;
    final frac = feeBps % 100;
    return '$whole.${frac.toString().padLeft(2, '0')}%';
  }

  /// Pool pair label (e.g. "MTK / LOS")
  String get pairLabel {
    final a = _shortLabel(tokenA);
    final b = _shortLabel(tokenB);
    return '$a / $b';
  }

  static String _shortLabel(String addr) {
    if (addr == 'LOS' || addr == 'native') return 'LOS';
    if (addr.length <= 10) return addr;
    return '${addr.substring(0, 8)}..';
  }
}

/// Swap quote — estimated output before execution
class DexQuote {
  final String amountOut; // Integer string
  final int feeBps;
  final int priceImpactBps;

  DexQuote({
    required this.amountOut,
    required this.feeBps,
    required this.priceImpactBps,
  });

  factory DexQuote.fromJson(Map<String, dynamic> json) {
    return DexQuote(
      amountOut: (json['amount_out'] ?? '0').toString(),
      feeBps: (json['fee'] is int)
          ? json['fee']
          : int.tryParse(json['fee']?.toString() ?? '0') ?? 0,
      priceImpactBps: (json['price_impact_bps'] is int)
          ? json['price_impact_bps']
          : int.tryParse(json['price_impact_bps']?.toString() ?? '0') ?? 0,
    );
  }

  /// Price impact as display percentage (e.g. "1.00%")
  String get priceImpactDisplay {
    final whole = priceImpactBps ~/ 100;
    final frac = priceImpactBps % 100;
    return '$whole.${frac.toString().padLeft(2, '0')}%';
  }
}

/// User's LP position in a pool
class LpPosition {
  final String contractAddress;
  final String poolId;
  final String user;
  final String lpShares; // Integer string

  LpPosition({
    required this.contractAddress,
    required this.poolId,
    required this.user,
    required this.lpShares,
  });

  factory LpPosition.fromJson(Map<String, dynamic> json) {
    return LpPosition(
      contractAddress: (json['contract'] ?? '').toString(),
      poolId: (json['pool_id'] ?? '0').toString(),
      user: (json['user'] ?? '').toString(),
      lpShares: (json['lp_shares'] ?? '0').toString(),
    );
  }

  bool get isEmpty => lpShares == '0' || lpShares.isEmpty;
}
