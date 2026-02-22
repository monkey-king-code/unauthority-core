// Validator Earnings Model - Tracks gas fee revenue
import '../constants/blockchain.dart';

/// Type-safe int parser: handles int, double, String, null from JSON.
int _parseIntField(dynamic v, [int fallback = 0]) {
  if (v == null) return fallback;
  if (v is int) return v;
  if (v is double) return v.toInt();
  return int.tryParse(v.toString()) ?? fallback;
}

/// Parse a numeric JSON field to CIL (integer) for storage.
/// If the API returns a LOS double/string (e.g. "12.5"), convert to CIL.
/// If it returns an int large enough to be CIL, use directly.
int _parseLosFieldToCil(dynamic v) {
  if (v == null) return 0;
  if (v is int) {
    // Values > totalSupply are already CIL
    if (v > BlockchainConstants.totalSupply) return v;
    return v * BlockchainConstants.cilPerLos;
  }
  if (v is double) {
    return BlockchainConstants.losStringToCil(v.toString());
  }
  if (v is String) {
    if (v.contains('.')) return BlockchainConstants.losStringToCil(v);
    final parsed = int.tryParse(v);
    if (parsed != null) {
      if (parsed > BlockchainConstants.totalSupply) return parsed;
      return parsed * BlockchainConstants.cilPerLos;
    }
  }
  return 0;
}

class ValidatorEarnings {
  final String validatorAddress;
  final int totalEarningsCil;
  final int last24HoursCil;
  final int last7DaysCil;
  final int last30DaysCil;
  final int revenueShareBps; // basis points (e.g. 250 = 2.50%)
  final int totalTransactionsProcessed;
  final List<DailyEarning> dailyHistory;

  ValidatorEarnings({
    required this.validatorAddress,
    required this.totalEarningsCil,
    required this.last24HoursCil,
    required this.last7DaysCil,
    required this.last30DaysCil,
    required this.revenueShareBps,
    required this.totalTransactionsProcessed,
    required this.dailyHistory,
  });

  /// Display formatters (integer-only)
  String get totalEarningsDisplay =>
      BlockchainConstants.formatCilAsLos(totalEarningsCil);
  String get last24HoursDisplay =>
      BlockchainConstants.formatCilAsLos(last24HoursCil);
  String get last7DaysDisplay =>
      BlockchainConstants.formatCilAsLos(last7DaysCil);
  String get last30DaysDisplay =>
      BlockchainConstants.formatCilAsLos(last30DaysCil);
  String get revenueShareDisplay =>
      '${revenueShareBps ~/ 100}.${(revenueShareBps % 100).toString().padLeft(2, '0')}%';

  factory ValidatorEarnings.fromJson(Map<String, dynamic> json) {
    // Parse revenue share: API sends as 0-100 float (e.g. 2.5 = 2.5%)
    int revShareBps = 0;
    final revRaw = json['revenue_share_percent'];
    if (revRaw is double) {
      revShareBps = (revRaw * 100).round();
    } else if (revRaw is int) {
      revShareBps = revRaw * 100;
    } else if (revRaw is String) {
      revShareBps = ((double.tryParse(revRaw) ?? 0) * 100).round();
    }

    return ValidatorEarnings(
      validatorAddress: (json['validator_address'] ?? '').toString(),
      totalEarningsCil: _parseLosFieldToCil(json['total_earnings_los']),
      last24HoursCil: _parseLosFieldToCil(json['last_24h_los']),
      last7DaysCil: _parseLosFieldToCil(json['last_7d_los']),
      last30DaysCil: _parseLosFieldToCil(json['last_30d_los']),
      revenueShareBps: revShareBps,
      totalTransactionsProcessed: _parseIntField(
        json['total_transactions_processed'],
      ),
      dailyHistory: (json['daily_history'] as List<dynamic>?)
              ?.map((item) => DailyEarning.fromJson(item))
              .toList() ??
          [],
    );
  }
}

class DailyEarning {
  final DateTime date;
  final int earningsCil;
  final int transactionsProcessed;

  DailyEarning({
    required this.date,
    required this.earningsCil,
    required this.transactionsProcessed,
  });

  /// Display formatter
  String get earningsDisplay => BlockchainConstants.formatCilAsLos(earningsCil);

  factory DailyEarning.fromJson(Map<String, dynamic> json) {
    return DailyEarning(
      date:
          DateTime.tryParse((json['date'] ?? '').toString()) ?? DateTime.now(),
      earningsCil: _parseLosFieldToCil(json['earnings_los']),
      transactionsProcessed: _parseIntField(json['transactions_processed']),
    );
  }

  Map<String, dynamic> toJson() {
    return {
      'date': date.toIso8601String(),
      'earnings_cil': earningsCil,
      'transactions_processed': transactionsProcessed,
    };
  }
}
