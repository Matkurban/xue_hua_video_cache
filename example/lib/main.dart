import 'dart:developer' as developer;

import 'package:flutter/material.dart';
import 'package:video_player/video_player.dart';
import 'package:xue_hua_video_cache/xue_hua_video_cache.dart';

const _sampleMp4 =
    'https://jsontodart.cn/api/object/7976982000/msg_video_7976982000_1782918277290246.mp4';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await XueHuaVideoCache.initialize(logPrint: true);
  runApp(const ExampleApp());
}

class ExampleApp extends StatefulWidget {
  const ExampleApp({super.key});

  @override
  State<ExampleApp> createState() => _ExampleAppState();
}

class _ExampleAppState extends State<ExampleApp> {
  @override
  void dispose() {
    XueHuaVideoCache.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'xue_hua_video_cache',
      home: const VideoPlayPage(),
    );
  }
}

class VideoPlayPage extends StatefulWidget {
  const VideoPlayPage({super.key});

  @override
  State<VideoPlayPage> createState() => _VideoPlayPageState();
}

class _VideoPlayPageState extends State<VideoPlayPage> {
  VideoPlayerController? _controller;
  String? _error;
  bool _cached = false;

  @override
  void initState() {
    super.initState();
    _init();
  }

  Future<void> _init() async {
    try {
      // butterfly.mp4 is ~2.4MB; default segment size is 2MB — cache both segments.
      // await XueHuaVideoCache.precache(_sampleMp4, cacheSegments: 2);
      // _cached = await XueHuaVideoCache.isCached(_sampleMp4, cacheSegments: 2);
      final uri = _sampleMp4.toLocalUri();
      final controller = VideoPlayerController.networkUrl(uri);
      await controller.initialize();
      controller.play();
      if (!mounted) {
        await controller.dispose();
        return;
      }
      setState(() => _controller = controller);
    } catch (e,s) {
      developer.log(e.toString(),stackTrace: s);
      if (mounted) setState(() => _error = e.toString());
    }
  }

  Future<void> _replay() async {
    final controller = _controller;
    if (controller == null || !controller.value.isInitialized) return;
    await controller.seekTo(Duration.zero);
    await controller.play();
  }

  @override
  void dispose() {
    _controller?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('Video cache example')),
      body: Center(
        child: _error != null
            ? Text('Error: $_error')
            : Column(
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  Text('Cached: $_cached'),
                  const SizedBox(height: 16),
                  if (_controller != null &&
                      _controller!.value.isInitialized) ...[
                    AspectRatio(
                      aspectRatio: _controller!.value.aspectRatio,
                      child: VideoPlayer(_controller!),
                    ),
                    const SizedBox(height: 16),
                    FilledButton.icon(
                      onPressed: _replay,
                      icon: const Icon(Icons.replay),
                      label: const Text('重新播放'),
                    ),
                  ] else
                    const CircularProgressIndicator(),
                ],
              ),
      ),
    );
  }
}
