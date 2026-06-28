# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

[1.0.0]: #
