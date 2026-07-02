import 'package:flutter_test/flutter_test.dart';
import 'package:xue_hua_video_cache/src/rust/frb_generated.dart';
import 'package:xue_hua_video_cache/xue_hua_video_cache.dart';

import 'sample_mp4_url.dart';

void main() {
  setUpAll(() async {
    await RustLib.init();
  });
  group('XueHUAEVideoCache facade', () {
    test('isInitialized is false before init', () {
      expect(XueHuaVideoCache.isInitialized, isFalse);
    });

    test('dispose is exposed', () {
      expect(XueHuaVideoCache.dispose, isA<Function>());
    });

    test('precache and isCached are exposed', () {
      expect(XueHuaVideoCache.precache, isA<Function>());
      expect(XueHuaVideoCache.isCached, isA<Function>());
    });

    test('parseHlsMasterPlaylist is exposed', () {
      expect(XueHuaVideoCache.parseHlsMasterPlaylist, isNotNull);
    });

    test('download helpers are exposed', () {
      expect(XueHuaVideoCache.pauseTaskById, isA<Function>());
      expect(XueHuaVideoCache.cancelAllTasks, isA<Function>());
      expect(XueHuaVideoCache.pauseAllTasks, isA<Function>());
      expect(XueHuaVideoCache.cancelTaskAboutUrl, isA<Function>());
    });
  });

  group('UrlExt', () {
    test('toLocalUrl uses default port before init', () {
      expect(sampleMp4TestUrl.toLocalUrl(), contains('127.0.0.1'));
      expect(XueHuaVideoCache.isInitialized, isFalse);
    });
  });
}
