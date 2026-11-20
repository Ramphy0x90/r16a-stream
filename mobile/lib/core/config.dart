import 'package:flutter/foundation.dart';

/// One Authentik provider the app can sign in with (SPEC 6.1). Release builds use the
/// production provider, debug builds the dev provider, so a debug build never holds
/// production tokens and the two never catch each other's redirects.
class OidcProfile {
  const OidcProfile({
    required this.slug,
    required this.clientId,
    required this.scheme,
  });

  /// Application slug in Authentik, also the client id.
  final String slug;
  final String clientId;

  /// Custom URL scheme registered for this build on Android and iOS.
  final String scheme;

  String get redirectUrl => '$scheme:/oauth2redirect';
  String get logoutRedirectUrl => '$scheme:/logout';

  static const release = OidcProfile(
    slug: 'r16a-stream-mobile',
    clientId: 'r16a-stream-mobile',
    scheme: 'cloud.r16a.stream',
  );
  static const dev = OidcProfile(
    slug: 'r16a-stream-mobile-dev',
    clientId: 'r16a-stream-mobile-dev',
    scheme: 'cloud.r16a.stream.dev',
  );
}

/// Build-time settings from `--dart-define`. Nothing here is secret, the app is a public
/// client and holds no client secret.
class AppConfig {
  const AppConfig({
    required this.apiBaseUrl,
    required this.authentikUrl,
    required this.profile,
  });

  factory AppConfig.fromEnvironment() {
    const profile = String.fromEnvironment('R16A_OIDC_PROFILE');
    return AppConfig(
      apiBaseUrl: const String.fromEnvironment('R16A_API_BASE_URL'),
      authentikUrl: const String.fromEnvironment('R16A_AUTHENTIK_URL'),
      profile: switch (profile) {
        'release' => OidcProfile.release,
        'dev' => OidcProfile.dev,
        // Must match the redirect scheme the build registered, debug builds register the dev
        // scheme and profile and release builds the production one.
        _ => kDebugMode ? OidcProfile.dev : OidcProfile.release,
      },
    );
  }

  /// Origin of the API, for example `https://stream.r16a.cloud`.
  final String apiBaseUrl;

  /// Origin of Authentik, for example `https://auth.r16a.cloud`.
  final String authentikUrl;
  final OidcProfile profile;

  /// Refuses a release build that would talk to the API or Authentik without TLS, where the
  /// bearer and refresh tokens would cross the network in clear. Debug builds may use plain
  /// HTTP against a local API.
  void validate({bool release = kReleaseMode}) {
    if (apiBaseUrl.isEmpty || authentikUrl.isEmpty) {
      throw StateError('R16A_API_BASE_URL and R16A_AUTHENTIK_URL must be set');
    }
    if (release &&
        !(apiBaseUrl.startsWith('https://') &&
            authentikUrl.startsWith('https://'))) {
      throw StateError('release builds need https for the API and Authentik');
    }
  }

  String get discoveryUrl =>
      '$authentikUrl/application/o/${profile.slug}/.well-known/openid-configuration';
}
