import 'package:flutter/material.dart';

/// Validator app color constants — distinct from wallet app (purple).
/// Validator uses ORANGE accent to visually differentiate from wallet.
class ValidatorColors {
  ValidatorColors._();

  /// Primary accent color (Orange) — visually distinct from wallet purple (0xFF6B4CE6)
  static const Color accent = Color(0xFFE67E22);

  /// Dark background — slightly different from wallet (0xFF0A0E1A)
  static const Color background = Color(0xFF0D1117);

  /// Card background — distinct from wallet (0xFF1A1F2E)
  static const Color cardBg = Color(0xFF161B22);
}
