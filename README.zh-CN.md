# xue_hua_video_cache

[English](README.md) | **简体中文**

Flutter 视频缓存插件，核心由 Rust 实现。通过本地 HTTP 代理 + LRU 内存/磁盘缓存 + 分段 Range 下载，支持 **MP4** 与 **HLS (M3U8)**，可与 `video_player` 等播放器配合使用。

## 功能特性

- **统一入口** — `XueHUAEVideoCache.initialize()` 启动本地代理与下载池
- **透明播放** — 使用 `String.toLocalUri()` / `toLocalUrl()` 将远程 URL 改写为本地代理地址
- **预缓存** — `XueHuaVideoCache.precache()`，可选进度流 `progressListen`
- **下载管理** — `XueHuaVideoCache.pauseTaskById()` / `resumeTaskByUrl()` / `cancelAllTasks()`；HLS 感知的 `cancelTaskAboutUrl`
- **可配置缓存键** — `ignoreQueryKeys` 忽略易变 query 参数（如 `token`）
- **跨平台** — Android、iOS、macOS、Linux、Windows（FFI + flutter_rust_bridge 2.12）

## 环境要求

- Flutter ≥ 3.3.0，Dart ≥ 3.12
- Rust 工具链（经 Cargokit 编译原生库）
- 可访问源站网络，以及本地代理 `127.0.0.1`

## 安装

```yaml
dependencies:
  xue_hua_video_cache: ^lasted
```

## 快速开始

```dart
import 'package:flutter/widgets.dart';
import 'package:xue_hua_video_cache/xue_hua_video_cache.dart';
import 'package:video_player/video_player.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await XueHuaVideoCache.initialize();
  runApp(const MyApp());
}

// 预缓存后通过本地代理播放
const url = 'https://example.com/video.mp4';
await XueHuaVideoCache.precache(url, cacheSegments: 2);
final controller = VideoPlayerController.networkUrl(url.toLocalUri());
await controller.initialize();
controller.play();
```

## API 概览

| API | 说明 |
|-----|------|
| `XueHUAEVideoCache.initialize(...)` | 启动代理、LRU 缓存与下载管理器 |
| `XueHUAEVideoCache.restart()` | 重启代理并刷新下载管理器 |
| `XueHUAEVideoCache.isRunning()` | 本地代理健康检查 |
| `XueHuaVideoCache.precache(...)` | 排队或下载分段；可选 `progressListen` |
| `XueHuaVideoCache.isCached(...)` | 检查是否已缓存指定数量的分段 |
| `XueHuaVideoCache.parseHlsMasterPlaylist(...)` | 解析远程 M3U8 主列表 |
| `String.toLocalUri()` / `toLocalUrl()` | 改写为本地代理 URL |
| `XueHuaVideoCache.downloadStream` | 下载任务事件流 |
| `XueHuaVideoCache.allDownloadTasks()` / `downloadingTasks()` | 列出下载任务 |
| `XueHuaVideoCache.pauseTaskById(...)` / `resumeTaskByUrl(...)` 等 | 暂停、恢复、取消下载 |
| `XueHuaVideoCache.removeCacheByUrl(...)` | 按 URL 移除缓存 |

### 自定义 cache key

若 URL 含会变化的 query 参数，可在初始化时忽略它们：

```dart
await XueHUAEVideoCache.initialize(
  ignoreQueryKeys: ['token', 'expires'],
);
```

### 初始化参数

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `maxMemoryCacheSize` | `100` | 内存 LRU 上限（MB） |
| `maxStorageCacheSize` | `1024` | 磁盘 LRU 上限（MB） |
| `segmentSize` | `2` | 下载分段大小（MB） |
| `maxConcurrentDownloads` | `4` | 最大并发下载数 |
| `cacheDir` | 应用缓存 `/videos` | 磁盘缓存根目录 |
| `logPrint` | `false` | Rust 侧调试日志 |

## 架构

```
Flutter（Dart 薄封装）
        ↕ flutter_rust_bridge 2.12
Rust 核心
  ├── LocalProxyServer   — 本地 HTTP 代理
  ├── LruCacheSingleton  — 内存 + 磁盘 LRU
  ├── DownloadPool       — 流式 Range 下载
  └── UrlParser（MP4 / M3U8 / default）
```

## 平台配置

插件会在 **`127.0.0.1`** 上启动本地 HTTP 代理。使用 `toLocalUri()` 改写 URL 后，播放器将通过 localhost 的明文 HTTP 拉流，因此移动端与沙盒桌面端需要显式放行该流量，并保留访问源站的正常网络权限。

完整示例见 [`example/`](example/)。

### Android

**1. 网络权限** — 在 `android/app/src/main/AndroidManifest.xml` 中添加：

```xml
<manifest xmlns:android="http://schemas.android.com/apk/res/android">
    <uses-permission android:name="android.permission.INTERNET"/>
    ...
</manifest>
```

**2. 允许 localhost 明文 HTTP** — 新建 `android/app/src/main/res/xml/network_security_config.xml`：

```xml
<?xml version="1.0" encoding="utf-8"?>
<network-security-config>
  <domain-config cleartextTrafficPermitted="true">
    <domain includeSubdomains="false">127.0.0.1</domain>
  </domain-config>
</network-security-config>
```

**3. 引用网络安全配置** — 在 `AndroidManifest.xml` 的 `<application>` 上声明：

```xml
<application
    android:networkSecurityConfig="@xml/network_security_config"
    ...>
```

> API 28+ 建议使用仅针对 `127.0.0.1` 的 `networkSecurityConfig`，而不是全局 `android:usesCleartextTraffic="true"`（后者会放行所有主机的明文流量）。

### iOS

在 `ios/Runner/Info.plist` 中为本地代理添加 App Transport Security 例外：

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

保持 `NSAllowsArbitraryLoads` 为 `false`，仅对 `127.0.0.1` 开放例外即可。

### macOS

沙盒化的 macOS 应用需要同时允许出站下载与本地监听。在 `macos/Runner/DebugProfile.entitlements` 与 `macos/Runner/Release.entitlements` 中添加：

```xml
<key>com.apple.security.network.client</key>
<true/>
<key>com.apple.security.network.server</key>
<true/>
```

- `network.client` — 从远程源站下载视频
- `network.server` — 运行 localhost HTTP 代理

### Linux 与 Windows

无需额外 manifest 或 entitlement 配置。确保宿主机能访问源站，并可在 `127.0.0.1` 上绑定/监听端口。

### 使用提示

- **分段大小** — 文件超过一个分段时，请预缓存足够分段（例如默认 2 MB 分段下，~2.4 MB 视频建议 `cacheSegments: 2`），或依赖代理按需拉取。

## 相关链接

- 测试样例视频：[butterfly.mp4](https://flutter.github.io/assets-for-api-docs/assets/videos/butterfly.mp4)
- [更新日志](CHANGELOG.md)

## 许可证

见 [LICENSE](LICENSE)。
