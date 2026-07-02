import 'dart:async';

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';

import '../src/precache_progress_event.dart';
import '../src/rust/api/video_caching.dart' as frb;

/// Pre-cache and cache-check API (delegates to Rust).
class VideoCaching {
  static Future<Stream<PrecacheProgressEvent>?> precache(
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
    final controller = StreamController<PrecacheProgressEvent>.broadcast();

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
        final event = PrecacheProgressEvent.fromInfo(info);
        controller.add(event);
      },
      onError: (Object error, StackTrace stackTrace) {
        if (!controller.isClosed) {
          controller.addError(error, stackTrace);
        }
      },
    );

    return controller.stream;
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
