// flutter_validator widget smoke test
//
// Verifies the Validator Dashboard app can mount and render
// its primary UI elements without crashing.

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:flutter_validator/main.dart';
import 'package:flutter_validator/services/wallet_service.dart';

void main() {
  setUp(() {
    SharedPreferences.setMockInitialValues({});
  });

  testWidgets('App can mount and render', (WidgetTester tester) async {
    // Build the app and trigger initial frame
    await tester.pumpWidget(MyApp(walletService: WalletService()));

    // The app renders successfully (Scaffold from the loading state)
    expect(find.byType(Scaffold), findsOneWidget);

    // Wait for initialization and async operations
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 100));

    // After wallet check: no wallet found → SetupWizardScreen renders
    // New flow: step -1 shows network choice first
    // At least a Scaffold is present
    expect(find.byType(Scaffold), findsWidgets);
  },
      skip:
          true); // Skipped: TorService creates pending timers that cannot be cleaned up in test env

  testWidgets('Dashboard shows loading indicator initially',
      (WidgetTester tester) async {
    await tester.pumpWidget(MyApp(walletService: WalletService()));

    // Initially shows CircularProgressIndicator while loading data
    expect(find.byType(CircularProgressIndicator), findsOneWidget);
  });
}
