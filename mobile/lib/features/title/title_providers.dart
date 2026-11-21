import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../core/api/catalog_api.dart';
import '../../core/api/generated/models/episode.dart';
import '../../core/api/generated/models/title_detail.dart';

/// One title page, freed when no screen shows it any more.
final titleDetailProvider = FutureProvider.autoDispose
    .family<TitleDetail, String>(
      (ref, id) => ref.watch(catalogApiProvider).title(id),
    );

/// Episodes of one season of one title.
final episodesProvider = FutureProvider.autoDispose
    .family<List<Episode>, (String, int)>(
      (ref, key) => ref.watch(catalogApiProvider).episodes(key.$1, key.$2),
    );
