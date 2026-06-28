import 'package:flutter_test/flutter_test.dart';
import 'package:xue_hua_video_cache/xue_hua_video_cache.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test('initialize twice simulates hot restart', () async {
    const cacheDir = '/tmp/xue_hua_hot_restart_test';
    await XueHuaVideoCache.initialize(cacheDir: cacheDir);
    expect(XueHuaVideoCache.isInitialized, isTrue);

    await XueHuaVideoCache.initialize(cacheDir: cacheDir);
    expect(await XueHuaVideoCache.isRunning(), isTrue);
  });
}
