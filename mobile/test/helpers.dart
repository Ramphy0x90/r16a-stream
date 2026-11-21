import 'package:flutter_localizations/flutter_localizations.dart'
    show GlobalWidgetsLocalizations;
import 'package:material_ui/material_ui.dart';
import 'package:r16a_stream/core/api/catalog_api.dart';
import 'package:r16a_stream/core/api/generated/models/episode.dart';
import 'package:r16a_stream/core/api/generated/models/genre.dart';
import 'package:r16a_stream/core/api/generated/models/title_card.dart';
import 'package:r16a_stream/core/api/generated/models/title_detail.dart';
import 'package:r16a_stream/core/api/generated/models/title_page.dart';
import 'package:r16a_stream/core/theme/app_theme.dart';
import 'package:r16a_stream/l10n/generated/app_localizations.dart';
import 'package:dio/dio.dart';

/// A widget inside the app's theme and localisations.
Widget wrap(Widget child) => MaterialApp(
  theme: AppTheme.dark(),
  localizationsDelegates: const [
    AppLocalizations.delegate,
    GlobalMaterialLocalizations.delegate,
    GlobalWidgetsLocalizations.delegate,
  ],
  supportedLocales: AppLocalizations.supportedLocales,
  home: child,
);

TitleCard card(int n, {String kind = 'movie'}) => TitleCard.fromJson({
  'id': 'id-$n',
  'kind': kind,
  'name': 'Title $n',
  'year': 2000 + n,
});

/// A scriptable catalog. Each browse call takes the next queued answer.
class FakeCatalog implements CatalogApi {
  final pages = <Future<TitlePage> Function()>[];
  final browseCalls = <(BrowseFilters, String?)>[];
  TitleDetail? detail;
  Object? detailError;
  Map<int, List<Episode>> episodesBySeason = {};

  static TitlePage page(List<TitleCard> items, String? next) =>
      TitlePage(items: items, nextCursor: next);

  @override
  Future<TitlePage> browse(BrowseFilters filters, {String? cursor}) {
    browseCalls.add((filters, cursor));
    return pages.removeAt(0)();
  }

  @override
  Future<TitleDetail> title(String id) async {
    final e = detailError;
    if (e != null) throw e;
    return detail!;
  }

  @override
  Future<List<Episode>> episodes(String id, int season) async =>
      episodesBySeason[season] ?? const [];

  /// Content languages the fake catalog offers.
  List<String> languageList = const ['en', 'it'];

  @override
  Future<List<String>> languages() async => languageList;

  @override
  Future<List<Genre>> genres() async => [
    Genre.fromJson({'id': 1, 'slug': 'drama', 'name': 'Drama'}),
  ];
}

DioException notFound() => DioException(
  requestOptions: RequestOptions(path: '/'),
  response: Response(
    requestOptions: RequestOptions(path: '/'),
    statusCode: 404,
  ),
);
