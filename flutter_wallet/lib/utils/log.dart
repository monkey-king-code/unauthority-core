import 'package:flutter/foundation.dart';

/// Production-safe logging utility.
/// On release builds (kReleaseMode), ALL logging is suppressed to prevent
/// information leakage via console output on desktop platforms.
///
/// Usage: Replace `debugPrint(msg)` with `losLog(msg)`.
void losLog(String message) {
  if (!kReleaseMode) {
    debugPrint(message);
  }
}
