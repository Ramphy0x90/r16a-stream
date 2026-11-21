import 'package:flutter_test/flutter_test.dart';
import 'package:r16a_stream/core/api/generated/models/image_set.dart';
import 'package:r16a_stream/core/ui/remote_image.dart';

void main() {
  final set = ImageSet.fromJson({
    'placeholderHsl': '160 84% 39%',
    'sources': [
      for (final w in [185, 342, 500])
        for (final f in ['avif', 'webp'])
          {
            'format': f,
            'width': w,
            'url': 'https://h/r16a-stream-media/i/x/$w.$f?X-Amz-Signature=$w',
          },
    ],
  });

  test('picks the narrowest WebP that covers the display, else the widest', () {
    expect(pickSource(set, 300)?.width, 342);
    expect(pickSource(set, 342)?.width, 342);
    expect(pickSource(set, 2000)?.width, 500);
    expect(pickSource(set, 10)?.format, 'webp');
  });

  test('the cache key is the object key, stable across daily signatures', () {
    expect(
      cacheKey('https://h/r16a-stream-media/i/x/342.webp?X-Amz-Signature=a'),
      cacheKey('https://h/r16a-stream-media/i/x/342.webp?X-Amz-Signature=b'),
    );
    expect(
      cacheKey('https://h/r16a-stream-media/i/x/342.webp?s=1'),
      '/r16a-stream-media/i/x/342.webp',
    );
  });

  test('placeholder colours parse only the token notation', () {
    expect(placeholderColor('160 84% 39%'), isNotNull);
    expect(placeholderColor('red'), isNull);
    expect(placeholderColor('400 10% 10%'), isNull);
    expect(placeholderColor(null), isNull);
  });
}
