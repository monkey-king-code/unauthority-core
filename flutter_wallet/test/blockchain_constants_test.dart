// Unit tests for BlockchainConstants — CIL/LOS conversion, formatting.
//
// Tests integer-only math functions used throughout the wallet
// to ensure ZERO f64 precision loss in financial logic.

import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_wallet/constants/blockchain.dart';

void main() {
  group('BlockchainConstants', () {
    test('cilPerLos is 10^11', () {
      expect(BlockchainConstants.cilPerLos, 100000000000);
    });

    test('totalSupply is 21,936,236', () {
      expect(BlockchainConstants.totalSupply, 21936236);
    });

    test('decimalPlaces is 11', () {
      expect(BlockchainConstants.decimalPlaces, 11);
    });

    test('addressPrefix is LOS', () {
      expect(BlockchainConstants.addressPrefix, 'LOS');
    });
  });

  group('cilToLosString', () {
    test('zero returns "0.00"', () {
      expect(BlockchainConstants.cilToLosString(0), '0.00');
    });

    test('exact 1 LOS', () {
      expect(BlockchainConstants.cilToLosString(100000000000), '1.00');
    });

    test('exact 5 LOS', () {
      expect(BlockchainConstants.cilToLosString(500000000000), '5.00');
    });

    test('fractional 0.3 LOS', () {
      // 0.3 LOS = 30_000_000_000 CIL — trailing zeros trimmed, 2 min.
      expect(BlockchainConstants.cilToLosString(30000000000), '0.30');
    });

    test('fractional 0.5 LOS', () {
      expect(BlockchainConstants.cilToLosString(50000000000), '0.50');
    });

    test('1 CIL (smallest unit)', () {
      expect(BlockchainConstants.cilToLosString(1), '0.00000000001');
    });

    test('large amount: 1000 LOS', () {
      expect(BlockchainConstants.cilToLosString(100000000000000), '1000.00');
    });

    test('negative CIL', () {
      expect(BlockchainConstants.cilToLosString(-100000000000), '-1.00');
    });

    test('negative fractional', () {
      expect(BlockchainConstants.cilToLosString(-50000000000), '-0.50');
    });

    test('mixed whole + fraction', () {
      // 1.5 LOS = 150_000_000_000 CIL
      expect(BlockchainConstants.cilToLosString(150000000000), '1.50');
    });

    test('trims trailing zeros but keeps at least 2', () {
      // 10.1 LOS = 1_010_000_000_000 CIL
      expect(BlockchainConstants.cilToLosString(1010000000000), '10.10');
    });
  });

  group('losStringToCil', () {
    test('empty string returns 0', () {
      expect(BlockchainConstants.losStringToCil(''), 0);
    });

    test('whitespace string returns 0', () {
      expect(BlockchainConstants.losStringToCil('   '), 0);
    });

    test('integer 1 → 100_000_000_000', () {
      expect(BlockchainConstants.losStringToCil('1'), 100000000000);
    });

    test('integer 100 → cilPerLos * 100', () {
      expect(BlockchainConstants.losStringToCil('100'),
          100 * BlockchainConstants.cilPerLos);
    });

    test('decimal 0.3 → 30_000_000_000', () {
      expect(BlockchainConstants.losStringToCil('0.3'), 30000000000);
    });

    test('decimal 0.5 → 50_000_000_000', () {
      expect(BlockchainConstants.losStringToCil('0.5'), 50000000000);
    });

    test('decimal 1.5 → 150_000_000_000', () {
      expect(BlockchainConstants.losStringToCil('1.5'), 150000000000);
    });

    test('full precision 0.00000000001 → 1', () {
      expect(BlockchainConstants.losStringToCil('0.00000000001'), 1);
    });

    test('over-precision is truncated', () {
      // 12 decimal places should truncate the 12th
      expect(BlockchainConstants.losStringToCil('0.000000000012'),
          BlockchainConstants.losStringToCil('0.00000000001'));
    });

    test('round-trip: CIL → string → CIL is lossless', () {
      const testValues = [0, 1, 100, 50000000000, 100000000000, 150000000000];
      for (final cil in testValues) {
        final str = BlockchainConstants.cilToLosString(cil);
        final backToCil = BlockchainConstants.losStringToCil(str);
        expect(backToCil, cil, reason: 'Round-trip failed for CIL=$cil ($str)');
      }
    });

    test('handles leading/trailing whitespace', () {
      expect(BlockchainConstants.losStringToCil('  1.5  '), 150000000000);
    });
  });

  group('formatCilAsLos', () {
    test('zero returns "0.00"', () {
      expect(BlockchainConstants.formatCilAsLos(0), '0.00');
    });

    test('exact 1 LOS', () {
      expect(BlockchainConstants.formatCilAsLos(100000000000), '1.00');
    });

    test('fractional 0.5 LOS', () {
      expect(BlockchainConstants.formatCilAsLos(50000000000), '0.50');
    });

    test('maxDecimals truncates', () {
      // 1 CIL = 0.00000000001, with maxDecimals=6 → truncated to 0.000000
      // but since that's all zeros, trimmed to 0.00
      expect(BlockchainConstants.formatCilAsLos(1, maxDecimals: 6), '0.00');
    });

    test('maxDecimals=2 truncates fractional', () {
      // 0.1234... LOS = 12_340_000_000 CIL → maxDecimals=2 → "0.12"
      expect(BlockchainConstants.formatCilAsLos(12340000000, maxDecimals: 2),
          '0.12');
    });

    test('large amount 5000 LOS', () {
      expect(BlockchainConstants.formatCilAsLos(500000000000000), '5000.00');
    });

    test('negative amount', () {
      expect(BlockchainConstants.formatCilAsLos(-100000000000), '-1.00');
    });

    test('keeps at least 2 decimal places even after trim', () {
      expect(BlockchainConstants.formatCilAsLos(100000000000), '1.00');
    });
  });
}
