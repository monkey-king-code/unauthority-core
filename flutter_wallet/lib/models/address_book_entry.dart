class AddressBookEntry {
  final String id;
  final String name;
  final String address;
  final String? notes;
  final DateTime createdAt;
  final DateTime? updatedAt;

  AddressBookEntry({
    required this.id,
    required this.name,
    required this.address,
    this.notes,
    required this.createdAt,
    this.updatedAt,
  });

  // Create from JSON
  factory AddressBookEntry.fromJson(Map<String, dynamic> json) {
    return AddressBookEntry(
      id: json['id'] as String,
      name: json['name'] as String,
      address: json['address'] as String,
      notes: json['notes'] as String?,
      createdAt: DateTime.parse(json['createdAt'] as String),
      updatedAt: json['updatedAt'] != null
          ? DateTime.parse(json['updatedAt'] as String)
          : null,
    );
  }

  // Convert to JSON
  Map<String, dynamic> toJson() {
    return {
      'id': id,
      'name': name,
      'address': address,
      'notes': notes,
      'createdAt': createdAt.toIso8601String(),
      'updatedAt': updatedAt?.toIso8601String(),
    };
  }

  // Copy with modifications
  AddressBookEntry copyWith({
    String? id,
    String? name,
    String? address,
    String? notes,
    DateTime? createdAt,
    DateTime? updatedAt,
  }) {
    return AddressBookEntry(
      id: id ?? this.id,
      name: name ?? this.name,
      address: address ?? this.address,
      notes: notes ?? this.notes,
      createdAt: createdAt ?? this.createdAt,
      updatedAt: updatedAt ?? this.updatedAt,
    );
  }

  // Get short address for display
  String getShortAddress() {
    if (address.length <= 16) return address;
    return '${address.substring(0, 8)}...${address.substring(address.length - 8)}';
  }

  @override
  bool operator ==(Object other) {
    if (identical(this, other)) return true;
    return other is AddressBookEntry && other.id == id;
  }

  @override
  int get hashCode => id.hashCode;

  @override
  String toString() {
    return 'AddressBookEntry{id: $id, name: $name, address: ${getShortAddress()}}';
  }
}

// Address book collection manager
class AddressBook {
  final List<AddressBookEntry> entries;

  AddressBook({required this.entries});

  factory AddressBook.empty() {
    return AddressBook(entries: []);
  }

  factory AddressBook.fromJson(Map<String, dynamic> json) {
    final entriesList = json['entries'] as List<dynamic>? ?? [];
    return AddressBook(
      entries: entriesList
          .map((e) => AddressBookEntry.fromJson(e as Map<String, dynamic>))
          .toList(),
    );
  }

  Map<String, dynamic> toJson() {
    return {
      'entries': entries.map((e) => e.toJson()).toList(),
    };
  }

  // Add entry
  AddressBook addEntry(AddressBookEntry entry) {
    return AddressBook(entries: [...entries, entry]);
  }

  // Update entry
  AddressBook updateEntry(AddressBookEntry updatedEntry) {
    final newEntries = entries.map((e) {
      return e.id == updatedEntry.id ? updatedEntry : e;
    }).toList();
    return AddressBook(entries: newEntries);
  }

  // Remove entry
  AddressBook removeEntry(String entryId) {
    return AddressBook(
      entries: entries.where((e) => e.id != entryId).toList(),
    );
  }

  // Find entry by ID
  AddressBookEntry? findById(String id) {
    try {
      return entries.firstWhere((e) => e.id == id);
    } catch (_) {
      return null;
    }
  }

  // Find entry by address
  AddressBookEntry? findByAddress(String address) {
    try {
      return entries.firstWhere((e) => e.address == address);
    } catch (_) {
      return null;
    }
  }

  // Search entries by name or address
  List<AddressBookEntry> search(String query) {
    if (query.isEmpty) return entries;

    final lowerQuery = query.toLowerCase();
    return entries.where((e) {
      return e.name.toLowerCase().contains(lowerQuery) ||
          e.address.toLowerCase().contains(lowerQuery) ||
          (e.notes?.toLowerCase().contains(lowerQuery) ?? false);
    }).toList();
  }

  // Check if address exists
  bool hasAddress(String address) {
    return entries.any((e) => e.address == address);
  }

  // Check if name exists (case-insensitive)
  bool hasName(String name) {
    final lowerName = name.toLowerCase();
    return entries.any((e) => e.name.toLowerCase() == lowerName);
  }

  // Get sorted entries (by name)
  List<AddressBookEntry> getSortedByName() {
    final sorted = List<AddressBookEntry>.from(entries);
    sorted.sort((a, b) => a.name.toLowerCase().compareTo(b.name.toLowerCase()));
    return sorted;
  }

  // Get sorted entries (by date, newest first)
  List<AddressBookEntry> getSortedByDate() {
    final sorted = List<AddressBookEntry>.from(entries);
    sorted.sort((a, b) => b.createdAt.compareTo(a.createdAt));
    return sorted;
  }

  int get length => entries.length;
  bool get isEmpty => entries.isEmpty;
  bool get isNotEmpty => entries.isNotEmpty;
}
