import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:r16a_stream/core/api/catalog_api.dart';
import 'package:r16a_stream/core/api/generated/models/episode.dart';
import 'package:r16a_stream/core/api/generated/models/season_summary.dart';
import 'package:r16a_stream/core/api/generated/models/title_detail.dart';
import 'package:r16a_stream/features/title/title_screen.dart';

import '../helpers.dart';

TitleDetail show() => TitleDetail.fromJson({
  'id': 't1',
  'kind': 'show',
  'name': 'The Wire',
  'year': 2002,
  'genres': [
    {'id': 1, 'slug': 'drama', 'name': 'Drama'},
  ],
  'credits': [
    {'personName': 'David Simon', 'role': 'creator'},
    {'personName': 'Dominic West', 'character': 'McNulty', 'role': 'cast'},
  ],
  'seasons': [
    {'number': 0, 'episodeCount': 1},
    {'number': 1, 'episodeCount': 2},
  ],
  'languages': <String>[],
});

Episode ep(int n, String name) => Episode.fromJson({
  'id': 'e$n',
  'number': n,
  'name': name,
  'runtimeMinutes': 58,
  'languages': <String>[],
});

void main() {
  Future<void> pump(WidgetTester tester, FakeCatalog api) async {
    await tester.pumpWidget(
      ProviderScope(
        overrides: [catalogApiProvider.overrideWithValue(api)],
        child: wrap(const TitleScreen(id: 't1')),
      ),
    );
    await tester.pumpAndSettle();
  }

  test('the default season skips the specials', () {
    SeasonSummary s(int n) => SeasonSummary(number: n, episodeCount: 1);
    expect(defaultSeason([s(0), s(1), s(2)]), 1);
    expect(defaultSeason([s(0)]), 0);
    expect(defaultSeason([]), isNull);
  });

  testWidgets('shows the title, credits and the first season', (tester) async {
    final api = FakeCatalog()
      ..detail = show()
      ..episodesBySeason = {
        0: [ep(1, 'Special')],
        1: [ep(1, 'The Target'), ep(2, 'The Detail')],
      };
    await pump(tester, api);
    expect(find.text('The Wire'), findsOneWidget);
    expect(find.text('David Simon'), findsOneWidget);
    expect(find.text('Creator'), findsOneWidget);
    expect(find.text('The Target'), findsOneWidget);
    await tester.tap(find.text('Specials'));
    await tester.pumpAndSettle();
    expect(find.text('Special'), findsOneWidget);
    expect(find.text('The Target'), findsNothing);
  });

  testWidgets('an unknown title says so instead of offering a retry', (
    tester,
  ) async {
    final api = FakeCatalog()..detailError = notFound();
    await pump(tester, api);
    expect(
      find.text('This title does not exist or is not available.'),
      findsOneWidget,
    );
    expect(find.text('Try again'), findsNothing);
  });
}
