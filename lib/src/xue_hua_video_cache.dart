import 'dart:async';
import 'dart:io';

import 'package:path_provider/path_provider.dart';

import '../parser/video_caching.dart';
import 'download_task.dart';
import 'precache_progress_event.dart';
import 'rust/api/download_manager.dart' as frb_dm;
import 'rust/api/video_caching.dart' as frb_vc;
import 'rust/api/video_proxy.dart' as frb;
import 'rust/frb_generated.dart';
import 'rust/global/cache_key_config.dart';
import 'rust/proxy/platform_kind.dart';

/// Forwards download pool control to the Rust backend.
class _DownloadBridge {
  Stream<DownloadTask> get stream =>
      frb_dm.downloadManagerSubscribe().map(DownloadTask.fromInfo);

  Future<List<DownloadTask>> get allTasks async =>
      (await frb_dm.downloadManagerAllTasks())
          .map(DownloadTask.fromInfo)
          .toList();

  Future<List<DownloadTask>> get downloadingTasks async =>
      (await frb_dm.downloadManagerDownloadingTasks())
          .map(DownloadTask.fromInfo)
          .toList();

  Future<void> pauseTaskById(String taskId) =>
      frb_dm.downloadManagerPauseTaskById(taskId: taskId);

  Future<void> resumeTaskById(String taskId) =>
      frb_dm.downloadManagerResumeTaskById(taskId: taskId);

  Future<void> cancelTaskById(String taskId) =>
      frb_dm.downloadManagerCancelTaskById(taskId: taskId);

  Future<void> cancelTaskByUrl(String url) =>
      frb_dm.downloadManagerCancelTaskByUrl(url: url);

  Future<void> pauseTaskByUrl(String url) =>
      frb_dm.downloadManagerPauseTaskByUrl(url: url);

  Future<void> resumeTaskByUrl(String url) =>
      frb_dm.downloadManagerResumeTaskByUrl(url: url);

  Future<void> pauseAllTasks() => frb_dm.downloadManagerPauseAllTasks();

  Future<void> cancelAllTasks() => frb_dm.downloadManagerCancelAllTasks();

  Future<void> cancelTaskAboutUrl(String url) =>
      frb_dm.downloadManagerCancelTaskAboutUrl(url: url);
}

/// Unified plugin entry for local video proxy and caching (Rust backend).
class XueHuaVideoCache {
  static bool _initialized = false;
  static late _DownloadBridge _download;

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
    if (!RustLib.instance.initialized) {
      await RustLib.init();
    }
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
    _download = _DownloadBridge();
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
    _download = _DownloadBridge();
    await frb.videoProxyRestart();
  }

  static Future<bool> isRunning() async {
    if (!_initialized) return false;
    return frb.videoProxyIsRunning();
  }

  /// Stops the health monitor, download pool, and local proxy listener.
  ///
  /// After [dispose], plugin APIs return an error until the process restarts.
  /// [initialize] cannot be called again in the same process after [dispose]
  /// (native state is one-shot). Flutter hot restart re-runs [initialize]
  /// safely when native state was not disposed.
  static Future<void> dispose() async {
    if (!_initialized) return;
    await frb.videoProxyDispose();
    _initialized = false;
  }

  static Future<Stream<PrecacheProgressEvent>?> precache(
    String url, {
    Map<String, Object>? headers,
    int cacheSegments = 2,
    bool downloadNow = true,
    bool progressListen = false,
  }) => VideoCaching.precache(
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
  }) => VideoCaching.isCached(
    url,
    headers: headers,
    cacheSegments: cacheSegments,
  );

  static Future<HlsMasterPlaylist?> parseHlsMasterPlaylist(
    String url, {
    Map<String, Object>? headers,
  }) => VideoCaching.parseHlsMasterPlaylist(url, headers: headers);

  static Future<void> removeCacheByUrl(String url, {bool singleFile = false}) =>
      frb_vc.lruRemoveCacheByUrl(url: url, singleFile: singleFile);

  static Stream<DownloadTask> get downloadStream => _download.stream;

  static Future<List<DownloadTask>> allDownloadTasks() => _download.allTasks;

  static Future<List<DownloadTask>> downloadingTasks() =>
      _download.downloadingTasks;

  static Future<void> pauseTaskById(String taskId) =>
      _download.pauseTaskById(taskId);

  static Future<void> resumeTaskById(String taskId) =>
      _download.resumeTaskById(taskId);

  static Future<void> cancelTaskById(String taskId) =>
      _download.cancelTaskById(taskId);

  static Future<void> cancelTaskByUrl(String url) =>
      _download.cancelTaskByUrl(url);

  static Future<void> pauseTaskByUrl(String url) =>
      _download.pauseTaskByUrl(url);

  static Future<void> resumeTaskByUrl(String url) =>
      _download.resumeTaskByUrl(url);

  static Future<void> pauseAllTasks() => _download.pauseAllTasks();

  static Future<void> cancelAllTasks() => _download.cancelAllTasks();

  static Future<void> cancelTaskAboutUrl(String url) =>
      _download.cancelTaskAboutUrl(url);
}
