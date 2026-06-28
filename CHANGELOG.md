# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.0] - 2026-06-28

First stable release. Rust port of [flutter_video_caching](https://github.com/windows7lake/flutter_video_caching) v1.1.4 with a unified Dart entry point and production-hardening fixes.

### Added

- **`XueHUAEVideoCache.initialize()`** — unified plugin entry (proxy, LRU cache, download manager)
- **Rust core** — local HTTP proxy, LRU memory/disk cache, streaming Range downloads, MP4/M3U8 parsers
- **Dart facade** — `VideoCaching.precache` / `isCached` / `parseHlsMasterPlaylist`, `String.toLocalUri()`
- **DownloadManager** — FRB-backed task stream; pause, resume, cancel by id or URL; HLS-aware `cancelTaskAboutUrl`
- **`CacheKeyConfig`** — `ignoreQueryKeys` for stable cache keys when query params vary
- **Health monitor** — 10s proxy health check with full `restart()` on failure
- **Example app** — butterfly.mp4 demo with `video_player`
- **Tests** — Rust unit tests; opt-in butterfly network E2E; `./scripts/test_butterfly.sh` smoke script
- **Documentation** — bilingual README (EN / zh-CN) and this changelog

### Fixed

- Download wait loops no longer hang indefinitely on `Failed` / `Cancelled` (timeouts + early exit)
- `fail_pool_download` retries (up to 3) with debounced `roundTask` scheduling
- `download_now: false` precache now triggers the download pool via `round_task`
- Health check performs full `VideoProxyState::restart()` (proxy + download manager)
- Proxy path parsing: `to_origin_url` correctly restores remote URLs from `GET /path?origin=...` requests
- macOS treated as iOS platform kind for AVFoundation-compatible proxy behavior
- Example precaches enough segments for multi-segment MP4 files; proper `initialize()` error handling

### Changed

- Public API uses `XueHUAEVideoCache` instead of exposing raw `VideoProxy`
- Network E2E tests are opt-in (`#[ignore]`, `RUN_NETWORK_E2E=1`) to keep default CI fast
- Removed unused Dart stubs under `lib/http/` and `lib/match/`

### Platform support

- Android, iOS, macOS, Linux, Windows (FFI plugin via Cargokit)

### Known limitations

- No custom `UrlMatcher` / `HttpClientBuilder` on the Dart side (use `ignoreQueryKeys` instead)
- Dart LRU introspection API is minimal compared to upstream (`removeCacheByUrl` only)
- HLS long-video cancellation and `download_now: false` queueing should be validated in production workloads
- Large segment files are read fully into memory on download completion (memory pressure for very large segments)

[1.0.0]: #
