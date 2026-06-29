import 'rust/api/video_caching.dart' as frb;

/// Progress update emitted during [VideoCaching.precache] when
/// `progressListen` is enabled.
class PrecacheProgressEvent {
  const PrecacheProgressEvent({
    required this.progress,
    required this.url,
    this.startRange,
    this.endRange,
    this.segmentUrl,
    this.parentUrl,
    this.fileName,
    this.hlsKey,
    this.totalSegments,
    this.currentSegmentIndex,
  });

  final double progress;
  final String url;
  final int? startRange;
  final int? endRange;
  final String? segmentUrl;
  final String? parentUrl;
  final String? fileName;
  final String? hlsKey;
  final int? totalSegments;
  final int? currentSegmentIndex;

  factory PrecacheProgressEvent.fromInfo(frb.PrecacheProgressInfo info) =>
      PrecacheProgressEvent(
        progress: info.progress,
        url: info.url,
        startRange: info.startRange?.toInt(),
        endRange: info.endRange?.toInt(),
        segmentUrl: info.segmentUrl,
        parentUrl: info.parentUrl,
        fileName: info.fileName,
        hlsKey: info.hlsKey,
        totalSegments: info.totalSegments,
        currentSegmentIndex: info.currentSegmentIndex,
      );

  @override
  int get hashCode =>
      progress.hashCode ^
      url.hashCode ^
      startRange.hashCode ^
      endRange.hashCode ^
      segmentUrl.hashCode ^
      parentUrl.hashCode ^
      fileName.hashCode ^
      hlsKey.hashCode ^
      totalSegments.hashCode ^
      currentSegmentIndex.hashCode;

  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      other is PrecacheProgressEvent &&
          runtimeType == other.runtimeType &&
          progress == other.progress &&
          url == other.url &&
          startRange == other.startRange &&
          endRange == other.endRange &&
          segmentUrl == other.segmentUrl &&
          parentUrl == other.parentUrl &&
          fileName == other.fileName &&
          hlsKey == other.hlsKey &&
          totalSegments == other.totalSegments &&
          currentSegmentIndex == other.currentSegmentIndex;
}
