// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for German (`de`).
class AppLocalizationsDe extends AppLocalizations {
  AppLocalizationsDe([String locale = 'de']) : super(locale);

  @override
  String get appName => 'r16a-stream';

  @override
  String get homeHeading => 'Willkommen';

  @override
  String get homeComingSoon => 'Deine Bibliothek erscheint hier.';

  @override
  String get signInHeading => 'Bei r16a-stream anmelden';

  @override
  String get signInLead =>
      'Du meldest dich im Browser mit deinem r16a-Konto an und kommst danach hierher zurück.';

  @override
  String get signInButton => 'Anmelden';

  @override
  String get signInFailed =>
      'Die Anmeldung hat nicht geklappt. Bitte versuche es erneut.';

  @override
  String get signInDenied =>
      'Dein Konto hat keinen Zugang zu r16a-stream. Bitte eine Administratorin oder einen Administrator, dich hinzuzufügen.';

  @override
  String get signInUnavailable =>
      'Der Server ist gerade nicht erreichbar. Bitte versuche es später erneut.';

  @override
  String get signOut => 'Abmelden';
}
