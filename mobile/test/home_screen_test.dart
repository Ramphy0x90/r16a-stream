import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:r16a_stream/app.dart';
import 'package:r16a_stream/core/auth/auth_controller.dart';
import 'package:r16a_stream/core/theme/tokens.g.dart';

import 'fakes.dart';

void main() {
  testWidgets('without stored tokens the app opens on the sign-in screen', (
    tester,
  ) async {
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          tokenStoreProvider.overrideWithValue(MemoryStore()),
          oidcClientProvider.overrideWithValue(FakeOidc()),
        ],
        child: const R16aStreamApp(),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('Sign in to r16a-stream'), findsOneWidget);
    expect(find.text('Welcome'), findsNothing);
  });

  test('brand token matches the web value #10b981', () {
    final color = R16aPalette.brand500.toColor();
    expect(color.toARGB32(), 0xFF10B981);
  });

  test('dark theme puts near-black text on the emerald fill', () {
    expect(R16aSemantic.dark.textOnBrand.lightness, lessThan(0.1));
    expect(R16aSemantic.dark.brandFill, R16aPalette.brand500);
  });
}
