import 'package:flutter/widgets.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'app.dart';
import 'core/config.dart';

void main() {
  // Fails at start rather than later with tokens on a clear-text connection.
  AppConfig.fromEnvironment().validate();
  runApp(const ProviderScope(child: R16aStreamApp()));
}
