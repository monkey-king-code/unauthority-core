import 'package:flutter/services.dart';

/// Centralized clipboard utility with auto-clear.
///
/// All sensitive copy operations should use this utility instead of
/// raw `Clipboard.setData()` to ensure clipboard is auto-cleared
/// after a configurable timeout.
class SecureClipboard {
  /// Default timeout before clipboard is auto-cleared (30 seconds).
  static const Duration defaultTimeout = Duration(seconds: 30);

  /// Copy text to clipboard with auto-clear after [timeout].
  ///
  /// For non-sensitive data (addresses, tx hashes), use a longer timeout.
  /// For secrets (seed phrases), use the default 30s or shorter.
  static Future<void> copy(
    String text, {
    Duration timeout = defaultTimeout,
  }) async {
    await Clipboard.setData(ClipboardData(text: text));

    // Schedule auto-clear
    Future.delayed(timeout, () {
      Clipboard.setData(const ClipboardData(text: ''));
    });
  }

  /// Copy with a longer timeout (60s) for non-secret but still sensitive data
  /// like addresses and transaction hashes.
  static Future<void> copyPublic(String text) async {
    await copy(text, timeout: const Duration(seconds: 60));
  }
}
