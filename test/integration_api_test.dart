import 'package:flutter_test/flutter_test.dart';
import 'package:xue_hua_video_cache/xue_hua_video_cache.dart';

const _sampleMp4 =
    'https://flutter.github.io/assets-for-api-docs/assets/videos/butterfly.mp4';

void main() {
  setUpAll(() async {
    await RustLib.init();
  });

  group('XueHUAEVideoCache facade', () {
    test('isInitialized is false before init', () {
      expect(XueHUAEVideoCache.isInitialized, isFalse);
    });

    test('dispose is exposed', () {
      expect(XueHUAEVideoCache.dispose, isA<Function>());
    });
  });

  group('UrlExt', () {
    test('toLocalUrl uses default port before init', () {
      expect(_sampleMp4.toLocalUrl(), contains('127.0.0.1'));
      expect(XueHUAEVideoCache.isInitialized, isFalse);
    });
  });

  group('VideoCaching facade', () {
    test('parseHlsMasterPlaylist is exposed', () {
      expect(VideoCaching.parseHlsMasterPlaylist, isNotNull);
    });
  });

  group('DownloadManager facade', () {
    test('declares cancel helpers without Rust init', () {
      final manager = DownloadManager();
      expect(manager.cancelTaskAboutUrl, isA<Function>());
      expect(manager.cancelAllTask, isA<Function>());
      expect(manager.pauseAllTasks, isA<Function>());
    });
  });
}
