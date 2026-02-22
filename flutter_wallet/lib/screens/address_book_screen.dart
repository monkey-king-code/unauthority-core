import '../utils/log.dart';
import '../utils/secure_clipboard.dart';
import 'package:flutter/material.dart';
import '../models/address_book_entry.dart';
import '../services/address_book_service.dart';
import '../utils/address_validator.dart';

class AddressBookScreen extends StatefulWidget {
  const AddressBookScreen({super.key});

  @override
  State<AddressBookScreen> createState() => _AddressBookScreenState();
}

class _AddressBookScreenState extends State<AddressBookScreen> {
  final AddressBookService _addressBookService = AddressBookService();
  List<AddressBookEntry> _entries = [];
  List<AddressBookEntry> _filteredEntries = [];
  bool _isLoading = true;
  String _searchQuery = '';
  final TextEditingController _searchController = TextEditingController();

  @override
  void initState() {
    super.initState();
    _loadEntries();
  }

  @override
  void dispose() {
    _searchController.dispose();
    super.dispose();
  }

  Future<void> _loadEntries() async {
    losLog('📒 [AddressBookScreen._loadEntries] Loading entries...');
    setState(() => _isLoading = true);
    try {
      final entries = await _addressBookService.getEntriesSortedByName();
      if (!mounted) return;
      setState(() {
        _entries = entries;
        _filteredEntries = entries;
        _isLoading = false;
      });
      losLog(
          '📒 [AddressBookScreen._loadEntries] Loaded ${entries.length} entries');
    } catch (e) {
      if (!mounted) return;
      setState(() => _isLoading = false);
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: Text('Error loading address book: $e'),
            backgroundColor: Colors.red,
          ),
        );
      }
    }
  }

  void _filterEntries(String query) {
    setState(() {
      _searchQuery = query;
      if (query.isEmpty) {
        _filteredEntries = _entries;
      } else {
        final addressBook = AddressBook(entries: _entries);
        _filteredEntries = addressBook.search(query);
      }
    });
  }

  Future<void> _showAddDialog() async {
    final nameController = TextEditingController();
    final addressController = TextEditingController();
    final notesController = TextEditingController();
    final formKey = GlobalKey<FormState>();

    await showDialog(
      context: context,
      builder: (dialogContext) => AlertDialog(
        title: const Text('Add Address'),
        content: Form(
          key: formKey,
          child: SingleChildScrollView(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                TextFormField(
                  controller: nameController,
                  decoration: const InputDecoration(
                    labelText: 'Name *',
                    hintText: 'e.g., Alice, Exchange, DAO Treasury',
                    prefixIcon: Icon(Icons.label),
                  ),
                  validator: (value) {
                    if (value == null || value.trim().isEmpty) {
                      return 'Please enter a name';
                    }
                    if (value.trim().length < 2) {
                      return 'Name must be at least 2 characters';
                    }
                    return null;
                  },
                ),
                const SizedBox(height: 16),
                TextFormField(
                  controller: addressController,
                  decoration: const InputDecoration(
                    labelText: 'LOS Address *',
                    hintText: 'LOS...',
                    prefixIcon: Icon(Icons.account_balance_wallet),
                  ),
                  validator: (value) {
                    if (value == null || value.trim().isEmpty) {
                      return 'Please enter an address';
                    }
                    return AddressValidator.getValidationError(value.trim());
                  },
                ),
                const SizedBox(height: 16),
                TextFormField(
                  controller: notesController,
                  decoration: const InputDecoration(
                    labelText: 'Notes (optional)',
                    hintText: 'Add notes about this address',
                    prefixIcon: Icon(Icons.note),
                  ),
                  maxLines: 2,
                  maxLength: 200,
                ),
              ],
            ),
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(dialogContext),
            child: const Text('CANCEL'),
          ),
          ElevatedButton(
            onPressed: () async {
              if (!formKey.currentState!.validate()) return;

              try {
                losLog(
                    '📒 [AddressBookScreen._showAddDialog] Adding: ${nameController.text.trim()}');
                await _addressBookService.addEntry(
                  name: nameController.text.trim(),
                  address: addressController.text.trim(),
                  notes: notesController.text.trim().isEmpty
                      ? null
                      : notesController.text.trim(),
                );

                if (dialogContext.mounted) {
                  Navigator.pop(dialogContext);
                  ScaffoldMessenger.of(dialogContext).showSnackBar(
                    const SnackBar(
                      content: Text('Address added successfully'),
                      backgroundColor: Colors.green,
                    ),
                  );
                  losLog(
                      '📒 [AddressBookScreen._showAddDialog] Added successfully');
                  _loadEntries();
                }
              } catch (e) {
                losLog('📒 [AddressBookScreen._showAddDialog] ERROR: $e');
                if (dialogContext.mounted) {
                  ScaffoldMessenger.of(dialogContext).showSnackBar(
                    SnackBar(
                      content: Text(e.toString()),
                      backgroundColor: Colors.red,
                    ),
                  );
                }
              }
            },
            child: const Text('ADD'),
          ),
        ],
      ),
    );
  }

  Future<void> _showEditDialog(AddressBookEntry entry) async {
    final nameController = TextEditingController(text: entry.name);
    final addressController = TextEditingController(text: entry.address);
    final notesController = TextEditingController(text: entry.notes ?? '');
    final formKey = GlobalKey<FormState>();

    await showDialog(
      context: context,
      builder: (dialogContext) => AlertDialog(
        title: const Text('Edit Address'),
        content: Form(
          key: formKey,
          child: SingleChildScrollView(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                TextFormField(
                  controller: nameController,
                  decoration: const InputDecoration(
                    labelText: 'Name *',
                    prefixIcon: Icon(Icons.label),
                  ),
                  validator: (value) {
                    if (value == null || value.trim().isEmpty) {
                      return 'Please enter a name';
                    }
                    if (value.trim().length < 2) {
                      return 'Name must be at least 2 characters';
                    }
                    return null;
                  },
                ),
                const SizedBox(height: 16),
                TextFormField(
                  controller: addressController,
                  decoration: const InputDecoration(
                    labelText: 'LOS Address *',
                    prefixIcon: Icon(Icons.account_balance_wallet),
                  ),
                  validator: (value) {
                    if (value == null || value.trim().isEmpty) {
                      return 'Please enter an address';
                    }
                    return AddressValidator.getValidationError(value.trim());
                  },
                ),
                const SizedBox(height: 16),
                TextFormField(
                  controller: notesController,
                  decoration: const InputDecoration(
                    labelText: 'Notes (optional)',
                    prefixIcon: Icon(Icons.note),
                  ),
                  maxLines: 2,
                  maxLength: 200,
                ),
              ],
            ),
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(dialogContext),
            child: const Text('CANCEL'),
          ),
          ElevatedButton(
            onPressed: () async {
              if (!formKey.currentState!.validate()) return;

              try {
                await _addressBookService.updateEntry(
                  entryId: entry.id,
                  name: nameController.text.trim(),
                  address: addressController.text.trim(),
                  notes: notesController.text.trim().isEmpty
                      ? null
                      : notesController.text.trim(),
                );

                if (dialogContext.mounted) {
                  Navigator.pop(dialogContext);
                  ScaffoldMessenger.of(dialogContext).showSnackBar(
                    const SnackBar(
                      content: Text('Address updated successfully'),
                      backgroundColor: Colors.green,
                    ),
                  );
                  losLog(
                      '📒 [AddressBookScreen._showEditDialog] Updated successfully');
                  _loadEntries();
                }
              } catch (e) {
                losLog('📒 [AddressBookScreen._showEditDialog] ERROR: $e');
                if (dialogContext.mounted) {
                  ScaffoldMessenger.of(dialogContext).showSnackBar(
                    SnackBar(
                      content: Text(e.toString()),
                      backgroundColor: Colors.red,
                    ),
                  );
                }
              }
            },
            child: const Text('SAVE'),
          ),
        ],
      ),
    );
  }

  Future<void> _deleteEntry(AddressBookEntry entry) async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Delete Address'),
        content: Text(
          'Are you sure you want to delete "${entry.name}"?\n\nThis action cannot be undone.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('CANCEL'),
          ),
          ElevatedButton(
            onPressed: () => Navigator.pop(context, true),
            style: ElevatedButton.styleFrom(backgroundColor: Colors.red),
            child: const Text('DELETE'),
          ),
        ],
      ),
    );

    if (confirmed == true) {
      try {
        losLog('📒 [AddressBookScreen._deleteEntry] Deleting ${entry.name}...');
        await _addressBookService.deleteEntry(entry.id);
        if (mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            const SnackBar(
              content: Text('Address deleted successfully'),
              backgroundColor: Colors.green,
            ),
          );
          losLog('📒 [AddressBookScreen._deleteEntry] Deleted ${entry.name}');
          _loadEntries();
        }
      } catch (e) {
        if (mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            SnackBar(
              content: Text('Error deleting address: $e'),
              backgroundColor: Colors.red,
            ),
          );
        }
      }
    }
  }

  void _copyAddress(String address) {
    SecureClipboard.copyPublic(address);
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(
        content: Text('Address copied to clipboard'),
        duration: Duration(seconds: 2),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('Address Book'),
        centerTitle: true,
      ),
      floatingActionButton: FloatingActionButton(
        onPressed: _showAddDialog,
        child: const Icon(Icons.add),
      ),
      body: Column(
        children: [
          // Search bar
          Padding(
            padding: const EdgeInsets.all(16),
            child: TextField(
              controller: _searchController,
              decoration: InputDecoration(
                hintText: 'Search by name, address, or notes',
                prefixIcon: const Icon(Icons.search),
                suffixIcon: _searchQuery.isNotEmpty
                    ? IconButton(
                        icon: const Icon(Icons.clear),
                        onPressed: () {
                          _searchController.clear();
                          _filterEntries('');
                        },
                      )
                    : null,
                border: const OutlineInputBorder(),
              ),
              onChanged: _filterEntries,
            ),
          ),

          // Content
          Expanded(
            child: _isLoading
                ? const Center(child: CircularProgressIndicator())
                : _entries.isEmpty
                    ? Center(
                        child: Column(
                          mainAxisAlignment: MainAxisAlignment.center,
                          children: [
                            const Icon(
                              Icons.contact_page,
                              size: 64,
                              color: Colors.grey,
                            ),
                            const SizedBox(height: 16),
                            const Text(
                              'No saved addresses',
                              style: TextStyle(
                                fontSize: 18,
                                fontWeight: FontWeight.bold,
                                color: Colors.grey,
                              ),
                            ),
                            const SizedBox(height: 8),
                            const Text(
                              'Tap + to add your first address',
                              style: TextStyle(color: Colors.grey),
                            ),
                          ],
                        ),
                      )
                    : _filteredEntries.isEmpty
                        ? const Center(
                            child: Column(
                              mainAxisAlignment: MainAxisAlignment.center,
                              children: [
                                Icon(Icons.search_off,
                                    size: 64, color: Colors.grey),
                                SizedBox(height: 16),
                                Text(
                                  'No results found',
                                  style: TextStyle(
                                    fontSize: 18,
                                    color: Colors.grey,
                                  ),
                                ),
                              ],
                            ),
                          )
                        : ListView.builder(
                            itemCount: _filteredEntries.length,
                            padding: const EdgeInsets.only(bottom: 80),
                            itemBuilder: (context, index) {
                              final entry = _filteredEntries[index];
                              return Card(
                                margin: const EdgeInsets.symmetric(
                                  horizontal: 16,
                                  vertical: 4,
                                ),
                                child: ListTile(
                                  leading: CircleAvatar(
                                    child: Text(
                                      entry.name.substring(0, 1).toUpperCase(),
                                      style: const TextStyle(
                                        fontWeight: FontWeight.bold,
                                      ),
                                    ),
                                  ),
                                  title: Text(
                                    entry.name,
                                    style: const TextStyle(
                                      fontWeight: FontWeight.bold,
                                    ),
                                  ),
                                  subtitle: Column(
                                    crossAxisAlignment:
                                        CrossAxisAlignment.start,
                                    children: [
                                      Text(
                                        entry.getShortAddress(),
                                        style: const TextStyle(
                                          fontFamily: 'monospace',
                                          fontSize: 11,
                                        ),
                                      ),
                                      if (entry.notes != null &&
                                          entry.notes!.isNotEmpty) ...[
                                        const SizedBox(height: 4),
                                        Text(
                                          entry.notes!,
                                          style: const TextStyle(
                                            fontSize: 11,
                                            color: Colors.grey,
                                            fontStyle: FontStyle.italic,
                                          ),
                                          maxLines: 1,
                                          overflow: TextOverflow.ellipsis,
                                        ),
                                      ],
                                    ],
                                  ),
                                  trailing: PopupMenuButton<String>(
                                    onSelected: (value) {
                                      switch (value) {
                                        case 'copy':
                                          _copyAddress(entry.address);
                                          break;
                                        case 'edit':
                                          _showEditDialog(entry);
                                          break;
                                        case 'delete':
                                          _deleteEntry(entry);
                                          break;
                                      }
                                    },
                                    itemBuilder: (context) => [
                                      const PopupMenuItem(
                                        value: 'copy',
                                        child: Row(
                                          children: [
                                            Icon(Icons.copy, size: 20),
                                            SizedBox(width: 8),
                                            Text('Copy Address'),
                                          ],
                                        ),
                                      ),
                                      const PopupMenuItem(
                                        value: 'edit',
                                        child: Row(
                                          children: [
                                            Icon(Icons.edit, size: 20),
                                            SizedBox(width: 8),
                                            Text('Edit'),
                                          ],
                                        ),
                                      ),
                                      const PopupMenuItem(
                                        value: 'delete',
                                        child: Row(
                                          children: [
                                            Icon(Icons.delete,
                                                size: 20, color: Colors.red),
                                            SizedBox(width: 8),
                                            Text('Delete',
                                                style: TextStyle(
                                                    color: Colors.red)),
                                          ],
                                        ),
                                      ),
                                    ],
                                  ),
                                  onTap: () {
                                    // Return selected address to caller
                                    Navigator.pop(context, entry.address);
                                  },
                                ),
                              );
                            },
                          ),
          ),
        ],
      ),
    );
  }
}
