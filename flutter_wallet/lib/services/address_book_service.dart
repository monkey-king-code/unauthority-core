import '../utils/log.dart';
import 'dart:convert';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:uuid/uuid.dart';
import '../models/address_book_entry.dart';
import '../utils/address_validator.dart';

class AddressBookService {
  static const String _storageKey = 'los_address_book';
  final _uuid = const Uuid();

  // Load address book from storage
  Future<AddressBook> loadAddressBook() async {
    losLog(
        '📒 [AddressBookService.loadAddressBook] Loading address book...');
    try {
      final prefs = await SharedPreferences.getInstance();
      final jsonString = prefs.getString(_storageKey);

      if (jsonString == null || jsonString.isEmpty) {
        return AddressBook.empty();
      }

      final jsonData = json.decode(jsonString) as Map<String, dynamic>;
      final addressBook = AddressBook.fromJson(jsonData);
      losLog(
          '📒 [AddressBookService.loadAddressBook] Loaded ${addressBook.entries.length} entries');
      return addressBook;
    } catch (e) {
      losLog('Error loading address book: $e');
      return AddressBook.empty();
    }
  }

  // Save address book to storage
  Future<void> saveAddressBook(AddressBook addressBook) async {
    try {
      final prefs = await SharedPreferences.getInstance();
      final jsonString = json.encode(addressBook.toJson());
      await prefs.setString(_storageKey, jsonString);
    } catch (e) {
      losLog('Error saving address book: $e');
      rethrow;
    }
  }

  // Add new entry
  Future<AddressBookEntry> addEntry({
    required String name,
    required String address,
    String? notes,
  }) async {
    losLog('📒 [AddressBookService.addEntry] Adding contact: $name');
    // Validate address format
    final validationError = AddressValidator.getValidationError(address);
    if (validationError != null) {
      throw Exception(validationError);
    }

    // Load current address book
    final addressBook = await loadAddressBook();

    // Check if address already exists
    if (addressBook.hasAddress(address)) {
      throw Exception('This address already exists in your address book');
    }

    // Check if name already exists
    if (addressBook.hasName(name)) {
      throw Exception('An entry with this name already exists');
    }

    // Create new entry
    final entry = AddressBookEntry(
      id: _uuid.v4(),
      name: name,
      address: address,
      notes: notes,
      createdAt: DateTime.now(),
    );

    // Add to address book and save
    final updatedBook = addressBook.addEntry(entry);
    await saveAddressBook(updatedBook);

    losLog('📒 [AddressBookService.addEntry] Added contact: $name');
    return entry;
  }

  // Update existing entry
  Future<void> updateEntry({
    required String entryId,
    String? name,
    String? address,
    String? notes,
  }) async {
    losLog('📒 [AddressBookService.updateEntry] Updating entry: $entryId');
    final addressBook = await loadAddressBook();
    final existingEntry = addressBook.findById(entryId);

    if (existingEntry == null) {
      throw Exception('Entry not found');
    }

    // Validate address if changed
    if (address != null && address != existingEntry.address) {
      final validationError = AddressValidator.getValidationError(address);
      if (validationError != null) {
        throw Exception(validationError);
      }

      // Check if new address already exists in another entry
      final addressEntry = addressBook.findByAddress(address);
      if (addressEntry != null && addressEntry.id != entryId) {
        throw Exception('This address already exists in another entry');
      }
    }

    // Check if new name already exists in another entry
    if (name != null && name != existingEntry.name) {
      final entries = addressBook.entries.where(
          (e) => e.id != entryId && e.name.toLowerCase() == name.toLowerCase());
      if (entries.isNotEmpty) {
        throw Exception('An entry with this name already exists');
      }
    }

    // Create updated entry
    final updatedEntry = existingEntry.copyWith(
      name: name,
      address: address,
      notes: notes,
      updatedAt: DateTime.now(),
    );

    // Update and save
    final updatedBook = addressBook.updateEntry(updatedEntry);
    await saveAddressBook(updatedBook);
    losLog('📒 [AddressBookService.updateEntry] Updated entry: $entryId');
  }

  // Delete entry
  Future<void> deleteEntry(String entryId) async {
    losLog('📒 [AddressBookService.deleteEntry] Deleting entry: $entryId');
    final addressBook = await loadAddressBook();
    final updatedBook = addressBook.removeEntry(entryId);
    await saveAddressBook(updatedBook);
    losLog('📒 [AddressBookService.deleteEntry] Deleted entry: $entryId');
  }

  // Get all entries
  Future<List<AddressBookEntry>> getAllEntries() async {
    final addressBook = await loadAddressBook();
    return addressBook.entries;
  }

  // Get entries sorted by name
  Future<List<AddressBookEntry>> getEntriesSortedByName() async {
    final addressBook = await loadAddressBook();
    return addressBook.getSortedByName();
  }

  // Get entries sorted by date
  Future<List<AddressBookEntry>> getEntriesSortedByDate() async {
    final addressBook = await loadAddressBook();
    return addressBook.getSortedByDate();
  }

  // Search entries
  Future<List<AddressBookEntry>> searchEntries(String query) async {
    final addressBook = await loadAddressBook();
    return addressBook.search(query);
  }

  // Find entry by address
  Future<AddressBookEntry?> findByAddress(String address) async {
    final addressBook = await loadAddressBook();
    return addressBook.findByAddress(address);
  }

  // Check if address exists
  Future<bool> hasAddress(String address) async {
    final addressBook = await loadAddressBook();
    return addressBook.hasAddress(address);
  }

  // Get entry count
  Future<int> getEntryCount() async {
    final addressBook = await loadAddressBook();
    return addressBook.length;
  }

  // Export address book as JSON string (for backup)
  Future<String> exportAsJson() async {
    losLog(
        '📒 [AddressBookService.exportAsJson] Exporting address book...');
    final addressBook = await loadAddressBook();
    losLog(
        '📒 [AddressBookService.exportAsJson] Exported ${addressBook.entries.length} entries');
    return json.encode(addressBook.toJson());
  }

  // Import address book from JSON string (for restore)
  Future<void> importFromJson(String jsonString) async {
    losLog(
        '📒 [AddressBookService.importFromJson] Importing address book from JSON...');
    try {
      final jsonData = json.decode(jsonString) as Map<String, dynamic>;
      final addressBook = AddressBook.fromJson(jsonData);
      await saveAddressBook(addressBook);
      losLog(
          '📒 [AddressBookService.importFromJson] Imported ${addressBook.entries.length} entries');
    } catch (e) {
      throw Exception('Invalid address book format: $e');
    }
  }

  // Clear all entries (with confirmation)
  Future<void> clearAll() async {
    losLog('📒 [AddressBookService.clearAll] Clearing all entries...');
    await saveAddressBook(AddressBook.empty());
    losLog('📒 [AddressBookService.clearAll] All entries cleared');
  }
}
