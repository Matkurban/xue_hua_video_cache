import '../src/rust/api/video_caching.dart' as frb;

class LruCacheSingleton {
  static final LruCacheSingleton _instance = LruCacheSingleton._();
  factory LruCacheSingleton() => _instance;
  LruCacheSingleton._();

  Future<void> removeCacheByUrl(String url, {bool singleFile = false}) {
    return frb.lruRemoveCacheByUrl(url: url, singleFile: singleFile);
  }
}
