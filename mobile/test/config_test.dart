import 'package:flutter_test/flutter_test.dart';
import 'package:r16a_stream/core/config.dart';

void main() {
  AppConfig cfg(String api, String auth) => AppConfig(
    apiBaseUrl: api,
    authentikUrl: auth,
    profile: OidcProfile.release,
  );

  test('release builds refuse clear-text endpoints', () {
    expect(
      () => cfg(
        'http://stream.example',
        'https://auth.example',
      ).validate(release: true),
      throwsStateError,
    );
    expect(
      () => cfg(
        'https://stream.example',
        'http://auth.example',
      ).validate(release: true),
      throwsStateError,
    );
    cfg(
      'https://stream.example',
      'https://auth.example',
    ).validate(release: true);
  });

  test('debug builds may use a local API over http but need both URLs', () {
    cfg(
      'http://10.0.2.2:8081',
      'https://auth.example',
    ).validate(release: false);
    expect(
      () => cfg('', 'https://auth.example').validate(release: false),
      throwsStateError,
    );
  });
}
