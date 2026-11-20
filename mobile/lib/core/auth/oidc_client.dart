import 'package:dio/dio.dart';
import 'package:flutter_appauth/flutter_appauth.dart';

import '../config.dart';
import 'tokens.dart';

/// The calls to Authentik. An interface so the token logic can be tested without a browser.
abstract interface class OidcClient {
  /// Authorization code flow with PKCE S256, state and nonce in the system browser.
  Future<Tokens> signIn();

  Future<Tokens> refresh(String refreshToken);

  /// RFC 7009 revocation of the refresh token.
  Future<void> revoke(String refreshToken);

  /// Opens Authentik's end-session page so the browser session ends too. The provider's logout
  /// flow redirects back to the app (SPEC 6.1.1).
  Future<void> endSession();
}

const _scopes = ['openid', 'profile', 'offline_access', 'r16a-stream'];

class AppAuthOidcClient implements OidcClient {
  AppAuthOidcClient(this._config, {FlutterAppAuth? appAuth, Dio? http})
    : _appAuth = appAuth ?? const FlutterAppAuth(),
      _http =
          http ??
          Dio(
            BaseOptions(
              connectTimeout: const Duration(seconds: 10),
              receiveTimeout: const Duration(seconds: 10),
              followRedirects: false,
            ),
          );

  final AppConfig _config;
  final FlutterAppAuth _appAuth;
  final Dio _http;

  OidcProfile get _p => _config.profile;

  @override
  Future<Tokens> signIn() async {
    try {
      final r = await _appAuth.authorizeAndExchangeCode(
        AuthorizationTokenRequest(
          _p.clientId,
          _p.redirectUrl,
          discoveryUrl: _config.discoveryUrl,
          scopes: _scopes,
        ),
      );
      return _tokens(r, previousRefresh: null);
    } on FlutterAppAuthUserCancelledException {
      throw const SignInCancelled();
    } on FlutterAppAuthPlatformException {
      throw const AuthUnavailable();
    }
  }

  @override
  Future<Tokens> refresh(String refreshToken) async {
    try {
      final r = await _appAuth.token(
        TokenRequest(
          _p.clientId,
          _p.redirectUrl,
          discoveryUrl: _config.discoveryUrl,
          refreshToken: refreshToken,
          grantType: GrantType.refreshToken,
          scopes: _scopes,
        ),
      );
      return _tokens(r, previousRefresh: refreshToken);
    } on FlutterAppAuthPlatformException catch (e) {
      if (e.platformErrorDetails.error == 'invalid_grant') {
        throw const RefreshRejected();
      }
      throw const AuthUnavailable();
    }
  }

  @override
  Future<void> revoke(String refreshToken) async {
    final discovery = await _http.get<Map<String, dynamic>>(
      _config.discoveryUrl,
    );
    final endpoint = discovery.data?['revocation_endpoint'];
    if (endpoint is! String) return;
    await _http.post<void>(
      endpoint,
      data: {
        'token': refreshToken,
        'token_type_hint': 'refresh_token',
        'client_id': _p.clientId,
      },
      options: Options(contentType: Headers.formUrlEncodedContentType),
    );
  }

  @override
  Future<void> endSession() async {
    // No ID token hint, Authentik 2026.2.1 ignores it, and the app keeps no ID token. The
    // redirect back comes from the provider's own logout flow.
    await _appAuth.endSession(
      EndSessionRequest(
        postLogoutRedirectUrl: _p.logoutRedirectUrl,
        discoveryUrl: _config.discoveryUrl,
      ),
    );
  }

  Tokens _tokens(TokenResponse r, {required String? previousRefresh}) {
    final access = r.accessToken;
    final refresh = r.refreshToken ?? previousRefresh;
    if (access == null || refresh == null) {
      throw const AuthUnavailable();
    }
    return Tokens(
      accessToken: access,
      refreshToken: refresh,
      accessExpiresAt:
          (r.accessTokenExpirationDateTime ??
                  DateTime.now().add(const Duration(minutes: 1)))
              .toUtc(),
    );
  }
}
