import '../utils/log.dart';
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../models/account_profile.dart';
import '../services/account_management_service.dart';
import '../services/wallet_service.dart';
import '../screens/account_management_screen.dart';

class AccountSwitcher extends StatefulWidget {
  final Function(AccountProfile)? onAccountChanged;

  const AccountSwitcher({
    super.key,
    this.onAccountChanged,
  });

  @override
  State<AccountSwitcher> createState() => _AccountSwitcherState();
}

class _AccountSwitcherState extends State<AccountSwitcher> {
  final _accountService = AccountManagementService();
  AccountProfile? _activeAccount;
  List<AccountProfile> _accounts = [];

  @override
  void initState() {
    super.initState();
    _loadActiveAccount();
  }

  Future<void> _loadActiveAccount() async {
    try {
      final accountsList = await _accountService.loadAccounts();
      if (!mounted) return;
      setState(() {
        _activeAccount = accountsList.activeAccount;
        _accounts = accountsList.accounts;
      });
    } catch (e) {
      losLog('Error loading active account: $e');
    }
  }

  Future<void> _switchAccount(AccountProfile account) async {
    try {
      final walletService = context.read<WalletService>();
      await _accountService.switchAccount(account.id,
          walletService: walletService);
      if (!mounted) return;
      setState(() => _activeAccount = account);

      widget.onAccountChanged?.call(account);

      if (mounted) {
        Navigator.pop(context);
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: Text('Switched to ${account.name}'),
            backgroundColor: Colors.green.shade700,
            duration: const Duration(seconds: 1),
          ),
        );
      }
    } catch (e) {
      losLog('⚠️ [AccountSwitcher] Switch failed: $e');
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Error: $e')),
        );
      }
    }
  }

  void _showAccountSelector() {
    showModalBottomSheet(
      context: context,
      isScrollControlled: true,
      builder: (context) => DraggableScrollableSheet(
        initialChildSize: 0.5,
        minChildSize: 0.3,
        maxChildSize: 0.9,
        expand: false,
        builder: (context, scrollController) => Column(
          children: [
            // Header
            Container(
              padding: const EdgeInsets.all(16),
              decoration: BoxDecoration(
                border: Border(
                  bottom: BorderSide(
                    color: Colors.grey.shade800,
                  ),
                ),
              ),
              child: Row(
                children: [
                  const Expanded(
                    child: Text(
                      'Switch Account',
                      style: TextStyle(
                        fontSize: 18,
                        fontWeight: FontWeight.bold,
                      ),
                    ),
                  ),
                  TextButton.icon(
                    onPressed: () async {
                      Navigator.pop(context);
                      final result = await Navigator.push(
                        context,
                        MaterialPageRoute(
                          builder: (_) => const AccountManagementScreen(),
                        ),
                      );
                      if (result == true) {
                        _loadActiveAccount();
                        widget.onAccountChanged?.call(_activeAccount!);
                      }
                    },
                    icon: const Icon(Icons.settings, size: 18),
                    label: const Text('Manage'),
                  ),
                ],
              ),
            ),

            // Accounts List
            Expanded(
              child: ListView.builder(
                controller: scrollController,
                padding: const EdgeInsets.symmetric(vertical: 8),
                itemCount: _accounts.length,
                itemBuilder: (context, index) {
                  final account = _accounts[index];
                  final isActive = account.id == _activeAccount?.id;

                  return ListTile(
                    leading: Container(
                      width: 40,
                      height: 40,
                      decoration: BoxDecoration(
                        color: isActive
                            ? Colors.blue.shade700
                            : Colors.grey.shade800,
                        borderRadius: BorderRadius.circular(20),
                      ),
                      child: Icon(
                        account.isHardwareWallet
                            ? Icons.security
                            : Icons.account_balance_wallet,
                        size: 20,
                        color: isActive ? Colors.white : Colors.grey.shade500,
                      ),
                    ),
                    title: Text(
                      account.name,
                      style: TextStyle(
                        fontWeight:
                            isActive ? FontWeight.bold : FontWeight.normal,
                        color: isActive ? Colors.blue.shade300 : null,
                      ),
                    ),
                    subtitle: Text(
                      account.truncatedAddress,
                      style: const TextStyle(
                        fontFamily: 'monospace',
                        fontSize: 11,
                      ),
                    ),
                    trailing: isActive
                        ? Icon(Icons.check_circle, color: Colors.blue.shade400)
                        : null,
                    onTap: isActive ? null : () => _switchAccount(account),
                  );
                },
              ),
            ),
          ],
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    if (_activeAccount == null) {
      return const SizedBox.shrink();
    }

    return InkWell(
      onTap: _showAccountSelector,
      borderRadius: BorderRadius.circular(20),
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
        decoration: BoxDecoration(
          color: Colors.grey.shade900,
          borderRadius: BorderRadius.circular(20),
          border: Border.all(
            color: Colors.grey.shade800,
          ),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Container(
              width: 24,
              height: 24,
              decoration: BoxDecoration(
                color: Colors.blue.shade700,
                borderRadius: BorderRadius.circular(12),
              ),
              child: Icon(
                _activeAccount!.isHardwareWallet
                    ? Icons.security
                    : Icons.account_balance_wallet,
                size: 14,
                color: Colors.white,
              ),
            ),
            const SizedBox(width: 8),
            ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 120),
              child: Text(
                _activeAccount!.name,
                style: const TextStyle(
                  fontSize: 14,
                  fontWeight: FontWeight.w500,
                ),
                overflow: TextOverflow.ellipsis,
              ),
            ),
            const SizedBox(width: 4),
            Icon(
              Icons.keyboard_arrow_down,
              size: 18,
              color: Colors.grey.shade500,
            ),
          ],
        ),
      ),
    );
  }
}
