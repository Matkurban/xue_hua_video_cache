use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use parking_lot::Mutex;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use url::Url;

use crate::download::DownloadTask;
use crate::ext::log_ext::{log_d, log_w};
use crate::ext::socket_ext::append_headers_and_body;
use crate::ext::string_ext::to_safe_uri;
use crate::ext::uri_ext::{hls_key_for_url, uri_generate_md5};
use crate::proxy::ProxyRuntime;

use super::download_wait::{CACHE_POLL_TIMEOUT, TASK_WAIT_TIMEOUT, wait_for_cache};
use super::hls_concurrent_orchestrator::hls_concurrent_loop;
use super::hls_parser::{HlsMediaPlaylist, HlsPlaylist};
use super::hls_playlist_resolver::{
    HlsPlaylistResolver, hls_download, hls_push_task, segment_to_task,
};
use super::hls_playlist_rewriter::rewrite_m3u8_playlist;
use super::hls_registry::{HlsSegment, find_segment_by_uri, register_playlist_segments};
use super::range_response::{apply_range_to_task, build_buffer_response, parse_range_from_headers};
use super::segment_resolver::SegmentResolver;
use super::url_parser::{PrecacheProgress, UrlParser};

pub struct UrlParserM3U8 {
    runtime: Arc<ProxyRuntime>,
    resolver: HlsPlaylistResolver,
}

impl UrlParserM3U8 {
    pub fn new(runtime: Arc<ProxyRuntime>) -> Self {
        let resolver = HlsPlaylistResolver::new(runtime.clone());
        Self { runtime, resolver }
    }

    pub async fn parse_playlist(
        &self,
        uri: &Url,
        headers: Option<&HashMap<String, String>>,
        hls_key: Option<&str>,
    ) -> Option<HlsPlaylist> {
        self.resolver.parse_playlist(uri, headers, hls_key).await
    }

    pub async fn parse_media_playlist(
        &self,
        uri: &Url,
        headers: Option<&HashMap<String, String>>,
        hls_key: Option<&str>,
    ) -> Option<HlsMediaPlaylist> {
        self.resolver
            .parse_media_playlist(uri, headers, hls_key)
            .await
    }

    pub async fn parse_segment(
        &self,
        uri: &Url,
        headers: Option<&HashMap<String, String>>,
    ) -> Vec<HlsSegment> {
        self.resolver.parse_segment(uri, headers).await
    }

    async fn hls_segments_cached(
        &self,
        segments: &[HlsSegment],
        hls_key: &str,
        headers: Option<&HashMap<String, String>>,
    ) -> bool {
        for segment in segments {
            let task = segment_to_task(segment, hls_key, headers);
            if self.cache(&task).await.is_none() {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod hls_key_tests {
    use crate::ext::string_ext::generate_md5;
    use crate::ext::uri_ext::hls_key_for_url;

    #[test]
    fn precache_key_uses_normalized_url_not_raw_md5() {
        let url = "  https://cdn.example.com/master.m3u8  ";
        assert_ne!(generate_md5(url), hls_key_for_url(url));
    }
}

#[async_trait]
impl UrlParser for UrlParserM3U8 {
    async fn cache(&self, task: &DownloadTask) -> Option<Bytes> {
        SegmentResolver::resolve(&self.runtime, task).await
    }

    async fn download(&self, task: Arc<Mutex<DownloadTask>>) -> Option<Bytes> {
        hls_download(&self.runtime, task).await
    }

    async fn push(&self, task: Arc<Mutex<DownloadTask>>) {
        let _ = hls_push_task(&self.runtime, task).await;
    }

    async fn parse(
        &self,
        mut stream: TcpStream,
        uri: Url,
        headers: HashMap<String, String>,
    ) -> bool {
        let runtime = &self.runtime;
        let config = runtime.ctx.config.read().clone();
        let matcher = runtime.ctx.url_matcher.as_ref();

        let result = async {
            let hls_key = uri_generate_md5(&uri);
            let mut task = DownloadTask::new(uri.clone(), None);
            task.headers = Some(headers.clone());
            task.hls_key = Some(hls_key.clone());

            let hls_segment = find_segment_by_uri(&uri);
            if let Some(ref segment) = hls_segment {
                task.hls_key = Some(segment.key.clone());
            }

            let range_spec = parse_range_from_headers(&headers);
            if let Some(ref spec) = range_spec {
                apply_range_to_task(&mut task, spec);
            }
            let mut data = self.cache(&task).await;
            if data.is_none() {
                let dm = runtime.downloads();
                if dm.is_url_downloading(&task) {
                    data = wait_for_cache(|| self.cache(&task), CACHE_POLL_TIMEOUT).await;
                }
                if data.is_none() {
                    hls_concurrent_loop(runtime.clone(), hls_segment.clone(), headers.clone())
                        .await;
                    let arc = Arc::new(Mutex::new(task.clone()));
                    arc.lock().priority += 2;
                    data = self.download(arc).await;
                }
            }
            let mut data = data.ok_or("download failed")?;

            let mut content_type = "application/octet-stream".to_string();
            if matcher.match_m3u8(&uri) {
                let (buffer, segments) = rewrite_m3u8_playlist(&uri, &data, &hls_key, &config);
                register_playlist_segments(&hls_key, segments);
                data = Bytes::from(buffer);
                content_type = "application/vnd.apple.mpegurl".to_string();
            } else if matcher.match_m3u8_key(&uri) {
                content_type = "application/octet-stream".to_string();
            } else if matcher.match_m3u8_segment(&uri) {
                content_type = "video/MP2T".to_string();
            }

            let response = build_buffer_response(data, range_spec.as_ref());
            let data = response.body;

            let mut header_lines = vec![
                response.status_line.to_string(),
                format!("Content-Type: {content_type}"),
                "Connection: keep-alive".to_string(),
            ];
            if content_type == "video/MP2T" {
                header_lines.push("Accept-Ranges: bytes".to_string());
            }
            if let Some(content_range) = response.content_range {
                header_lines.push(format!("Content-Range: {content_range}"));
            }
            header_lines.push(format!("Content-Length: {}", response.content_length));
            let header_block = header_lines.join("\r\n");
            if !append_headers_and_body(&mut stream, &header_block, &data).await {
                return Err("write failed".into());
            }
            stream.flush().await.ok();
            log_d(&format!("Return request data: {uri}"));
            Ok::<(), String>(())
        }
        .await;

        if let Err(ref e) = result {
            log_w(&format!("[UrlParserM3U8] ⚠ ⚠ ⚠ parse socket close: {e}"));
        }
        let _ = stream.shutdown().await;
        log_d("Connection closed\n");
        result.is_ok()
    }

    async fn is_cached(
        &self,
        url: &str,
        headers: Option<HashMap<String, String>>,
        cache_segments: usize,
    ) -> bool {
        let mut segments = self
            .parse_segment(&to_safe_uri(url), headers.as_ref())
            .await;
        if segments.is_empty() {
            return false;
        }
        let mut cache_segments = cache_segments;
        if cache_segments > segments.len() {
            cache_segments = segments.len();
        }
        let hls_key = hls_key_for_url(url);
        for segment in segments.drain(..cache_segments) {
            let task = segment_to_task(&segment, &hls_key, headers.as_ref());
            if self.cache(&task).await.is_none() {
                return false;
            }
        }
        true
    }

    async fn precache(
        &self,
        url: &str,
        headers: Option<HashMap<String, String>>,
        cache_segments: usize,
        download_now: bool,
        progress_tx: Option<mpsc::UnboundedSender<PrecacheProgress>>,
    ) -> Result<(), String> {
        let tx = progress_tx;

        let uri = to_safe_uri(url);
        let mut segments = self.parse_segment(&uri, headers.as_ref()).await;
        if segments.is_empty() {
            return Err(format!("No HLS segments found for {url}"));
        }
        let mut cache_segments = cache_segments;
        if cache_segments > segments.len() {
            cache_segments = segments.len();
        }
        let selected: Vec<HlsSegment> = segments.drain(..cache_segments).collect();
        let hls_key = hls_key_for_url(url);
        let url_owned = url.to_string();

        if download_now {
            let mut downloaded = 0usize;
            let total = selected.len();
            let mut failures = 0usize;
            for segment in selected {
                let task = Arc::new(Mutex::new(segment_to_task(
                    &segment,
                    &hls_key,
                    headers.as_ref(),
                )));
                let snapshot = task.lock().clone();
                let mut success = self.cache(&snapshot).await.is_some();
                if !success {
                    if self.download(task.clone()).await.is_some() {
                        success = true;
                    } else {
                        failures += 1;
                    }
                }
                if success {
                    downloaded += 1;
                    if let Some(ref sender) = tx {
                        let t = task.lock();
                        let _ = sender.send(PrecacheProgress {
                            progress: downloaded as f64 / total as f64,
                            url: t.url(),
                            start_range: Some(t.start_range),
                            end_range: t.end_range,
                            segment_url: Some(segment.url.clone()),
                            parent_url: Some(url_owned.clone()),
                            file_name: Some(t.file_name.clone()),
                            hls_key: Some(hls_key.clone()),
                            total_segments: Some(total),
                            current_segment_index: Some(downloaded - 1),
                        });
                    }
                }
            }
            if failures > 0 {
                return Err(format!(
                    "Failed to precache {failures} of {total} HLS segment(s) for {url}"
                ));
            }
        } else {
            for segment in &selected {
                let task = Arc::new(Mutex::new(segment_to_task(
                    segment,
                    &hls_key,
                    headers.as_ref(),
                )));
                self.push(task).await;
            }
            let headers_for_wait = headers.clone();
            let deadline = tokio::time::Instant::now() + TASK_WAIT_TIMEOUT;
            while tokio::time::Instant::now() < deadline {
                if self
                    .hls_segments_cached(&selected, &hls_key, headers_for_wait.as_ref())
                    .await
                {
                    return Ok(());
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
            return Err(format!("Queued HLS precache timed out for {url}"));
        }
        Ok(())
    }
}
