import 'rust/api/download_manager.dart' as frb;
import 'rust/download/download_status.dart';

/// Download task snapshot from the Rust download pool.
class DownloadTask {
  DownloadTask({
    required this.id,
    required this.uri,
    required this.priority,
    required this.progress,
    required this.cachedBytes,
    required this.downloadedBytes,
    required this.totalBytes,
    required this.status,
    this.hlsKey,
  });

  final String id;
  final Uri uri;
  final int priority;
  final double progress;
  final int cachedBytes;
  final int downloadedBytes;
  final int totalBytes;
  final DownloadStatus status;
  final String? hlsKey;

  factory DownloadTask.fromInfo(frb.DownloadTaskInfo info) => DownloadTask(
    id: info.id,
    uri: Uri.parse(info.url),
    priority: info.priority,
    progress: info.progress,
    cachedBytes: info.cachedBytes.toInt(),
    downloadedBytes: info.downloadedBytes.toInt(),
    totalBytes: info.totalBytes.toInt(),
    status: info.status,
    hlsKey: info.hlsKey,
  );

  String get url => uri.toString();
}
