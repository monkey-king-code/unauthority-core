import 'utils/log.dart';
import 'dart:async';
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import 'screens/home_screen.dart';
import 'screens/wallet_setup_screen.dart';
import 'services/account_management_service.dart';
import 'services/wallet_service.dart';
import 'services/api_service.dart';
import 'services/tor_service.dart';
import 'services/network_config.dart';
import 'services/dilithium_service.dart';
import 'services/network_status_service.dart';
import 'services/network_preference_service.dart';

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
    // Initialize Dilithium5 native library (non-blocking, graceful fallback)
    await DilithiumService.initialize();
    runApp(const MyApp());
  }, (error, stackTrace) {
    // Catches uncaught async exceptions from zones without error handlers
    // (e.g. socks5_proxy RangeError from non-SOCKS5 port responses)
    losLog('⚠️ Uncaught async error: $error');
    losLog('   ${stackTrace.toString().split('\n').take(3).join('\n   ')}');
  });
}

class MyApp extends StatelessWidget {
  const MyApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MultiProvider(
      providers: [
        Provider<TorService>(
          create: (_) => TorService(),
          dispose: (_, tor) => tor.stop(),
        ),
        Provider<WalletService>(create: (_) => WalletService()),
        Provider<ApiService>(
          create: (ctx) => ApiService(torService: ctx.read<TorService>()),
          dispose: (_, api) => api.dispose(),
        ),
        ChangeNotifierProvider<NetworkStatusService>(
          create: (context) => NetworkStatusService(context.read<ApiService>()),
        ),
      ],
      child: MaterialApp(
        title: 'LOS Wallet',
        debugShowCheckedModeBanner: false,
        theme: ThemeData(
          colorScheme: ColorScheme.fromSeed(
            seedColor: const Color(0xFF6B4CE6),
            brightness: Brightness.dark,
          ),
          useMaterial3: true,
          scaffoldBackgroundColor: const Color(0xFF0A0E1A),
          cardTheme: const CardThemeData(
            color: Color(0xFF1A1F2E),
            elevation: 4,
          ),
        ),
        home: const SplashScreen(),
      ),
    );
  }
}

class SplashScreen extends StatefulWidget {
  const SplashScreen({super.key});

  @override
  State<SplashScreen> createState() => _SplashScreenState();
}

class _SplashScreenState extends State<SplashScreen> {
  bool _showNetworkChoice = false;
  NetworkEnvironment _selectedNetwork = NetworkEnvironment.mainnet;

  @override
  void initState() {
    super.initState();
    _initializeApp();
  }

  Future<void> _initializeApp() async {
    // Load persisted network choice but ALWAYS show selection screen
    final savedNetwork = await NetworkPreferenceService.load();

    if (!mounted) return;

    // Show network choice screen with saved preference pre-selected
    setState(() {
      _selectedNetwork = savedNetwork ?? NetworkEnvironment.mainnet;
      _showNetworkChoice = true;
    });
  }

  Future<void> _proceedWithNetwork() async {
    setState(() => _showNetworkChoice = false);

    final walletService = context.read<WalletService>();
    final apiService = context.read<ApiService>();

    // Apply selected network — ALWAYS sync WalletConfig + save preference
    try {
      apiService.switchEnvironment(_selectedNetwork);
      await NetworkPreferenceService.save(_selectedNetwork);
    } catch (e) {
      // Network switch failed (e.g. no mainnet nodes yet)
      if (!mounted) return;
      await _showErrorDialog(
        'Network Unavailable',
        'Cannot switch to ${_selectedNetwork.name}. Please try again later.',
      );
      return;
    }

    // Test connection for testnet
    if (_selectedNetwork == NetworkEnvironment.testnet) {
      try {
        await apiService.getHealth().timeout(const Duration(seconds: 10));
      } catch (e) {
        if (!mounted) return;
        await _showTestnetErrorDialog();
        return;
      }
    }

    // One-time migration: SharedPreferences → FlutterSecureStorage
    await walletService.migrateFromSharedPreferences();

    // One-time migration: account seed phrases → SecureStorage
    final accountService = AccountManagementService();
    await accountService.migrateSecretsFromSharedPreferences();

    final wallet = await walletService.getCurrentWallet();

    if (!mounted) return;

    if (wallet == null) {
      Navigator.of(context).pushReplacement(
        MaterialPageRoute(builder: (_) => const WalletSetupScreen()),
      );
    } else {
      Navigator.of(context).pushReplacement(
        MaterialPageRoute(builder: (_) => const HomeScreen()),
      );
    }
  }

  Future<void> _showTestnetErrorDialog() async {
    await showDialog(
      context: context,
      barrierDismissible: false,
      builder: (context) => AlertDialog(
        title: const Row(
          children: [
            Icon(Icons.warning, color: Colors.orange),
            SizedBox(width: 8),
            Text('Testnet Unavailable'),
          ],
        ),
        content: const SingleChildScrollView(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(
                'No testnet nodes are currently online.',
                style: TextStyle(fontWeight: FontWeight.bold),
              ),
              SizedBox(height: 16),
              Text('To run your own testnet node:'),
              SizedBox(height: 8),
              Text('1. Read the documentation:\n   docs/VALIDATOR_GUIDE.md'),
              SizedBox(height: 8),
              Text(
                  '2. Configure testnet host in:\n   flutter_wallet/assets/network_config.json'),
              SizedBox(height: 8),
              Text('3. Or switch to Mainnet to use the live network.'),
            ],
          ),
        ),
        actions: [
          TextButton(
            onPressed: () {
              Navigator.of(context).pop();
              setState(() {
                _selectedNetwork = NetworkEnvironment.mainnet;
                _showNetworkChoice = true;
              });
            },
            child: const Text('Switch to Mainnet'),
          ),
          TextButton(
            onPressed: () {
              Navigator.of(context).pop();
              setState(() => _showNetworkChoice = true);
            },
            child: const Text('Retry'),
          ),
        ],
      ),
    );
  }

  Future<void> _showErrorDialog(String title, String message) async {
    await showDialog(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(title),
        content: Text(message),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(),
            child: const Text('OK'),
          ),
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: Center(
        child:
            _showNetworkChoice ? _buildNetworkChoice() : _buildLoadingScreen(),
      ),
    );
  }

  Widget _buildLoadingScreen() {
    return Column(
      mainAxisAlignment: MainAxisAlignment.center,
      children: [
        Icon(
          Icons.account_balance_wallet,
          size: 80,
          color: Theme.of(context).colorScheme.primary,
        ),
        const SizedBox(height: 24),
        const Text(
          'LOS WALLET',
          style: TextStyle(
            fontSize: 32,
            fontWeight: FontWeight.bold,
            letterSpacing: 2,
          ),
        ),
        const SizedBox(height: 8),
        Text(
          'Unauthority Blockchain',
          style: TextStyle(
            fontSize: 16,
            color: Colors.grey[400],
          ),
        ),
        const SizedBox(height: 48),
        const CircularProgressIndicator(),
      ],
    );
  }

  Widget _buildNetworkChoice() {
    return Padding(
      padding: const EdgeInsets.all(24.0),
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          Icon(
            Icons.account_balance_wallet,
            size: 80,
            color: Theme.of(context).colorScheme.primary,
          ),
          const SizedBox(height: 24),
          const Text(
            'LOS WALLET',
            style: TextStyle(
              fontSize: 32,
              fontWeight: FontWeight.bold,
              letterSpacing: 2,
            ),
          ),
          const SizedBox(height: 8),
          Text(
            'Unauthority Blockchain',
            style: TextStyle(
              fontSize: 16,
              color: Colors.grey[400],
            ),
          ),
          const SizedBox(height: 48),
          const Text(
            'Select Network',
            style: TextStyle(
              fontSize: 20,
              fontWeight: FontWeight.bold,
            ),
          ),
          const SizedBox(height: 24),
          SegmentedButton<NetworkEnvironment>(
            segments: const [
              ButtonSegment(
                value: NetworkEnvironment.mainnet,
                label: Text('MAINNET'),
                icon: Icon(Icons.lock),
              ),
              ButtonSegment(
                value: NetworkEnvironment.testnet,
                label: Text('TESTNET'),
                icon: Icon(Icons.bug_report),
              ),
            ],
            selected: {_selectedNetwork},
            onSelectionChanged: (Set<NetworkEnvironment> selected) {
              setState(() => _selectedNetwork = selected.first);
            },
            style: ButtonStyle(
              backgroundColor: WidgetStateProperty.resolveWith((states) {
                if (states.contains(WidgetState.selected)) {
                  return _selectedNetwork == NetworkEnvironment.mainnet
                      ? Colors.green.withValues(alpha: 0.3)
                      : Colors.orange.withValues(alpha: 0.3);
                }
                return null;
              }),
            ),
          ),
          const SizedBox(height: 16),
          Text(
            _selectedNetwork == NetworkEnvironment.mainnet
                ? 'Connected to live Mainnet (.onion via Tor)'
                : 'Testnet for development and testing',
            style: TextStyle(
              fontSize: 12,
              color: Colors.grey[400],
            ),
            textAlign: TextAlign.center,
          ),
          const SizedBox(height: 48),
          ElevatedButton.icon(
            onPressed: _proceedWithNetwork,
            icon: const Icon(Icons.arrow_forward),
            label: const Text('Continue'),
            style: ElevatedButton.styleFrom(
              padding: const EdgeInsets.symmetric(horizontal: 32, vertical: 16),
            ),
          ),
        ],
      ),
    );
  }
}
