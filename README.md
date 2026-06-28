# xue_hua_video_cache

**English** | [简体中文](README.zh-CN.md)

A Flutter video caching plugin with a Rust core. It is a port of [flutter_video_caching](https://github.com/windows7lake/flutter_video_caching) (upstream v1.1.4; reference sources live in `third_party/flutter_video_caching/`).

Local HTTP proxy + LRU memory/disk cache + segmented Range downloads for **MP4** and **HLS (M3U8)** playback with players such as `video_player`.

## Features

- **Unified entry** — `XueHUAEVideoCache.initialize()` starts the local proxy and download pool
- **Transparent playback** — rewrite remote URLs with `String.toLocalUri()` / `toLocalUrl()`
- **Pre-cache** — `VideoCaching.precache()` with optional progress stream
- **Download manager** — pause / resume / cancel by task id or URL; HLS-aware `cancelTaskAboutUrl`
- **Configurable cache keys** — `ignoreQueryKeys` strips volatile query params (e.g. `token`)
- **Cross-platform** — Android, iOS, macOS, Linux, Windows (FFI + flutter_rust_bridge 2.12)

## Requirements

- Flutter ≥ 3.3.0, Dart ≥ 3.12
- Rust toolchain (for building the native library via Cargokit)
- Network access to origin servers (and `127.0.0.1` for the local proxy)

## Installation

```yaml
dependencies:
  xue_hua_video_cache: ^1.0.0
```

Or use a path/git dependency while developing:

```yaml
dependencies:
  xue_hua_video_cache:
    path: ../xue_hua_video_cache
```

## Quick start

```dart
import 'package:flutter/widgets.dart';
import 'package:xue_hua_video_cache/xue_hua_video_cache.dart';
import 'package:video_player/video_player.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await XueHUAEVideoCache.initialize();
  runApp(const MyApp());
}

// Pre-cache then play through the local proxy
const url = 'https://example.com/video.mp4';
await VideoCaching.precache(url, cacheSegments: 2);
final controller = VideoPlayerController.networkUrl(url.toLocalUri());
await controller.initialize();
controller.play();
```

## API overview

| API | Description |
|-----|-------------|
| `XueHUAEVideoCache.initialize(...)` | Start proxy, LRU cache, and download manager |
| `XueHUAEVideoCache.restart()` | Restart proxy and refresh download manager |
| `XueHUAEVideoCache.isRunning()` | Health check for the local proxy |
| `VideoCaching.precache(...)` | Queue or download segments; optional `progressListen` |
| `VideoCaching.isCached(...)` | Check whether N segments are cached |
| `VideoCaching.parseHlsMasterPlaylist(...)` | Parse remote M3U8 master playlist |
| `String.toLocalUri()` / `toLocalUrl()` | Rewrite URL to local proxy |
| `XueHUAEVideoCache.downloadManager` | Task stream and cancel/pause/resume APIs |
| `LruCacheSingleton.removeCacheByUrl(...)` | Remove cache entry by URL |

### Cache key customization

If URLs carry volatile query parameters, ignore them when computing cache keys:

```dart
await XueHUAEVideoCache.initialize(
  ignoreQueryKeys: ['token', 'expires'],
);
```

### Initialize options

| Parameter | Default | Description |
|-----------|---------|-------------|
| `maxMemoryCacheSize` | `100` | Memory LRU cap (MB) |
| `maxStorageCacheSize` | `1024` | Disk LRU cap (MB) |
| `segmentSize` | `2` | Download segment size (MB) |
| `maxConcurrentDownloads` | `4` | Parallel download limit |
| `cacheDir` | app cache `/videos` | On-disk cache root |
| `logPrint` | `false` | Rust-side debug logs |

## Architecture

```
Flutter (Dart facade)
        ↕ flutter_rust_bridge 2.12
Rust core
  ├── LocalProxyServer   — localhost HTTP proxy
  ├── LruCacheSingleton  — memory + disk LRU
  ├── DownloadPool       — streaming Range downloads
  └── UrlParser (MP4 / M3U8 / default)
```

## Testing

Fast smoke test (curl + Rust unit tests, no network E2E by default):

```bash
./scripts/test_butterfly.sh
```

Rust unit tests:

```bash
cd rust && cargo test
```

Opt-in network E2E (butterfly.mp4):

```bash
cd rust && cargo test -- --ignored butterfly
RUN_NETWORK_E2E=1 cd example && flutter test integration_test/butterfly_mp4_e2e_test.dart -d macos
```

Run the example app:

```bash
cd example && flutter run
```

## Platform notes

- **Android / iOS** — allow cleartext HTTP to `127.0.0.1`; see `example/android/` and `example/ios/`.
- **macOS** — `com.apple.security.network.client` entitlement for outbound downloads; see `example/macos/Runner/*.entitlements`.
- **Segment sizing** — for files larger than one segment, pre-cache enough segments (e.g. `cacheSegments: 2` for ~2.4 MB video with 2 MB segments) or rely on on-the-fly proxy downloads.

## Development

Regenerate FRB bindings after changing `rust/src/api/`:

```bash
flutter_rust_bridge_codegen generate
```

## Related

- Upstream: [flutter_video_caching](https://github.com/windows7lake/flutter_video_caching)
- Sample video used in tests: [butterfly.mp4](https://flutter.github.io/assets-for-api-docs/assets/videos/butterfly.mp4)
- [CHANGELOG](CHANGELOG.md)

## License

See [LICENSE](LICENSE).
