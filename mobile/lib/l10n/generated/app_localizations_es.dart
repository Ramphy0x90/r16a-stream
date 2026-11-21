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

  @override
  String get retry => 'Reintentar';

  @override
  String get loading => 'Cargando';

  @override
  String get catalogError => 'Algo salió mal. Inténtalo de nuevo.';

  @override
  String get catalogEmpty => 'Todavía no hay nada aquí.';

  @override
  String get searchHint => 'Buscar títulos';

  @override
  String get filterAll => 'Todos';

  @override
  String get filterGenre => 'Género';

  @override
  String get filterAllGenres => 'Todos los géneros';

  @override
  String get kindMovie => 'Película';

  @override
  String get kindShow => 'Serie';

  @override
  String get sortRecent => 'Añadidos recientemente';

  @override
  String get sortAz => 'De la A a la Z';

  @override
  String get titleMissing => 'Este título no existe o no está disponible.';

  @override
  String get creditsHeading => 'Reparto y equipo';

  @override
  String get episodesHeading => 'Episodios';

  @override
  String get specials => 'Especiales';

  @override
  String get noEpisodes => 'Todavía no hay episodios.';

  @override
  String get roleDirector => 'Dirección';

  @override
  String get roleCreator => 'Creación';

  @override
  String get roleWriter => 'Guion';

  @override
  String get roleCast => 'Reparto';

  @override
  String runtimeMinutes(int minutes) {
    return '$minutes min';
  }

  @override
  String seasonNumber(int number) {
    return 'Temporada $number';
  }

  @override
  String episodeNumber(int number) {
    return 'Episodio $number';
  }

  @override
  String get filterLanguage => 'Idioma';

  @override
  String get filterAllLanguages => 'Todos los idiomas';
}
