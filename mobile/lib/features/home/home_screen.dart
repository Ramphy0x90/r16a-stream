import 'package:material_ui/material_ui.dart';

import '../../l10n/generated/app_localizations.dart';
import '../auth/sign_in_screen.dart';

/// Phase 1 placeholder for the home grid.
class HomeScreen extends StatelessWidget {
  const HomeScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context);
    final text = Theme.of(context).textTheme;
    return Scaffold(
      appBar: AppBar(
        title: Text(l10n.appName),
        actions: const [SignOutAction()],
      ),
      body: SafeArea(
        child: Padding(
          padding: const EdgeInsets.all(16),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(l10n.homeHeading, style: text.headlineSmall),
              const SizedBox(height: 8),
              Text(l10n.homeComingSoon, style: text.bodyMedium),
            ],
          ),
        ),
      ),
    );
  }
}
