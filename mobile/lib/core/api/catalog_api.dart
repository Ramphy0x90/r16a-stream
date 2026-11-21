import 'package:dio/dio.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../auth/auth_controller.dart';
import 'generated/models/episode.dart';
import 'generated/models/genre.dart';
import 'generated/models/title_detail.dart';
import 'generated/models/title_page.dart';

/// Browse filters, the same set the web keeps in its URL (SPEC 5.1).
class BrowseFilters {
  const BrowseFilters({
    this.type,
    this.genre,
    this.language,
    this.q,
    this.sort = 'recent',
  });

  /// `movie`, `show`, or null for both.
  final String? type;

  /// Genre slug.
  final String? genre;

  /// Content language, a BCP 47 tag from `/api/v1/languages`.
  final String? language;

  /// Search words. When set the server orders by relevance and ignores [sort].
  final String? q;

  /// `recent` or `az`.
  final String sort;

  BrowseFilters copyWith({
    String? Function()? type,
    String? Function()? genre,
    String? Function()? language,
    String? Function()? q,
    String? sort,
  }) => BrowseFilters(
    type: type == null ? this.type : type(),
    genre: genre == null ? this.genre : genre(),
    language: language == null ? this.language : language(),
    q: q == null ? this.q : q(),
    sort: sort ?? this.sort,
  );

  Map<String, String> toQuery() => {
    'type': ?type,
    'genre': ?genre,
    'language': ?language,
    if (q != null && q!.trim().isNotEmpty) 'q': q!.trim(),
    if (sort != 'recent') 'sort': sort,
  };

  @override
  bool operator ==(Object other) =>
      other is BrowseFilters &&
      other.type == type &&
      other.genre == genre &&
      other.language == language &&
      other.q == q &&
      other.sort == sort;

  @override
  int get hashCode => Object.hash(type, genre, language, q, sort);
}

/// Read access to published titles. Every response shape comes from the generated models.
class CatalogApi {
  CatalogApi(this._dio);

  final Dio _dio;

  Future<TitlePage> browse(BrowseFilters filters, {String? cursor}) async {
    final res = await _dio.get<Map<String, Object?>>(
      '/api/v1/titles',
      queryParameters: {...filters.toQuery(), 'cursor': ?cursor},
    );
    return TitlePage.fromJson(res.data ?? const {});
  }

  Future<TitleDetail> title(String id) async {
    final res = await _dio.get<Map<String, Object?>>(
      '/api/v1/titles/${Uri.encodeComponent(id)}',
    );
    return TitleDetail.fromJson(res.data ?? const {});
  }

  Future<List<Episode>> episodes(String id, int season) async {
    final res = await _dio.get<List<Object?>>(
      '/api/v1/titles/${Uri.encodeComponent(id)}/seasons/$season/episodes',
    );
    return (res.data ?? const [])
        .map((e) => Episode.fromJson(e! as Map<String, Object?>))
        .toList();
  }

  /// Content languages at least one published title can be watched in.
  Future<List<String>> languages() async {
    final res = await _dio.get<List<Object?>>('/api/v1/languages');
    return (res.data ?? const []).whereType<String>().toList();
  }

  Future<List<Genre>> genres() async {
    final res = await _dio.get<List<Object?>>('/api/v1/genres');
    return (res.data ?? const [])
        .map((e) => Genre.fromJson(e! as Map<String, Object?>))
        .toList();
  }
}

final catalogApiProvider = Provider<CatalogApi>(
  (ref) => CatalogApi(ref.watch(apiClientProvider)),
);
