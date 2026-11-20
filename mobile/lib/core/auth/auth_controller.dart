import 'dart:async';

import 'package:dio/dio.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/api_client.dart';
import '../api/generated/models/me_response.dart';
import '../config.dart';
import 'oidc_client.dart';
import 'token_manager.dart';
import 'token_store.dart';
import 'tokens.dart';

final appConfigProvider = Provider<AppConfig>(
  (ref) => AppConfig.fromEnvironment(),
);

final tokenStoreProvider = Provider<TokenStore>((ref) => SecureTokenStore());

final oidcClientProvider = Provider<OidcClient>(
  (ref) => AppAuthOidcClient(ref.watch(appConfigProvider)),
);

final tokenManagerProvider = Provider<TokenManager>((ref) {
  final m = TokenManager(
    client: ref.watch(oidcClientProvider),
    store: ref.watch(tokenStoreProvider),
  );
  ref.onDispose(m.dispose);
  return m;
});

final apiClientProvider = Provider<Dio>(
  (ref) => buildApiClient(
    ref.watch(appConfigProvider).apiBaseUrl,
    ref.watch(tokenManagerProvider),
  ),
);

/// The part of `GET /api/v1/me` the shell needs, read through the generated `MeResponse`.
class Me {
  const Me({required this.displayName, required this.isAdmin});

  final String displayName;

  /// Shows admin hints only. The API checks admin rights on every call.
  final bool isAdmin;
}

/// Why the sign-in screen is shown.
enum SignInProblem { none, failed, denied, unavailable }

sealed class AuthState {
  const AuthState();
}

/// Reading stored tokens at start-up.
class AuthRestoring extends AuthState {
  const AuthRestoring();
}

class SignedOut extends AuthState {
  const SignedOut([this.problem = SignInProblem.none]);
  final SignInProblem problem;
}

class SignedIn extends AuthState {
  const SignedIn(this.me);

  /// Null while the API cannot be reached, the session is kept for when it can.
  final Me? me;
}

final authControllerProvider = NotifierProvider<AuthController, AuthState>(
  AuthController.new,
);

class AuthController extends Notifier<AuthState> {
  bool _busy = false;

  TokenManager get _tokens => ref.read(tokenManagerProvider);

  @override
  AuthState build() {
    final sub = _tokens.signedOut.listen((_) => state = const SignedOut());
    ref.onDispose(sub.cancel);
    unawaited(_restore());
    return const AuthRestoring();
  }

  Future<void> _restore() async {
    if (!await _tokens.hasSession()) {
      state = const SignedOut();
      return;
    }
    state = await _loadMe();
  }

  Future<void> signIn() async {
    if (_busy) return;
    _busy = true;
    try {
      await _tokens.signIn();
      state = await _loadMe();
    } on SignInCancelled {
      state = const SignedOut();
    } on Object {
      state = const SignedOut(SignInProblem.failed);
    } finally {
      _busy = false;
    }
  }

  Future<void> signOut() async {
    state = const SignedOut();
    await _tokens.signOut();
  }

  /// Asks the API who this is. A 401 after a fresh token means Authentik let the user in but
  /// the token lacks the r16a-stream access claim, so the session is dropped as denied.
  Future<AuthState> _loadMe() async {
    try {
      final res = await ref
          .read(apiClientProvider)
          .get<Map<String, dynamic>>('/api/v1/me');
      final me = MeResponse.fromJson(res.data ?? const {});
      return SignedIn(Me(displayName: me.displayName, isAdmin: me.isAdmin));
    } on DioException catch (e) {
      if (e.response?.statusCode == 401) {
        await _tokens.signOut();
        return const SignedOut(SignInProblem.denied);
      }
      return const SignedIn(null);
    }
  }
}
