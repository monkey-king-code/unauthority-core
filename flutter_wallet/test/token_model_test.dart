// Unit tests for USP-01 Token model — fromJson parsing and display helpers.

import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_wallet/models/token.dart';

void main() {
  group('Token.fromJson', () {
    test('parses full JSON', () {
      final t = Token.fromJson({
        'contract_address': 'LOS1abc123def456',
        'name': 'MyToken',
        'symbol': 'MTK',
        'decimals': 8,
        'total_supply': '1000000',
        'is_wrapped': false,
        'wrapped_origin': '',
        'max_supply': '5000000',
        'bridge_operator': '',
        'owner': 'LOSowner123',
      });
      expect(t.contractAddress, 'LOS1abc123def456');
      expect(t.name, 'MyToken');
      expect(t.symbol, 'MTK');
      expect(t.decimals, 8);
      expect(t.totalSupply, '1000000');
      expect(t.isWrapped, false);
      expect(t.maxSupply, '5000000');
      expect(t.owner, 'LOSowner123');
    });

    test('parses wrapped token', () {
      final t = Token.fromJson({
        'contract_address': 'LOS1wrapped',
        'name': 'Wrapped Bitcoin',
        'symbol': 'wBTC',
        'decimals': 8,
        'total_supply': 2100,
        'is_wrapped': true,
        'wrapped_origin': 'BTC',
      });
      expect(t.isWrapped, true);
      expect(t.wrappedOrigin, 'BTC');
      expect(t.displayName, 'wBTC (Wrapped BTC)');
    });

    test('handles "contract" key alias', () {
      final t = Token.fromJson({
        'contract': 'LOS1short',
        'name': 'Alias',
        'symbol': 'ALS',
        'decimals': 11,
        'total_supply': '999',
      });
      expect(t.contractAddress, 'LOS1short');
    });

    test('defaults for missing fields', () {
      final t = Token.fromJson({});
      expect(t.contractAddress, '');
      expect(t.name, '');
      expect(t.symbol, '');
      expect(t.decimals, 11); // fallback
      expect(t.totalSupply, '0');
      expect(t.isWrapped, false);
      expect(t.wrappedOrigin, '');
    });
  });

  group('Token display helpers', () {
    test('shortAddress truncates long address', () {
      final t = Token(
          contractAddress: 'LOS1abcdefghij1234567890xyz',
          name: 'T',
          symbol: 'T',
          decimals: 11,
          totalSupply: '0');
      expect(t.shortAddress.length, lessThan(t.contractAddress.length));
      expect(t.shortAddress, contains('...'));
    });

    test('shortAddress keeps short address as-is', () {
      final t = Token(
          contractAddress: 'LOS1short',
          name: 'T',
          symbol: 'T',
          decimals: 11,
          totalSupply: '0');
      expect(t.shortAddress, 'LOS1short');
    });

    test('displayName for non-wrapped', () {
      final t = Token(
          contractAddress: 'c',
          name: 'Token',
          symbol: 'TKN',
          decimals: 11,
          totalSupply: '0');
      expect(t.displayName, 'TKN');
    });

    test('displayName for wrapped', () {
      final t = Token(
          contractAddress: 'c',
          name: 'Wrapped ETH',
          symbol: 'wETH',
          decimals: 18,
          totalSupply: '0',
          isWrapped: true,
          wrappedOrigin: 'ETH');
      expect(t.displayName, 'wETH (Wrapped ETH)');
    });
  });

  group('TokenBalance.fromJson', () {
    test('parses correctly', () {
      final b = TokenBalance.fromJson({
        'contract': 'LOS1token',
        'holder': 'LOS1holder',
        'balance': '50000',
      });
      expect(b.contractAddress, 'LOS1token');
      expect(b.holder, 'LOS1holder');
      expect(b.balance, '50000');
      expect(b.isZero, false);
    });

    test('isZero for "0" balance', () {
      final b = TokenBalance.fromJson({'balance': '0'});
      expect(b.isZero, true);
    });

    test('isZero for missing balance', () {
      final b = TokenBalance.fromJson({});
      expect(b.isZero, true);
    });
  });

  group('TokenAllowance.fromJson', () {
    test('parses correctly', () {
      final a = TokenAllowance.fromJson({
        'contract': 'LOS1token',
        'owner': 'LOS1owner',
        'spender': 'LOS1spender',
        'allowance': '10000',
      });
      expect(a.contractAddress, 'LOS1token');
      expect(a.owner, 'LOS1owner');
      expect(a.spender, 'LOS1spender');
      expect(a.allowance, '10000');
    });
  });
}
