# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.1.0] - 2026-06-29

### Changed

- **Dart layer** — merged shallow `DownloadManager` and `LruCacheSingleton` wrappers into `XueHuaVideoCache`; `DownloadTask` moved to `lib/src/download_task.dart`. Public barrel API unchanged.
- **Rust runtime seam** — introduced `ProxyRuntime` bundle (`AppContext` + `DownloadManager` + `LruCache`); parsers receive injected runtime via `UrlParserFactory`; FRB api uses `require_runtime()`. Parser layer no longer calls `require_state()` or `LruCacheSingleton::instance()`.
- **Range parser modules** — split `RangeParserCommon` into `SegmentResolver`, `SegmentFetcher`, `RangeResponder`, and `PrecacheOrchestrator`; MP4/Default parsers delegate to the four modules.
- **Precache progress (breaking)** — `XueHuaVideoCache.precache` with `progressListen: true` now returns `Stream<PrecacheProgressEvent>?` instead of `StreamController<Map>?`; use `PrecacheProgressEvent` from the main barrel.
- **HLS parser modules** — split `UrlParserM3U8` into `hls_registry`, `hls_playlist_resolver`, `hls_concurrent_orchestrator`, and `hls_playlist_rewriter`; `UrlParserM3U8` is a thin router; HLS cache delegates to `SegmentResolver::resolve`.

### Evaluated / No change

- **Rust api vs domain layer** — evaluated merging `rust/src/api/` with `parser/video_caching.rs`; decided to keep both: `api/` remains the FRB seam (DTO conversion, `StreamSink` wiring, `require_runtime()`); `parser/video_caching.rs` stays as domain dispatch via `UrlParserFactory`.

## [1.0.1] - 2026-06-28

### Fixed

- **Hot restart** — calling `XueHuaVideoCache.initialize()` after a Flutter hot restart no longer throws `XueHUAEVideoCache already initialized`; native state is reused and the proxy is restarted idempotently
- **`RustLib.init()`** — skip re-initialization when flutter_rust_bridge is already loaded in the same process

### Added

- Regression test `test/hot_restart_init_test.dart` and Rust unit test `init_twice_is_idempotent`

## [1.0.0] - 2026-06-28

First stable release with a Rust core and unified Dart entry point.

### Added

- **`XueHuaVideoCache`** — unified static API for proxy init, pre-cache, download control, and cache removal
- **`XueHuaVideoCache.initialize()`** — starts local proxy, LRU cache, and download pool
- **Rust core** — local HTTP proxy, LRU memory/disk cache, streaming Range downloads, MP4/M3U8 parsers
- **Pre-cache & cache check** — `XueHuaVideoCache.precache` / `isCached` / `parseHlsMasterPlaylist`
- **Download helpers** — `downloadStream`, `allDownloadTasks`, pause / resume / cancel by id or URL; HLS-aware `cancelTaskAboutUrl`
- **URL extensions** — `String.toLocalUri()` / `toLocalUrl()` and related helpers
- **`ignoreQueryKeys`** — stable cache keys when query params vary (e.g. `token`)
- **Health monitor** — 10s proxy health check with full `restart()` on failure
- **Example app** — butterfly.mp4 demo with `video_player`
- **Tests** — Rust unit tests; opt-in butterfly network E2E; `./scripts/test_butterfly.sh` smoke script
- **Documentation** — bilingual README (EN / zh-CN) and this changelog

### Platform support

- Android, iOS, macOS, Linux, Windows (FFI plugin via Cargokit)

### Known limitations

- No custom `UrlMatcher` / `HttpClientBuilder` on the Dart side (use `ignoreQueryKeys` instead)
- Dart LRU introspection API is minimal (`removeCacheByUrl` only)
- HLS long-video cancellation and `download_now: false` queueing should be validated in production workloads
- Large segment files are read fully into memory on download completion (memory pressure for very large segments)
