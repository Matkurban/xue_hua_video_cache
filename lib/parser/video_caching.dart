import 'dart:async';

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';

import '../src/rust/api/video_caching.dart' as frb;

/// Pre-cache and cache-check API (delegates to Rust).
class VideoCaching {
  static Future<StreamController<Map>?> precache(
    String url, {
    Map<String, Object>? headers,
    int cacheSegments = 2,
    bool downloadNow = true,
    bool progressListen = false,
  }) async {
    final headerMap = headers?.map((k, v) => MapEntry(k, v.toString()));

    if (!progressListen) {
      await frb.videoCachingPrecache(
        url: url,
        headers: headerMap,
        cacheSegments: cacheSegments,
        downloadNow: downloadNow,
        progressListen: false,
        sink: null,
      );
      return null;
    }

    final sink = RustStreamSink<frb.PrecacheProgressInfo>();
    final controller = StreamController<Map>.broadcast();

    unawaited(
      frb
          .videoCachingPrecache(
            url: url,
            headers: headerMap,
            cacheSegments: cacheSegments,
            downloadNow: downloadNow,
            progressListen: true,
            sink: sink,
          )
          .then((_) {
            if (!controller.isClosed) {
              controller.close();
            }
          })
          .catchError((Object error, StackTrace stackTrace) {
            if (!controller.isClosed) {
              controller.addError(error, stackTrace);
              controller.close();
            }
          }),
    );

    sink.stream.listen(
      (info) {
        if (controller.isClosed) return;
        controller.add({
          'progress': info.progress,
          'url': info.url,
          if (info.startRange != null) 'startRange': info.startRange!.toInt(),
          if (info.endRange != null) 'endRange': info.endRange!.toInt(),
          if (info.segmentUrl != null) 'segmentUrl': info.segmentUrl,
          if (info.parentUrl != null) 'parentUrl': info.parentUrl,
          if (info.fileName != null) 'fileName': info.fileName,
          if (info.hlsKey != null) 'hlsKey': info.hlsKey,
          if (info.totalSegments != null) 'totalSegments': info.totalSegments,
          if (info.currentSegmentIndex != null)
            'currentSegmentIndex': info.currentSegmentIndex,
        });
        if (info.progress >= 1.0 && !controller.isClosed) {
          controller.close();
        }
      },
      onError: (Object error, StackTrace stackTrace) {
        if (!controller.isClosed) {
          controller.addError(error, stackTrace);
        }
      },
    );

    return controller;
  }

  static Future<bool> isCached(
    String url, {
    Map<String, Object>? headers,
    int cacheSegments = 2,
  }) {
    return frb.videoCachingIsCached(
      url: url,
      headers: headers?.map((k, v) => MapEntry(k, v.toString())),
      cacheSegments: cacheSegments,
    );
  }

  static Future<HlsMasterPlaylist?> parseHlsMasterPlaylist(
    String url, {
    Map<String, Object>? headers,
  }) async {
    final info = await frb.videoCachingParseHlsMasterPlaylist(
      url: url,
      headers: headers?.map((k, v) => MapEntry(k, v.toString())),
    );
    if (info == null) return null;
    return HlsMasterPlaylist(mediaPlaylistUrls: info.mediaPlaylistUrls);
  }
}

/// HLS master playlist parsed from a remote M3U8 URL.
class HlsMasterPlaylist {
  const HlsMasterPlaylist({required this.mediaPlaylistUrls});

  final List<String> mediaPlaylistUrls;
}
