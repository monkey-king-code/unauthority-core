import 'utils/log.dart';
import 'dart:async';
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import 'screens/dashboard_screen.dart';
import 'screens/setup_wizard_screen.dart';
import 'screens/node_control_screen.dart';
import 'services/api_service.dart';
import 'services/network_config.dart';
import 'services/dilithium_service.dart';
import 'services/wallet_service.dart';
import 'services/network_status_service.dart';
import 'services/node_process_service.dart';
import 'services/tor_service.dart';

void main() async {
  // Catch unhandled async exceptions (e.g. from SOCKS5 proxy failures)
  // so they don't spam [ERROR:flutter/runtime/dart_vm_initializer.cc] to console.
  runZonedGuarded(() async {
    WidgetsFlutterBinding.ensureInitialized();

    // Global Flutter error handler — log but don't crash
    FlutterError.onError = (details) {
      losLog('⚠️ FlutterError: ${details.exceptionAsString()}');
      losLog(
          '   ${details.stack?.toString().split('\n').take(3).join('\n   ')}');
    };

    // Load bootstrap node addresses from assets/network_config.json
    await NetworkConfig.load();

    // Initialize Dilithium5 post-quantum crypto (loads native lib if available)
    await DilithiumService.initialize();
    losLog(
      DilithiumService.isAvailable
          ? '🔐 Dilithium5 ready (PK: ${DilithiumService.publicKeyBytes}B, SK: ${DilithiumService.secretKeyBytes}B)'
          : '⚠️  Dilithium5 not available — SHA256 fallback active',
    );

    // Migrate any plaintext secrets from SharedPreferences → SecureStorage
    final walletService = WalletService();
    await walletService.migrateFromSharedPreferences();

    runApp(MyApp(walletService: walletService));
  }, (error, stackTrace) {
    // Catches uncaught async exceptions from zones without error handlers
    // (e.g. socks5_proxy RangeError from non-SOCKS5 port responses)
    losLog('⚠️ Uncaught async error: $error');
    losLog('   ${stackTrace.toString().split('\n').take(3).join('\n   ')}');
  });
}

class MyApp extends StatefulWidget {
  final WalletService walletService;

  const MyApp({super.key, required this.walletService});

  /// Call this to force the app to re-check wallet state (e.g., after unregister)
  static void resetToSetup(BuildContext context) {
    context.findAncestorStateOfType<_MyAppState>()?.resetRouter();
  }

  @override
  State<MyApp> createState() => _MyAppState();
}

class _MyAppState extends State<MyApp> {
  Key _routerKey = UniqueKey();

  void resetRouter() {
    setState(() => _routerKey = UniqueKey());
  }

  @override
  Widget build(BuildContext context) {
    return MultiProvider(
      providers: [
        Provider<TorService>(
          create: (_) => TorService(),
          dispose: (_, tor) => tor.stop(),
        ),
        Provider<ApiService>(
          create: (ctx) => ApiService(torService: ctx.read<TorService>()),
          dispose: (_, api) => api.dispose(),
        ),
        Provider<WalletService>.value(value: widget.walletService),
        ChangeNotifierProvider<NetworkStatusService>(
          create: (context) => NetworkStatusService(context.read<ApiService>()),
        ),
        ChangeNotifierProvider<NodeProcessService>(
          create: (_) => NodeProcessService(),
        ),
      ],
      child: MaterialApp(
        title: 'LOS Validator & Miner',
        debugShowCheckedModeBanner: false,
        theme: ThemeData(
          colorScheme: ColorScheme.fromSeed(
            seedColor:
                const Color(0xFFE67E22), // Orange — distinct from wallet purple
            brightness: Brightness.dark,
          ),
          useMaterial3: true,
          scaffoldBackgroundColor:
              const Color(0xFF0D1117), // Darker — GitHub-dark feel
          cardTheme: const CardThemeData(
            color: Color(0xFF161B22), // Distinct from wallet (0xFF1A1F2E)
            elevation: 4,
          ),
          appBarTheme: const AppBarTheme(
            backgroundColor: Color(0xFF161B22),
            foregroundColor: Color(0xFFE67E22),
          ),
        ),
        home: _AppRouter(key: _routerKey),
        routes: {
          '/dashboard': (_) => const DashboardScreen(),
        },
      ),
    );
  }
}

/// Routes based on wallet registration state:
/// - No wallet → SetupWizard (import wallet + validate balance >= 1 LOS)
/// - Wallet registered → NodeControlScreen (dashboard + settings)
class _AppRouter extends StatefulWidget {
  const _AppRouter({super.key});

  @override
  State<_AppRouter> createState() => _AppRouterState();
}

class _AppRouterState extends State<_AppRouter> {
  bool _loading = true;
  bool _hasWallet = false;

  @override
  void initState() {
    super.initState();
    _checkWallet();
  }

  Future<void> _checkWallet() async {
    losLog('🔄 [Validator] Checking wallet state...');
    try {
      final walletService = context.read<WalletService>();
      final wallet = await walletService.getCurrentWallet();
      if (!mounted) return;
      losLog('🔄 [Validator] Wallet found: ${wallet != null}');
      setState(() {
        _hasWallet = wallet != null;
        _loading = false;
      });
    } catch (e) {
      losLog('❌ [Validator] _checkWallet error: $e');
      if (!mounted) return;
      setState(() {
        _hasWallet = false;
        _loading = false;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    if (_loading) {
      return Scaffold(
        body: Center(
          child: Column(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              Icon(
                Icons.dns_rounded,
                size: 80,
                color: Theme.of(context).colorScheme.primary,
              ),
              const SizedBox(height: 24),
              const Text(
                'LOS VALIDATOR & MINER',
                style: TextStyle(
                  fontSize: 28,
                  fontWeight: FontWeight.bold,
                  letterSpacing: 3,
                ),
              ),
              const SizedBox(height: 8),
              Text(
                'Unauthority Validator & Miner',
                style: TextStyle(
                  fontSize: 16,
                  color: Colors.grey[400],
                ),
              ),
              const SizedBox(height: 48),
              const CircularProgressIndicator(),
            ],
          ),
        ),
      );
    }
    if (_hasWallet) {
      return const NodeControlScreen();
    }
    return SetupWizardScreen(
      onSetupComplete: () {
        setState(() => _hasWallet = true);
      },
    );
  }
}
