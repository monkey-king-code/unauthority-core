/// LOS Blockchain Constants for Validator Dashboard
/// Must be synchronized with backend: crates/los-core/src/lib.rs
///
/// Backend definition:
///   pub const CIL_PER_LOS: u128 = 100_000_000_000; // 10^11
///   1 LOS = 100,000,000,000 CIL (smallest unit)
library;

class BlockchainConstants {
  BlockchainConstants._();

  /// Fixed total supply of LOS tokens
  static const int totalSupply = 21936236;

  /// CIL per LOS - the smallest unit conversion factor
  /// 1 LOS = 100,000,000,000 CIL (10^11 precision)
  /// CRITICAL: Must match backend CIL_PER_LOS exactly
  static const int cilPerLos = 100000000000; // 10^11

  /// Number of decimal places for display
  static const int decimalPlaces = 11;

  /// LOS address prefix
  static const String addressPrefix = 'LOS';

  /// Convert CIL to LOS as an exact string using integer-only math.
  /// No floating-point precision loss — safe for all display contexts.
  /// Examples:
  ///   0 → "0.00"
  ///   100000000000 → "1.00"
  ///   30000000000 → "0.30000000000"
  static String cilToLosString(int cilAmount) {
    if (cilAmount == 0) return '0.00';
    final negative = cilAmount < 0;
    final abs = negative ? -cilAmount : cilAmount;
    final whole = abs ~/ cilPerLos;
    final frac = abs % cilPerLos;
    final sign = negative ? '-' : '';
    if (frac == 0) return '$sign$whole.00';
    // Pad fractional part to decimalPlaces digits, then trim trailing zeros
    // but keep at least 2 decimal places.
    var fracStr = frac.toString().padLeft(decimalPlaces, '0');
    while (fracStr.length > 2 && fracStr.endsWith('0')) {
      fracStr = fracStr.substring(0, fracStr.length - 1);
    }
    return '$sign$whole.$fracStr';
  }

  /// Integer square root — Newton's method.
  /// NOTE: No longer used for voting power (C-01 linear fix).
  /// Kept for potential AMM LP token calculation.
  static int isqrt(int n) {
    if (n <= 0) return 0;
    if (n == 1) return 1;
    int x = n;
    int y = (x + 1) ~/ 2;
    while (y < x) {
      x = y;
      y = (x + n ~/ x) ~/ 2;
    }
    return x;
  }

  /// Convert LOS string to CIL using integer-only math.
  /// Avoids IEEE 754 f64 precision loss that causes off-by-1 CIL errors.
  static int losStringToCil(String losStr) {
    final trimmed = losStr.trim();
    if (trimmed.isEmpty) return 0;

    final parts = trimmed.split('.');
    final wholePart = int.parse(parts[0].isEmpty ? '0' : parts[0]);

    if (parts.length == 1) {
      return wholePart * cilPerLos;
    }

    var fracStr = parts[1];
    if (fracStr.length > decimalPlaces) {
      fracStr = fracStr.substring(0, decimalPlaces);
    } else {
      fracStr = fracStr.padRight(decimalPlaces, '0');
    }

    final fracVoid = int.parse(fracStr);
    return wholePart * cilPerLos + fracVoid;
  }

  /// Format CIL amount directly for display as LOS string.
  /// 100% integer-only arithmetic — ZERO floating-point operations.
  /// Shows up to [maxDecimals] places, trimming trailing zeros (min 2).
  static String formatCilAsLos(int cilAmount, {int maxDecimals = 6}) {
    if (cilAmount == 0) return '0.00';
    final negative = cilAmount < 0;
    final abs = negative ? -cilAmount : cilAmount;
    final whole = abs ~/ cilPerLos;
    final frac = abs % cilPerLos;
    final sign = negative ? '-' : '';
    if (frac == 0) return '$sign$whole.00';
    // Full fractional part padded to decimalPlaces digits
    var fracStr = frac.toString().padLeft(decimalPlaces, '0');
    // Truncate to maxDecimals
    if (maxDecimals < fracStr.length) {
      fracStr = fracStr.substring(0, maxDecimals);
    }
    // Trim trailing zeros but keep at least 2 decimal places
    while (fracStr.length > 2 && fracStr.endsWith('0')) {
      fracStr = fracStr.substring(0, fracStr.length - 1);
    }
    return '$sign$whole.$fracStr';
  }
}
