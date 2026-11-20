import 'package:flutter_secure_storage/flutter_secure_storage.dart';

import 'tokens.dart';

/// Where tokens survive app restarts. Only the secure store is ever used for them, never
/// shared preferences (SPEC 6.4).
abstract interface class TokenStore {
  Future<Tokens?> read();

  /// Writes the refresh token before the access token. If the app dies in between, the next
  /// start still holds the newest refresh token, which matters because Authentik rotates it on
  /// every use and the previous one no longer works.
  Future<void> write(Tokens tokens);

  Future<void> clear();
}

class SecureTokenStore implements TokenStore {
  SecureTokenStore([FlutterSecureStorage? storage])
    : _storage =
          storage ??
          const FlutterSecureStorage(
            // Readable after the first unlock so a refresh can run in the background, and never
            // migrated to another device or into a backup.
            iOptions: IOSOptions(
              accessibility: KeychainAccessibility.first_unlock_this_device,
            ),
            // Keys live in the Android keystore. Backups are off in the manifest as well.
            aOptions: AndroidOptions(),
          );

  final FlutterSecureStorage _storage;

  static const _refresh = 'r16a.refresh_token';
  static const _access = 'r16a.access_token';
  static const _expiry = 'r16a.access_expires_at';

  @override
  Future<Tokens?> read() async {
    final refresh = await _storage.read(key: _refresh);
    if (refresh == null) return null;
    final expiry = int.tryParse(await _storage.read(key: _expiry) ?? '');
    return Tokens(
      accessToken: await _storage.read(key: _access) ?? '',
      refreshToken: refresh,
      accessExpiresAt: DateTime.fromMillisecondsSinceEpoch(
        expiry ?? 0,
        isUtc: true,
      ),
    );
  }

  @override
  Future<void> write(Tokens tokens) async {
    await _storage.write(key: _refresh, value: tokens.refreshToken);
    await _storage.write(key: _access, value: tokens.accessToken);
    await _storage.write(
      key: _expiry,
      value: tokens.accessExpiresAt.millisecondsSinceEpoch.toString(),
    );
  }

  @override
  Future<void> clear() async {
    await _storage.delete(key: _refresh);
    await _storage.delete(key: _access);
    await _storage.delete(key: _expiry);
  }
}
