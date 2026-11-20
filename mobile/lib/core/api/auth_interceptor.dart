import 'package:dio/dio.dart';

import '../auth/token_manager.dart';

/// Adds the bearer token to API calls. On 401 it refreshes once, behind the token manager's
/// single-flight lock, and repeats the call. A second 401 or a refused refresh ends the
/// session through the token manager (SPEC 6.4).
class AuthInterceptor extends Interceptor {
  AuthInterceptor(this._tokens, this._dio);

  final TokenManager _tokens;
  final Dio _dio;

  static const _retried = 'r16a.retried';

  @override
  Future<void> onRequest(
    RequestOptions options,
    RequestInterceptorHandler handler,
  ) async {
    final token = await _tokens.accessToken();
    if (token != null) {
      options.headers['Authorization'] = 'Bearer $token';
    }
    handler.next(options);
  }

  @override
  Future<void> onError(
    DioException err,
    ErrorInterceptorHandler handler,
  ) async {
    final req = err.requestOptions;
    if (err.response?.statusCode != 401 || req.extra[_retried] == true) {
      return handler.next(err);
    }
    try {
      final fresh = await _tokens.refresh();
      if (fresh == null) return handler.next(err);
      req.extra[_retried] = true;
      req.headers['Authorization'] = 'Bearer ${fresh.accessToken}';
      handler.resolve(await _dio.fetch<dynamic>(req));
    } on DioException catch (e) {
      handler.next(e);
    } on Object {
      handler.next(err);
    }
  }
}
