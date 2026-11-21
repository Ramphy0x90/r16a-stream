import 'package:cached_network_image/cached_network_image.dart';
import 'package:material_ui/material_ui.dart';

import '../api/generated/models/image_set.dart';
import '../api/generated/models/image_source.dart';

/// The WebP source that covers [pixels] with the least waste, else the widest one. WebP and
/// not AVIF, because Flutter's decoders read WebP on every platform.
ImageSource? pickSource(ImageSet image, double pixels) {
  final webp = image.sources.where((s) => s.format == 'webp').toList()
    ..sort((a, b) => a.width.compareTo(b.width));
  if (webp.isEmpty) return null;
  return webp.firstWhere((s) => s.width >= pixels, orElse: () => webp.last);
}

/// Cache key of an image, its object key. Presigned URLs change daily, the object never does,
/// so keying on the URL would download every image again every day (SPEC 043).
String cacheKey(String url) => Uri.parse(url).path;

/// The placeholder colour from the API's `H S% L%` notation, or null when it does not parse.
Color? placeholderColor(String? hsl) {
  final m = RegExp(r'^(\d{1,3}) (\d{1,3})% (\d{1,3})%$').firstMatch(hsl ?? '');
  if (m == null) return null;
  final h = double.parse(m.group(1)!);
  final s = double.parse(m.group(2)!) / 100;
  final l = double.parse(m.group(3)!) / 100;
  if (h > 360 || s > 1 || l > 1) return null;
  return HSLColor.fromAHSL(1, h, s, l).toColor();
}

/// A media bucket image, decoded at the size it is shown and cached on disk by object key.
/// Shows the average colour while loading and [fallback] when there is no image.
class RemoteImage extends StatelessWidget {
  const RemoteImage({
    super.key,
    required this.image,
    required this.fallback,
    this.semanticLabel,
  });

  final ImageSet? image;
  final Widget fallback;
  final String? semanticLabel;

  @override
  Widget build(BuildContext context) {
    final set = image;
    if (set == null) return fallback;
    final background =
        placeholderColor(set.placeholderHsl) ??
        Theme.of(context).colorScheme.surfaceContainer;
    return LayoutBuilder(
      builder: (context, constraints) {
        final dpr = MediaQuery.devicePixelRatioOf(context);
        final pixels = constraints.maxWidth.isFinite
            ? constraints.maxWidth * dpr
            : 500.0;
        final source = pickSource(set, pixels);
        if (source == null) return fallback;
        return ColoredBox(
          color: background,
          child: CachedNetworkImage(
            imageUrl: source.url,
            cacheKey: cacheKey(source.url),
            memCacheWidth: pixels.round(),
            fit: BoxFit.cover,
            fadeInDuration: const Duration(milliseconds: 150),
            errorWidget: (context, url, error) => fallback,
            imageBuilder: semanticLabel == null
                ? null
                : (context, provider) => Semantics(
                    label: semanticLabel,
                    image: true,
                    child: Image(image: provider, fit: BoxFit.cover),
                  ),
          ),
        );
      },
    );
  }
}
