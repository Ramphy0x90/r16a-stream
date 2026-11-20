// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for Spanish Castilian (`es`).
class AppLocalizationsEs extends AppLocalizations {
  AppLocalizationsEs([String locale = 'es']) : super(locale);

  @override
  String get appName => 'r16a-stream';

  @override
  String get homeHeading => 'Bienvenido';

  @override
  String get homeComingSoon => 'Tu biblioteca aparecerá aquí.';

  @override
  String get signInHeading => 'Inicia sesión en r16a-stream';

  @override
  String get signInLead =>
      'Inicias sesión con tu cuenta r16a en el navegador y luego vuelves aquí.';

  @override
  String get signInButton => 'Iniciar sesión';

  @override
  String get signInFailed => 'No se pudo iniciar sesión. Inténtalo de nuevo.';

  @override
  String get signInDenied =>
      'Tu cuenta no tiene acceso a r16a-stream. Pide a un administrador que te añada.';

  @override
  String get signInUnavailable =>
      'No se puede acceder al servidor en este momento. Inténtalo más tarde.';

  @override
  String get signOut => 'Cerrar sesión';
}
