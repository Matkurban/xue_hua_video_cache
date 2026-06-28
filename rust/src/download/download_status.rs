#[flutter_rust_bridge::frb]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadStatus {
    Idle,
    Downloading,
    Paused,
    Completed,
    Cancelled,
    Failed,
}
