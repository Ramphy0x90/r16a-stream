import 'package:dio/dio.dart';

import '../auth/token_manager.dart';
import 'auth_interceptor.dart';

/// The dio client for the r16a-stream API. Every call carries the bearer token.
Dio buildApiClient(String baseUrl, TokenManager tokens) {
  final dio = Dio(
    BaseOptions(
      baseUrl: baseUrl,
      connectTimeout: const Duration(seconds: 10),
      receiveTimeout: const Duration(seconds: 20),
      // The API never redirects API calls, and following one could carry the bearer token to
      // another host.
      followRedirects: false,
    ),
  );
  dio.interceptors.add(AuthInterceptor(tokens, dio, baseUrl));
  return dio;
}
