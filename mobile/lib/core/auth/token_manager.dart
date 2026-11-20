import 'dart:async';

import 'oidc_client.dart';
import 'token_store.dart';
import 'tokens.dart';

/// Hands out a valid access token and refreshes it single-flight.
///
/// Authentik rotates the refresh token on every use and treats a second use of the old one as
/// reuse, which ends the session (SPEC 6.1 point 4, T-AUTH-14). So every caller that needs a
/// refresh at the same time waits for the one refresh already running.
class TokenManager {
  TokenManager({
    required this._client,
    required this._store,
    DateTime Function()? clock,
  }) : _clock = clock ?? DateTime.now;

  final OidcClient _client;
  final TokenStore _store;
  final DateTime Function() _clock;

  Tokens? _tokens;
  bool _loaded = false;
  Future<Tokens>? _refreshing;

  /// Bumped on every sign-in and sign-out. A refresh that started in an older generation must
  /// not write its result, or a sign-out could be undone by a refresh finishing after it.
  int _generation = 0;

  /// Refresh this long before expiry, so a request does not leave with a token that dies on
  /// the way.
  static const refreshMargin = Duration(seconds: 30);

  final _signedOut = StreamController<void>.broadcast();

  /// Fires when the session ended on its own, because the refresh token was refused.
  Stream<void> get signedOut => _signedOut.stream;

  Future<bool> hasSession() async {
    await _load();
    return _tokens != null;
  }

  /// A valid access token, or null when signed out.
  Future<String?> accessToken() async {
    await _load();
    final t = _tokens;
    if (t == null) return null;
    if (!t.expiresWithin(refreshMargin, _clock().toUtc())) {
      return t.accessToken;
    }
    return (await refresh())?.accessToken;
  }

  /// Refreshes once for all concurrent callers. Returns null and ends the session when
  /// Authentik refuses the refresh token. Rethrows [AuthUnavailable] without ending it.
  Future<Tokens?> refresh() async {
    await _load();
    final current = _tokens;
    if (current == null) return null;
    final running = _refreshing ??= _doRefresh(current.refreshToken)
        .whenComplete(() => _refreshing = null);
    try {
      return await running;
    } on RefreshRejected {
      return null;
    }
  }

  Future<Tokens> _doRefresh(String refreshToken) async {
    final generation = _generation;
    try {
      final next = await _client.refresh(refreshToken);
      if (generation != _generation) {
        // Signed out meanwhile. Revoke what Authentik just issued instead of keeping it.
        unawaited(_revokeQuietly(next.refreshToken));
        throw const RefreshRejected();
      }
      // Stored before anyone uses the new access token, see TokenStore.write.
      await _store.write(next);
      _tokens = next;
      return next;
    } on RefreshRejected {
      if (generation == _generation) {
        await _endLocally();
        _signedOut.add(null);
      }
      rethrow;
    }
  }

  /// Opens the system browser for the Authentik login with PKCE.
  Future<void> signIn() async {
    final t = await _client.signIn();
    _generation++;
    await _store.write(t);
    _tokens = t;
    _loaded = true;
  }

  /// Revokes the refresh token, forgets the tokens and ends the Authentik browser session.
  /// Local state is cleared even when Authentik is unreachable.
  Future<void> signOut() async {
    await _load();
    // A refresh in flight rotates the token. Wait for it so the newest one is revoked.
    final running = _refreshing;
    if (running != null) {
      try {
        await running;
      } on Object {
        // Whatever it ended with, the session ends now.
      }
    }
    _generation++;
    final t = _tokens;
    await _endLocally();
    if (t != null) {
      await _revokeQuietly(t.refreshToken);
    }
    try {
      await _client.endSession();
    } on Object {
      // The user may close the browser tab, the local sign-out already happened.
    }
  }

  Future<void> _revokeQuietly(String refreshToken) async {
    try {
      await _client.revoke(refreshToken);
    } on Object {
      // Best effort. The token is gone from the device either way.
    }
  }

  Future<void> _endLocally() async {
    _tokens = null;
    await _store.clear();
  }

  Future<void> _load() async {
    if (_loaded) return;
    _tokens = await _store.read();
    _loaded = true;
  }

  Future<void> dispose() => _signedOut.close();
}
