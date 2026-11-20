import 'package:material_ui/material_ui.dart';

import 'tokens.g.dart';

/// Material themes built from the generated HSL tokens, the same values as the web.
abstract final class AppTheme {
  static ThemeData dark() => _build(R16aSemantic.dark, Brightness.dark);

  static ThemeData light() => _build(R16aSemantic.light, Brightness.light);

  static ThemeData _build(R16aSemantic t, Brightness brightness) {
    final scheme = ColorScheme(
      brightness: brightness,
      primary: t.brandFill.toColor(),
      onPrimary: t.textOnBrand.toColor(),
      secondary: t.brandText.toColor(),
      onSecondary: t.textOnBrand.toColor(),
      error: t.dangerFill.toColor(),
      onError: t.textOnDanger.toColor(),
      surface: t.surface.toColor(),
      onSurface: t.text.toColor(),
      onSurfaceVariant: t.textMuted.toColor(),
      surfaceContainer: t.surfaceRaised.toColor(),
      outline: t.borderStrong.toColor(),
      outlineVariant: t.border.toColor(),
    );
    return ThemeData(
      colorScheme: scheme,
      scaffoldBackgroundColor: t.surface.toColor(),
      useMaterial3: true,
      materialTapTargetSize: MaterialTapTargetSize.padded,
    );
  }
}
