import 'package:dio/dio.dart';

import '../auth/token_manager.dart';

/// Adds the bearer token to API calls. On 401 it refreshes once, behind the token manager's
/// single-flight lock, and repeats the call. A second 401 or a refused refresh ends the
/// session through the token manager (SPEC 6.4).
class AuthInterceptor extends Interceptor {
  AuthInterceptor(this._tokens, this._dio, String apiBaseUrl)
    : _apiOrigin = Uri.parse(apiBaseUrl).origin;

  final TokenManager _tokens;
  final Dio _dio;

  /// The token goes only to the API itself. Presigned media and image URLs point at the bucket
  /// paths or other hosts, and `Authorization` must never reach Garage (SPEC 10.3).
  final String _apiOrigin;

  bool _isApi(RequestOptions o) =>
      o.uri.hasScheme && o.uri.origin == _apiOrigin;

  static const _retried = 'r16a.retried';

  @override
  Future<void> onRequest(
    RequestOptions options,
    RequestInterceptorHandler handler,
  ) async {
    final token = _isApi(options) ? await _tokens.accessToken() : null;
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
    if (err.response?.statusCode != 401 ||
        req.extra[_retried] == true ||
        !_isApi(req)) {
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
