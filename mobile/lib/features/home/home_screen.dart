import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/api/catalog_api.dart';
import '../../core/api/generated/models/genre.dart';
import '../../core/ui/language_names.dart';
import '../../core/api/generated/models/title_card.dart';
import '../../core/api/generated/models/title_kind.dart';
import '../../core/ui/remote_image.dart';
import '../../core/ui/states.dart';
import '../../l10n/generated/app_localizations.dart';
import '../auth/sign_in_screen.dart';
import 'home_controller.dart';

/// Home grid with search and filters. Reads the controllers and hands values to dumb widgets.
class HomeScreen extends ConsumerWidget {
  const HomeScreen({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final l10n = AppLocalizations.of(context);
    final filters = ref.watch(homeFiltersProvider);
    final genres = ref.watch(genresProvider).value ?? const <Genre>[];
    final languages = ref.watch(languagesProvider).value ?? const <String>[];
    final setFilters = ref.read(homeFiltersProvider.notifier).set;
    return Scaffold(
      appBar: AppBar(
        title: Text(l10n.appName),
        actions: const [SignOutAction()],
      ),
      body: SafeArea(
        child: Column(
          children: [
            FilterBar(
              filters: filters,
              genres: genres,
              languages: languages,
              onChanged: setFilters,
            ),
            const Expanded(child: _Results()),
          ],
        ),
      ),
    );
  }
}

/// The grid area, rebuilt only when the paging state changes.
class _Results extends ConsumerWidget {
  const _Results();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final l10n = AppLocalizations.of(context);
    final state = ref.watch(homeControllerProvider);
    final controller = ref.read(homeControllerProvider.notifier);
    return switch (state.status) {
      LoadStatus.loading => const SkeletonGrid(),
      LoadStatus.error => StateMessage(
        icon: Icons.cloud_off,
        message: l10n.catalogError,
        actionLabel: l10n.retry,
        onAction: controller.retry,
      ),
      LoadStatus.ready when state.items.isEmpty => StateMessage(
        icon: Icons.movie_filter_outlined,
        message: l10n.catalogEmpty,
      ),
      LoadStatus.ready => TitleGrid(
        items: state.items,
        hasMore: state.hasMore,
        loadingMore: state.loadingMore,
        moreFailed: state.moreFailed,
        onNearEnd: controller.loadMore,
        onOpen: (id) => context.go('/title/$id'),
      ),
    };
  }
}

/// Search field and filter chips.
class FilterBar extends StatefulWidget {
  const FilterBar({
    super.key,
    required this.filters,
    required this.genres,
    this.languages = const [],
    required this.onChanged,
  });

  final BrowseFilters filters;
  final List<Genre> genres;

  /// Shown as a filter only when there is a choice.
  final List<String> languages;
  final ValueChanged<BrowseFilters> onChanged;

  @override
  State<FilterBar> createState() => _FilterBarState();
}

class _FilterBarState extends State<FilterBar> {
  late final TextEditingController _search = TextEditingController(
    text: widget.filters.q ?? '',
  );

  @override
  void dispose() {
    _search.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context);
    final f = widget.filters;
    return Padding(
      padding: const EdgeInsets.fromLTRB(12, 8, 12, 4),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          TextField(
            controller: _search,
            maxLength: 200,
            textInputAction: TextInputAction.search,
            decoration: InputDecoration(
              hintText: l10n.searchHint,
              prefixIcon: const Icon(Icons.search),
              counterText: '',
              border: const OutlineInputBorder(),
              isDense: true,
            ),
            onSubmitted: (value) => widget.onChanged(
              f.copyWith(q: () => value.trim().isEmpty ? null : value.trim()),
            ),
          ),
          const SizedBox(height: 8),
          SingleChildScrollView(
            scrollDirection: Axis.horizontal,
            child: Row(
              children: [
                for (final (value, label) in [
                  (null, l10n.filterAll),
                  ('movie', l10n.kindMovie),
                  ('show', l10n.kindShow),
                ])
                  Padding(
                    padding: const EdgeInsets.only(right: 8),
                    child: ChoiceChip(
                      label: Text(label),
                      selected: f.type == value,
                      onSelected: (_) =>
                          widget.onChanged(f.copyWith(type: () => value)),
                    ),
                  ),
                DropdownButton<String?>(
                  value: f.genre,
                  hint: Text(l10n.filterGenre),
                  underline: const SizedBox.shrink(),
                  items: [
                    DropdownMenuItem(
                      value: null,
                      child: Text(l10n.filterAllGenres),
                    ),
                    for (final g in widget.genres)
                      DropdownMenuItem(value: g.slug, child: Text(g.name)),
                  ],
                  onChanged: (slug) =>
                      widget.onChanged(f.copyWith(genre: () => slug)),
                ),
                const SizedBox(width: 8),
                if (widget.languages.length > 1) ...[
                  DropdownButton<String?>(
                    key: const Key('language-filter'),
                    value: widget.languages.contains(f.language)
                        ? f.language
                        : null,
                    hint: Text(l10n.filterLanguage),
                    underline: const SizedBox.shrink(),
                    items: [
                      DropdownMenuItem(
                        value: null,
                        child: Text(l10n.filterAllLanguages),
                      ),
                      for (final tag in widget.languages)
                        DropdownMenuItem(
                          value: tag,
                          child: Text(languageName(tag)),
                        ),
                    ],
                    onChanged: (tag) =>
                        widget.onChanged(f.copyWith(language: () => tag)),
                  ),
                  const SizedBox(width: 8),
                ],
                DropdownButton<String>(
                  value: f.sort,
                  underline: const SizedBox.shrink(),
                  items: [
                    DropdownMenuItem(
                      value: 'recent',
                      child: Text(l10n.sortRecent),
                    ),
                    DropdownMenuItem(value: 'az', child: Text(l10n.sortAz)),
                  ],
                  onChanged: (sort) =>
                      widget.onChanged(f.copyWith(sort: sort ?? 'recent')),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

const _gridDelegate = SliverGridDelegateWithMaxCrossAxisExtent(
  maxCrossAxisExtent: 160,
  childAspectRatio: 0.52,
  mainAxisSpacing: 16,
  crossAxisSpacing: 12,
);

/// The poster grid. Asks for the next page when the user scrolls near its end.
class TitleGrid extends StatelessWidget {
  const TitleGrid({
    super.key,
    required this.items,
    required this.hasMore,
    required this.loadingMore,
    required this.moreFailed,
    required this.onNearEnd,
    required this.onOpen,
  });

  final List<TitleCard> items;
  final bool hasMore;
  final bool loadingMore;
  final bool moreFailed;
  final VoidCallback onNearEnd;
  final ValueChanged<String> onOpen;

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context);
    return NotificationListener<ScrollNotification>(
      onNotification: (n) {
        if (hasMore && !moreFailed && n.metrics.extentAfter < 600) onNearEnd();
        return false;
      },
      child: CustomScrollView(
        slivers: [
          SliverPadding(
            padding: const EdgeInsets.all(12),
            sliver: SliverGrid.builder(
              gridDelegate: _gridDelegate,
              itemCount: items.length,
              itemBuilder: (context, i) =>
                  TitleTile(card: items[i], onTap: onOpen),
            ),
          ),
          if (hasMore)
            SliverToBoxAdapter(
              child: Padding(
                padding: const EdgeInsets.all(16),
                child: Center(
                  child: moreFailed
                      ? OutlinedButton(
                          onPressed: onNearEnd,
                          child: Text(l10n.retry),
                        )
                      : const SizedBox.square(
                          dimension: 28,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        ),
                ),
              ),
            ),
        ],
      ),
    );
  }
}

/// One poster with its name and year.
class TitleTile extends StatelessWidget {
  const TitleTile({super.key, required this.card, required this.onTap});

  final TitleCard card;
  final ValueChanged<String> onTap;

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context);
    final text = Theme.of(context).textTheme;
    final kind = card.kind == TitleKind.movie ? l10n.kindMovie : l10n.kindShow;
    return Semantics(
      button: true,
      label: [
        card.name,
        kind,
        if (card.year != null) '${card.year}',
      ].join(', '),
      excludeSemantics: true,
      child: InkWell(
        onTap: () => onTap(card.id),
        borderRadius: BorderRadius.circular(8),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            AspectRatio(
              aspectRatio: 2 / 3,
              child: ClipRRect(
                borderRadius: BorderRadius.circular(8),
                child: RemoteImage(
                  image: card.poster,
                  fallback: InitialsTile(name: card.name),
                ),
              ),
            ),
            const SizedBox(height: 6),
            Text(
              card.name,
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
              style: text.titleSmall,
            ),
            Text(
              card.year == null ? kind : '$kind · ${card.year}',
              style: text.bodySmall?.copyWith(
                color: Theme.of(context).colorScheme.onSurfaceVariant,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// Grey tiles in the grid's shape while the first page loads.
class SkeletonGrid extends StatelessWidget {
  const SkeletonGrid({super.key});

  @override
  Widget build(BuildContext context) {
    return Semantics(
      label: AppLocalizations.of(context).loading,
      child: GridView.builder(
        padding: const EdgeInsets.all(12),
        physics: const NeverScrollableScrollPhysics(),
        gridDelegate: _gridDelegate,
        itemCount: 12,
        itemBuilder: (context, i) => const Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            AspectRatio(aspectRatio: 2 / 3, child: Skeleton()),
            SizedBox(height: 6),
            SizedBox(height: 14, width: 100, child: Skeleton(radius: 4)),
          ],
        ),
      ),
    );
  }
}
