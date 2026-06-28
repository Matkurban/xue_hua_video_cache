use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use parking_lot::Mutex;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use url::Url;

use crate::download::DownloadTask;

use super::url_parser::{PrecacheProgress, UrlParser};
use super::url_parser_common::{RangeParseMode, RangeParserCommon};

pub struct UrlParserDefault;

#[async_trait]
impl UrlParser for UrlParserDefault {
    async fn cache(&self, task: &DownloadTask) -> Option<Bytes> {
        RangeParserCommon::cache(task).await
    }

    async fn download(&self, task: Arc<Mutex<DownloadTask>>) -> Option<Bytes> {
        RangeParserCommon::download(task).await
    }

    async fn push(&self, task: Arc<Mutex<DownloadTask>>) {
        let _ = RangeParserCommon::push(task).await;
    }

    async fn parse(&self, stream: TcpStream, uri: Url, headers: HashMap<String, String>) -> bool {
        RangeParserCommon::parse(stream, uri, headers, RangeParseMode::Default).await
    }

    async fn is_cached(
        &self,
        url: &str,
        headers: Option<HashMap<String, String>>,
        cache_segments: usize,
    ) -> bool {
        RangeParserCommon::is_cached(url, headers, cache_segments).await
    }

    async fn precache(
        &self,
        url: &str,
        headers: Option<HashMap<String, String>>,
        cache_segments: usize,
        download_now: bool,
        progress_tx: Option<mpsc::UnboundedSender<PrecacheProgress>>,
    ) -> Result<(), String> {
        RangeParserCommon::precache(url, headers, cache_segments, download_now, progress_tx).await
    }
}
