# xue_hua_video_cache

**English** | [简体中文](README.zh-CN.md)

A Flutter video caching plugin with a Rust core. Local HTTP proxy + LRU memory/disk cache + segmented Range downloads for **MP4** and **HLS (M3U8)** playback with players such as `video_player`.

## Features

- **Unified entry** — `XueHUAEVideoCache.initialize()` starts the local proxy and download pool
- **Transparent playback** — rewrite remote URLs with `String.toLocalUri()` / `toLocalUrl()`
- **Pre-cache** — `XueHuaVideoCache.precache()` with optional progress stream
- **Download manager** — `XueHuaVideoCache.pauseTaskById()` / `resumeTaskByUrl()` / `cancelAllTasks()`; HLS-aware `cancelTaskAboutUrl`
- **Configurable cache keys** — `ignoreQueryKeys` strips volatile query params (e.g. `token`)
- **Cross-platform** — Android, iOS, macOS, Linux, Windows (FFI + flutter_rust_bridge 2.12)

## Requirements

- Flutter ≥ 3.3.0, Dart ≥ 3.12
- Rust toolchain (for building the native library via Cargokit)
- Network access to origin servers (and `127.0.0.1` for the local proxy)

## Installation

```yaml
dependencies:
  xue_hua_video_cache: ^lasted
```

## Quick start

```dart
import 'package:flutter/widgets.dart';
import 'package:xue_hua_video_cache/xue_hua_video_cache.dart';
import 'package:video_player/video_player.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await XueHuaVideoCache.initialize();
  runApp(const MyApp());
}

// Pre-cache then play through the local proxy
const url = 'https://example.com/video.mp4';
await XueHuaVideoCache.precache(url, cacheSegments: 2);
final controller = VideoPlayerController.networkUrl(url.toLocalUri());
await controller.initialize();
controller.play();
```

### Pre-cache progress

When `progressListen: true`, `precache` returns a typed progress stream:

```dart
final stream = await XueHuaVideoCache.precache(
  url,
  cacheSegments: 2,
  progressListen: true,
);
stream?.listen((PrecacheProgressEvent event) {
  print('${(event.progress * 100).toStringAsFixed(0)}% ${event.url}');
});
```

Range (MP4) events populate `startRange` / `endRange`; HLS events may also include
`segmentUrl`, `parentUrl`, `hlsKey`, and segment index fields.

## API overview

| API | Description |
|-----|-------------|
| `XueHUAEVideoCache.initialize(...)` | Start proxy, LRU cache, and download manager |
| `XueHUAEVideoCache.restart()` | Restart proxy and refresh download manager |
| `XueHUAEVideoCache.isRunning()` | Health check for the local proxy |
| `XueHuaVideoCache.precache(...)` | Queue or download segments; optional `progressListen` |
| `XueHuaVideoCache.isCached(...)` | Check whether N segments are cached |
| `XueHuaVideoCache.parseHlsMasterPlaylist(...)` | Parse remote M3U8 master playlist |
| `String.toLocalUri()` / `toLocalUrl()` | Rewrite URL to local proxy |
| `XueHuaVideoCache.downloadStream` | Download task event stream |
| `XueHuaVideoCache.allDownloadTasks()` / `downloadingTasks()` | List download tasks |
| `XueHuaVideoCache.pauseTaskById(...)` / `resumeTaskByUrl(...)` / etc. | Pause, resume, cancel downloads |
| `XueHuaVideoCache.removeCacheByUrl(...)` | Remove cache entry by URL |

### Cache key customization

If URLs carry volatile query parameters, ignore them when computing cache keys:

```dart
await XueHuaVideoCache.initialize(
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

## Platform setup

The plugin starts a **local HTTP proxy** on `127.0.0.1`. After rewriting URLs with `toLocalUri()`, the media player loads video over plain HTTP from localhost. Mobile and sandboxed desktop targets therefore need explicit permission for that traffic, plus normal network access for origin downloads.

Reference implementations live under [`example/`](example/).

### Android

**1. Internet permission** — add to `android/app/src/main/AndroidManifest.xml`:

```xml
<manifest xmlns:android="http://schemas.android.com/apk/res/android">
    <uses-permission android:name="android.permission.INTERNET"/>
    ...
</manifest>
```

**2. Cleartext HTTP for localhost** — create `android/app/src/main/res/xml/network_security_config.xml`:

```xml
<?xml version="1.0" encoding="utf-8"?>
<network-security-config>
  <domain-config cleartextTrafficPermitted="true">
    <domain includeSubdomains="false">127.0.0.1</domain>
  </domain-config>
</network-security-config>
```

**3. Reference the config** — on the `<application>` tag in `AndroidManifest.xml`:

```xml
<application
    android:networkSecurityConfig="@xml/network_security_config"
    ...>
```

> On API 28+, scoped `networkSecurityConfig` is preferred over `android:usesCleartextTraffic="true"`, which allows cleartext for all hosts.

### iOS

Add an App Transport Security exception for the local proxy in `ios/Runner/Info.plist`:

```xml
<key>NSAppTransportSecurity</key>
<dict>
    <key>NSAllowsArbitraryLoads</key>
    <false/>
    <key>NSExceptionDomains</key>
    <dict>
        <key>127.0.0.1</key>
        <dict>
            <key>NSExceptionAllowsInsecureHTTPLoads</key>
            <true/>
            <key>NSIncludesSubdomains</key>
            <false/>
        </dict>
    </dict>
</dict>
```

Keep `NSAllowsArbitraryLoads` disabled; only `127.0.0.1` needs the exception.

### macOS

Sandboxed macOS apps need both outbound downloads and a local listener. Add to `macos/Runner/DebugProfile.entitlements` and `macos/Runner/Release.entitlements`:

```xml
<key>com.apple.security.network.client</key>
<true/>
<key>com.apple.security.network.server</key>
<true/>
```

- `network.client` — fetch video from remote origins
- `network.server` — run the localhost HTTP proxy

### Linux & Windows

No extra manifest or entitlement changes. Ensure the host can reach origin servers and bind/listen on `127.0.0.1`.

### Tips

- **Segment sizing** — for files larger than one segment, pre-cache enough segments (e.g. `cacheSegments: 2` for ~2.4 MB video with 2 MB segments) or rely on on-the-fly proxy downloads.

## Related

- Sample video used in tests: [butterfly.mp4](https://flutter.github.io/assets-for-api-docs/assets/videos/butterfly.mp4)
- [CHANGELOG](CHANGELOG.md)

## License

See [LICENSE](LICENSE).
