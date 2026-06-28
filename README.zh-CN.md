# xue_hua_video_cache

[English](README.md) | **简体中文**

Flutter 视频缓存插件，核心由 Rust 实现。本项目为 [flutter_video_caching](https://github.com/windows7lake/flutter_video_caching) 的移植版（对照 upstream v1.1.4，参考源码位于 `third_party/flutter_video_caching/`）。

通过本地 HTTP 代理 + LRU 内存/磁盘缓存 + 分段 Range 下载，支持 **MP4** 与 **HLS (M3U8)**，可与 `video_player` 等播放器配合使用。

## 功能特性

- **统一入口** — `XueHUAEVideoCache.initialize()` 启动本地代理与下载池
- **透明播放** — 使用 `String.toLocalUri()` / `toLocalUrl()` 将远程 URL 改写为本地代理地址
- **预缓存** — `VideoCaching.precache()`，可选进度流 `progressListen`
- **下载管理** — 按任务 ID / URL 暂停、恢复、取消；HLS 感知的 `cancelTaskAboutUrl`
- **可配置缓存键** — `ignoreQueryKeys` 忽略易变 query 参数（如 `token`）
- **跨平台** — Android、iOS、macOS、Linux、Windows（FFI + flutter_rust_bridge 2.12）

## 环境要求

- Flutter ≥ 3.3.0，Dart ≥ 3.12
- Rust 工具链（经 Cargokit 编译原生库）
- 可访问源站网络，以及本地代理 `127.0.0.1`

## 安装

```yaml
dependencies:
  xue_hua_video_cache: ^1.0.0
```

开发阶段可使用 path / git 依赖：

```yaml
dependencies:
  xue_hua_video_cache:
    path: ../xue_hua_video_cache
```

## 快速开始

```dart
import 'package:flutter/widgets.dart';
import 'package:xue_hua_video_cache/xue_hua_video_cache.dart';
import 'package:video_player/video_player.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await XueHUAEVideoCache.initialize();
  runApp(const MyApp());
}

// 预缓存后通过本地代理播放
const url = 'https://example.com/video.mp4';
await VideoCaching.precache(url, cacheSegments: 2);
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
| `VideoCaching.precache(...)` | 排队或下载分段；可选 `progressListen` |
| `VideoCaching.isCached(...)` | 检查是否已缓存指定数量的分段 |
| `VideoCaching.parseHlsMasterPlaylist(...)` | 解析远程 M3U8 主列表 |
| `String.toLocalUri()` / `toLocalUrl()` | 改写为本地代理 URL |
| `XueHUAEVideoCache.downloadManager` | 任务流及取消/暂停/恢复 API |
| `LruCacheSingleton.removeCacheByUrl(...)` | 按 URL 移除缓存 |

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

## 测试

快速冒烟测试（curl + Rust 单元测试，默认不含网络 E2E）：

```bash
./scripts/test_butterfly.sh
```

Rust 单元测试：

```bash
cd rust && cargo test
```

Opt-in 网络 E2E（butterfly.mp4）：

```bash
cd rust && cargo test -- --ignored butterfly
RUN_NETWORK_E2E=1 cd example && flutter test integration_test/butterfly_mp4_e2e_test.dart -d macos
```

运行示例应用：

```bash
cd example && flutter run
```

## 平台说明

- **Android / iOS** — 需允许访问 `127.0.0.1` 明文 HTTP，参见 `example/android/`、`example/ios/`。
- **macOS** — 出站网络需 `com.apple.security.network.client`，参见 `example/macos/Runner/*.entitlements`。
- **分段大小** — 文件超过一个分段时，请预缓存足够分段（例如默认 2 MB 分段下，~2.4 MB 视频建议 `cacheSegments: 2`），或依赖代理按需拉取。

## 开发

修改 `rust/src/api/` 后重新生成 FRB 绑定：

```bash
flutter_rust_bridge_codegen generate
```

## 相关链接

- 上游项目：[flutter_video_caching](https://github.com/windows7lake/flutter_video_caching)
- 测试样例视频：[butterfly.mp4](https://flutter.github.io/assets-for-api-docs/assets/videos/butterfly.mp4)
- [更新日志](CHANGELOG.md)

## 许可证

见 [LICENSE](LICENSE)。
