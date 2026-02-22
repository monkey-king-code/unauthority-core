/// USP-01 Token Model - Fungible Token Standard
///
/// Represents a USP-01 compliant token deployed on the LOS blockchain.
/// All amounts are stored as integer strings to avoid f64 precision loss.
library;

class Token {
  final String contractAddress;
  final String name;
  final String symbol;
  final int decimals;
  final String totalSupply; // Integer string (no f64)
  final bool isWrapped;
  final String wrappedOrigin; // e.g. "BTC", "ETH" for wrapped assets
  final String maxSupply; // "0" = unlimited
  final String bridgeOperator;
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
    this.bridgeOperator = '',
    this.owner = '',
  });

  factory Token.fromJson(Map<String, dynamic> json) {
    return Token(
      contractAddress:
          (json['contract'] ?? json['contract_address'] ?? '').toString(),
      name: (json['name'] ?? '').toString(),
      symbol: (json['symbol'] ?? '').toString(),
      decimals: (json['decimals'] is int)
          ? json['decimals']
          : int.tryParse(json['decimals']?.toString() ?? '11') ?? 11,
      totalSupply: (json['total_supply'] ?? '0').toString(),
      isWrapped: json['is_wrapped'] == true,
      wrappedOrigin: (json['wrapped_origin'] ?? '').toString(),
      maxSupply: (json['max_supply'] ?? '0').toString(),
      bridgeOperator: (json['bridge_operator'] ?? '').toString(),
      owner: (json['owner'] ?? '').toString(),
    );
  }

  /// Display-friendly token label
  String get displayName =>
      isWrapped ? '$symbol (Wrapped $wrappedOrigin)' : symbol;

  /// Shortened contract address for display
  String get shortAddress {
    if (contractAddress.length <= 16) return contractAddress;
    return '${contractAddress.substring(0, 10)}...${contractAddress.substring(contractAddress.length - 6)}';
  }
}

/// Token balance for a specific holder
class TokenBalance {
  final String contractAddress;
  final String holder;
  final String balance; // Integer string

  TokenBalance({
    required this.contractAddress,
    required this.holder,
    required this.balance,
  });

  factory TokenBalance.fromJson(Map<String, dynamic> json) {
    return TokenBalance(
      contractAddress: (json['contract'] ?? '').toString(),
      holder: (json['holder'] ?? '').toString(),
      balance: (json['balance'] ?? '0').toString(),
    );
  }

  /// Check if balance is zero
  bool get isZero => balance == '0' || balance.isEmpty;
}

/// Token allowance (approve/transferFrom)
class TokenAllowance {
  final String contractAddress;
  final String owner;
  final String spender;
  final String allowance; // Integer string

  TokenAllowance({
    required this.contractAddress,
    required this.owner,
    required this.spender,
    required this.allowance,
  });

  factory TokenAllowance.fromJson(Map<String, dynamic> json) {
    return TokenAllowance(
      contractAddress: (json['contract'] ?? '').toString(),
      owner: (json['owner'] ?? '').toString(),
      spender: (json['spender'] ?? '').toString(),
      allowance: (json['allowance'] ?? '0').toString(),
    );
  }
}
