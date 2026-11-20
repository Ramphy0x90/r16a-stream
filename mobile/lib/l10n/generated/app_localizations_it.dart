// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for Italian (`it`).
class AppLocalizationsIt extends AppLocalizations {
  AppLocalizationsIt([String locale = 'it']) : super(locale);

  @override
  String get appName => 'r16a-stream';

  @override
  String get homeHeading => 'Benvenuto';

  @override
  String get homeComingSoon => 'La tua libreria apparirà qui.';

  @override
  String get signInHeading => 'Accedi a r16a-stream';

  @override
  String get signInLead =>
      'Accedi con il tuo account r16a nel browser e poi torni qui.';

  @override
  String get signInButton => 'Accedi';

  @override
  String get signInFailed => 'Accesso non riuscito. Riprova.';

  @override
  String get signInDenied =>
      'Il tuo account non ha accesso a r16a-stream. Chiedi a un amministratore di aggiungerti.';

  @override
  String get signInUnavailable =>
      'Il server non è raggiungibile al momento. Riprova più tardi.';

  @override
  String get signOut => 'Esci';
}
