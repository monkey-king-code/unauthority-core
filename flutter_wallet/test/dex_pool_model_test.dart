// Unit tests for DEX Pool models — fromJson parsing and display helpers.

import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_wallet/models/dex_pool.dart';

void main() {
  group('DexPool.fromJson', () {
    test('parses full pool JSON', () {
      final p = DexPool.fromJson({
        'contract': 'LOS1dex',
        'pool_id': '42',
        'token_a': 'LOS',
        'token_b': 'LOS1mtk',
        'reserve_a': '1000000',
        'reserve_b': '2000000',
        'total_lp': '1414213',
        'fee_bps': 30,
        'creator': 'LOS1admin',
        'last_trade': 1700000000,
        'spot_price_scaled': '2000000000000',
      });
      expect(p.contractAddress, 'LOS1dex');
      expect(p.poolId, '42');
      expect(p.tokenA, 'LOS');
      expect(p.tokenB, 'LOS1mtk');
      expect(p.reserveA, '1000000');
      expect(p.reserveB, '2000000');
      expect(p.totalLp, '1414213');
      expect(p.feeBps, 30);
      expect(p.creator, 'LOS1admin');
      expect(p.lastTrade, 1700000000);
      expect(p.spotPriceScaled, '2000000000000');
    });

    test('uses named contract param', () {
      final p = DexPool.fromJson({
        'pool_id': '0',
        'token_a': 'A',
        'token_b': 'B',
      }, contract: 'LOS1override');
      expect(p.contractAddress, 'LOS1override');
    });

    test('defaults for missing fields', () {
      final p = DexPool.fromJson({});
      expect(p.contractAddress, '');
      expect(p.poolId, '0');
      expect(p.tokenA, '');
      expect(p.tokenB, '');
      expect(p.reserveA, '0');
      expect(p.reserveB, '0');
      expect(p.totalLp, '0');
      expect(p.feeBps, 30); // default fee
      expect(p.creator, '');
      expect(p.lastTrade, 0);
    });
  });

  group('DexPool display helpers', () {
    test('feeDisplay for 30 bps → "0.30%"', () {
      final p = _pool(feeBps: 30);
      expect(p.feeDisplay, '0.30%');
    });

    test('feeDisplay for 100 bps → "1.00%"', () {
      final p = _pool(feeBps: 100);
      expect(p.feeDisplay, '1.00%');
    });

    test('feeDisplay for 5 bps → "0.05%"', () {
      final p = _pool(feeBps: 5);
      expect(p.feeDisplay, '0.05%');
    });

    test('pairLabel with native LOS', () {
      final p = _pool(tokenA: 'LOS', tokenB: 'LOS1mtk123456789');
      expect(p.pairLabel, contains('LOS'));
      expect(p.pairLabel, contains('/'));
    });

    test('pairLabel with short addresses', () {
      final p = _pool(tokenA: 'LOS', tokenB: 'MTK');
      expect(p.pairLabel, 'LOS / MTK');
    });

    test('pairLabel truncates long addresses', () {
      final p =
          _pool(tokenA: 'LOS1abcdefghij1234', tokenB: 'LOS1xyz987654321abc');
      // Should show truncated versions
      expect(p.pairLabel, contains('..'));
    });
  });

  group('DexQuote.fromJson', () {
    test('parses full JSON', () {
      final q = DexQuote.fromJson({
        'amount_out': '990099',
        'fee': 30,
        'price_impact_bps': 200,
      });
      expect(q.amountOut, '990099');
      expect(q.feeBps, 30);
      expect(q.priceImpactBps, 200);
    });

    test('defaults for missing fields', () {
      final q = DexQuote.fromJson({});
      expect(q.amountOut, '0');
      expect(q.feeBps, 0);
      expect(q.priceImpactBps, 0);
    });

    test('priceImpactDisplay for 200 bps → "2.00%"', () {
      final q = DexQuote(amountOut: '1', feeBps: 30, priceImpactBps: 200);
      expect(q.priceImpactDisplay, '2.00%');
    });

    test('priceImpactDisplay for 5 bps → "0.05%"', () {
      final q = DexQuote(amountOut: '1', feeBps: 30, priceImpactBps: 5);
      expect(q.priceImpactDisplay, '0.05%');
    });
  });

  group('LpPosition.fromJson', () {
    test('parses correctly', () {
      final lp = LpPosition.fromJson({
        'contract': 'LOS1dex',
        'pool_id': '1',
        'user': 'LOS1user',
        'lp_shares': '50000',
      });
      expect(lp.contractAddress, 'LOS1dex');
      expect(lp.poolId, '1');
      expect(lp.user, 'LOS1user');
      expect(lp.lpShares, '50000');
    });

    test('defaults for missing fields', () {
      final lp = LpPosition.fromJson({});
      expect(lp.contractAddress, '');
      expect(lp.poolId, '0');
      expect(lp.user, '');
      expect(lp.lpShares, '0');
    });
  });
}

/// Helper to build a DexPool with overridable fields
DexPool _pool({
  String tokenA = 'LOS',
  String tokenB = 'LOS1tokenB',
  int feeBps = 30,
}) {
  return DexPool(
    contractAddress: 'LOS1dex',
    poolId: '0',
    tokenA: tokenA,
    tokenB: tokenB,
    reserveA: '1000',
    reserveB: '2000',
    totalLp: '1414',
    feeBps: feeBps,
    creator: 'LOS1creator',
  );
}
