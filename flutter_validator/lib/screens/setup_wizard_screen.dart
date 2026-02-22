import '../utils/log.dart';
import '../constants/colors.dart';
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/wallet_service.dart';
import '../services/api_service.dart';
import '../services/node_process_service.dart';
import '../services/tor_service.dart';
import '../services/network_config.dart';
import '../services/network_preference_service.dart';
import '../constants/blockchain.dart';

/// Validator Setup Wizard - 3-step flow:
/// 1. Import Wallet (seed phrase / private key / address)
/// 2. Validate balance >= 1000 LOS -> Confirm
/// 3. Start Tor hidden service + los-node binary -> Go to Dashboard
class SetupWizardScreen extends StatefulWidget {
  final VoidCallback onSetupComplete;

  const SetupWizardScreen({super.key, required this.onSetupComplete});

  @override
  State<SetupWizardScreen> createState() => _SetupWizardScreenState();
}

enum _ImportMethod { seedPhrase, privateKey, walletAddress }

class _SetupWizardScreenState extends State<SetupWizardScreen> {
  int _currentStep = -1; // -1=network choice, 0=import, 1=confirm, 2=launching
  _ImportMethod _importMethod = _ImportMethod.seedPhrase;
  NetworkEnvironment _selectedNetwork = NetworkEnvironment.mainnet;

  final _seedController = TextEditingController();
  final _privateKeyController = TextEditingController();
  final _addressController = TextEditingController();

  bool _isValidating = false;
  bool _isLaunching = false;
  bool _isGenesisMonitor =
      false; // Genesis bootstrap validator → monitor-only mode
  String? _error;
  String? _validatedAddress;
  int? _validatedBalanceCil; // Balance in CIL — integer precision

  // Launch progress
  String _launchStatus = '';
  double _launchProgress = 0.0;

  /// Minimum validator stake in CIL (1000 LOS * cilPerLos)
  static final int _minStakeCil = 1000 * BlockchainConstants.cilPerLos;

  @override
  void initState() {
    super.initState();
    _loadInitialNetwork();
  }

  Future<void> _loadInitialNetwork() async {
    // Load persisted network choice but ALWAYS show selection screen
    final savedNetwork = await NetworkPreferenceService.load();
    if (!mounted) return;
    setState(() {
      _selectedNetwork = savedNetwork ?? NetworkEnvironment.mainnet;
    });
  }

  Future<void> _proceedWithNetwork() async {
    final apiService = context.read<ApiService>();

    // Apply selected network — ALWAYS sync config + save preference
    try {
      apiService.switchEnvironment(_selectedNetwork);
      await NetworkPreferenceService.save(_selectedNetwork);
    } catch (e) {
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

    // Proceed to import step
    setState(() => _currentStep = 0);
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
                  '2. Configure testnet host in:\n   flutter_validator/assets/network_config.json'),
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
              });
            },
            child: const Text('Switch to Mainnet'),
          ),
          TextButton(
            onPressed: () => Navigator.of(context).pop(),
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
  void dispose() {
    // SECURITY FIX F9: Clear sensitive input from controllers on disposal.
    _seedController.clear();
    _privateKeyController.clear();
    _seedController.dispose();
    _privateKeyController.dispose();
    _addressController.dispose();
    super.dispose();
  }

  Future<void> _importAndValidate() async {
    losLog(
        '🛡️ [SetupWizardScreen._importAndValidate] Import method: ${_importMethod.name}');
    setState(() {
      _isValidating = true;
      _error = null;
    });

    try {
      final walletService = context.read<WalletService>();
      final apiService = context.read<ApiService>();
      String? walletAddress;

      switch (_importMethod) {
        case _ImportMethod.seedPhrase:
          final seed = _seedController.text.trim();
          if (seed.isEmpty) throw Exception('Please enter your seed phrase');
          final words = seed.split(RegExp(r'\s+'));
          if (words.length != 12 && words.length != 24) {
            throw Exception(
                'Seed phrase must be 12 or 24 words (got ${words.length})');
          }
          final wallet = await walletService.importWallet(seed);
          walletAddress = wallet['address'];
          break;

        case _ImportMethod.privateKey:
          final pk = _privateKeyController.text.trim();
          if (pk.isEmpty) throw Exception('Please enter your private key');
          final wallet = await walletService.importByPrivateKey(pk);
          walletAddress = wallet['address'];
          break;

        case _ImportMethod.walletAddress:
          final addr = _addressController.text.trim();
          if (addr.isEmpty) throw Exception('Please enter your wallet address');
          if (!addr.startsWith('LOS')) {
            throw Exception('Invalid address format');
          }
          await walletService.importByAddress(addr);
          walletAddress = addr;
          break;
      }

      if (walletAddress == null || walletAddress.isEmpty) {
        throw Exception('Failed to derive wallet address');
      }

      losLog('Checking balance for $walletAddress...');
      final account = await apiService.getBalance(walletAddress);

      // Compare in CIL integers — no f64 precision loss
      if (account.balance < _minStakeCil) {
        throw Exception(
          'Insufficient balance: ${BlockchainConstants.cilToLosString(account.balance)} LOS.\n'
          'Minimum validator stake is 1,000 LOS.\n'
          'Fund your wallet first using the LOS Wallet app.',
        );
      }

      // Check if this address is an active genesis bootstrap validator.
      // If yes → monitor-only mode (no new node spawn, no new onion).
      final isGenesisActive =
          await apiService.isActiveGenesisValidator(walletAddress);

      if (!mounted) return;
      setState(() {
        _validatedAddress = walletAddress;
        _validatedBalanceCil = account.balance;
        _isGenesisMonitor = isGenesisActive;
        _currentStep = 1;
      });
      losLog(
          '🛡️ [SetupWizardScreen._importAndValidate] Success: address=$walletAddress, balance=${account.balance} CIL');
    } catch (e) {
      losLog('🛡️ [SetupWizardScreen._importAndValidate] Error: $e');
      if (!mounted) return;
      setState(() => _error = e.toString().replaceAll('Exception: ', ''));
    } finally {
      if (mounted) {
        setState(() => _isValidating = false);
      }
    }
  }

  Future<void> _launchNode() async {
    setState(() {
      _isLaunching = true;
      _currentStep = 2;
      _launchProgress = 0.1;
      _error = null;
    });

    try {
      final walletService = context.read<WalletService>();

      // ── MONITOR-ONLY MODE ──
      // Genesis bootstrap validator is already running as a CLI node.
      // Don't spawn a new los-node, don't create a new .onion address.
      // Just save the wallet and go straight to the dashboard.
      if (_isGenesisMonitor) {
        if (!mounted) return;
        setState(() {
          _launchStatus = 'Genesis bootstrap validator detected.\n'
              'Entering monitor-only mode...';
          _launchProgress = 0.3;
        });

        await walletService.setMonitorMode(true);

        if (!mounted) return;
        setState(() {
          _launchStatus = 'Connecting to bootstrap node dashboard...';
          _launchProgress = 0.7;
        });

        await Future.delayed(const Duration(seconds: 1));

        if (!mounted) return;
        setState(() {
          _launchStatus = 'Monitor mode active!';
          _launchProgress = 1.0;
        });

        await Future.delayed(const Duration(seconds: 1));
        if (mounted) widget.onSetupComplete();
        return;
      }

      // ── NORMAL MODE ── Spawn new validator node with Tor hidden service
      await walletService.setMonitorMode(false);

      if (!mounted) return;
      setState(() {
        _launchStatus = 'Initializing Tor hidden service...';
      });

      final nodeService = context.read<NodeProcessService>();
      final torService = context.read<TorService>();

      // Step A: Start Tor with hidden service
      if (!mounted) return;
      setState(() {
        _launchStatus =
            'Starting Tor hidden service...\nThis may take up to 2 minutes on first run.';
        _launchProgress = 0.2;
      });

      // Find an available port (avoid conflict with bootstrap nodes on 3030-3033)
      final nodePort =
          await NodeProcessService.findAvailablePort(preferred: 3035);
      losLog('📡 Selected port $nodePort for validator node');

      final onionAddress = await torService.startWithHiddenService(
        localPort: nodePort,
        onionPort: 80,
      );

      if (!mounted) return;
      if (onionAddress == null) {
        // MAINNET PARITY: Tor hidden service is MANDATORY.
        // Without .onion, the validator cannot be reached by peers
        // and cannot participate in consensus. Hard failure.
        throw Exception(
          'Tor hidden service failed to start.\n'
          'A .onion address is required for validator operation.\n'
          'Check Tor installation and retry.',
        );
      }

      // CRITICAL: Exclude own .onion from API failover peer list.
      // Spec: "flutter_validator MUST NOT use its own local onion
      // address for API consumption".
      final apiService = context.read<ApiService>();
      apiService.setExcludedOnion('http://$onionAddress');

      setState(() {
        _launchStatus = 'Tor ready!\nStarting validator node...';
        _launchProgress = 0.5;
      });

      // Step B: Start los-node
      if (!mounted) return;
      setState(() {
        _launchProgress = 0.6;
      });

      // Load bootstrap nodes so los-node can discover peers on the network.
      // MAINNET PARITY: ALWAYS use .onion P2P addresses.
      // No localhost/127.0.0.1 — Tor onion routing is mandatory.
      await NetworkConfig.load();
      const networkMode =
          String.fromEnvironment('NETWORK', defaultValue: 'mainnet');
      final nodes = networkMode == 'mainnet'
          ? NetworkConfig.mainnetNodes
          : NetworkConfig.testnetNodes;
      final bootstrapAddresses = nodes.map((n) => n.p2pAddress).join(',');
      losLog('\ud83c\udf10 Bootstrap nodes (.onion): $bootstrapAddresses');

      // P2P port: auto-derived from API port + 1000 (matches los-node)
      final p2pPort = nodePort + 1000;
      losLog('📡 P2P port: $p2pPort');

      // Tor SOCKS5 proxy: MANDATORY for dialing .onion bootstrap peers.
      // MAINNET PARITY: Without SOCKS5, los-node cannot reach any peer.
      if (!torService.isRunning) {
        throw Exception(
          'Tor SOCKS5 proxy is not running.\n'
          'Cannot connect to .onion bootstrap peers without Tor.',
        );
      }
      final torSocks5 = '127.0.0.1:${torService.activeSocksPort}';
      losLog('🧅 Tor SOCKS5: $torSocks5');

      // Retrieve mnemonic so los-node can derive the same Dilithium5 keypair
      final walletWithMnemonic =
          await walletService.getCurrentWallet(includeMnemonic: true);
      final mnemonic = walletWithMnemonic?['mnemonic'];

      // If node is already running (e.g. survived hot-reload or previous session),
      // skip starting and proceed directly to registration.
      final bool nodeAlreadyRunning = nodeService.isRunning;
      int activePort = nodePort;

      if (nodeAlreadyRunning) {
        losLog(
            '✅ Node already running on port ${nodeService.apiPort}, skipping start');
        activePort = nodeService.apiPort;
      } else {
        final started = await nodeService.start(
          port: nodePort,
          onionAddress: onionAddress,
          bootstrapNodes:
              bootstrapAddresses.isNotEmpty ? bootstrapAddresses : null,
          seedPhrase: mnemonic,
          p2pPort: p2pPort,
          torSocks5: torSocks5,
        );

        if (!started) {
          throw Exception(
              nodeService.errorMessage ?? 'Failed to start los-node');
        }
      }

      // Step C: Register as validator on the local node
      // This requires a Dilithium5 signed proof of key ownership.
      // The node will broadcast the registration to all peers via gossipsub.
      if (!mounted) return;
      setState(() {
        _launchStatus = 'Registering as validator...';
        _launchProgress = 0.8;
      });

      // Give the node a moment to fully initialize its API server
      await Future.delayed(const Duration(seconds: 2));

      final isAddressOnly = await walletService.isAddressOnlyImport();
      if (!isAddressOnly) {
        final wallet = await walletService.getCurrentWallet();
        final address = wallet?['address'];
        final publicKey = wallet?['public_key'];

        if (address != null && publicKey != null) {
          final timestamp =
              DateTime.now().millisecondsSinceEpoch ~/ 1000; // Unix seconds
          final message = 'REGISTER_VALIDATOR:$address:$timestamp';
          final signature = await walletService.signTransaction(message);

          // Include our onion address so peers can connect to us
          final myOnion = torService.onionAddress;

          // Register on the LOCAL node (sets is_validator locally)
          final localApi = ApiService(
            customUrl: 'http://127.0.0.1:$activePort',
          );
          await localApi.ensureReady();

          try {
            final result = await localApi.registerValidator(
              address: address,
              publicKey: publicKey,
              signature: signature,
              timestamp: timestamp,
              onionAddress: myOnion,
            );
            losLog('✅ Local registration: ${result['msg']}');
          } catch (e) {
            losLog('⚠️ Local registration deferred: $e');
          } finally {
            localApi.dispose();
          }

          // Also register on the BOOTSTRAP node so the network knows about us.
          if (mounted) {
            final bootstrapApi = context.read<ApiService>();
            try {
              await bootstrapApi.ensureReady();
              final result = await bootstrapApi.registerValidator(
                address: address,
                publicKey: publicKey,
                signature: signature,
                timestamp: timestamp,
                onionAddress: myOnion,
              );
              losLog('✅ Bootstrap registration: ${result['msg']}');
            } catch (e) {
              // Non-fatal: local node still runs, bootstrap may be unreachable
              losLog('⚠️ Bootstrap registration deferred: $e');
            }
          }
        }
      }

      if (!mounted) return;
      setState(() {
        _launchStatus = 'Validator node is running!';
        _launchProgress = 1.0;
      });

      await Future.delayed(const Duration(seconds: 2));
      if (mounted) widget.onSetupComplete();
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _error = e.toString().replaceAll('Exception: ', '');
        _isLaunching = false;
        _launchStatus = '';
        _currentStep = 1;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: SafeArea(
        child: Padding(
          padding: const EdgeInsets.all(24.0),
          child: switch (_currentStep) {
            -1 => _buildNetworkChoiceStep(),
            0 => _buildImportStep(),
            1 => _buildConfirmationStep(),
            2 => _buildLaunchingStep(),
            _ => _buildNetworkChoiceStep(),
          },
        ),
      ),
    );
  }

  Widget _buildNetworkChoiceStep() {
    return Center(
      child: SingleChildScrollView(
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            const Icon(Icons.dns, size: 80, color: ValidatorColors.accent),
            const SizedBox(height: 24),
            const Text(
              'LOS VALIDATOR',
              style: TextStyle(
                fontSize: 32,
                fontWeight: FontWeight.bold,
                letterSpacing: 3,
              ),
            ),
            const SizedBox(height: 8),
            Text(
              'Unauthority Node Dashboard',
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
                backgroundColor: ValidatorColors.accent,
                foregroundColor: Colors.white,
                padding:
                    const EdgeInsets.symmetric(horizontal: 32, vertical: 16),
              ),
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildImportStep() {
    return SingleChildScrollView(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          const SizedBox(height: 32),
          const Icon(Icons.verified_user,
              size: 80, color: ValidatorColors.accent),
          const SizedBox(height: 16),
          const Text('Register Validator',
              style: TextStyle(fontSize: 28, fontWeight: FontWeight.bold),
              textAlign: TextAlign.center),
          const SizedBox(height: 8),
          Text(
              'Import your wallet to register as a validator node.\nMinimum stake: 1,000 LOS',
              style: TextStyle(fontSize: 14, color: Colors.grey[400]),
              textAlign: TextAlign.center),
          const SizedBox(height: 32),
          const Text('Import Method',
              style: TextStyle(fontSize: 16, fontWeight: FontWeight.w600)),
          const SizedBox(height: 12),
          _buildMethodSelector(),
          const SizedBox(height: 24),
          _buildInputField(),
          const SizedBox(height: 24),
          if (_error != null) ...[
            _buildErrorBox(_error!),
            const SizedBox(height: 16),
          ],
          ElevatedButton(
              onPressed: _isValidating ? null : _importAndValidate,
              style: ElevatedButton.styleFrom(
                  backgroundColor: ValidatorColors.accent,
                  foregroundColor: Colors.white,
                  padding: const EdgeInsets.symmetric(vertical: 16),
                  shape: RoundedRectangleBorder(
                      borderRadius: BorderRadius.circular(12))),
              child: _isValidating
                  ? const SizedBox(
                      width: 24,
                      height: 24,
                      child: CircularProgressIndicator(
                          strokeWidth: 2, color: Colors.white))
                  : const Text('VALIDATE & CONTINUE',
                      style: TextStyle(
                          fontSize: 16, fontWeight: FontWeight.bold))),
          const SizedBox(height: 24),
          Container(
              padding: const EdgeInsets.all(12),
              decoration: BoxDecoration(
                  color: Colors.blue.withValues(alpha: 0.1),
                  borderRadius: BorderRadius.circular(8)),
              child: const Row(children: [
                Icon(Icons.info_outlined, color: Colors.blue, size: 20),
                SizedBox(width: 8),
                Expanded(
                    child: Text(
                        'Your wallet needs at least 1,000 LOS to register as a validator. '
                        'Use the LOS Wallet app to send/receive funds.',
                        style: TextStyle(fontSize: 12, color: Colors.blue))),
              ])),
        ],
      ),
    );
  }

  Widget _buildConfirmationStep() {
    return SingleChildScrollView(
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          const SizedBox(height: 40),
          const Icon(Icons.check_circle, size: 80, color: Colors.green),
          const SizedBox(height: 24),
          const Text('Wallet Verified!',
              style: TextStyle(fontSize: 28, fontWeight: FontWeight.bold),
              textAlign: TextAlign.center),
          const SizedBox(height: 8),
          Text('Your wallet is eligible to run a validator node.',
              style: TextStyle(fontSize: 14, color: Colors.grey[400]),
              textAlign: TextAlign.center),
          const SizedBox(height: 32),
          Card(
              child: Padding(
                  padding: const EdgeInsets.all(16),
                  child: Column(children: [
                    _infoRow('Address',
                        '${_validatedAddress!.substring(0, 12)}...${_validatedAddress!.substring(_validatedAddress!.length - 8)}'),
                    const Divider(),
                    _infoRow('Balance',
                        '${BlockchainConstants.cilToLosString(_validatedBalanceCil!)} LOS'),
                    const Divider(),
                    _infoRow('Min Stake', '1,000 LOS'),
                    const Divider(),
                    _infoRow('Status', 'Eligible'),
                  ]))),
          const SizedBox(height: 24),
          if (_error != null) ...[
            _buildErrorBox(_error!),
            const SizedBox(height: 16),
          ],
          if (_isGenesisMonitor) ...[
            // Genesis bootstrap validator — monitor-only flow
            Container(
                padding: const EdgeInsets.all(16),
                decoration: BoxDecoration(
                    color: Colors.amber.withValues(alpha: 0.1),
                    borderRadius: BorderRadius.circular(8),
                    border:
                        Border.all(color: Colors.amber.withValues(alpha: 0.5))),
                child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      const Row(children: [
                        Icon(Icons.shield, color: Colors.amber, size: 20),
                        SizedBox(width: 8),
                        Text('Genesis Bootstrap Validator',
                            style: TextStyle(
                                fontWeight: FontWeight.bold,
                                color: Colors.amber,
                                fontSize: 14)),
                      ]),
                      const SizedBox(height: 8),
                      Text(
                          'This address is an active genesis bootstrap validator.\n'
                          'The node is already running via CLI — no new node will be spawned.\n\n'
                          'You will enter monitor-only mode to view the dashboard.',
                          style:
                              TextStyle(fontSize: 13, color: Colors.grey[300])),
                    ])),
          ] else ...[
            // Normal validator — full node spawn flow
            Card(
                color: ValidatorColors.cardBg.withValues(alpha: 0.6),
                child: Padding(
                    padding: const EdgeInsets.all(16),
                    child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          const Text('What happens next:',
                              style: TextStyle(
                                  fontWeight: FontWeight.bold, fontSize: 14)),
                          const SizedBox(height: 8),
                          _stepItem(
                              '1', 'Setup Tor hidden service (.onion address)'),
                          _stepItem('2', 'Start los-node validator binary'),
                          _stepItem('3', 'Sync blockchain from network'),
                          _stepItem('4', 'Register as active validator'),
                        ]))),
          ],
          const SizedBox(height: 24),
          ElevatedButton.icon(
              onPressed: _isLaunching ? null : _launchNode,
              icon: Icon(_isGenesisMonitor
                  ? Icons.monitor_heart
                  : Icons.rocket_launch),
              label: Text(
                  _isGenesisMonitor
                      ? 'OPEN DASHBOARD (MONITOR MODE)'
                      : 'START VALIDATOR NODE',
                  style: const TextStyle(
                      fontSize: 16, fontWeight: FontWeight.bold)),
              style: ElevatedButton.styleFrom(
                  backgroundColor:
                      _isGenesisMonitor ? ValidatorColors.accent : Colors.green,
                  foregroundColor: Colors.white,
                  padding:
                      const EdgeInsets.symmetric(horizontal: 32, vertical: 16),
                  shape: RoundedRectangleBorder(
                      borderRadius: BorderRadius.circular(12)))),
          const SizedBox(height: 12),
          TextButton(
              onPressed: () => setState(() {
                    _currentStep = 0;
                    _error = null;
                  }),
              child: const Text('Back to wallet import')),
        ],
      ),
    );
  }

  Widget _stepItem(String num, String text) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Row(children: [
        Container(
            width: 24,
            height: 24,
            decoration: BoxDecoration(
                color: ValidatorColors.accent.withValues(alpha: 0.3),
                borderRadius: BorderRadius.circular(12)),
            child: Center(
                child: Text(num,
                    style: const TextStyle(
                        fontSize: 12, fontWeight: FontWeight.bold)))),
        const SizedBox(width: 12),
        Expanded(
            child: Text(text,
                style: TextStyle(fontSize: 13, color: Colors.grey[300]))),
      ]),
    );
  }

  Widget _buildLaunchingStep() {
    return Center(
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          if (_launchProgress < 1.0)
            const SizedBox(
                width: 80,
                height: 80,
                child: CircularProgressIndicator(
                    strokeWidth: 4, color: ValidatorColors.accent))
          else
            Icon(_isGenesisMonitor ? Icons.monitor_heart : Icons.check_circle,
                size: 80, color: Colors.green),
          const SizedBox(height: 32),
          Text(
              _launchProgress < 1.0
                  ? (_isGenesisMonitor
                      ? 'Entering Monitor Mode...'
                      : 'Starting Validator Node...')
                  : (_isGenesisMonitor
                      ? 'Monitor Mode Active!'
                      : 'Validator Running!'),
              style: const TextStyle(fontSize: 24, fontWeight: FontWeight.bold),
              textAlign: TextAlign.center),
          const SizedBox(height: 16),
          Text(_launchStatus,
              style: TextStyle(fontSize: 14, color: Colors.grey[400]),
              textAlign: TextAlign.center),
          const SizedBox(height: 32),
          SizedBox(
              width: 300,
              child: LinearProgressIndicator(
                  value: _launchProgress,
                  backgroundColor: Colors.grey[800],
                  color: _launchProgress < 1.0
                      ? ValidatorColors.accent
                      : Colors.green,
                  minHeight: 8,
                  borderRadius: BorderRadius.circular(4))),
          const SizedBox(height: 8),
          Text('${(_launchProgress * 100).toInt()}%',
              style: TextStyle(fontSize: 12, color: Colors.grey[500])),
        ],
      ),
    );
  }

  Widget _buildErrorBox(String message) {
    return Container(
        padding: const EdgeInsets.all(12),
        decoration: BoxDecoration(
            color: Colors.red.withValues(alpha: 0.1),
            borderRadius: BorderRadius.circular(8),
            border: Border.all(color: Colors.red.withValues(alpha: 0.3))),
        child: Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
          const Icon(Icons.error_outline, color: Colors.red, size: 20),
          const SizedBox(width: 8),
          Expanded(
              child: Text(message,
                  style: const TextStyle(color: Colors.red, fontSize: 13))),
        ]));
  }

  Widget _buildMethodSelector() {
    return Column(children: [
      _methodTile(_ImportMethod.seedPhrase, Icons.key, 'Seed Phrase',
          '12 or 24 word mnemonic'),
      _methodTile(_ImportMethod.privateKey, Icons.vpn_key, 'Private Key',
          'Hex-encoded private key'),
      _methodTile(_ImportMethod.walletAddress, Icons.account_balance_wallet,
          'Wallet Address', 'LOS address (view-only)'),
    ]);
  }

  Widget _methodTile(
      _ImportMethod method, IconData icon, String title, String subtitle) {
    final selected = _importMethod == method;
    return Card(
      color: selected ? ValidatorColors.accent.withValues(alpha: 0.2) : null,
      shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(8),
          side: BorderSide(
              color: selected ? ValidatorColors.accent : Colors.transparent,
              width: 1.5)),
      child: ListTile(
          leading: Icon(icon,
              color: selected ? ValidatorColors.accent : Colors.grey),
          title: Text(title,
              style: TextStyle(
                  fontWeight: selected ? FontWeight.bold : FontWeight.normal)),
          subtitle: Text(subtitle,
              style: TextStyle(fontSize: 12, color: Colors.grey[400])),
          trailing: selected
              ? const Icon(Icons.check_circle, color: ValidatorColors.accent)
              : null,
          onTap: () => setState(() {
                _importMethod = method;
                _error = null;
              })),
    );
  }

  Widget _buildInputField() {
    switch (_importMethod) {
      case _ImportMethod.seedPhrase:
        return TextField(
            controller: _seedController,
            maxLines: 3,
            decoration: InputDecoration(
                labelText: 'Seed Phrase',
                hintText: 'Enter your 12 or 24 word seed phrase...',
                border:
                    OutlineInputBorder(borderRadius: BorderRadius.circular(12)),
                prefixIcon: const Icon(Icons.key)));
      case _ImportMethod.privateKey:
        return TextField(
            controller: _privateKeyController,
            obscureText: true,
            decoration: InputDecoration(
                labelText: 'Private Key',
                hintText: 'Enter hex-encoded private key...',
                border:
                    OutlineInputBorder(borderRadius: BorderRadius.circular(12)),
                prefixIcon: const Icon(Icons.vpn_key)));
      case _ImportMethod.walletAddress:
        return TextField(
            controller: _addressController,
            decoration: InputDecoration(
                labelText: 'Wallet Address',
                hintText: 'LOS...',
                border:
                    OutlineInputBorder(borderRadius: BorderRadius.circular(12)),
                prefixIcon: const Icon(Icons.account_balance_wallet)));
    }
  }

  Widget _infoRow(String label, String value) {
    return Padding(
        padding: const EdgeInsets.symmetric(vertical: 4),
        child:
            Row(mainAxisAlignment: MainAxisAlignment.spaceBetween, children: [
          Text(label, style: TextStyle(color: Colors.grey[400])),
          Text(value, style: const TextStyle(fontWeight: FontWeight.bold)),
        ]));
  }
}
