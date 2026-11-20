import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:r16a_stream/core/auth/token_manager.dart';
import 'package:r16a_stream/core/auth/tokens.dart';

import '../../fakes.dart';

void main() {
  final expired = DateTime.now().toUtc().subtract(const Duration(seconds: 1));

  test('a valid access token is used without refreshing', () async {
    final oidc = FakeOidc();
    final m = TokenManager(client: oidc, store: MemoryStore(tokens('1')));
    expect(await m.accessToken(), 'at-1');
    expect(oidc.refreshCalls, 0);
  });

  test('concurrent callers share one refresh (T-AUTH-14)', () async {
    final oidc = FakeOidc()..gate = Completer<void>();
    final store = MemoryStore(tokens('1', expires: expired));
    final m = TokenManager(client: oidc, store: store);
    final calls = [
      m.accessToken(),
      m.accessToken(),
      m.refresh(),
      m.accessToken(),
    ];
    await Future<void>.delayed(Duration.zero);
    oidc.gate!.complete();
    final results = await Future.wait(calls);
    expect(oidc.refreshCalls, 1);
    expect(results.whereType<String>(), everyElement('at-2'));
    expect(store.value?.refreshToken, 'rt-2', reason: 'rotated token stored');
  });

  test('the rotated refresh token is stored before the new access token is handed out', () async {
    final oidc = FakeOidc();
    final store = MemoryStore(tokens('1', expires: expired));
    final m = TokenManager(client: oidc, store: store);
    final at = await m.accessToken();
    expect(at, 'at-2');
    expect(store.writes.single.refreshToken, 'rt-2');
  });

  test('a refused refresh token ends the session', () async {
    final oidc = FakeOidc()..refreshError = const RefreshRejected();
    final store = MemoryStore(tokens('1', expires: expired));
    final m = TokenManager(client: oidc, store: store);
    final ended = expectLater(m.signedOut, emits(null));
    expect(await m.accessToken(), isNull);
    await ended;
    expect(store.value, isNull);
    expect(await m.hasSession(), isFalse);
  });

  test('an unreachable Authentik keeps the session for later', () async {
    final oidc = FakeOidc()..refreshError = const AuthUnavailable();
    final store = MemoryStore(tokens('1', expires: expired));
    final m = TokenManager(client: oidc, store: store);
    await expectLater(m.accessToken(), throwsA(isA<AuthUnavailable>()));
    expect(store.value?.refreshToken, 'rt-1');
  });

  test('sign-out clears local tokens even when revocation fails', () async {
    final oidc = FakeOidc()..revokeError = Exception('offline');
    final store = MemoryStore(tokens('1'));
    final m = TokenManager(client: oidc, store: store);
    await m.signOut();
    expect(store.value, isNull);
    expect(oidc.revokeCalls, 1);
    expect(oidc.endSessionCalls, 1);
  });

  test('tokens never print their values', () {
    expect(tokens('secret').toString(), isNot(contains('secret')));
  });

  test('signing out during a refresh revokes the newest token and stays signed out', () async {
    final oidc = FakeOidc()..gate = Completer<void>();
    final store = MemoryStore(tokens('1', expires: expired));
    final m = TokenManager(client: oidc, store: store);
    final pending = m.accessToken();
    await Future<void>.delayed(Duration.zero);
    final out = m.signOut();
    oidc.gate!.complete();
    await out;
    await pending.catchError((_) => null);
    expect(
      store.value,
      isNull,
      reason: 'the refresh result is not written after sign-out',
    );
    expect(await m.hasSession(), isFalse);
    expect(
      oidc.revokedTokens,
      contains('rt-2'),
      reason: 'the rotated token is revoked',
    );
  });
}
