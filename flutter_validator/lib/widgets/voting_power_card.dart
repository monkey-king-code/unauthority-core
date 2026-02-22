import 'package:flutter/material.dart';
import 'dart:math' as math;
import '../models/account.dart';

class VotingPowerCard extends StatelessWidget {
  final ValidatorInfo validatorInfo;
  final List<ValidatorInfo> allValidators;

  const VotingPowerCard({
    super.key,
    required this.validatorInfo,
    required this.allValidators,
  });

  @override
  Widget build(BuildContext context) {
    final votingPowerPctStr = validatorInfo.getVotingPowerPercentageStr(
      allValidators,
    );
    final totalNetworkStake = allValidators.fold(
      0,
      (int sum, v) => sum + v.stake,
    );
    final totalVotingPower = allValidators.fold(
      0,
      (int sum, v) => sum + v.votingPowerInt,
    );

    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16.0),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            // Header
            Row(
              children: [
                Icon(
                  Icons.how_to_vote,
                  size: 24,
                  color: Colors.purple.shade400,
                ),
                const SizedBox(width: 8),
                const Text(
                  'Voting Power',
                  style: TextStyle(fontSize: 18, fontWeight: FontWeight.bold),
                ),
                const Spacer(),
                Container(
                  padding: const EdgeInsets.symmetric(
                    horizontal: 12,
                    vertical: 4,
                  ),
                  decoration: BoxDecoration(
                    color: Colors.purple.withValues(alpha: 0.2),
                    borderRadius: BorderRadius.circular(12),
                  ),
                  child: Text(
                    'Linear',
                    style: TextStyle(
                      color: Colors.purple.shade400,
                      fontWeight: FontWeight.bold,
                      fontSize: 12,
                    ),
                  ),
                ),
              ],
            ),
            const SizedBox(height: 16),
            const Divider(),
            const SizedBox(height: 16),

            // Large Voting Power Display
            Center(
              child: Column(
                children: [
                  Text(
                    validatorInfo.votingPowerDisplay,
                    style: TextStyle(
                      fontSize: 48,
                      fontWeight: FontWeight.bold,
                      color: Colors.purple.shade400,
                    ),
                  ),
                  const SizedBox(height: 8),
                  const Text(
                    'Voting Power',
                    style: TextStyle(fontSize: 14, color: Colors.grey),
                  ),
                  const SizedBox(height: 4),
                  Text(
                    '$votingPowerPctStr% of Network',
                    style: TextStyle(
                      fontSize: 12,
                      color: Colors.purple.shade400,
                      fontWeight: FontWeight.w500,
                    ),
                  ),
                ],
              ),
            ),

            const SizedBox(height: 24),

            // Formula Explanation
            Container(
              padding: const EdgeInsets.all(12),
              decoration: BoxDecoration(
                color: Colors.blue.withValues(alpha: 0.1),
                borderRadius: BorderRadius.circular(8),
                border: Border.all(color: Colors.blue.withValues(alpha: 0.3)),
              ),
              child: Row(
                children: [
                  Icon(
                    Icons.info_outline,
                    color: Colors.blue.shade400,
                    size: 20,
                  ),
                  const SizedBox(width: 8),
                  Expanded(
                    child: RichText(
                      text: TextSpan(
                        style: const TextStyle(
                          fontSize: 12,
                          color: Colors.white70,
                        ),
                        children: [
                          const TextSpan(text: 'Formula: '),
                          TextSpan(
                            text: 'Voting Power = Stake (1 LOS = 1 Vote)',
                            style: TextStyle(
                              fontWeight: FontWeight.bold,
                              color: Colors.blue.shade400,
                            ),
                          ),
                        ],
                      ),
                    ),
                  ),
                ],
              ),
            ),

            const SizedBox(height: 24),

            // Stats Grid
            Row(
              children: [
                Expanded(
                  child: _buildStatCard(
                    context,
                    icon: Icons.account_balance_wallet,
                    iconColor: Colors.green.shade400,
                    label: 'Your Stake',
                    value: '${validatorInfo.stakeDisplay} LOS',
                  ),
                ),
                const SizedBox(width: 12),
                Expanded(
                  child: _buildStatCard(
                    context,
                    icon: Icons.group,
                    iconColor: Colors.blue.shade400,
                    label: 'Network Stake',
                    value: '$totalNetworkStake LOS',
                  ),
                ),
              ],
            ),

            const SizedBox(height: 12),

            Row(
              children: [
                Expanded(
                  child: _buildStatCard(
                    context,
                    icon: Icons.ballot,
                    iconColor: Colors.purple.shade400,
                    label: 'Your Power',
                    value: validatorInfo.votingPowerDisplay,
                  ),
                ),
                const SizedBox(width: 12),
                Expanded(
                  child: _buildStatCard(
                    context,
                    icon: Icons.poll,
                    iconColor: Colors.orange.shade400,
                    label: 'Total Power',
                    value: '$totalVotingPower',
                  ),
                ),
              ],
            ),

            const SizedBox(height: 24),

            // Voting Power Bar
            Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Row(
                  mainAxisAlignment: MainAxisAlignment.spaceBetween,
                  children: [
                    const Text(
                      'Network Share',
                      style: TextStyle(fontSize: 12, color: Colors.grey),
                    ),
                    Text(
                      '$votingPowerPctStr%',
                      style: const TextStyle(
                        fontSize: 12,
                        fontWeight: FontWeight.bold,
                      ),
                    ),
                  ],
                ),
                const SizedBox(height: 8),
                ClipRRect(
                  borderRadius: BorderRadius.circular(4),
                  child: LinearProgressIndicator(
                    // For progress bar we need a 0.0-1.0 double — this is UI rendering, not financial logic
                    value: totalVotingPower > 0
                        ? math.min(
                            validatorInfo.votingPowerInt / totalVotingPower,
                            1.0)
                        : 0.0,
                    minHeight: 8,
                    backgroundColor: Colors.grey.shade800,
                    valueColor: AlwaysStoppedAnimation<Color>(
                      Colors.purple.shade400,
                    ),
                  ),
                ),
              ],
            ),

            const SizedBox(height: 16),

            // Linear Voting Info
            Container(
              padding: const EdgeInsets.all(12),
              decoration: BoxDecoration(
                color: Colors.green.withValues(alpha: 0.1),
                borderRadius: BorderRadius.circular(8),
                border: Border.all(color: Colors.green.withValues(alpha: 0.3)),
              ),
              child: Row(
                children: [
                  Icon(Icons.shield, color: Colors.green.shade400, size: 20),
                  const SizedBox(width: 8),
                  Expanded(
                    child: Text(
                      'LOS uses linear voting (1 LOS = 1 vote). This is Sybil-neutral: splitting stake into multiple identities yields the same total power.',
                      style: TextStyle(
                        fontSize: 11,
                        color: Colors.green.shade400,
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildStatCard(
    BuildContext context, {
    required IconData icon,
    required Color iconColor,
    required String label,
    required String value,
  }) {
    return Container(
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(
        color: Colors.grey.shade900,
        borderRadius: BorderRadius.circular(8),
      ),
      child: Column(
        children: [
          Icon(icon, color: iconColor, size: 24),
          const SizedBox(height: 8),
          Text(label, style: const TextStyle(fontSize: 12, color: Colors.grey)),
          const SizedBox(height: 4),
          Text(
            value,
            style: const TextStyle(fontSize: 13, fontWeight: FontWeight.bold),
            overflow: TextOverflow.ellipsis,
            textAlign: TextAlign.center,
          ),
        ],
      ),
    );
  }
}
