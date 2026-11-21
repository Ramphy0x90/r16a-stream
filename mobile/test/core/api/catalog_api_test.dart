import 'dart:convert';
import 'dart:typed_data';

import 'package:dio/dio.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:r16a_stream/core/api/catalog_api.dart';

class JsonAdapter implements HttpClientAdapter {
  JsonAdapter(this.body);
  final Object body;
  Uri? last;

  @override
  Future<ResponseBody> fetch(
    RequestOptions o,
    Stream<Uint8List>? s,
    Future<void>? c,
  ) async {
    last = o.uri;
    return ResponseBody.fromString(
      jsonEncode(body),
      200,
      headers: {
        Headers.contentTypeHeader: [Headers.jsonContentType],
      },
    );
  }

  @override
  void close({bool force = false}) {}
}

void main() {
  test('browse sends only set filters and parses the page', () async {
    final adapter = JsonAdapter({
      'items': [
        {'id': 'a', 'kind': 'show', 'name': 'The Wire', 'year': 2002},
      ],
      'nextCursor': 'c2',
    });
    final dio = Dio(BaseOptions(baseUrl: 'https://api.test'))
      ..httpClientAdapter = adapter;
    final page = await CatalogApi(dio).browse(
      const BrowseFilters(type: 'show', q: '  wire ', sort: 'az'),
      cursor: 'c1',
    );
    expect(adapter.last?.queryParameters, {
      'type': 'show',
      'q': 'wire',
      'sort': 'az',
      'cursor': 'c1',
    });
    expect(page.items.single.name, 'The Wire');
    expect(page.nextCursor, 'c2');

    await CatalogApi(dio).browse(const BrowseFilters());
    expect(
      adapter.last?.queryParameters,
      isEmpty,
      reason: 'defaults are not sent',
    );
  });

  test('filters compare by value, so an unchanged filter does not reload', () {
    expect(
      const BrowseFilters(type: 'movie'),
      const BrowseFilters(type: 'movie'),
    );
    expect(
      const BrowseFilters(type: 'movie').copyWith(type: () => null),
      const BrowseFilters(),
    );
  });
}
