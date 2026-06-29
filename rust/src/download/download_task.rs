use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use bytes::Bytes;
use tokio_util::sync::CancellationToken;
use url::Url;

use super::download_status::DownloadStatus;

static AUTO_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub struct DownloadTask {
    pub id: String,
    pub uri: Url,
    pub priority: i32,
    pub progress: f64,
    pub cached_bytes: i64,
    pub downloaded_bytes: i64,
    pub total_bytes: i64,
    pub status: DownloadStatus,
    pub start_range: i64,
    pub end_range: Option<i64>,
    pub headers: Option<HashMap<String, String>>,
    pub hls_key: Option<String>,
    pub retry_times: i32,
    pub cancel_token: Option<CancellationToken>,
    pub last_progress_at: Option<Instant>,
    pub data: Bytes,
    pub cache_dir: String,
    pub file_name: String,
}

impl DownloadTask {
    pub fn new(uri: Url, file_name: Option<String>) -> Self {
        let id = AUTO_ID.fetch_add(1, Ordering::SeqCst).to_string();
        let file_name = file_name.unwrap_or_else(|| uri.to_string());
        Self {
            id,
            uri,
            priority: 1,
            progress: 0.0,
            cached_bytes: 0,
            downloaded_bytes: 0,
            total_bytes: 0,
            status: DownloadStatus::Idle,
            start_range: 0,
            end_range: None,
            headers: None,
            hls_key: None,
            retry_times: 0,
            cancel_token: None,
            last_progress_at: None,
            data: Bytes::new(),
            cache_dir: String::new(),
            file_name,
        }
    }

    pub fn reset_id() {
        AUTO_ID.store(1, Ordering::SeqCst);
    }

    pub fn url(&self) -> String {
        self.uri.to_string()
    }

    pub fn reset(&mut self) {
        self.downloaded_bytes = 0;
        self.total_bytes = 0;
        self.progress = 0.0;
        self.start_range = 0;
        self.end_range = None;
        self.data = Bytes::new();
    }
}
