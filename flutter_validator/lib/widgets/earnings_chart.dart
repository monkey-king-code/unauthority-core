// Simple Earnings Chart - Line chart showing daily earnings trend
import 'package:flutter/material.dart';
import '../models/validator_earnings.dart';

class EarningsChart extends StatelessWidget {
  final List<DailyEarning> dailyHistory;

  const EarningsChart({super.key, required this.dailyHistory});

  @override
  Widget build(BuildContext context) {
    if (dailyHistory.isEmpty) {
      return const Center(
        child: Text(
          'No earnings data available',
          style: TextStyle(color: Colors.grey),
        ),
      );
    }

    final maxEarnings =
        dailyHistory.map((e) => e.earningsCil).reduce((a, b) => a > b ? a : b);
    final minEarnings =
        dailyHistory.map((e) => e.earningsCil).reduce((a, b) => a < b ? a : b);

    return CustomPaint(
      painter: _EarningsChartPainter(
        dailyHistory: dailyHistory,
        maxEarnings: maxEarnings,
        minEarnings: minEarnings,
      ),
      child: Container(),
    );
  }
}

class _EarningsChartPainter extends CustomPainter {
  final List<DailyEarning> dailyHistory;
  final int maxEarnings;
  final int minEarnings;

  _EarningsChartPainter({
    required this.dailyHistory,
    required this.maxEarnings,
    required this.minEarnings,
  });

  @override
  void paint(Canvas canvas, Size size) {
    if (dailyHistory.isEmpty) return;

    final paint = Paint()
      ..color = Colors.green
      ..strokeWidth = 2
      ..style = PaintingStyle.stroke;

    final fillPaint = Paint()
      ..color = Colors.green.withValues(alpha: 0.2)
      ..style = PaintingStyle.fill;

    final gridPaint = Paint()
      ..color = Colors.grey.withValues(alpha: 0.2)
      ..strokeWidth = 1;

    // Draw grid lines
    for (int i = 0; i <= 4; i++) {
      final y = size.height * i / 4;
      canvas.drawLine(Offset(0, y), Offset(size.width, y), gridPaint);
    }

    // Calculate points
    final points = <Offset>[];

    // Guard: single data point — can't compute step or draw a line
    if (dailyHistory.length < 2) {
      final pointPaint = Paint()
        ..color = Colors.green
        ..style = PaintingStyle.fill;
      canvas.drawCircle(Offset(size.width / 2, size.height / 2), 4, pointPaint);
      return;
    }

    final stepX = size.width / (dailyHistory.length - 1);

    for (int i = 0; i < dailyHistory.length; i++) {
      final earning = dailyHistory[i].earningsCil;
      // Prevent division by zero when all earnings are equal
      final range = maxEarnings - minEarnings;
      final normalizedY = range > 0 ? (earning - minEarnings) / range : 0.5;
      final x = i * stepX;
      final y = size.height - (normalizedY * size.height);
      points.add(Offset(x, y));
    }

    // Draw filled area
    final fillPath = Path();
    fillPath.moveTo(0, size.height);
    for (final point in points) {
      fillPath.lineTo(point.dx, point.dy);
    }
    fillPath.lineTo(size.width, size.height);
    fillPath.close();
    canvas.drawPath(fillPath, fillPaint);

    // Draw line
    final linePath = Path();
    linePath.moveTo(points.first.dx, points.first.dy);
    for (int i = 1; i < points.length; i++) {
      linePath.lineTo(points[i].dx, points[i].dy);
    }
    canvas.drawPath(linePath, paint);

    // Draw points
    final pointPaint = Paint()
      ..color = Colors.green
      ..style = PaintingStyle.fill;

    for (final point in points) {
      canvas.drawCircle(point, 3, pointPaint);
    }
  }

  @override
  bool shouldRepaint(covariant _EarningsChartPainter oldDelegate) =>
      oldDelegate.dailyHistory != dailyHistory ||
      oldDelegate.maxEarnings != maxEarnings ||
      oldDelegate.minEarnings != minEarnings;
}
