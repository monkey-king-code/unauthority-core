class AccountProfile {
  final String id;
  final String name;
  final String address;
  final String? seedPhrase; // Encrypted in storage
  final String? publicKey;
  final DateTime createdAt;
  final bool isHardwareWallet;
  final String? hardwareWalletId;

  AccountProfile({
    required this.id,
    required this.name,
    required this.address,
    this.seedPhrase,
    this.publicKey,
    required this.createdAt,
    this.isHardwareWallet = false,
    this.hardwareWalletId,
  });

  factory AccountProfile.fromJson(Map<String, dynamic> json) {
    return AccountProfile(
      id: json['id'] ?? '',
      name: json['name'] ?? '',
      address: json['address'] ?? '',
      seedPhrase: json['seed_phrase'],
      publicKey: json['public_key'],
      createdAt: json['created_at'] != null
          ? DateTime.parse(json['created_at'])
          : DateTime.now(),
      isHardwareWallet: json['is_hardware_wallet'] ?? false,
      hardwareWalletId: json['hardware_wallet_id'],
    );
  }

  /// Serialize for SharedPreferences — NEVER includes seed phrase.
  /// Seed phrases are stored separately in FlutterSecureStorage.
  Map<String, dynamic> toJson() {
    return {
      'id': id,
      'name': name,
      'address': address,
      // seed_phrase intentionally omitted — stored in SecureStorage only
      'public_key': publicKey,
      'created_at': createdAt.toIso8601String(),
      'is_hardware_wallet': isHardwareWallet,
      'hardware_wallet_id': hardwareWalletId,
    };
  }

  AccountProfile copyWith({
    String? id,
    String? name,
    String? address,
    String? seedPhrase,
    String? publicKey,
    DateTime? createdAt,
    bool? isHardwareWallet,
    String? hardwareWalletId,
  }) {
    return AccountProfile(
      id: id ?? this.id,
      name: name ?? this.name,
      address: address ?? this.address,
      seedPhrase: seedPhrase ?? this.seedPhrase,
      publicKey: publicKey ?? this.publicKey,
      createdAt: createdAt ?? this.createdAt,
      isHardwareWallet: isHardwareWallet ?? this.isHardwareWallet,
      hardwareWalletId: hardwareWalletId ?? this.hardwareWalletId,
    );
  }

  String get truncatedAddress {
    if (address.length <= 20) return address;
    return '${address.substring(0, 10)}...${address.substring(address.length - 8)}';
  }

  String get accountType {
    if (isHardwareWallet) return 'Hardware Wallet';
    return 'Software Wallet';
  }
}

class AccountsList {
  final List<AccountProfile> accounts;
  final String? activeAccountId;

  AccountsList({
    required this.accounts,
    this.activeAccountId,
  });

  factory AccountsList.fromJson(Map<String, dynamic> json) {
    return AccountsList(
      accounts: (json['accounts'] as List?)
              ?.map((a) => AccountProfile.fromJson(a))
              .toList() ??
          [],
      activeAccountId: json['active_account_id'],
    );
  }

  Map<String, dynamic> toJson() {
    return {
      'accounts': accounts.map((a) => a.toJson()).toList(),
      'active_account_id': activeAccountId,
    };
  }

  AccountProfile? get activeAccount {
    if (activeAccountId == null) return null;
    try {
      return accounts.firstWhere((a) => a.id == activeAccountId);
    } catch (e) {
      return null;
    }
  }

  AccountsList copyWith({
    List<AccountProfile>? accounts,
    String? activeAccountId,
  }) {
    return AccountsList(
      accounts: accounts ?? this.accounts,
      activeAccountId: activeAccountId ?? this.activeAccountId,
    );
  }

  AccountsList addAccount(AccountProfile account) {
    return AccountsList(
      accounts: [...accounts, account],
      activeAccountId: activeAccountId,
    );
  }

  AccountsList removeAccount(String accountId) {
    return AccountsList(
      accounts: accounts.where((a) => a.id != accountId).toList(),
      activeAccountId: activeAccountId == accountId ? null : activeAccountId,
    );
  }

  AccountsList updateAccount(AccountProfile account) {
    return AccountsList(
      accounts: accounts.map((a) => a.id == account.id ? account : a).toList(),
      activeAccountId: activeAccountId,
    );
  }

  AccountsList setActiveAccount(String accountId) {
    return AccountsList(
      accounts: accounts,
      activeAccountId: accountId,
    );
  }
}
