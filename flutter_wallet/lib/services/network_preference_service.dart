import 'package:shared_preferences/shared_preferences.dart';
import '../utils/log.dart';
import 'api_service.dart';
import '../config/testnet_config.dart';

/// Persists the user's network choice (testnet/mainnet) across app restarts.
///
/// Without this, the app defaults back to the compile-time value on every restart,
/// which is confusing when the user has manually switched networks.
class NetworkPreferenceService {
  static const String _key = 'los_network_preference';

  /// Save the user's network choice to persistent storage.
  static Future<void> save(NetworkEnvironment env) async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString(_key, env.name);
    losLog('💾 NetworkPreference: saved ${env.name}');
  }

  /// Load the user's persisted network choice.
  /// Returns null if no preference was saved (use compile-time default).
  static Future<NetworkEnvironment?> load() async {
    final prefs = await SharedPreferences.getInstance();
    final value = prefs.getString(_key);
    if (value == null) return null;
    switch (value) {
      case 'mainnet':
        return NetworkEnvironment.mainnet;
      case 'testnet':
        return NetworkEnvironment.testnet;
      default:
        losLog('⚠️ NetworkPreference: unknown value "$value" — ignoring');
        return null;
    }
  }

  /// Clear the persisted preference (revert to compile-time default).
  static Future<void> clear() async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.remove(_key);
    losLog('🗑️ NetworkPreference: cleared');
  }

  /// Apply persisted preference to ApiService and WalletConfig.
  /// Call this once during app initialization, after NetworkConfig.load().
  /// Silently no-ops if SharedPreferences is unavailable (e.g. in tests).
  static Future<void> applyToServices(ApiService apiService) async {
    try {
      final saved = await load();
      if (saved != null && saved != apiService.environment) {
        losLog(
            '🔄 NetworkPreference: restoring ${saved.name} (was ${apiService.environment.name})');
        apiService.switchEnvironment(saved);

        // Sync WalletConfig with the persisted choice
        if (saved == NetworkEnvironment.mainnet) {
          WalletConfig.useMainnet();
        } else {
          WalletConfig.useConsensusTestnet();
        }
      }
    } catch (e) {
      // SharedPreferences may not be available (e.g. in widget tests)
      losLog('⚠️ NetworkPreference: could not apply ($e)');
    }
  }
}
