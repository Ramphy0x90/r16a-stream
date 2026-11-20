import 'dart:convert';
import 'dart:typed_data';

import 'package:dio/dio.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:r16a_stream/core/api/api_client.dart';
import 'package:r16a_stream/core/auth/token_manager.dart';

import '../../fakes.dart';

/// Answers with the queued statuses and records the Authorization header of each call.
class ScriptedAdapter implements HttpClientAdapter {
  ScriptedAdapter(this.statuses);

  final List<int> statuses;
  final seen = <String?>[];

  @override
  Future<ResponseBody> fetch(
    RequestOptions options,
    Stream<Uint8List>? requestStream,
    Future<void>? cancelFuture,
  ) async {
    seen.add(options.headers['Authorization'] as String?);
    final status = statuses.removeAt(0);
    return ResponseBody.fromString(
      jsonEncode({'ok': status == 200}),
      status,
      headers: {
        Headers.contentTypeHeader: [Headers.jsonContentType],
      },
    );
  }

  @override
  void close({bool force = false}) {}
}

void main() {
  test('adds the bearer token, refreshes once on 401 and retries', () async {
    final oidc = FakeOidc();
    final manager = TokenManager(client: oidc, store: MemoryStore(tokens('1')));
    final dio = buildApiClient('http://api.test', manager);
    final adapter = ScriptedAdapter([401, 200]);
    dio.httpClientAdapter = adapter;

    final res = await dio.get<Map<String, dynamic>>('/api/v1/me');
    expect(res.statusCode, 200);
    expect(adapter.seen, ['Bearer at-1', 'Bearer at-2']);
    expect(oidc.refreshCalls, 1);
  });

  test('a second 401 is returned, not retried forever', () async {
    final oidc = FakeOidc();
    final manager = TokenManager(client: oidc, store: MemoryStore(tokens('1')));
    final dio = buildApiClient('http://api.test', manager);
    final adapter = ScriptedAdapter([401, 401]);
    dio.httpClientAdapter = adapter;

    await expectLater(
      dio.get<void>('/api/v1/me'),
      throwsA(
        isA<DioException>().having(
          (e) => e.response?.statusCode,
          'status',
          401,
        ),
      ),
    );
    expect(adapter.seen.length, 2);
    expect(oidc.refreshCalls, 1);
  });

  test('no token, no header', () async {
    final manager = TokenManager(client: FakeOidc(), store: MemoryStore());
    final dio = buildApiClient('http://api.test', manager);
    final adapter = ScriptedAdapter([401]);
    dio.httpClientAdapter = adapter;
    await expectLater(
      dio.get<void>('/api/v1/me'),
      throwsA(isA<DioException>()),
    );
    expect(adapter.seen, [null]);
  });

  test(
    'the token never goes to another origin, such as a presigned media URL',
    () async {
      final manager = TokenManager(
        client: FakeOidc(),
        store: MemoryStore(tokens('1')),
      );
      final dio = buildApiClient('http://api.test', manager);
      final adapter = ScriptedAdapter([200, 200]);
      dio.httpClientAdapter = adapter;
      await dio.get<void>('http://media.test/r16a-stream-media/x.m4s');
      await dio.get<void>('https://api.test/api/v1/me');
      expect(adapter.seen, [
        null,
        null,
      ], reason: 'other host, and same host with another scheme');
    },
  );
}
