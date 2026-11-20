import 'dart:async';

import 'package:r16a_stream/core/auth/oidc_client.dart';
import 'package:r16a_stream/core/auth/token_store.dart';
import 'package:r16a_stream/core/auth/tokens.dart';

Tokens tokens(String id, {DateTime? expires}) => Tokens(
  accessToken: 'at-$id',
  refreshToken: 'rt-$id',
  accessExpiresAt:
      expires ?? DateTime.now().toUtc().add(const Duration(minutes: 5)),
);

/// In-memory store that records every write.
class MemoryStore implements TokenStore {
  MemoryStore([this.value]);

  Tokens? value;
  final writes = <Tokens>[];
  int clears = 0;

  @override
  Future<Tokens?> read() async => value;

  @override
  Future<void> write(Tokens t) async {
    writes.add(t);
    value = t;
  }

  @override
  Future<void> clear() async {
    clears++;
    value = null;
  }
}

/// Scriptable Authentik. Refreshes wait on [gate] when it is set, so tests can line up
/// concurrent callers.
class FakeOidc implements OidcClient {
  int refreshCalls = 0;
  int revokeCalls = 0;
  int endSessionCalls = 0;
  Object? refreshError;
  Object? revokeError;
  Completer<void>? gate;
  Tokens next = tokens('2');
  Tokens signInResult = tokens('1');
  Object? signInError;

  @override
  Future<Tokens> refresh(String refreshToken) async {
    refreshCalls++;
    await gate?.future;
    final e = refreshError;
    if (e != null) throw e;
    return next;
  }

  @override
  Future<Tokens> signIn() async {
    final e = signInError;
    if (e != null) throw e;
    return signInResult;
  }

  @override
  Future<void> revoke(String refreshToken) async {
    revokeCalls++;
    final e = revokeError;
    if (e != null) throw e;
  }

  @override
  Future<void> endSession() async => endSessionCalls++;
}
