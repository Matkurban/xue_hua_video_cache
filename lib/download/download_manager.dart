import 'dart:async';

import '../src/rust/api/download_manager.dart' as frb;
import '../src/rust/download/download_status.dart' as frb_status;

export '../src/rust/download/download_status.dart';

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
  final frb_status.DownloadStatus status;
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

/// Download manager facade backed by the Rust download pool.
class DownloadManager {
  Stream<DownloadTask> get stream =>
      frb.downloadManagerSubscribe().map(DownloadTask.fromInfo);

  Future<List<DownloadTask>> get allTasks async =>
      (await frb.downloadManagerAllTasks()).map(DownloadTask.fromInfo).toList();

  Future<List<DownloadTask>> get downloadingTasks async =>
      (await frb.downloadManagerDownloadingTasks())
          .map(DownloadTask.fromInfo)
          .toList();

  Future<void> pauseTaskById(String taskId) =>
      frb.downloadManagerPauseTaskById(taskId: taskId);

  Future<void> resumeTaskById(String taskId) =>
      frb.downloadManagerResumeTaskById(taskId: taskId);

  Future<void> cancelTaskById(String taskId) =>
      frb.downloadManagerCancelTaskById(taskId: taskId);

  Future<void> cancelTaskByUrl(String url) =>
      frb.downloadManagerCancelTaskByUrl(url: url);

  Future<void> pauseTaskByUrl(String url) =>
      frb.downloadManagerPauseTaskByUrl(url: url);

  Future<void> resumeTaskByUrl(String url) =>
      frb.downloadManagerResumeTaskByUrl(url: url);

  Future<void> pauseAllTasks() => frb.downloadManagerPauseAllTasks();

  Future<void> cancelAllTask() => frb.downloadManagerCancelAllTasks();

  Future<void> cancelTaskAboutUrl(String url) =>
      frb.downloadManagerCancelTaskAboutUrl(url: url);

  void dispose() {}
}
