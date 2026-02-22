// Unit tests for Validator BlockchainConstants — including isqrt.
//
// Tests integer-only math and isqrt utility function.
// Note: isqrt is kept for AMM/DEX use, NOT for voting power (which is linear).

import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_validator/constants/blockchain.dart';

void main() {
  group('BlockchainConstants', () {
    test('cilPerLos is 10^11', () {
      expect(BlockchainConstants.cilPerLos, 100000000000);
    });

    test('totalSupply is 21,936,236', () {
      expect(BlockchainConstants.totalSupply, 21936236);
    });
  });

  group('isqrt (Integer Square Root)', () {
    test('isqrt(0) = 0', () {
      expect(BlockchainConstants.isqrt(0), 0);
    });

    test('isqrt(1) = 1', () {
      expect(BlockchainConstants.isqrt(1), 1);
    });

    test('isqrt(4) = 2', () {
      expect(BlockchainConstants.isqrt(4), 2);
    });

    test('isqrt(9) = 3', () {
      expect(BlockchainConstants.isqrt(9), 3);
    });

    test('isqrt(16) = 4', () {
      expect(BlockchainConstants.isqrt(16), 4);
    });

    test('isqrt(100) = 10', () {
      expect(BlockchainConstants.isqrt(100), 10);
    });

    test('isqrt(1000) = 31 (floor)', () {
      // sqrt(1000) ≈ 31.62 → floor = 31
      expect(BlockchainConstants.isqrt(1000), 31);
    });

    test('isqrt(10000) = 100', () {
      expect(BlockchainConstants.isqrt(10000), 100);
    });

    test('isqrt(2) = 1', () {
      // sqrt(2) ≈ 1.41 → floor = 1
      expect(BlockchainConstants.isqrt(2), 1);
    });

    test('isqrt(3) = 1', () {
      expect(BlockchainConstants.isqrt(3), 1);
    });

    test('isqrt(5) = 2', () {
      // sqrt(5) ≈ 2.23 → floor = 2
      expect(BlockchainConstants.isqrt(5), 2);
    });

    test('isqrt(negative) = 0', () {
      expect(BlockchainConstants.isqrt(-1), 0);
      expect(BlockchainConstants.isqrt(-100), 0);
    });

    test('isqrt(1000000) = 1000', () {
      // Perfect square
      expect(BlockchainConstants.isqrt(1000000), 1000);
    });

    test('isqrt result squared is <= n', () {
      // Validate that isqrt(n)^2 <= n < (isqrt(n)+1)^2
      const testValues = [2, 3, 5, 7, 10, 15, 50, 99, 100, 999, 1000, 123456];
      for (final n in testValues) {
        final s = BlockchainConstants.isqrt(n);
        expect(s * s, lessThanOrEqualTo(n),
            reason: 'isqrt($n)=$s but $s²=${s * s} > $n');
        expect((s + 1) * (s + 1), greaterThan(n),
            reason: 'isqrt($n)=$s but (${s + 1})²=${(s + 1) * (s + 1)} <= $n');
      }
    });

    test('isqrt utility: sqrt(1000) for AMM calculations', () {
      // isqrt kept for AMM/DEX use — NOT used for voting (voting is linear)
      expect(BlockchainConstants.isqrt(1000), 31);
    });
  });

  group('cilToLosString (Validator)', () {
    test('zero returns "0.00"', () {
      expect(BlockchainConstants.cilToLosString(0), '0.00');
    });

    test('exact 1 LOS', () {
      expect(BlockchainConstants.cilToLosString(100000000000), '1.00');
    });

    test('fractional 0.5 LOS', () {
      expect(BlockchainConstants.cilToLosString(50000000000), '0.50');
    });

    test('round-trip with losStringToCil', () {
      const cils = [0, 1, 999, 50000000000, 100000000000];
      for (final c in cils) {
        final s = BlockchainConstants.cilToLosString(c);
        final back = BlockchainConstants.losStringToCil(s);
        expect(back, c, reason: 'Round-trip failed for $c → "$s" → $back');
      }
    });
  });

  group('losStringToCil (Validator)', () {
    test('integer "1" → cilPerLos', () {
      expect(BlockchainConstants.losStringToCil('1'),
          BlockchainConstants.cilPerLos);
    });

    test('decimal "0.3" → 30_000_000_000', () {
      expect(BlockchainConstants.losStringToCil('0.3'), 30000000000);
    });

    test('empty string → 0', () {
      expect(BlockchainConstants.losStringToCil(''), 0);
    });
  });

  group('formatCilAsLos (Validator)', () {
    test('zero → "0.00"', () {
      expect(BlockchainConstants.formatCilAsLos(0), '0.00');
    });

    test('1 LOS → "1.00"', () {
      expect(BlockchainConstants.formatCilAsLos(100000000000), '1.00');
    });

    test('negative → "-1.00"', () {
      expect(BlockchainConstants.formatCilAsLos(-100000000000), '-1.00');
    });

    test('maxDecimals=2 truncates', () {
      expect(BlockchainConstants.formatCilAsLos(12340000000, maxDecimals: 2),
          '0.12');
    });
  });
}
