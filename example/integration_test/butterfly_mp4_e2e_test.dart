import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:xue_hua_video_cache/xue_hua_video_cache.dart';

import '../../test/sample_mp4_url.dart';

const _runE2e = bool.fromEnvironment('RUN_E2E', defaultValue: false);

Future<bool> _waitCached(String url, {required int segments}) async {
  const timeout = Duration(seconds: 45);
  final deadline = DateTime.now().add(timeout);
  while (DateTime.now().isBefore(deadline)) {
    if (await XueHuaVideoCache.isCached(url, cacheSegments: segments)) {
      return true;
    }
    await Future<void>.delayed(const Duration(milliseconds: 400));
  }
  return false;
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  if (!_runE2e) {
    test(
      'butterfly mp4 precache (skipped — pass --dart-define=RUN_E2E=true)',
      () {},
      skip: true,
    );
    return;
  }

  group('butterfly mp4 network E2E', () {
    setUpAll(() async {
      final cacheRoot =
          '${Directory.systemTemp.path}/xue_hua_butterfly_e2e_${DateTime.now().microsecondsSinceEpoch}';
      await Directory(cacheRoot).create(recursive: true);
      await XueHuaVideoCache.initialize(
        cacheDir: cacheRoot,
        logPrint: false,
        segmentSize: 2,
        maxConcurrentDownloads: 2,
      );
    });

    tearDownAll(() async {
      await XueHuaVideoCache.dispose();
    });

    test('precache marks segments cached and proxy URL is local', () async {
      expect(
        await XueHuaVideoCache.isCached(sampleMp4TestUrl, cacheSegments: 2),
        isFalse,
      );

      await XueHuaVideoCache.precache(
        sampleMp4TestUrl,
        cacheSegments: 2,
        downloadNow: true,
      );

      expect(
        await _waitCached(sampleMp4TestUrl, segments: 2),
        isTrue,
        reason: 'timed out waiting for sample mp4 segments',
      );

      final local = sampleMp4TestUrl.toLocalUrl();
      expect(local, contains('127.0.0.1'));

      final uri = Uri.parse(local);
      final client = HttpClient();
      try {
        final request = await client.getUrl(uri);
        request.headers.set(HttpHeaders.rangeHeader, 'bytes=0-4095');
        final response = await request.close();
        expect(response.statusCode, anyOf(200, 206));
        final bytes = await response.fold<List<int>>(
          <int>[],
          (previous, element) => previous..addAll(element),
        );
        expect(bytes.length, greaterThan(0));
      } finally {
        client.close(force: true);
      }
    });
  });
}
