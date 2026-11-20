// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for English (`en`).
class AppLocalizationsEn extends AppLocalizations {
  AppLocalizationsEn([String locale = 'en']) : super(locale);

  @override
  String get appName => 'r16a-stream';

  @override
  String get homeHeading => 'Welcome';

  @override
  String get homeComingSoon => 'Your library will appear here.';

  @override
  String get signInHeading => 'Sign in to r16a-stream';

  @override
  String get signInLead =>
      'You sign in with your r16a account in the browser and come back here afterwards.';

  @override
  String get signInButton => 'Sign in';

  @override
  String get signInFailed => 'Signing in did not work. Please try again.';

  @override
  String get signInDenied =>
      'Your account does not have access to r16a-stream. Ask an administrator to add you.';

  @override
  String get signInUnavailable =>
      'The server cannot be reached right now. Please try again later.';

  @override
  String get signOut => 'Sign out';
}
