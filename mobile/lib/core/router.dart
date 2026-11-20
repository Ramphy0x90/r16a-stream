import 'package:flutter/foundation.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../features/auth/sign_in_screen.dart';
import '../features/home/home_screen.dart';
import 'auth/auth_controller.dart';

/// App routes. Screens navigate with `context.go()` and `context.pop()` only.
///
/// Signed-out users only ever see the sign-in screen. The redirect is a convenience, the API
/// refuses every call without a valid token anyway.
final routerProvider = Provider<GoRouter>((ref) {
  final auth = ValueNotifier<AuthState>(ref.read(authControllerProvider));
  ref.listen(authControllerProvider, (_, next) => auth.value = next);

  final router = GoRouter(
    refreshListenable: auth,
    redirect: (context, state) {
      final atSignIn = state.matchedLocation == '/signin';
      return switch (auth.value) {
        AuthRestoring() => null,
        SignedOut() => atSignIn ? null : '/signin',
        SignedIn() => atSignIn ? '/' : null,
      };
    },
    routes: [
      GoRoute(path: '/', builder: (context, state) => const HomeScreen()),
      GoRoute(
        path: '/signin',
        builder: (context, state) => const SignInScreen(),
      ),
    ],
  );
  ref.onDispose(() {
    router.dispose();
    auth.dispose();
  });
  return router;
});
