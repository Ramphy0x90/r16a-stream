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

  @override
  String get retry => 'Erneut versuchen';

  @override
  String get loading => 'Wird geladen';

  @override
  String get catalogError =>
      'Etwas ist schiefgelaufen. Bitte versuche es erneut.';

  @override
  String get catalogEmpty => 'Hier ist noch nichts.';

  @override
  String get searchHint => 'Titel suchen';

  @override
  String get filterAll => 'Alle';

  @override
  String get filterGenre => 'Genre';

  @override
  String get filterAllGenres => 'Alle Genres';

  @override
  String get kindMovie => 'Film';

  @override
  String get kindShow => 'Serie';

  @override
  String get sortRecent => 'Zuletzt hinzugefügt';

  @override
  String get sortAz => 'A bis Z';

  @override
  String get titleMissing =>
      'Diesen Titel gibt es nicht oder er ist nicht verfügbar.';

  @override
  String get creditsHeading => 'Besetzung und Stab';

  @override
  String get episodesHeading => 'Folgen';

  @override
  String get specials => 'Specials';

  @override
  String get noEpisodes => 'Noch keine Folgen.';

  @override
  String get roleDirector => 'Regie';

  @override
  String get roleCreator => 'Idee';

  @override
  String get roleWriter => 'Drehbuch';

  @override
  String get roleCast => 'Besetzung';

  @override
  String runtimeMinutes(int minutes) {
    return '$minutes Min.';
  }

  @override
  String seasonNumber(int number) {
    return 'Staffel $number';
  }

  @override
  String episodeNumber(int number) {
    return 'Folge $number';
  }

  @override
  String get filterLanguage => 'Sprache';

  @override
  String get filterAllLanguages => 'Alle Sprachen';
}
