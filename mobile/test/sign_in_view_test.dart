import 'package:flutter_localizations/flutter_localizations.dart'
    show GlobalWidgetsLocalizations;
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';
import 'package:r16a_stream/core/auth/auth_controller.dart';
import 'package:r16a_stream/features/auth/sign_in_screen.dart';
import 'package:r16a_stream/l10n/generated/app_localizations.dart';

Widget _wrap(Widget child) => MaterialApp(
  localizationsDelegates: const [
    AppLocalizations.delegate,
    GlobalMaterialLocalizations.delegate,
    GlobalWidgetsLocalizations.delegate,
  ],
  supportedLocales: AppLocalizations.supportedLocales,
  home: child,
);

void main() {
  testWidgets('explains a denied login and starts a new one on tap', (
    tester,
  ) async {
    var taps = 0;
    await tester.pumpWidget(
      _wrap(SignInView(problem: SignInProblem.denied, onSignIn: () => taps++)),
    );
    expect(find.textContaining('does not have access'), findsOneWidget);
    await tester.tap(find.text('Sign in'));
    expect(taps, 1);
  });

  testWidgets('shows no message on a plain visit', (tester) async {
    await tester.pumpWidget(
      _wrap(SignInView(problem: SignInProblem.none, onSignIn: () {})),
    );
    expect(find.textContaining('did not work'), findsNothing);
  });
}
