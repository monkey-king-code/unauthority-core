import '../utils/log.dart';
import 'dart:convert';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:uuid/uuid.dart';
import '../models/account_profile.dart';
import 'wallet_service.dart';

class AccountManagementService {
  static const String _storageKey = 'los_accounts';
  static const String _activeAccountKey = 'los_active_account';

  /// Prefix for seed phrase keys in SecureStorage.
  /// Full key: "los_account_seed_{accountId}"
  static const String _seedKeyPrefix = 'los_account_seed_';

  final _uuid = const Uuid();

  /// Encrypted storage backed by platform Keychain (iOS/macOS) or Keystore (Android)
  /// useDataProtectionKeyChain: false = legacy keychain (works with ad-hoc signing)
  static const _secureStorage = FlutterSecureStorage(
    aOptions: AndroidOptions(encryptedSharedPreferences: true),
    iOptions: IOSOptions(accessibility: KeychainAccessibility.first_unlock),
    mOptions: MacOsOptions(useDataProtectionKeyChain: false),
  );

  /// One-time migration: move any seed phrases from the old SharedPreferences
  /// JSON (pre-audit format with seed_phrase in toJson) into SecureStorage.
  /// Call this once on app startup.
  Future<void> migrateSecretsFromSharedPreferences() async {
    final prefs = await SharedPreferences.getInstance();
    final accountsJson = prefs.getString(_storageKey);
    if (accountsJson == null) return;

    try {
      final Map<String, dynamic> data = json.decode(accountsJson);
      final accounts = data['accounts'] as List? ?? [];
      bool migrated = false;

      for (final accountMap in accounts) {
        final seed = accountMap['seed_phrase'];
        final id = accountMap['id'];
        if (seed != null && id != null) {
          await _secureStorage.write(key: '$_seedKeyPrefix$id', value: seed);
          accountMap.remove('seed_phrase');
          migrated = true;
        }
      }

      if (migrated) {
        // Re-save without seed phrases
        await prefs.setString(_storageKey, json.encode(data));
        losLog('🔒 Migrated account seed phrases to SecureStorage');
      }
    } catch (e) {
      losLog('Migration error (non-fatal): $e');
    }
  }

  /// Load all accounts from storage.
  /// Account metadata is in SharedPreferences (no secrets).
  /// Seed phrases are loaded separately from FlutterSecureStorage.
  Future<AccountsList> loadAccounts() async {
    losLog(
        '👤 [AccountManagementService.loadAccounts] Loading accounts...');
    final prefs = await SharedPreferences.getInstance();
    final accountsJson = prefs.getString(_storageKey);
    final activeAccountId = prefs.getString(_activeAccountKey);

    if (accountsJson == null) {
      return AccountsList(accounts: [], activeAccountId: null);
    }

    try {
      final Map<String, dynamic> data = json.decode(accountsJson);
      final rawAccounts = (data['accounts'] as List?)
              ?.map((a) => AccountProfile.fromJson(a))
              .toList() ??
          [];

      // Restore seed phrases from SecureStorage
      final accounts = <AccountProfile>[];
      for (final account in rawAccounts) {
        final seed = await _secureStorage.read(
          key: '$_seedKeyPrefix${account.id}',
        );
        accounts.add(
          seed != null ? account.copyWith(seedPhrase: seed) : account,
        );
      }

      losLog(
          '👤 [AccountManagementService.loadAccounts] Loaded ${accounts.length} accounts, active: $activeAccountId');
      return AccountsList(accounts: accounts, activeAccountId: activeAccountId);
    } catch (e) {
      losLog('👤 [AccountManagementService.loadAccounts] Error: $e');
      losLog('Error loading accounts: $e');
      return AccountsList(accounts: [], activeAccountId: null);
    }
  }

  /// Save accounts to storage.
  /// Metadata (no secrets) → SharedPreferences.
  /// Seed phrases → FlutterSecureStorage keyed by account ID.
  Future<void> saveAccounts(AccountsList accountsList) async {
    losLog(
        '👤 [AccountManagementService.saveAccounts] Saving ${accountsList.accounts.length} accounts...');
    final prefs = await SharedPreferences.getInstance();
    // toJson() on AccountProfile now excludes seed_phrase
    final accountsJson = json.encode({
      'accounts': accountsList.accounts.map((a) => a.toJson()).toList(),
    });

    await prefs.setString(_storageKey, accountsJson);

    // Persist each account's seed phrase to SecureStorage
    for (final account in accountsList.accounts) {
      if (account.seedPhrase != null) {
        await _secureStorage.write(
          key: '$_seedKeyPrefix${account.id}',
          value: account.seedPhrase!,
        );
      }
    }

    if (accountsList.activeAccountId != null) {
      await prefs.setString(_activeAccountKey, accountsList.activeAccountId!);
    } else {
      await prefs.remove(_activeAccountKey);
    }
    losLog(
        '👤 [AccountManagementService.saveAccounts] Saved ${accountsList.accounts.length} accounts');
  }

  /// Create new account
  Future<AccountProfile> createAccount({
    required String name,
    required String address,
    required String seedPhrase,
    String? publicKey,
  }) async {
    losLog(
        '👤 [AccountManagementService.createAccount] Creating account: $name');
    final account = AccountProfile(
      id: _uuid.v4(),
      name: name,
      address: address,
      seedPhrase: seedPhrase,
      publicKey: publicKey,
      createdAt: DateTime.now(),
    );

    final accountsList = await loadAccounts();
    final updatedList = accountsList.addAccount(account);

    // Set as active if it's the first account
    if (updatedList.accounts.length == 1) {
      await saveAccounts(updatedList.setActiveAccount(account.id));
    } else {
      await saveAccounts(updatedList);
    }

    losLog(
        '👤 [AccountManagementService.createAccount] Created account ${account.id}: $address');
    return account;
  }

  /// Import account from seed phrase
  Future<AccountProfile> importAccount({
    required String name,
    required String address,
    required String seedPhrase,
    String? publicKey,
  }) async {
    losLog(
        '👤 [AccountManagementService.importAccount] Importing account: $name, $address');
    // Check if account already exists
    final accountsList = await loadAccounts();
    final existing = accountsList.accounts.where((a) => a.address == address);

    if (existing.isNotEmpty) {
      throw Exception('Account with this address already exists');
    }

    final result = await createAccount(
      name: name,
      address: address,
      seedPhrase: seedPhrase,
      publicKey: publicKey,
    );
    losLog(
        '👤 [AccountManagementService.importAccount] Imported account: $name, $address');
    return result;
  }

  /// Add hardware wallet account
  Future<AccountProfile> addHardwareWalletAccount({
    required String name,
    required String address,
    required String publicKey,
    required String hardwareWalletId,
  }) async {
    final account = AccountProfile(
      id: _uuid.v4(),
      name: name,
      address: address,
      publicKey: publicKey,
      createdAt: DateTime.now(),
      isHardwareWallet: true,
      hardwareWalletId: hardwareWalletId,
    );

    final accountsList = await loadAccounts();
    final updatedList = accountsList.addAccount(account);
    await saveAccounts(updatedList);

    return account;
  }

  /// Switch active account and restore its wallet keys into the primary
  /// WalletService storage so all signing/balance operations use the
  /// correct keypair.
  ///
  /// [walletService] — the app's shared WalletService instance (from Provider).
  /// This avoids creating a throwaway WalletService() that's disconnected
  /// from the widget tree.
  Future<void> switchAccount(String accountId,
      {WalletService? walletService}) async {
    losLog(
        '👤 [AccountManagementService.switchAccount] Switching to account: $accountId');
    final accountsList = await loadAccounts();

    // Verify account exists
    final account = accountsList.accounts.firstWhere(
      (a) => a.id == accountId,
      orElse: () => throw Exception('Account not found'),
    );

    final updatedList = accountsList.setActiveAccount(accountId);
    await saveAccounts(updatedList);

    // Restore this account's wallet into the primary WalletService keys
    // so that signing, balance checks, etc. use the right keypair.
    final ws = walletService ?? WalletService();
    if (account.seedPhrase != null && !account.isHardwareWallet) {
      await ws.importWallet(account.seedPhrase!);
    } else if (account.isHardwareWallet) {
      // Hardware wallets don't have seed phrases to restore
    } else {
      // Address-only account — restore address via importByAddress
      await ws.importByAddress(account.address);
    }
    losLog(
        '👤 [AccountManagementService.switchAccount] Switched to account: ${account.name}');
  }

  /// Rename account
  Future<void> renameAccount(String accountId, String newName) async {
    losLog(
        '👤 [AccountManagementService.renameAccount] Renaming account $accountId to $newName');
    final accountsList = await loadAccounts();
    final account = accountsList.accounts.firstWhere(
      (a) => a.id == accountId,
      orElse: () => throw Exception('Account not found'),
    );

    final updatedAccount = account.copyWith(name: newName);
    final updatedList = accountsList.updateAccount(updatedAccount);
    await saveAccounts(updatedList);
    losLog(
        '👤 [AccountManagementService.renameAccount] Renamed account $accountId to $newName');
  }

  /// Delete account
  Future<void> deleteAccount(String accountId) async {
    losLog(
        '👤 [AccountManagementService.deleteAccount] Deleting account: $accountId');
    final accountsList = await loadAccounts();

    // Don't allow deleting the last account
    if (accountsList.accounts.length <= 1) {
      throw Exception('Cannot delete the last account');
    }

    // Purge seed phrase from SecureStorage
    await _secureStorage.delete(key: '$_seedKeyPrefix$accountId');

    final updatedList = accountsList.removeAccount(accountId);

    // Set first account as active if we deleted the active one
    if (updatedList.activeAccountId == null &&
        updatedList.accounts.isNotEmpty) {
      await saveAccounts(
        updatedList.setActiveAccount(updatedList.accounts.first.id),
      );
    } else {
      await saveAccounts(updatedList);
    }
    losLog(
        '👤 [AccountManagementService.deleteAccount] Deleted account: $accountId');
  }

  /// Get active account
  Future<AccountProfile?> getActiveAccount() async {
    final accountsList = await loadAccounts();
    return accountsList.activeAccount;
  }

  /// Get all accounts
  Future<List<AccountProfile>> getAllAccounts() async {
    final accountsList = await loadAccounts();
    return accountsList.accounts;
  }

  /// Check if account name already exists
  Future<bool> isNameTaken(String name, {String? excludeId}) async {
    final accountsList = await loadAccounts();
    return accountsList.accounts.any(
      (a) => a.name.toLowerCase() == name.toLowerCase() && a.id != excludeId,
    );
  }

  /// Get account count
  Future<int> getAccountCount() async {
    final accountsList = await loadAccounts();
    return accountsList.accounts.length;
  }
}
