import 'package:dio/dio.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/api/generated/models/credit.dart';
import '../../core/api/generated/models/credit_role.dart';
import '../../core/api/generated/models/episode.dart';
import '../../core/api/generated/models/season_summary.dart';
import '../../core/api/generated/models/title_detail.dart';
import '../../core/api/generated/models/title_kind.dart';
import '../../core/ui/remote_image.dart';
import '../../core/ui/states.dart';
import '../../l10n/generated/app_localizations.dart';
import 'title_providers.dart';

/// The title page. Shows open on their first regular season, specials stay one tap away.
class TitleScreen extends ConsumerStatefulWidget {
  const TitleScreen({super.key, required this.id});

  final String id;

  @override
  ConsumerState<TitleScreen> createState() => _TitleScreenState();
}

class _TitleScreenState extends ConsumerState<TitleScreen> {
  /// The season the user picked, null until they pick one.
  int? _picked;

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context);
    final detail = ref.watch(titleDetailProvider(widget.id));
    return Scaffold(
      appBar: AppBar(),
      body: detail.when(
        loading: () => const TitleSkeleton(),
        error: (e, _) => e is DioException && e.response?.statusCode == 404
            ? StateMessage(icon: Icons.search_off, message: l10n.titleMissing)
            : StateMessage(
                icon: Icons.cloud_off,
                message: l10n.catalogError,
                actionLabel: l10n.retry,
                onAction: () => ref.invalidate(titleDetailProvider(widget.id)),
              ),
        data: (t) {
          final season = _picked ?? defaultSeason(t.seasons);
          return TitleView(
            title: t,
            season: season,
            onSeason: (n) => setState(() => _picked = n),
            episodes: season == null
                ? null
                : _EpisodesSection(titleId: t.id, season: season),
          );
        },
      ),
    );
  }
}

/// The first season above 0, else the first one.
int? defaultSeason(List<SeasonSummary> seasons) {
  if (seasons.isEmpty) return null;
  return seasons
      .firstWhere((s) => s.number > 0, orElse: () => seasons.first)
      .number;
}

class _EpisodesSection extends ConsumerWidget {
  const _EpisodesSection({required this.titleId, required this.season});

  final String titleId;
  final int season;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final l10n = AppLocalizations.of(context);
    final key = (titleId, season);
    return ref
        .watch(episodesProvider(key))
        .when(
          loading: () => const SizedBox(height: 120, child: Skeleton()),
          error: (_, _) => StateMessage(
            icon: Icons.cloud_off,
            message: l10n.catalogError,
            actionLabel: l10n.retry,
            onAction: () => ref.invalidate(episodesProvider(key)),
          ),
          data: (list) => list.isEmpty
              ? Padding(
                  padding: const EdgeInsets.all(16),
                  child: Text(l10n.noEpisodes),
                )
              : EpisodeList(episodes: list),
        );
  }
}

/// Everything on the title page, from values only.
class TitleView extends StatelessWidget {
  const TitleView({
    super.key,
    required this.title,
    required this.season,
    required this.onSeason,
    required this.episodes,
  });

  final TitleDetail title;
  final int? season;
  final ValueChanged<int> onSeason;
  final Widget? episodes;

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context);
    final text = Theme.of(context).textTheme;
    final muted = Theme.of(context).colorScheme.onSurfaceVariant;
    final t = title;
    final meta = [
      t.kind == TitleKind.movie ? l10n.kindMovie : l10n.kindShow,
      if (t.year != null) '${t.year}',
      if (t.runtimeMinutes != null) l10n.runtimeMinutes(t.runtimeMinutes!),
    ].join(' · ');
    final leads = t.credits.where((c) => c.role != CreditRole.cast).toList();
    final cast = t.credits
        .where((c) => c.role == CreditRole.cast)
        .take(12)
        .toList();
    return CustomScrollView(
      slivers: [
        if (t.backdrop != null)
          SliverToBoxAdapter(
            child: AspectRatio(
              aspectRatio: 16 / 9,
              child: RemoteImage(
                image: t.backdrop,
                fallback: const SizedBox.shrink(),
              ),
            ),
          ),
        SliverPadding(
          padding: const EdgeInsets.all(16),
          sliver: SliverList.list(
            children: [
              Row(
                crossAxisAlignment: CrossAxisAlignment.end,
                children: [
                  SizedBox(
                    width: 96,
                    child: AspectRatio(
                      aspectRatio: 2 / 3,
                      child: ClipRRect(
                        borderRadius: BorderRadius.circular(8),
                        child: RemoteImage(
                          image: t.poster,
                          fallback: InitialsTile(name: t.name),
                          semanticLabel: t.name,
                        ),
                      ),
                    ),
                  ),
                  const SizedBox(width: 16),
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(t.name, style: text.headlineSmall),
                        const SizedBox(height: 4),
                        Text(
                          meta,
                          style: text.bodyMedium?.copyWith(color: muted),
                        ),
                        if (t.originalName != null)
                          Text(
                            t.originalName!,
                            style: text.bodySmall?.copyWith(color: muted),
                          ),
                      ],
                    ),
                  ),
                ],
              ),
              if (t.genres.isNotEmpty) ...[
                const SizedBox(height: 12),
                Wrap(
                  spacing: 8,
                  runSpacing: 8,
                  children: [
                    for (final g in t.genres) Chip(label: Text(g.name)),
                  ],
                ),
              ],
              if (t.overview != null) ...[
                const SizedBox(height: 12),
                Text(t.overview!, style: text.bodyMedium),
              ],
              if (leads.isNotEmpty || cast.isNotEmpty) ...[
                const SizedBox(height: 20),
                Text(l10n.creditsHeading, style: text.titleMedium),
                const SizedBox(height: 8),
                for (final c in leads)
                  _CreditLine(credit: c, label: roleLabel(l10n, c.role)),
                for (final c in cast)
                  _CreditLine(credit: c, label: c.character ?? ''),
              ],
              if (t.seasons.isNotEmpty && season != null) ...[
                const SizedBox(height: 20),
                Text(l10n.episodesHeading, style: text.titleMedium),
                const SizedBox(height: 8),
                SingleChildScrollView(
                  scrollDirection: Axis.horizontal,
                  child: Row(
                    children: [
                      for (final s in t.seasons)
                        Padding(
                          padding: const EdgeInsets.only(right: 8),
                          child: ChoiceChip(
                            label: Text(
                              s.number == 0
                                  ? l10n.specials
                                  : l10n.seasonNumber(s.number),
                            ),
                            selected: s.number == season,
                            onSelected: (_) => onSeason(s.number),
                          ),
                        ),
                    ],
                  ),
                ),
                const SizedBox(height: 8),
                ?episodes,
              ],
            ],
          ),
        ),
      ],
    );
  }
}

String roleLabel(AppLocalizations l10n, CreditRole role) => switch (role) {
  CreditRole.director => l10n.roleDirector,
  CreditRole.creator => l10n.roleCreator,
  CreditRole.writer => l10n.roleWriter,
  CreditRole.cast => l10n.roleCast,
};

class _CreditLine extends StatelessWidget {
  const _CreditLine({required this.credit, required this.label});

  final Credit credit;
  final String label;

  @override
  Widget build(BuildContext context) {
    final muted = Theme.of(context).colorScheme.onSurfaceVariant;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 2),
      child: Row(
        children: [
          Expanded(child: Text(credit.personName)),
          Expanded(
            child: Text(label, style: TextStyle(color: muted)),
          ),
        ],
      ),
    );
  }
}

/// Episodes of one season in order.
class EpisodeList extends StatelessWidget {
  const EpisodeList({super.key, required this.episodes});

  final List<Episode> episodes;

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context);
    final scheme = Theme.of(context).colorScheme;
    return Column(
      children: [
        for (final e in episodes)
          Container(
            margin: const EdgeInsets.only(bottom: 8),
            padding: const EdgeInsets.all(12),
            decoration: BoxDecoration(
              color: scheme.surfaceContainer,
              borderRadius: BorderRadius.circular(8),
            ),
            child: Row(
              children: [
                SizedBox(
                  width: 32,
                  child: Text(
                    '${e.number}',
                    style: TextStyle(color: scheme.onSurfaceVariant),
                  ),
                ),
                Expanded(child: Text(e.name ?? l10n.episodeNumber(e.number))),
                if (e.runtimeMinutes != null)
                  Text(
                    l10n.runtimeMinutes(e.runtimeMinutes!),
                    style: TextStyle(color: scheme.onSurfaceVariant),
                  ),
              ],
            ),
          ),
      ],
    );
  }
}

/// The page's shape while it loads.
class TitleSkeleton extends StatelessWidget {
  const TitleSkeleton({super.key});

  @override
  Widget build(BuildContext context) {
    return Semantics(
      label: AppLocalizations.of(context).loading,
      child: const Padding(
        padding: EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                SizedBox(width: 96, height: 144, child: Skeleton()),
                SizedBox(width: 16),
                Expanded(
                  child: SizedBox(height: 28, child: Skeleton(radius: 4)),
                ),
              ],
            ),
            SizedBox(height: 16),
            SizedBox(height: 80, child: Skeleton()),
          ],
        ),
      ),
    );
  }
}
