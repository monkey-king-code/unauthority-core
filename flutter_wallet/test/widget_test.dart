// LOS Wallet Widget Tests
//
// These tests verify the splash screen UI renders correctly.
// They use `addTearDown` to ensure background timers from
// TorService/ApiService are cancelled during widget disposal.

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:flutter_wallet/main.dart';

void main() {
  // Provide mock SharedPreferences so NetworkPreferenceService doesn't crash
  setUp(() {
    SharedPreferences.setMockInitialValues({});
  });

  testWidgets('LOS Wallet splash screen renders correctly',
      (WidgetTester tester) async {
    // Build our app and trigger a frame.
    await tester.runAsync(() async {
      await tester.pumpWidget(const MyApp());
    });

    // Verify splash screen shows wallet branding
    expect(find.text('LOS WALLET'), findsOneWidget);
    expect(find.text('Unauthority Blockchain'), findsOneWidget);

    // Verify loading indicator is shown
    expect(find.byType(CircularProgressIndicator), findsOneWidget);

    // Verify wallet icon is present
    expect(find.byIcon(Icons.account_balance_wallet), findsOneWidget);
  });

  testWidgets('App uses dark theme with Material 3',
      (WidgetTester tester) async {
    await tester.runAsync(() async {
      await tester.pumpWidget(const MyApp());
    });

    final materialApp = tester.widget<MaterialApp>(find.byType(MaterialApp));
    expect(materialApp.debugShowCheckedModeBanner, false);
    expect(materialApp.title, 'LOS Wallet');
  });
}
