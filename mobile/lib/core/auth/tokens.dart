/// Tokens of the signed-in user. The refresh token is the long-lived credential, the access
/// token lives five minutes and is sent to the API as a bearer token.
class Tokens {
  const Tokens({
    required this.accessToken,
    required this.refreshToken,
    required this.accessExpiresAt,
  });

  final String accessToken;
  final String refreshToken;
  final DateTime accessExpiresAt;

  /// True when the access token runs out within [margin], so it is refreshed before a call
  /// fails instead of after.
  bool expiresWithin(Duration margin, DateTime now) =>
      !accessExpiresAt.isAfter(now.add(margin));

  @override
  String toString() => 'Tokens([redacted])';
}

/// Authentik refused the refresh token, it was revoked, rotated away or expired. The user has
/// to sign in again.
class RefreshRejected implements Exception {
  const RefreshRejected();
}

/// Authentik could not be reached. Nothing is known about the session, so it is kept.
class AuthUnavailable implements Exception {
  const AuthUnavailable();
}

/// The user closed the browser before finishing the sign-in.
class SignInCancelled implements Exception {
  const SignInCancelled();
}
