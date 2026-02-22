import '../utils/log.dart';
import '../utils/secure_clipboard.dart';
import 'package:flutter/material.dart';
import 'package:bip39/bip39.dart' as bip39;
import 'package:provider/provider.dart';
import '../models/account_profile.dart';
import '../services/account_management_service.dart';
import '../services/dilithium_service.dart';
import '../services/wallet_service.dart';

class AccountManagementScreen extends StatefulWidget {
  const AccountManagementScreen({super.key});

  @override
  State<AccountManagementScreen> createState() =>
      _AccountManagementScreenState();
}

class _AccountManagementScreenState extends State<AccountManagementScreen> {
  final _accountService = AccountManagementService();
  List<AccountProfile> _accounts = [];
  String? _activeAccountId;
  bool _isLoading = true;

  @override
  void initState() {
    super.initState();
    _loadAccounts();
  }

  Future<void> _loadAccounts() async {
    losLog('👤 [AccountManagementScreen._loadAccounts] Loading accounts...');
    setState(() => _isLoading = true);
    try {
      final accountsList = await _accountService.loadAccounts();
      if (!mounted) return;
      setState(() {
        _accounts = accountsList.accounts;
        _activeAccountId = accountsList.activeAccountId;
        _isLoading = false;
      });
      losLog(
          '👤 [AccountManagementScreen._loadAccounts] Loaded ${_accounts.length} accounts, active: $_activeAccountId');
    } catch (e) {
      if (!mounted) return;
      setState(() => _isLoading = false);
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Error loading accounts: $e')),
        );
      }
    }
  }

  Future<void> _switchAccount(AccountProfile account) async {
    losLog(
        '👤 [AccountManagementScreen._switchAccount] Switching to ${account.name}...');
    try {
      await _accountService.switchAccount(account.id,
          walletService: context.read<WalletService>());
      if (!mounted) return;
      setState(() => _activeAccountId = account.id);
      losLog(
          '👤 [AccountManagementScreen._switchAccount] Switched to ${account.name}');

      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: Text('Switched to ${account.name}'),
            backgroundColor: Colors.green.shade700,
          ),
        );
        // Pop to refresh home screen
        Navigator.pop(context, true);
      }
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Error switching account: $e')),
        );
      }
    }
  }

  Future<void> _showRenameDialog(AccountProfile account) async {
    final controller = TextEditingController(text: account.name);

    final newName = await showDialog<String>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Rename Account'),
        content: TextField(
          controller: controller,
          decoration: const InputDecoration(
            labelText: 'Account Name',
            border: OutlineInputBorder(),
          ),
          autofocus: true,
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('Cancel'),
          ),
          ElevatedButton(
            onPressed: () {
              final name = controller.text.trim();
              if (name.isNotEmpty) {
                Navigator.pop(context, name);
              }
            },
            child: const Text('Rename'),
          ),
        ],
      ),
    );

    if (newName != null && newName != account.name) {
      try {
        losLog(
            '👤 [AccountManagementScreen._showRenameDialog] Renaming ${account.name} to $newName');
        await _accountService.renameAccount(account.id, newName);
        await _loadAccounts();
        losLog(
            '👤 [AccountManagementScreen._showRenameDialog] Renamed successfully');
        if (mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            SnackBar(content: Text('Renamed to $newName')),
          );
        }
      } catch (e) {
        if (mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            SnackBar(content: Text('Error renaming: $e')),
          );
        }
      }
    }
  }

  Future<void> _deleteAccount(AccountProfile account) async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Delete Account'),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text('Are you sure you want to delete "${account.name}"?'),
            const SizedBox(height: 16),
            Container(
              padding: const EdgeInsets.all(12),
              decoration: BoxDecoration(
                color: Colors.red.withValues(alpha: 0.1),
                borderRadius: BorderRadius.circular(8),
                border: Border.all(
                  color: Colors.red.withValues(alpha: 0.3),
                ),
              ),
              child: Row(
                children: [
                  const Icon(Icons.warning, color: Colors.red, size: 20),
                  const SizedBox(width: 8),
                  const Expanded(
                    child: Text(
                      'This action cannot be undone. Make sure you have backed up your seed phrase.',
                      style: TextStyle(fontSize: 12),
                    ),
                  ),
                ],
              ),
            ),
          ],
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('Cancel'),
          ),
          ElevatedButton(
            onPressed: () => Navigator.pop(context, true),
            style: ElevatedButton.styleFrom(
              backgroundColor: Colors.red,
            ),
            child: const Text('Delete'),
          ),
        ],
      ),
    );

    if (confirmed == true) {
      try {
        losLog(
            '👤 [AccountManagementScreen._deleteAccount] Deleting ${account.name}...');
        await _accountService.deleteAccount(account.id);
        await _loadAccounts();
        losLog(
            '👤 [AccountManagementScreen._deleteAccount] Deleted ${account.name}');
        if (mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            const SnackBar(content: Text('Account deleted')),
          );
        }
      } catch (e) {
        if (mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            SnackBar(content: Text('Error: $e')),
          );
        }
      }
    }
  }

  Future<void> _createNewAccount() async {
    final nameController = TextEditingController();

    final accountName = await showDialog<String>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Create New Account'),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            TextField(
              controller: nameController,
              decoration: const InputDecoration(
                labelText: 'Account Name',
                border: OutlineInputBorder(),
                hintText: 'e.g., Savings, Trading',
              ),
              autofocus: true,
            ),
            const SizedBox(height: 16),
            Container(
              padding: const EdgeInsets.all(12),
              decoration: BoxDecoration(
                color: Colors.blue.withValues(alpha: 0.1),
                borderRadius: BorderRadius.circular(8),
              ),
              child: const Row(
                children: [
                  Icon(Icons.info_outline, size: 20, color: Colors.blue),
                  SizedBox(width: 8),
                  Expanded(
                    child: Text(
                      'A new wallet will be generated with a new seed phrase.',
                      style: TextStyle(fontSize: 12),
                    ),
                  ),
                ],
              ),
            ),
          ],
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('Cancel'),
          ),
          ElevatedButton(
            onPressed: () {
              final name = nameController.text.trim();
              if (name.isNotEmpty) {
                Navigator.pop(context, name);
              }
            },
            child: const Text('Create'),
          ),
        ],
      ),
    );

    if (accountName != null) {
      try {
        losLog(
            '👤 [AccountManagementScreen._createNewAccount] Creating account: $accountName');
        // Check if name is taken
        final isTaken = await _accountService.isNameTaken(accountName);
        if (isTaken) {
          if (mounted) {
            ScaffoldMessenger.of(context).showSnackBar(
              const SnackBar(content: Text('Account name already exists')),
            );
          }
          return;
        }

        // Generate new wallet keys WITHOUT overwriting primary wallet storage.
        // We derive address from a fresh BIP39 mnemonic in-memory only;
        // the seed phrase is persisted by AccountManagementService in
        // FlutterSecureStorage keyed by the new account ID.
        final mnemonic = bip39.generateMnemonic(strength: 256);
        String address;

        if (DilithiumService.isAvailable) {
          final seed = bip39.mnemonicToSeed(mnemonic);
          try {
            final keypair = DilithiumService.generateKeypairFromSeed(seed);
            address = DilithiumService.publicKeyToAddress(keypair.publicKey);
          } finally {
            // Zero BIP39 seed bytes after keypair generation
            seed.fillRange(0, seed.length, 0);
          }
        } else {
          // SECURITY: Reject Ed25519 fallback on mainnet builds
          if (WalletService.mainnetMode) {
            throw Exception(
                'MAINNET SECURITY: Dilithium5 native library required for account creation. '
                'Ed25519 fallback is forbidden on mainnet.');
          }
          // Ed25519 + BLAKE2b fallback — testnet only
          final walletService = WalletService();
          address = await walletService.deriveAddressFromMnemonic(mnemonic);
        }

        // Create account — seed stored in SecureStorage, not SharedPrefs
        await _accountService.createAccount(
          name: accountName,
          address: address,
          seedPhrase: mnemonic,
        );

        await _loadAccounts();

        losLog(
            '👤 [AccountManagementScreen._createNewAccount] Created account: $accountName, address: $address');
        if (mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            SnackBar(
              content: Text('Account "$accountName" created'),
              backgroundColor: Colors.green.shade700,
            ),
          );

          // Show seed phrase backup dialog
          _showSeedPhraseBackup(accountName, mnemonic);
        }
      } catch (e) {
        if (mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            SnackBar(content: Text('Error creating account: $e')),
          );
        }
      }
    }
  }

  void _showSeedPhraseBackup(String accountName, String seedPhrase) {
    showDialog(
      context: context,
      barrierDismissible: false,
      builder: (context) => AlertDialog(
        title: const Text('⚠️ Backup Seed Phrase'),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              'Account: $accountName',
              style: const TextStyle(fontWeight: FontWeight.bold),
            ),
            const SizedBox(height: 16),
            Container(
              padding: const EdgeInsets.all(12),
              decoration: BoxDecoration(
                color: Colors.grey.shade900,
                borderRadius: BorderRadius.circular(8),
                border: Border.all(color: Colors.grey.shade700),
              ),
              child: SelectableText(
                seedPhrase,
                style: const TextStyle(
                  fontFamily: 'monospace',
                  fontSize: 12,
                ),
              ),
            ),
            const SizedBox(height: 16),
            Container(
              padding: const EdgeInsets.all(12),
              decoration: BoxDecoration(
                color: Colors.orange.withValues(alpha: 0.1),
                borderRadius: BorderRadius.circular(8),
              ),
              child: const Row(
                children: [
                  Icon(Icons.security, color: Colors.orange, size: 20),
                  SizedBox(width: 8),
                  Expanded(
                    child: Text(
                      'Write down and store this seed phrase safely. It\'s the only way to recover this account.',
                      style: TextStyle(fontSize: 11),
                    ),
                  ),
                ],
              ),
            ),
          ],
        ),
        actions: [
          ElevatedButton.icon(
            onPressed: () {
              SecureClipboard.copy(seedPhrase);
              ScaffoldMessenger.of(context).showSnackBar(
                const SnackBar(
                    content: Text('Seed phrase copied (auto-clears in 30s)')),
              );
            },
            icon: const Icon(Icons.copy, size: 18),
            label: const Text('Copy'),
          ),
          ElevatedButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('I\'ve Saved It'),
          ),
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('Manage Accounts'),
      ),
      body: _isLoading
          ? const Center(child: CircularProgressIndicator())
          : _accounts.isEmpty
              ? _buildEmptyState()
              : ListView.builder(
                  padding: const EdgeInsets.all(16),
                  itemCount: _accounts.length + 1, // +1 for header
                  itemBuilder: (context, index) {
                    if (index == 0) {
                      return _buildHeader();
                    }
                    return _buildAccountCard(_accounts[index - 1]);
                  },
                ),
      floatingActionButton: FloatingActionButton.extended(
        onPressed: _createNewAccount,
        icon: const Icon(Icons.add),
        label: const Text('New Account'),
      ),
    );
  }

  Widget _buildHeader() {
    return Padding(
      padding: const EdgeInsets.only(bottom: 16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              const Icon(Icons.account_circle, size: 32),
              const SizedBox(width: 12),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    const Text(
                      'Your Accounts',
                      style: TextStyle(
                        fontSize: 24,
                        fontWeight: FontWeight.bold,
                      ),
                    ),
                    Text(
                      '${_accounts.length} account${_accounts.length != 1 ? 's' : ''}',
                      style: const TextStyle(color: Colors.grey),
                    ),
                  ],
                ),
              ),
            ],
          ),
          const SizedBox(height: 8),
          const Text(
            'Tap an account to switch. Long press for more options.',
            style: TextStyle(fontSize: 12, color: Colors.grey),
          ),
        ],
      ),
    );
  }

  Widget _buildAccountCard(AccountProfile account) {
    final isActive = account.id == _activeAccountId;

    return Card(
      margin: const EdgeInsets.only(bottom: 12),
      color: isActive ? Colors.blue.shade900.withValues(alpha: 0.3) : null,
      child: InkWell(
        onTap: () => _switchAccount(account),
        onLongPress: () => _showAccountOptions(account),
        borderRadius: BorderRadius.circular(8),
        child: Padding(
          padding: const EdgeInsets.all(16),
          child: Row(
            children: [
              // Icon
              Container(
                width: 48,
                height: 48,
                decoration: BoxDecoration(
                  color: isActive ? Colors.blue.shade700 : Colors.grey.shade800,
                  borderRadius: BorderRadius.circular(24),
                ),
                child: Icon(
                  account.isHardwareWallet
                      ? Icons.security
                      : Icons.account_balance_wallet,
                  color: isActive ? Colors.white : Colors.grey.shade500,
                ),
              ),
              const SizedBox(width: 16),

              // Account Info
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(
                      children: [
                        Flexible(
                          child: Text(
                            account.name,
                            style: TextStyle(
                              fontSize: 16,
                              fontWeight: FontWeight.bold,
                              color: isActive ? Colors.blue.shade300 : null,
                            ),
                          ),
                        ),
                        if (isActive) ...[
                          const SizedBox(width: 8),
                          Container(
                            padding: const EdgeInsets.symmetric(
                              horizontal: 8,
                              vertical: 2,
                            ),
                            decoration: BoxDecoration(
                              color: Colors.blue.shade700,
                              borderRadius: BorderRadius.circular(10),
                            ),
                            child: const Text(
                              'ACTIVE',
                              style: TextStyle(
                                fontSize: 10,
                                fontWeight: FontWeight.bold,
                              ),
                            ),
                          ),
                        ],
                      ],
                    ),
                    const SizedBox(height: 4),
                    Text(
                      account.truncatedAddress,
                      style: const TextStyle(
                        fontFamily: 'monospace',
                        fontSize: 12,
                        color: Colors.grey,
                      ),
                    ),
                    const SizedBox(height: 4),
                    Row(
                      children: [
                        Icon(
                          account.isHardwareWallet ? Icons.usb : Icons.vpn_key,
                          size: 12,
                          color: Colors.grey,
                        ),
                        const SizedBox(width: 4),
                        Text(
                          account.accountType,
                          style: const TextStyle(
                            fontSize: 10,
                            color: Colors.grey,
                          ),
                        ),
                      ],
                    ),
                  ],
                ),
              ),

              // Actions
              IconButton(
                icon: const Icon(Icons.more_vert),
                onPressed: () => _showAccountOptions(account),
              ),
            ],
          ),
        ),
      ),
    );
  }

  void _showAccountOptions(AccountProfile account) {
    final isActive = account.id == _activeAccountId;

    showModalBottomSheet(
      context: context,
      builder: (context) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            ListTile(
              leading: const Icon(Icons.edit),
              title: const Text('Rename'),
              onTap: () {
                Navigator.pop(context);
                _showRenameDialog(account);
              },
            ),
            if (!isActive)
              ListTile(
                leading: const Icon(Icons.swap_horiz),
                title: const Text('Switch to this account'),
                onTap: () {
                  Navigator.pop(context);
                  _switchAccount(account);
                },
              ),
            ListTile(
              leading: const Icon(Icons.copy),
              title: const Text('Copy address'),
              onTap: () {
                SecureClipboard.copyPublic(account.address);
                Navigator.pop(context);
                ScaffoldMessenger.of(context).showSnackBar(
                  const SnackBar(content: Text('Address copied')),
                );
              },
            ),
            if (_accounts.length > 1)
              ListTile(
                leading: const Icon(Icons.delete, color: Colors.red),
                title:
                    const Text('Delete', style: TextStyle(color: Colors.red)),
                onTap: () {
                  Navigator.pop(context);
                  _deleteAccount(account);
                },
              ),
            const SizedBox(height: 8),
          ],
        ),
      ),
    );
  }

  Widget _buildEmptyState() {
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(32),
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            Icon(
              Icons.account_circle_outlined,
              size: 80,
              color: Colors.grey.shade600,
            ),
            const SizedBox(height: 24),
            const Text(
              'No Accounts',
              style: TextStyle(
                fontSize: 24,
                fontWeight: FontWeight.bold,
              ),
            ),
            const SizedBox(height: 8),
            const Text(
              'Create your first account to get started',
              textAlign: TextAlign.center,
              style: TextStyle(color: Colors.grey),
            ),
            const SizedBox(height: 24),
            ElevatedButton.icon(
              onPressed: _createNewAccount,
              icon: const Icon(Icons.add),
              label: const Text('Create Account'),
            ),
          ],
        ),
      ),
    );
  }
}
