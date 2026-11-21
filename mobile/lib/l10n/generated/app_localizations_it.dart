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

  @override
  String get retry => 'Riprova';

  @override
  String get loading => 'Caricamento';

  @override
  String get catalogError => 'Qualcosa è andato storto. Riprova.';

  @override
  String get catalogEmpty => 'Ancora niente qui.';

  @override
  String get searchHint => 'Cerca titoli';

  @override
  String get filterAll => 'Tutti';

  @override
  String get filterGenre => 'Genere';

  @override
  String get filterAllGenres => 'Tutti i generi';

  @override
  String get kindMovie => 'Film';

  @override
  String get kindShow => 'Serie';

  @override
  String get sortRecent => 'Aggiunti di recente';

  @override
  String get sortAz => 'Dalla A alla Z';

  @override
  String get titleMissing => 'Questo titolo non esiste o non è disponibile.';

  @override
  String get creditsHeading => 'Cast e troupe';

  @override
  String get episodesHeading => 'Episodi';

  @override
  String get specials => 'Speciali';

  @override
  String get noEpisodes => 'Ancora nessun episodio.';

  @override
  String get roleDirector => 'Regia';

  @override
  String get roleCreator => 'Ideatore';

  @override
  String get roleWriter => 'Sceneggiatura';

  @override
  String get roleCast => 'Cast';

  @override
  String runtimeMinutes(int minutes) {
    return '$minutes min';
  }

  @override
  String seasonNumber(int number) {
    return 'Stagione $number';
  }

  @override
  String episodeNumber(int number) {
    return 'Episodio $number';
  }

  @override
  String get filterLanguage => 'Lingua';

  @override
  String get filterAllLanguages => 'Tutte le lingue';
}
