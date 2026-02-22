import 'package:flutter/material.dart';
import '../constants/blockchain.dart';

/// Type-safe int parser: handles int, double, String, null from JSON.
int _parseIntField(dynamic v, [int fallback = 0]) {
  if (v == null) return fallback;
  if (v is int) return v;
  if (v is double) return v.toInt();
  return int.tryParse(v.toString()) ?? fallback;
}

class Account {
  final String address;
  final int balance; // In CIL (smallest unit)
  final int cilBalance; // Staked/locked CIL
  final List<Transaction> history;

  Account({
    required this.address,
    required this.balance,
    required this.cilBalance,
    required this.history,
  });

  factory Account.fromJson(Map<String, dynamic> json) {
    // Backend returns balance_cil (integer) and balance_cil_str (string)
    // as canonical CIL amounts. balance / balance_los are formatted strings.
    int parsedBalance;
    if (json['balance_cil_str'] != null) {
      // Prefer string variant for JSON precision safety (numbers > 2^53)
      parsedBalance = int.tryParse(json['balance_cil_str'].toString()) ?? 0;
    } else if (json['balance_cil'] != null) {
      final v = json['balance_cil'];
      parsedBalance = v is int ? v : int.tryParse(v.toString()) ?? 0;
    } else if (json['balance'] != null) {
      final val = json['balance'];
      if (val is int) {
        parsedBalance = val;
      } else if (val is String) {
        parsedBalance = BlockchainConstants.losStringToCil(val);
      } else {
        parsedBalance = 0;
      }
    } else {
      parsedBalance = 0;
    }

    return Account(
      address: json['address'] ?? '',
      balance: parsedBalance,
      cilBalance: 0,
      // Backend sends "transactions", not "history"
      history: ((json['transactions'] ?? json['history']) as List?)
              ?.map((tx) => Transaction.fromJson(tx))
              .toList() ??
          [],
    );
  }

  /// Balance formatted as LOS display string (integer-only math).
  String get balanceDisplay => BlockchainConstants.formatCilAsLos(balance);

  /// CIL balance formatted as LOS display string.
  String get cilBalanceDisplay =>
      BlockchainConstants.formatCilAsLos(cilBalance);
}

class Transaction {
  final String txid;
  final String from;
  final String to;
  final int amount; // In CIL
  final int timestamp;
  final String type;

  Transaction({
    required this.txid,
    required this.from,
    required this.to,
    required this.amount,
    required this.timestamp,
    required this.type,
  });

  factory Transaction.fromJson(Map<String, dynamic> json) {
    // Backend sends amount as:
    // - /history, /account: formatted LOS string like "10.00000000000"
    // - /transaction/{hash}, /block/{hash}: raw CIL integer in 'amount_cil'
    // _parseIntField fails on decimal strings → use _parseAmount for correct handling.
    final int parsedAmount;
    if (json.containsKey('amount_cil') && json['amount_cil'] != null) {
      // Raw CIL integer from /transaction or /block endpoint
      parsedAmount = _parseIntField(json['amount_cil']);
    } else {
      parsedAmount = _parseAmount(json['amount']);
    }

    return Transaction(
      txid: (json['txid'] ?? json['hash'] ?? '').toString(),
      from: (json['from'] ?? '').toString(),
      to: (json['to'] ?? json['target'] ?? '').toString(),
      amount: parsedAmount,
      timestamp: _parseIntField(json['timestamp']),
      type: (json['type'] ?? 'transfer').toString(),
    );
  }

  /// Parse amount from backend which may be:
  /// - int (LOS integer from older endpoints)
  /// - String like "10.00000000000" (formatted LOS from /history, /account)
  /// Returns value in CIL for internal consistency.
  static int _parseAmount(dynamic value) {
    if (value == null) return 0;
    if (value is int) {
      // Guard: values > total LOS supply are already in CIL
      if (value > BlockchainConstants.totalSupply) return value;
      return value * BlockchainConstants.cilPerLos;
    }
    if (value is double) {
      return BlockchainConstants.losStringToCil(value.toString());
    }
    if (value is String) {
      // Formatted LOS string like "10.00000000000" or plain integer string
      if (value.contains('.')) {
        return BlockchainConstants.losStringToCil(value);
      }
      // Plain integer string — try as CIL first
      final parsed = int.tryParse(value);
      if (parsed != null) {
        if (parsed > BlockchainConstants.totalSupply) return parsed;
        return parsed * BlockchainConstants.cilPerLos;
      }
    }
    return 0;
  }

  /// Convert amount from CIL to a display string
  String get amountDisplay => BlockchainConstants.formatCilAsLos(amount);
}

class BlockInfo {
  final int height;
  final String hash;
  final int timestamp;
  final int txCount;

  BlockInfo({
    required this.height,
    required this.hash,
    required this.timestamp,
    required this.txCount,
  });

  factory BlockInfo.fromJson(Map<String, dynamic> json) {
    return BlockInfo(
      // FIX C11-05: Type-safe int parsing
      height: _parseIntField(json['height']),
      hash: (json['hash'] ?? '').toString(),
      timestamp: _parseIntField(json['timestamp']),
      // Backend sends "transactions_count"; also accept legacy "tx_count"
      txCount: _parseIntField(json['transactions_count'] ?? json['tx_count']),
    );
  }
}

class ValidatorInfo {
  final String address;
  final int stake; // In LOS (backend already divides by CIL_PER_LOS)
  final bool isActive;
  final bool isGenesis; // Genesis bootstrap validator
  final int uptimePercentage; // Integer percent (0-100), matches backend u64
  final int totalSlashed; // In CIL
  final String status;

  ValidatorInfo({
    required this.address,
    required this.stake,
    required this.isActive,
    this.isGenesis = false,
    this.uptimePercentage = 100,
    this.totalSlashed = 0,
    this.status = 'active',
  });

  factory ValidatorInfo.fromJson(Map<String, dynamic> json) {
    // FIX C11-04: Type-safe int parsing for all numeric fields
    return ValidatorInfo(
      address: (json['address'] ?? '').toString(),
      stake: _parseIntField(json['stake']),
      isActive: json['is_active'] == true ||
          json['is_active'] == 1 ||
          (json['status'] ?? '').toString().toLowerCase() == 'active',
      isGenesis: json['is_genesis'] == true,
      uptimePercentage: _parseIntField(json['uptime_percentage']),
      totalSlashed: _parseIntField(json['total_slashed']),
      status: (json['status'] ?? 'active').toString(),
    );
  }

  /// Backend already sends stake as integer LOS (balance / CIL_PER_LOS).
  /// FIX C-02: Do NOT divide again — value is already in LOS.
  /// Returns display string without f64 conversion.
  String get stakeDisplay => '$stake';

  /// Linear voting power: stake in LOS (1 LOS = 1 vote).
  /// SECURITY FIX C-01: Changed to linear (Sybil-neutral).
  /// Matches backend: calculate_voting_power() returns stake directly.
  int get votingPowerInt {
    if (stake <= 0) return 0;
    return stake;
  }

  /// Voting power as display string.
  String get votingPowerDisplay => votingPowerInt.toString();

  /// Get voting power percentage relative to all validators.
  /// Returns percentage string (e.g. "25.50").
  /// Uses integer arithmetic for power calculation, f64 only for final % display.
  String getVotingPowerPercentageStr(List<ValidatorInfo> allValidators) {
    final totalPower =
        allValidators.fold(0, (int sum, v) => sum + v.votingPowerInt);
    if (totalPower == 0) return '0.00';
    // Integer percentage with 2 decimal places: (power * 10000) ~/ total / 100
    final bps = (votingPowerInt * 10000) ~/ totalPower;
    return '${bps ~/ 100}.${(bps % 100).toString().padLeft(2, '0')}';
  }

  /// Slashed amount in LOS for display (integer-only).
  String get totalSlashedDisplay =>
      BlockchainConstants.formatCilAsLos(totalSlashed);

  /// Uptime status text (integer percent comparison)
  String get uptimeStatus {
    if (uptimePercentage >= 99) return 'Excellent';
    if (uptimePercentage >= 95) return 'Good';
    if (uptimePercentage >= 90) return 'Warning';
    return 'Critical';
  }

  /// Uptime color (integer percent comparison)
  Color get uptimeColor {
    if (uptimePercentage >= 99) return const Color(0xFF4CAF50);
    if (uptimePercentage >= 95) return const Color(0xFF8BC34A);
    if (uptimePercentage >= 90) return const Color(0xFFFF9800);
    return const Color(0xFFF44336);
  }
}
