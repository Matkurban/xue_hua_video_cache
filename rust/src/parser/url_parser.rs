use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use parking_lot::Mutex;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use url::Url;

use crate::download::DownloadTask;

/// Progress update emitted during precache when `progress_listen` is enabled.
#[derive(Debug, Clone)]
pub struct PrecacheProgress {
    pub progress: f64,
    pub url: String,
    pub start_range: Option<i64>,
    pub end_range: Option<i64>,
    pub segment_url: Option<String>,
    pub parent_url: Option<String>,
    pub file_name: Option<String>,
    pub hls_key: Option<String>,
    pub total_segments: Option<usize>,
    pub current_segment_index: Option<usize>,
}

/// URL parser interface mirroring the Dart `UrlParser` abstraction.
#[async_trait]
pub trait UrlParser: Send + Sync {
    async fn cache(&self, task: &DownloadTask) -> Option<Bytes>;

    async fn download(&self, task: Arc<Mutex<DownloadTask>>) -> Option<Bytes>;

    async fn push(&self, task: Arc<Mutex<DownloadTask>>);

    async fn parse(&self, stream: TcpStream, uri: Url, headers: HashMap<String, String>) -> bool;

    async fn is_cached(
        &self,
        url: &str,
        headers: Option<HashMap<String, String>>,
        cache_segments: usize,
    ) -> bool;

    async fn precache(
        &self,
        url: &str,
        headers: Option<HashMap<String, String>>,
        cache_segments: usize,
        download_now: bool,
        progress_tx: Option<mpsc::UnboundedSender<PrecacheProgress>>,
    ) -> Result<(), String>;
}
