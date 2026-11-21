import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter/widgets.dart' show Key;
import 'package:material_ui/material_ui.dart' show TextField;
import 'package:r16a_stream/core/api/catalog_api.dart';
import 'package:r16a_stream/features/home/home_controller.dart';
import 'package:r16a_stream/features/home/home_screen.dart';

import '../helpers.dart';

void main() {
  ProviderContainer container(FakeCatalog api) {
    final c = ProviderContainer(
      overrides: [catalogApiProvider.overrideWithValue(api)],
    );
    addTearDown(c.dispose);
    return c;
  }

  Future<void> settle() => Future<void>.delayed(Duration.zero);

  test('pages with the cursor and stops on the last page', () async {
    final api = FakeCatalog()
      ..pages.add(() async => FakeCatalog.page([card(1), card(2)], 'c1'))
      ..pages.add(() async => FakeCatalog.page([card(3)], null));
    final c = container(api);
    c.read(homeControllerProvider);
    await settle();
    await c.read(homeControllerProvider.notifier).loadMore();
    final s = c.read(homeControllerProvider);
    expect(s.items.map((t) => t.name), ['Title 1', 'Title 2', 'Title 3']);
    expect(s.hasMore, isFalse);
    expect(api.browseCalls.last.$2, 'c1');
    await c.read(homeControllerProvider.notifier).loadMore();
    expect(api.browseCalls.length, 2, reason: 'no request past the last page');
  });

  test('a new filter wins over a slower answer for the old one', () async {
    final slow = Completer<void>();
    final api = FakeCatalog()
      ..pages.add(() async {
        await slow.future;
        return FakeCatalog.page([card(1)], null);
      })
      ..pages.add(() async => FakeCatalog.page([card(9, kind: 'show')], null));
    final c = container(api);
    c.read(homeControllerProvider);
    c.read(homeFiltersProvider.notifier).set(const BrowseFilters(type: 'show'));
    c.read(homeControllerProvider);
    await settle();
    slow.complete();
    await settle();
    expect(c.read(homeControllerProvider).items.single.name, 'Title 9');
  });

  test('an error can be retried', () async {
    final api = FakeCatalog()
      ..pages.add(() async => throw Exception('offline'))
      ..pages.add(() async => FakeCatalog.page([card(1)], null));
    final c = container(api);
    c.read(homeControllerProvider);
    await settle();
    expect(c.read(homeControllerProvider).status, LoadStatus.error);
    await c.read(homeControllerProvider.notifier).retry();
    expect(c.read(homeControllerProvider).items.length, 1);
  });

  Future<void> pumpHome(WidgetTester tester, FakeCatalog api) async {
    await tester.pumpWidget(
      ProviderScope(
        overrides: [catalogApiProvider.overrideWithValue(api)],
        child: wrap(const HomeScreen()),
      ),
    );
  }

  testWidgets('shows skeletons, then the grid', (tester) async {
    final gate = Completer<void>();
    final api = FakeCatalog()
      ..pages.add(() async {
        await gate.future;
        return FakeCatalog.page([card(1), card(2)], null);
      });
    await pumpHome(tester, api);
    await tester.pump();
    expect(find.byType(SkeletonGrid), findsOneWidget);
    gate.complete();
    await tester.pumpAndSettle();
    expect(find.text('Title 1'), findsOneWidget);
    expect(find.byType(TitleTile), findsNWidgets(2));
  });

  testWidgets('empty and error states', (tester) async {
    final api = FakeCatalog()
      ..pages.add(() async => throw Exception('offline'))
      ..pages.add(() async => FakeCatalog.page([], null));
    await pumpHome(tester, api);
    await tester.pumpAndSettle();
    expect(
      find.text('Something went wrong. Please try again.'),
      findsOneWidget,
    );
    await tester.tap(find.text('Try again'));
    await tester.pumpAndSettle();
    expect(find.text('Nothing here yet.'), findsOneWidget);
  });

  testWidgets('search and type filter reload with the new filters', (
    tester,
  ) async {
    final api = FakeCatalog()
      ..pages.add(() async => FakeCatalog.page([card(1)], null))
      ..pages.add(() async => FakeCatalog.page([card(2)], null))
      ..pages.add(() async => FakeCatalog.page([card(3)], null));
    await pumpHome(tester, api);
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), 'wire');
    await tester.testTextInput.receiveAction(TextInputAction.search);
    await tester.pumpAndSettle();
    expect(api.browseCalls.last.$1.q, 'wire');
    await tester.tap(find.text('Show').first);
    await tester.pumpAndSettle();
    expect(api.browseCalls.last.$1.type, 'show');
    expect(api.browseCalls.last.$1.q, 'wire', reason: 'filters combine');
  });

  testWidgets('the language filter lists languages by name and filters', (
    tester,
  ) async {
    final api = FakeCatalog()
      ..pages.add(() async => FakeCatalog.page([card(1)], null))
      ..pages.add(() async => FakeCatalog.page([card(2)], null));
    await pumpHome(tester, api);
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('language-filter')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Italiano').last);
    await tester.pumpAndSettle();
    expect(api.browseCalls.last.$1.language, 'it');
    expect(api.browseCalls.last.$1.toQuery()['language'], 'it');
  });

  testWidgets('one language is no choice, so there is no filter', (
    tester,
  ) async {
    final api = FakeCatalog()
      ..languageList = const ['en']
      ..pages.add(() async => FakeCatalog.page([card(1)], null));
    await pumpHome(tester, api);
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('language-filter')), findsNothing);
  });
}
