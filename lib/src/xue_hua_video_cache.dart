import 'dart:async';
import 'dart:io';

import 'package:path_provider/path_provider.dart';

import '../cache/lru_cache_singleton.dart';
import '../download/download_manager.dart';
import '../parser/video_caching.dart';
import 'rust/api/video_proxy.dart' as frb;
import 'rust/frb_generated.dart';
import 'rust/global/cache_key_config.dart';
import 'rust/proxy/platform_kind.dart';

/// Unified plugin entry for local video proxy and caching (Rust backend).
class XueHuaVideoCache {
  static bool _initialized = false;
  static late DownloadManager _downloadManager;

  static Future<void> initialize({
    String? ip,
    int? port,
    int maxMemoryCacheSize = 100,
    int maxStorageCacheSize = 1024,
    String cacheDir = '',
    bool logPrint = false,
    int segmentSize = 2,
    int maxConcurrentDownloads = 4,
    List<String> ignoreQueryKeys = const [],
  }) async {
    await RustLib.init();
    final cacheRoot = cacheDir.isNotEmpty
        ? cacheDir
        : '${(await getApplicationCacheDirectory()).path}/videos';
    await frb.setCacheRootPath(path: cacheRoot);
    await frb.videoProxyInit(
      ip: ip,
      port: port,
      maxMemoryCacheSize: maxMemoryCacheSize,
      maxStorageCacheSize: maxStorageCacheSize,
      cacheDir: '',
      logPrint: logPrint,
      segmentSize: segmentSize,
      maxConcurrentDownloads: maxConcurrentDownloads,
      platform: _platformKind(),
      cacheKeyConfig: CacheKeyConfig(ignoreQueryKeys: ignoreQueryKeys),
    );
    _downloadManager = DownloadManager();
    _initialized = true;
  }

  static bool get isInitialized => _initialized;

  static PlatformKind _platformKind() {
    if (Platform.isAndroid) return PlatformKind.android;
    if (Platform.isIOS) return PlatformKind.ios;
    return PlatformKind.other;
  }

  static Future<void> restart() async {
    if (!_initialized) {
      throw StateError(
        'XueHUAEVideoCache.initialize() must be called before restart()',
      );
    }
    _downloadManager = DownloadManager();
    await frb.videoProxyRestart();
  }

  static Future<bool> isRunning() async {
    if (!_initialized) return false;
    return frb.videoProxyIsRunning();
  }

  /// Stops the health monitor, download pool, and local proxy listener.
  ///
  /// After dispose, plugin APIs return an error until the process restarts.
  /// [initialize] cannot be called again in the same process (native state is one-shot).
  static Future<void> dispose() async {
    if (!_initialized) return;
    await frb.videoProxyDispose();
    _initialized = false;
  }

  static Future<StreamController<Map>?> precache(
    String url, {
    Map<String, Object>? headers,
    int cacheSegments = 2,
    bool downloadNow = true,
    bool progressListen = false,
  }) =>
      VideoCaching.precache(
        url,
        headers: headers,
        cacheSegments: cacheSegments,
        downloadNow: downloadNow,
        progressListen: progressListen,
      );

  static Future<bool> isCached(
    String url, {
    Map<String, Object>? headers,
    int cacheSegments = 2,
  }) =>
      VideoCaching.isCached(
        url,
        headers: headers,
        cacheSegments: cacheSegments,
      );

  static Future<HlsMasterPlaylist?> parseHlsMasterPlaylist(
    String url, {
    Map<String, Object>? headers,
  }) =>
      VideoCaching.parseHlsMasterPlaylist(url, headers: headers);

  static Future<void> removeCacheByUrl(
    String url, {
    bool singleFile = false,
  }) =>
      LruCacheSingleton().removeCacheByUrl(url, singleFile: singleFile);

  static Stream<DownloadTask> get downloadStream => _downloadManager.stream;

  static Future<List<DownloadTask>> allDownloadTasks() =>
      _downloadManager.allTasks;

  static Future<List<DownloadTask>> downloadingTasks() =>
      _downloadManager.downloadingTasks;

  static Future<void> pauseTaskById(String taskId) =>
      _downloadManager.pauseTaskById(taskId);

  static Future<void> resumeTaskById(String taskId) =>
      _downloadManager.resumeTaskById(taskId);

  static Future<void> cancelTaskById(String taskId) =>
      _downloadManager.cancelTaskById(taskId);

  static Future<void> cancelTaskByUrl(String url) =>
      _downloadManager.cancelTaskByUrl(url);

  static Future<void> pauseTaskByUrl(String url) =>
      _downloadManager.pauseTaskByUrl(url);

  static Future<void> resumeTaskByUrl(String url) =>
      _downloadManager.resumeTaskByUrl(url);

  static Future<void> pauseAllTasks() => _downloadManager.pauseAllTasks();

  static Future<void> cancelAllTasks() => _downloadManager.cancelAllTask();

  static Future<void> cancelTaskAboutUrl(String url) =>
      _downloadManager.cancelTaskAboutUrl(url);
}
