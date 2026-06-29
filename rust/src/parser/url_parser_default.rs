use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use parking_lot::Mutex;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use url::Url;

use crate::download::DownloadTask;
use crate::proxy::ProxyRuntime;

use super::precache_orchestrator::PrecacheOrchestrator;
use super::range_responder::{RangeParseMode, RangeResponder};
use super::segment_fetcher::SegmentFetcher;
use super::segment_resolver::SegmentResolver;
use super::url_parser::{PrecacheProgress, UrlParser};

pub struct UrlParserDefault {
    runtime: Arc<ProxyRuntime>,
}

impl UrlParserDefault {
    pub fn new(runtime: Arc<ProxyRuntime>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl UrlParser for UrlParserDefault {
    async fn cache(&self, task: &DownloadTask) -> Option<Bytes> {
        SegmentResolver::resolve(&self.runtime, task).await
    }

    async fn download(&self, task: Arc<Mutex<DownloadTask>>) -> Option<Bytes> {
        SegmentFetcher::download(&self.runtime, task).await
    }

    async fn push(&self, task: Arc<Mutex<DownloadTask>>) {
        let _ = SegmentFetcher::push(&self.runtime, task).await;
    }

    async fn parse(&self, stream: TcpStream, uri: Url, headers: HashMap<String, String>) -> bool {
        RangeResponder::respond(&self.runtime, stream, uri, headers, RangeParseMode::Default).await
    }

    async fn is_cached(
        &self,
        url: &str,
        headers: Option<HashMap<String, String>>,
        cache_segments: usize,
    ) -> bool {
        PrecacheOrchestrator::is_cached(&self.runtime, url, headers, cache_segments).await
    }

    async fn precache(
        &self,
        url: &str,
        headers: Option<HashMap<String, String>>,
        cache_segments: usize,
        download_now: bool,
        progress_tx: Option<mpsc::UnboundedSender<PrecacheProgress>>,
    ) -> Result<(), String> {
        PrecacheOrchestrator::precache(
            &self.runtime,
            url,
            headers,
            cache_segments,
            download_now,
            progress_tx,
        )
        .await
    }
}
