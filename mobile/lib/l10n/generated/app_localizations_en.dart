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

  @override
  String get retry => 'Try again';

  @override
  String get loading => 'Loading';

  @override
  String get catalogError => 'Something went wrong. Please try again.';

  @override
  String get catalogEmpty => 'Nothing here yet.';

  @override
  String get searchHint => 'Search titles';

  @override
  String get filterAll => 'All';

  @override
  String get filterGenre => 'Genre';

  @override
  String get filterAllGenres => 'All genres';

  @override
  String get kindMovie => 'Movie';

  @override
  String get kindShow => 'Show';

  @override
  String get sortRecent => 'Recently added';

  @override
  String get sortAz => 'A to Z';

  @override
  String get titleMissing => 'This title does not exist or is not available.';

  @override
  String get creditsHeading => 'Cast and crew';

  @override
  String get episodesHeading => 'Episodes';

  @override
  String get specials => 'Specials';

  @override
  String get noEpisodes => 'No episodes yet.';

  @override
  String get roleDirector => 'Director';

  @override
  String get roleCreator => 'Creator';

  @override
  String get roleWriter => 'Writer';

  @override
  String get roleCast => 'Cast';

  @override
  String runtimeMinutes(int minutes) {
    return '$minutes min';
  }

  @override
  String seasonNumber(int number) {
    return 'Season $number';
  }

  @override
  String episodeNumber(int number) {
    return 'Episode $number';
  }

  @override
  String get filterLanguage => 'Language';

  @override
  String get filterAllLanguages => 'All languages';
}
