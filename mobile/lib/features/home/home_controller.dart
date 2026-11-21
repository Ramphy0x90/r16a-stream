import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../core/api/catalog_api.dart';
import '../../core/api/generated/models/genre.dart';
import '../../core/api/generated/models/title_card.dart';

/// The grid's filters. A change reloads the first page.
class HomeFilters extends Notifier<BrowseFilters> {
  @override
  BrowseFilters build() => const BrowseFilters();

  void set(BrowseFilters next) => state = next;
}

final homeFiltersProvider = NotifierProvider<HomeFilters, BrowseFilters>(
  HomeFilters.new,
);

final genresProvider = FutureProvider<List<Genre>>(
  (ref) => ref.watch(catalogApiProvider).genres(),
);

final languagesProvider = FutureProvider<List<String>>(
  (ref) => ref.watch(catalogApiProvider).languages(),
);

enum LoadStatus { loading, ready, error }

class HomeState {
  const HomeState({
    this.status = LoadStatus.loading,
    this.items = const [],
    this.cursor,
    this.loadingMore = false,
    this.moreFailed = false,
  });

  final LoadStatus status;
  final List<TitleCard> items;

  /// Keyset cursor of the next page, null on the last page.
  final String? cursor;
  final bool loadingMore;

  /// The last next-page request failed. Scrolling or the retry button asks again.
  final bool moreFailed;

  bool get hasMore => cursor != null;

  HomeState copyWith({
    LoadStatus? status,
    List<TitleCard>? items,
    String? Function()? cursor,
    bool? loadingMore,
    bool? moreFailed,
  }) => HomeState(
    status: status ?? this.status,
    items: items ?? this.items,
    cursor: cursor == null ? this.cursor : cursor(),
    loadingMore: loadingMore ?? this.loadingMore,
    moreFailed: moreFailed ?? this.moreFailed,
  );
}

/// Pages through published titles with keyset cursors (SPEC 5.1). Answers that belong to an
/// older filter are dropped, so a fast change of filter never shows mixed results.
class HomeController extends Notifier<HomeState> {
  int _generation = 0;

  @override
  HomeState build() {
    final filters = ref.watch(homeFiltersProvider);
    _load(filters);
    return const HomeState();
  }

  Future<void> _load(BrowseFilters filters) async {
    final generation = ++_generation;
    try {
      final page = await ref.read(catalogApiProvider).browse(filters);
      if (generation != _generation) return;
      state = HomeState(
        status: LoadStatus.ready,
        items: page.items,
        cursor: page.nextCursor,
      );
    } on Object {
      if (generation != _generation) return;
      state = const HomeState(status: LoadStatus.error);
    }
  }

  Future<void> retry() async {
    state = const HomeState();
    await _load(ref.read(homeFiltersProvider));
  }

  Future<void> loadMore() async {
    final cursor = state.cursor;
    if (cursor == null ||
        state.loadingMore ||
        state.status != LoadStatus.ready) {
      return;
    }
    final generation = _generation;
    state = state.copyWith(loadingMore: true, moreFailed: false);
    try {
      final page = await ref
          .read(catalogApiProvider)
          .browse(ref.read(homeFiltersProvider), cursor: cursor);
      if (generation != _generation) return;
      state = state.copyWith(
        items: [...state.items, ...page.items],
        cursor: () => page.nextCursor,
        loadingMore: false,
      );
    } on Object {
      if (generation != _generation) return;
      state = state.copyWith(loadingMore: false, moreFailed: true);
    }
  }
}

final homeControllerProvider = NotifierProvider<HomeController, HomeState>(
  HomeController.new,
);
