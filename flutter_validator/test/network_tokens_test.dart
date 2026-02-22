// Unit tests for Validator network token/pool models.

import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_validator/models/network_tokens.dart';

void main() {
  group('Token.fromJson', () {
    test('parses full JSON', () {
      final t = Token.fromJson({
        'contract_address': 'LOS1abc',
        'name': 'TestToken',
        'symbol': 'TST',
        'decimals': 11,
        'total_supply': 1000000,
        'is_wrapped': false,
        'owner': 'LOS1owner',
      });
      expect(t.contractAddress, 'LOS1abc');
      expect(t.name, 'TestToken');
      expect(t.symbol, 'TST');
      expect(t.decimals, 11);
      expect(t.totalSupply, '1000000');
      expect(t.isWrapped, false);
      expect(t.owner, 'LOS1owner');
    });

    test('wrapped token', () {
      final t = Token.fromJson({
        'contract_address': 'LOS1wrap',
        'name': 'wBTC',
        'symbol': 'wBTC',
        'decimals': 8,
        'total_supply': 2100,
        'is_wrapped': true,
        'wrapped_origin': 'BTC',
      });
      expect(t.isWrapped, true);
      expect(t.wrappedOrigin, 'BTC');
    });

    test('defaults for empty JSON', () {
      final t = Token.fromJson({});
      expect(t.contractAddress, '');
      expect(t.name, '');
      expect(t.symbol, '');
      expect(t.totalSupply, '0');
    });
  });

  group('Token.shortAddress', () {
    test('truncates long addresses', () {
      final t = Token.fromJson({
        'contract_address': 'LOS1abcdefghij1234567890xyz',
        'name': 'T',
        'symbol': 'T',
      });
      expect(t.shortAddress, contains('...'));
      expect(t.shortAddress.length, lessThan(t.contractAddress.length));
    });

    test('keeps short addresses', () {
      final t = Token.fromJson({
        'contract_address': 'LOS1short',
        'name': 'T',
        'symbol': 'T',
      });
      expect(t.shortAddress, 'LOS1short');
    });
  });

  group('DexPool.fromJson', () {
    test('parses full JSON', () {
      final p = DexPool.fromJson({
        'contract_address': 'LOS1dex',
        'pool_id': '0',
        'token_a': 'LOS',
        'token_b': 'LOS1mtk',
        'reserve_a': 1000000,
        'reserve_b': 2000000,
        'total_lp': 1414213,
        'fee_bps': 30,
      });
      expect(p.contractAddress, 'LOS1dex');
      expect(p.poolId, '0');
      expect(p.tokenA, 'LOS');
      expect(p.tokenB, 'LOS1mtk');
      expect(p.reserveA, '1000000');
      expect(p.reserveB, '2000000');
      expect(p.totalLp, '1414213');
      expect(p.feeBps, 30);
    });

    test('defaults for empty JSON', () {
      final p = DexPool.fromJson({});
      expect(p.reserveA, '0');
      expect(p.reserveB, '0');
      expect(p.feeBps, 0);
    });
  });

  group('DexPool display helpers', () {
    test('feeDisplay for 30 bps', () {
      final p = DexPool(
          contractAddress: 'c',
          poolId: '0',
          tokenA: 'A',
          tokenB: 'B',
          reserveA: '0',
          reserveB: '0',
          totalLp: '0',
          feeBps: 30);
      expect(p.feeDisplay, '0.30%');
    });

    test('pairLabel', () {
      final p = DexPool(
          contractAddress: 'c',
          poolId: '0',
          tokenA: 'LOS',
          tokenB: 'MTK',
          reserveA: '0',
          reserveB: '0',
          totalLp: '0',
          feeBps: 30);
      expect(p.pairLabel, 'LOS / MTK');
    });
  });
}
