import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/auth/auth_controller.dart';
import '../../l10n/generated/app_localizations.dart';

/// Reads the auth state and hands it to [SignInView].
class SignInScreen extends ConsumerWidget {
  const SignInScreen({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final problem = ref.watch(
      authControllerProvider.select(
        (s) => s is SignedOut ? s.problem : SignInProblem.none,
      ),
    );
    return SignInView(
      problem: problem,
      onSignIn: () => ref.read(authControllerProvider.notifier).signIn(),
    );
  }
}

/// Sign-in page. The button opens Authentik in the system browser, the app never sees a
/// password.
class SignInView extends StatelessWidget {
  const SignInView({super.key, required this.problem, required this.onSignIn});

  final SignInProblem problem;
  final VoidCallback onSignIn;

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context);
    final text = Theme.of(context).textTheme;
    final message = switch (problem) {
      SignInProblem.none => null,
      SignInProblem.failed => l10n.signInFailed,
      SignInProblem.denied => l10n.signInDenied,
      SignInProblem.unavailable => l10n.signInUnavailable,
    };
    return Scaffold(
      body: SafeArea(
        child: Center(
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 420),
            child: Padding(
              padding: const EdgeInsets.all(24),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Text(l10n.signInHeading, style: text.headlineSmall),
                  const SizedBox(height: 12),
                  Text(l10n.signInLead, style: text.bodyMedium),
                  if (message != null) ...[
                    const SizedBox(height: 16),
                    Semantics(
                      liveRegion: true,
                      child: Text(
                        message,
                        style: text.bodyMedium?.copyWith(
                          color: Theme.of(context).colorScheme.error,
                        ),
                      ),
                    ),
                  ],
                  const SizedBox(height: 24),
                  FilledButton(
                    onPressed: onSignIn,
                    style: FilledButton.styleFrom(
                      minimumSize: const Size.fromHeight(48),
                    ),
                    child: Text(l10n.signInButton),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// App bar action that signs out.
class SignOutAction extends ConsumerWidget {
  const SignOutAction({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return IconButton(
      tooltip: AppLocalizations.of(context).signOut,
      icon: const Icon(Icons.logout),
      onPressed: () => ref.read(authControllerProvider.notifier).signOut(),
    );
  }
}
