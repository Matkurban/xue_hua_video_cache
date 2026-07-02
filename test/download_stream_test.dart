import 'package:flutter_test/flutter_test.dart';
import 'package:xue_hua_video_cache/xue_hua_video_cache.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  tearDown(() async {
    await XueHuaVideoCache.dispose();
  });

  test('downloadStream returns the same stream instance', () async {
    await XueHuaVideoCache.initialize(
      cacheDir: '/tmp/xue_hua_download_stream_test',
    );
    expect(
      identical(XueHuaVideoCache.downloadStream, XueHuaVideoCache.downloadStream),
      isTrue,
    );
  });
}
