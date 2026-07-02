import 'package:flutter_test/flutter_test.dart';
import 'package:xue_hua_video_cache/src/precache_progress_event.dart';
import 'package:xue_hua_video_cache/src/rust/api/video_caching.dart' as frb;

import 'sample_mp4_url.dart';

void main() {
  group('PrecacheProgressEvent.fromInfo', () {
    test('maps range progress fields', () {
      const url = sampleMp4TestUrl;
      final info = frb.PrecacheProgressInfo(
        progress: 0.5,
        url: url,
        startRange: 0,
        endRange: 2097151,
      );

      final event = PrecacheProgressEvent.fromInfo(info);

      expect(event.progress, 0.5);
      expect(event.url, url);
      expect(event.startRange, 0);
      expect(event.endRange, 2097151);
      expect(event.segmentUrl, isNull);
      expect(event.parentUrl, isNull);
      expect(event.fileName, isNull);
      expect(event.hlsKey, isNull);
      expect(event.totalSegments, isNull);
      expect(event.currentSegmentIndex, isNull);
    });

    test('maps HLS progress fields', () {
      const parentUrl = 'https://example.com/master.m3u8';
      const segmentUrl = 'https://example.com/seg0.ts';
      final info = frb.PrecacheProgressInfo(
        progress: 1.0,
        url: segmentUrl,
        startRange: 0,
        endRange: 1023,
        segmentUrl: segmentUrl,
        parentUrl: parentUrl,
        fileName: 'seg0.ts',
        hlsKey: 'abc123',
        totalSegments: 10,
        currentSegmentIndex: 9,
      );

      final event = PrecacheProgressEvent.fromInfo(info);

      expect(event.progress, 1.0);
      expect(event.url, segmentUrl);
      expect(event.startRange, 0);
      expect(event.endRange, 1023);
      expect(event.segmentUrl, segmentUrl);
      expect(event.parentUrl, parentUrl);
      expect(event.fileName, 'seg0.ts');
      expect(event.hlsKey, 'abc123');
      expect(event.totalSegments, 10);
      expect(event.currentSegmentIndex, 9);
    });

    test('value equality mirrors mapped fields', () {
      final info = frb.PrecacheProgressInfo(
        progress: 0.25,
        url: sampleMp4TestUrl,
        startRange: 100,
      );

      expect(
        PrecacheProgressEvent.fromInfo(info),
        PrecacheProgressEvent.fromInfo(info),
      );
    });
  });
}
