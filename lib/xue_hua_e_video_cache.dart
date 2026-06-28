import 'dart:io';

import 'package:path_provider/path_provider.dart';

import 'src/rust/api/video_proxy.dart' as frb;
import 'src/rust/frb_generated.dart';
import 'src/rust/global/cache_key_config.dart';
import 'src/rust/proxy/platform_kind.dart';
import 'download/download_manager.dart';

/// Unified plugin entry for local video proxy and caching (Rust backend).
class XueHUAEVideoCache {
  static bool _initialized = false;
  static late DownloadManager downloadManager;

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
    downloadManager = DownloadManager();
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
    downloadManager = DownloadManager();
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
}
