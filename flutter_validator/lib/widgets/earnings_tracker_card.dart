// Earnings Tracker Card - Display validator revenue from gas fees
import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import '../models/validator_earnings.dart';
import 'earnings_chart.dart';

class EarningsTrackerCard extends StatelessWidget {
  final ValidatorEarnings earnings;

  const EarningsTrackerCard({super.key, required this.earnings});

  @override
  Widget build(BuildContext context) {
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(20.0),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            // Header
            Row(
              children: [
                const Icon(Icons.account_balance_wallet, color: Colors.green),
                const SizedBox(width: 8),
                const Text(
                  'Validator Earnings',
                  style: TextStyle(fontSize: 20, fontWeight: FontWeight.bold),
                ),
                const Spacer(),
                Container(
                  padding: const EdgeInsets.symmetric(
                    horizontal: 12,
                    vertical: 6,
                  ),
                  decoration: BoxDecoration(
                    color: Colors.green.withValues(alpha: 0.2),
                    borderRadius: BorderRadius.circular(12),
                  ),
                  child: Text(
                    '${earnings.revenueShareDisplay} Network Share',
                    style: const TextStyle(
                      color: Colors.green,
                      fontWeight: FontWeight.bold,
                      fontSize: 12,
                    ),
                  ),
                ),
              ],
            ),
            const SizedBox(height: 24),

            // Total Earnings (Large Display)
            Center(
              child: Column(
                children: [
                  const Text(
                    'Total Earnings',
                    style: TextStyle(
                      fontSize: 14,
                      color: Colors.grey,
                      fontWeight: FontWeight.w500,
                    ),
                  ),
                  const SizedBox(height: 8),
                  Text(
                    '${earnings.totalEarningsDisplay} LOS',
                    style: const TextStyle(
                      fontSize: 32,
                      fontWeight: FontWeight.bold,
                      color: Colors.green,
                    ),
                  ),
                  const SizedBox(height: 4),
                  Text(
                    '${NumberFormat('#,###').format(earnings.totalTransactionsProcessed)} transactions processed',
                    style: const TextStyle(fontSize: 12, color: Colors.grey),
                  ),
                ],
              ),
            ),
            const SizedBox(height: 32),

            // Time Period Breakdown
            Row(
              children: [
                Expanded(
                  child: _buildTimeCard(
                    '24 Hours',
                    earnings.last24HoursDisplay,
                    Icons.today,
                  ),
                ),
                const SizedBox(width: 12),
                Expanded(
                  child: _buildTimeCard(
                    '7 Days',
                    earnings.last7DaysDisplay,
                    Icons.date_range,
                  ),
                ),
                const SizedBox(width: 12),
                Expanded(
                  child: _buildTimeCard(
                    '30 Days',
                    earnings.last30DaysDisplay,
                    Icons.calendar_month,
                  ),
                ),
              ],
            ),
            const SizedBox(height: 24),

            // Earnings Chart
            const Text(
              'Daily Earnings (Last 30 Days)',
              style: TextStyle(fontSize: 16, fontWeight: FontWeight.bold),
            ),
            const SizedBox(height: 16),
            SizedBox(
              height: 200,
              child: EarningsChart(dailyHistory: earnings.dailyHistory),
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildTimeCard(String label, String amountStr, IconData icon) {
    return Container(
      padding: const EdgeInsets.all(16),
      decoration: BoxDecoration(
        color: Colors.grey.shade900,
        borderRadius: BorderRadius.circular(12),
        border: Border.all(color: Colors.grey.shade800),
      ),
      child: Column(
        children: [
          Icon(icon, size: 24, color: Colors.green.shade400),
          const SizedBox(height: 8),
          Text(label, style: const TextStyle(fontSize: 12, color: Colors.grey)),
          const SizedBox(height: 4),
          Text(
            '+$amountStr',
            style: TextStyle(
              fontSize: 16,
              fontWeight: FontWeight.bold,
              color: Colors.green.shade400,
            ),
          ),
          const Text('LOS', style: TextStyle(fontSize: 10, color: Colors.grey)),
        ],
      ),
    );
  }
}
