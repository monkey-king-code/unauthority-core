// Address Validation Utilities for LOS Blockchain
//
// LOS address formats:
// - Testnet L1 (current): "LOS" + hex(SHA256(seed)[:20]) = 43 chars
// - Mainnet (future): "LOS" + Base58(version_byte + BLAKE2b160(dilithium5_pubkey) + checksum)
//
// This validator accepts both formats for forward compatibility.

import 'dart:typed_data';
import 'package:crypto/crypto.dart' as crypto;
import '../constants/blockchain.dart';

class AddressValidator {
  static const String _prefix = BlockchainConstants.addressPrefix; // "LOS"

  // Testnet L1: LOS + 40 hex chars = 43 total
  static const int testnetLength = 43;

  // Mainnet: LOS + Base58(1 + 20 + 4 bytes) ≈ 37-55 chars
  static const int _minLength = 37;
  static const int _maxLength = 55;

  /// Base58 alphabet (Bitcoin-style, no 0/O/I/l)
  static const String _base58Alphabet =
      '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';

  /// Validate LOS address format
  static bool isValidAddress(String address) {
    return getValidationError(address) == null;
  }

  /// Get validation error message for invalid address
  static String? getValidationError(String address) {
    if (address.isEmpty) {
      return 'Address cannot be empty';
    }

    if (!address.startsWith(_prefix)) {
      return 'Address must start with "$_prefix"';
    }

    if (address.length < _minLength) {
      return 'Address too short (minimum $_minLength characters)';
    }

    if (address.length > _maxLength) {
      return 'Address too long (maximum $_maxLength characters)';
    }

    // Extract address body (after "LOS" prefix)
    final body = address.substring(_prefix.length);

    // Check if it's testnet hex format (40 hex chars)
    if (body.length == 40 && _isHex(body)) {
      return null; // Valid testnet address
    }

    // Check if it's Base58-encoded (mainnet format)
    if (_isBase58(body)) {
      // Verify Base58Check checksum in pure Dart.
      // This catches mistyped addresses even when the native FFI validator
      // is unavailable. Format: Base58(version_byte + payload + checksum_4).
      final checksumError = _verifyBase58Checksum(body);
      if (checksumError != null) {
        return checksumError;
      }
      return null; // Valid Base58 address with valid checksum
    }

    return 'Invalid address encoding (expected hex or Base58)';
  }

  /// Check if string is valid hexadecimal
  static bool _isHex(String s) {
    return RegExp(r'^[0-9a-fA-F]+$').hasMatch(s);
  }

  /// Check if string contains only valid Base58 characters
  /// Base58 alphabet: 123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz
  /// Excludes: 0 (zero), O (capital o), I (capital i), l (lowercase L)
  static bool _isBase58(String s) {
    return RegExp(
            r'^[123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz]+$')
        .hasMatch(s);
  }

  /// Decode Base58 and verify the 4-byte SHA-256d checksum.
  /// Returns null if checksum is valid, or an error message string.
  static String? _verifyBase58Checksum(String base58) {
    // Decode Base58 to bytes
    final decoded = _base58Decode(base58);
    if (decoded == null || decoded.length < 5) {
      return 'Address too short for Base58Check';
    }

    // Last 4 bytes = checksum
    final payload = decoded.sublist(0, decoded.length - 4);
    final checksum = decoded.sublist(decoded.length - 4);

    // SHA-256d(payload) → first 4 bytes should match checksum
    final hash1 = crypto.sha256.convert(payload).bytes;
    final hash2 = crypto.sha256.convert(hash1).bytes;

    for (int i = 0; i < 4; i++) {
      if (hash2[i] != checksum[i]) {
        return 'Invalid address checksum (mistyped address?)';
      }
    }
    return null; // Valid checksum
  }

  /// Decode a Base58 string to bytes (big-endian, leading '1's → 0x00 bytes).
  static Uint8List? _base58Decode(String input) {
    try {
      BigInt value = BigInt.zero;
      for (int i = 0; i < input.length; i++) {
        final charIndex = _base58Alphabet.indexOf(input[i]);
        if (charIndex < 0) return null; // Invalid character
        value = value * BigInt.from(58) + BigInt.from(charIndex);
      }

      // Convert BigInt to bytes
      final hexStr = value.toRadixString(16);
      final padded = hexStr.length.isOdd ? '0$hexStr' : hexStr;
      final bytes = <int>[];
      for (int i = 0; i < padded.length; i += 2) {
        bytes.add(int.parse(padded.substring(i, i + 2), radix: 16));
      }

      // Count leading '1's (= leading 0x00 bytes in Base58)
      int leadingZeros = 0;
      for (int i = 0; i < input.length && input[i] == '1'; i++) {
        leadingZeros++;
      }

      final result = Uint8List(leadingZeros + bytes.length);
      // leadingZeros bytes are already 0 by default
      for (int i = 0; i < bytes.length; i++) {
        result[leadingZeros + i] = bytes[i];
      }
      return result;
    } catch (_) {
      return null;
    }
  }

  /// Check if address contains only valid characters
  static bool hasValidCharacters(String address) {
    if (!address.startsWith(_prefix)) {
      return false;
    }
    final body = address.substring(_prefix.length);
    return _isHex(body) || _isBase58(body);
  }

  /// Format address for display (truncate middle)
  static String formatAddress(String address,
      {int prefixLength = 10, int suffixLength = 8}) {
    if (address.length <= prefixLength + suffixLength) {
      return address;
    }

    return '${address.substring(0, prefixLength)}...${address.substring(address.length - suffixLength)}';
  }
}
