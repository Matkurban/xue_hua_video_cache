use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use async_recursion::async_recursion;
use async_trait::async_trait;
use base64::Engine;
use bytes::Bytes;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use regex::Regex;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use url::Url;

use crate::cache::LruCacheSingleton;
use crate::download::{DownloadStatus, DownloadTask};
use crate::ext::file_ext::FileExt;
use crate::ext::int_ext::to_memory_size;
use crate::ext::log_ext::{log_d, log_w};
use crate::ext::socket_ext::append_headers_and_body;
use crate::ext::string_ext::{generate_md5, to_local_url, to_safe_uri, to_safe_url};
use crate::ext::uri_ext::{uri_base, uri_generate_md5, uri_path_prefix};
use crate::global::Config;
use crate::proxy::require_state;
use rand::RngExt;

use super::download_wait::{
    CACHE_POLL_TIMEOUT, TASK_WAIT_TIMEOUT, find_completed_task_data, wait_for_cache,
    wait_for_task_completion,
};
use super::hls_parser::{HlsMediaPlaylist, HlsPlaylist, parse_playlist};
use super::url_parser::{PrecacheProgress, UrlParser};

static HLS_REGISTRY: Lazy<Mutex<HlsRegistry>> = Lazy::new(|| Mutex::new(HlsRegistry::default()));

const MAX_HLS_PLAYLIST_KEYS: usize = 8;

struct HlsRegistry {
    playlists: HashMap<String, Vec<HlsSegment>>,
    latest_url: HashMap<String, String>,
}

impl Default for HlsRegistry {
    fn default() -> Self {
        Self {
            playlists: HashMap::new(),
            latest_url: HashMap::new(),
        }
    }
}

impl HlsRegistry {
    fn playlist_keys(&self) -> Vec<String> {
        self.playlists.keys().cloned().collect()
    }

    fn set_latest(&mut self, key: &str, url: &str) {
        self.latest_url.insert(key.to_string(), url.to_string());
    }

    fn latest_for(&self, key: &str) -> Option<String> {
        self.latest_url.get(key).cloned()
    }

    fn add_segment(&mut self, segment: HlsSegment) {
        let list = self.playlists.entry(segment.key.clone()).or_default();
        if !list.iter().any(|e| e.url == segment.url) {
            list.push(segment);
        }
    }

    fn find_by_url(&self, url: &str) -> Option<HlsSegment> {
        self.playlists
            .values()
            .flatten()
            .find(|s| s.url == url)
            .cloned()
    }

    fn segments_for_key(&self, key: &str) -> Vec<HlsSegment> {
        self.playlists.get(key).cloned().unwrap_or_default()
    }

    fn update_status(&mut self, url: &str, status: DownloadStatus) {
        for list in self.playlists.values_mut() {
            if let Some(seg) = list.iter_mut().find(|e| e.url == url) {
                seg.status = status;
                return;
            }
        }
    }

    fn downloading_count(&self, key: &str) -> usize {
        self.playlists
            .get(key)
            .map(|list| {
                list.iter()
                    .filter(|e| e.status == DownloadStatus::Downloading)
                    .count()
            })
            .unwrap_or(0)
    }

    fn find_idle(&self, key: &str) -> Option<HlsSegment> {
        self.playlists
            .get(key)?
            .iter()
            .find(|e| e.status == DownloadStatus::Idle)
            .cloned()
    }

    fn evict_playlist(&mut self, key: &str) {
        self.playlists.remove(key);
        self.latest_url.remove(key);
    }
}

/// Single HLS segment tracked for concurrent download.
#[derive(Debug, Clone)]
pub struct HlsSegment {
    pub key: String,
    pub url: String,
    pub start_range: i64,
    pub end_range: Option<i64>,
    pub status: DownloadStatus,
}

impl HlsSegment {
    pub fn new(key: String, url: String) -> Self {
        Self {
            key,
            url,
            start_range: 0,
            end_range: None,
            status: DownloadStatus::Idle,
        }
    }

    pub fn with_range(key: String, url: String, start_range: i64, end_range: Option<i64>) -> Self {
        Self {
            key,
            url,
            start_range,
            end_range,
            status: DownloadStatus::Idle,
        }
    }
}

pub struct UrlParserM3U8;

#[async_trait]
impl UrlParser for UrlParserM3U8 {
    async fn cache(&self, task: &DownloadTask) -> Option<Bytes> {
        let singleton = LruCacheSingleton::instance();
        let state = require_state().ok()?;
        let config = state.ctx.config.read().clone();
        let matcher = state.ctx.url_matcher.as_ref();
        let match_url = task.match_url(&config, matcher);

        if let Some(data) = singleton.memory_get(&match_url).await {
            log_d(&format!(
                "From memory: {}, total memory size: {}",
                to_memory_size(data.len() as i64),
                singleton.memory_format_size().await
            ));
            return Some(data);
        }
        if let Some(data) = singleton.storage_get(&match_url).await {
            log_d(&format!("From file: {match_url}"));
            singleton.memory_put(&match_url, data.clone()).await;
            return Some(data);
        }
        None
    }

    async fn download(&self, task: Arc<Mutex<DownloadTask>>) -> Option<Bytes> {
        let state = require_state().ok()?;
        let url = task.lock().url();
        log_d(&format!("From network: {url}"));

        {
            let hls_key = task.lock().hls_key.clone();
            if let Some(hls_key) = hls_key {
                if let Ok(path) = FileExt::create_cache_path(Some(&hls_key)).await {
                    task.lock().cache_dir = path;
                }
            }
        }

        let dm = state.download_manager();
        let mut rx = dm.subscribe();
        dm.execute_task(task.clone()).await;

        let task_url = task.lock().url();
        wait_for_task_completion(&mut rx, |t| t.url() == task_url).await
    }

    async fn push(&self, task: Arc<Mutex<DownloadTask>>) {
        let _ = Self::push_task(task).await;
    }

    async fn parse(
        &self,
        mut stream: TcpStream,
        uri: Url,
        headers: HashMap<String, String>,
    ) -> bool {
        let state = match require_state() {
            Ok(s) => s,
            Err(_) => return false,
        };
        let config = state.ctx.config.read().clone();
        let matcher = state.ctx.url_matcher.as_ref();

        let result = async {
            let hls_key = uri_generate_md5(&uri);
            let mut task = DownloadTask::new(uri.clone(), None);
            task.headers = Some(headers.clone());
            task.hls_key = Some(hls_key.clone());

            if let Some(segment) = find_segment_by_uri(&uri) {
                task.hls_key = Some(segment.key.clone());
            }

            if let Some(range) = headers.get("range") {
                let mut range = range.as_str();
                if let Some(stripped) = range.strip_prefix("bytes=") {
                    range = stripped;
                }
                let parts: Vec<&str> = range.split('-').collect();
                if parts.len() == 2 {
                    task.start_range = parts[0].parse().unwrap_or(0);
                    task.end_range = parts[1].parse().ok();
                }
            }

            let hls_segment = find_segment_by_uri(&uri);
            let mut data = self.cache(&task).await;
            if data.is_none() {
                let dm = state.download_manager();
                if dm.is_url_downloading(&task) {
                    data = wait_for_cache(|| self.cache(&task), CACHE_POLL_TIMEOUT).await;
                }
                if data.is_none() {
                    self.concurrent_loop(hls_segment.as_ref(), &headers).await;
                    let arc = Arc::new(Mutex::new(task.clone()));
                    arc.lock().priority += 2;
                    data = self.download(arc).await;
                }
            }
            let mut data = data.ok_or("download failed")?;

            let mut content_type = "application/octet-stream".to_string();
            if matcher.match_m3u8(&uri) {
                let buffer = modify_m3u8_file(&uri, &data, &hls_key, &config);
                let modified = buffer.into_bytes();
                data = Bytes::from(modified);
                content_type = "application/vnd.apple.mpegurl".to_string();
            } else if matcher.match_m3u8_key(&uri) {
                content_type = "application/octet-stream".to_string();
            } else if matcher.match_m3u8_segment(&uri) {
                content_type = "video/MP2T".to_string();
            }

            let mut header_lines = vec![];
            if task.end_range.is_none() {
                header_lines.push("HTTP/1.1 200 OK".to_string());
            } else {
                header_lines.push("HTTP/1.1 206 Partial Content".to_string());
            }
            header_lines.push(format!("Content-Type: {content_type}"));
            header_lines.push("Connection: keep-alive".to_string());
            if content_type == "video/MP2T" {
                header_lines.push("Accept-Ranges: bytes".to_string());
            }
            if let Some(end) = task.end_range {
                header_lines.push(format!("Content-Range: bytes={}-{}", task.start_range, end));
                header_lines.push(format!("Content-Length: {}", end - task.start_range + 1));
            }
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
        let hls_key = generate_md5(url);
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
        let hls_key = generate_md5(url);
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
                if self.cache(&snapshot).await.is_none() {
                    if self.download(task.clone()).await.is_none() {
                        failures += 1;
                    }
                }
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
            if failures > 0 {
                return Err(format!(
                    "Failed to precache {failures} of {total} HLS segment(s) for {url}"
                ));
            }
        } else {
            for segment in selected {
                let task = Arc::new(Mutex::new(segment_to_task(
                    &segment,
                    &hls_key,
                    headers.as_ref(),
                )));
                Self::push_task(task).await?;
            }
            let headers_for_wait = headers.clone();
            let deadline = tokio::time::Instant::now() + TASK_WAIT_TIMEOUT;
            while tokio::time::Instant::now() < deadline {
                if self
                    .is_cached(url, headers_for_wait.clone(), cache_segments)
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

impl UrlParserM3U8 {
    async fn push_task(task: Arc<Mutex<DownloadTask>>) -> Result<(), String> {
        let state = require_state()?;
        let config = state.ctx.config.read().clone();
        let matcher = state.ctx.url_matcher.as_ref();
        let match_url = task.lock().match_url(&config, matcher);
        if LruCacheSingleton::instance()
            .memory_get(&match_url)
            .await
            .is_some()
        {
            return Ok(());
        }
        let hls_key = task.lock().hls_key.clone().unwrap_or_default();
        let cache_path = FileExt::create_cache_path(Some(&hls_key))
            .await
            .map_err(|e| e.to_string())?;
        let save_path = task.lock().save_path(&config, matcher);
        if Path::new(&save_path).exists() {
            return Ok(());
        }
        task.lock().cache_dir = cache_path;
        let dm = state.download_manager();
        dm.add_task(task).await;
        dm.round_task().await;
        Ok(())
    }

    pub async fn parse_playlist(
        &self,
        uri: &Url,
        headers: Option<&HashMap<String, String>>,
        hls_key: Option<&str>,
    ) -> Option<HlsPlaylist> {
        let mut task = DownloadTask::new(uri.clone(), None);
        task.headers = headers.cloned();
        task.hls_key = Some(hls_key.unwrap_or(&uri_generate_md5(uri)).to_string());

        let data = if let Some(cached) = self.cache(&task).await {
            cached
        } else {
            self.download(Arc::new(Mutex::new(task.clone()))).await?
        };

        let lines = read_lines_from_bytes(&data);
        parse_playlist(uri, &lines)
    }

    pub async fn parse_media_playlist(
        &self,
        uri: &Url,
        headers: Option<&HashMap<String, String>>,
        hls_key: Option<&str>,
    ) -> Option<HlsMediaPlaylist> {
        match self.parse_playlist(uri, headers, hls_key).await? {
            HlsPlaylist::Master(master) => {
                for media_url in master.media_playlist_urls {
                    let master_uri = to_safe_uri(&format!("{}{}", uri_base(uri), media_url));
                    if let Some(HlsPlaylist::Media(media)) = self
                        .parse_playlist(&master_uri, headers, Some(&uri_generate_md5(uri)))
                        .await
                    {
                        return Some(media);
                    }
                }
                None
            }
            HlsPlaylist::Media(media) => Some(media),
        }
    }

    pub async fn parse_segment(
        &self,
        uri: &Url,
        headers: Option<&HashMap<String, String>>,
    ) -> Vec<HlsSegment> {
        let Some(playlist) = self.parse_media_playlist(uri, headers, None).await else {
            return Vec::new();
        };
        let base_uri = playlist.base_uri.as_ref();
        let mut segments = Vec::new();
        for segment in playlist.segments {
            let Some(mut segment_url) = segment.url else {
                continue;
            };
            if !segment_url.starts_with("http") {
                segment_url = resolve_relative_url(base_uri, uri, &segment_url);
            }
            let end_range =
                if segment.byterange_offset.is_some() && segment.byterange_length.is_some() {
                    let offset = segment.byterange_offset.unwrap_or(0);
                    let length = segment.byterange_length.unwrap_or(0);
                    if length == 0 {
                        None
                    } else {
                        Some(offset + length - 1)
                    }
                } else {
                    None
                };
            segments.push(HlsSegment::with_range(
                generate_md5(&segment_url),
                segment_url,
                segment.byterange_offset.unwrap_or(0),
                end_range,
            ));
        }
        segments
    }

    async fn concurrent_loop(
        &self,
        hls_segment: Option<&HlsSegment>,
        headers: &HashMap<String, String>,
    ) {
        hls_concurrent_loop(hls_segment.cloned(), headers.clone()).await;
    }
}

#[async_recursion]
async fn hls_concurrent_loop(hls_segment: Option<HlsSegment>, headers: HashMap<String, String>) {
    let Some(hls_segment) = hls_segment else {
        return;
    };
    HLS_REGISTRY
        .lock()
        .set_latest(&hls_segment.key, &hls_segment.url);

    let evict_key = {
        let registry = HLS_REGISTRY.lock();
        let keys = registry.playlist_keys();
        if keys.len() <= MAX_HLS_PLAYLIST_KEYS {
            None
        } else {
            keys.iter()
                .find(|k| *k != &hls_segment.key)
                .cloned()
                .or_else(|| keys.first().cloned())
        }
    };
    if let Some(evict_key) = evict_key {
        let urls: Vec<String> = HLS_REGISTRY
            .lock()
            .segments_for_key(&evict_key)
            .into_iter()
            .map(|e| e.url)
            .collect();
        if let Ok(state) = require_state() {
            let pool = state.download_manager().pool();
            let ids: Vec<String> = pool
                .task_list()
                .into_iter()
                .filter(|t| urls.contains(&t.lock().url()))
                .map(|t| t.lock().id.clone())
                .collect();
            pool.remove_tasks_by_ids(&ids);
        }
        HLS_REGISTRY.lock().evict_playlist(&evict_key);
    }

    let segment = {
        let registry = HLS_REGISTRY.lock();
        let latest_url = registry
            .latest_for(&hls_segment.key)
            .unwrap_or_else(|| hls_segment.url.clone());
        registry.find_by_url(&latest_url)
    };
    let Some(segment) = segment else {
        return;
    };

    let downloading_count = HLS_REGISTRY.lock().downloading_count(&segment.key);
    if downloading_count >= 2 {
        return;
    }

    let cache_key = generate_md5(&segment.url);
    if LruCacheSingleton::instance()
        .memory_get(&cache_key)
        .await
        .is_some()
    {
        hls_concurrent_complete(segment, headers.clone(), None).await;
        return;
    }

    let state = match require_state() {
        Ok(s) => s,
        Err(_) => return,
    };
    let config = state.ctx.config.read().clone();
    let mut task = DownloadTask::new(to_safe_uri(&segment.url), None);
    task.headers = Some(headers.clone());
    if let Ok(path) = FileExt::create_cache_path(Some(&segment.key)).await {
        task.cache_dir = path.clone();
        let save_path = format!(
            "{}/{}",
            path,
            task.save_file_name(&config, state.ctx.url_matcher.as_ref())
        );
        if Path::new(&save_path).exists() {
            hls_concurrent_complete(segment, headers.clone(), None).await;
            return;
        }
    }

    let dm = state.download_manager();
    if dm.is_url_downloading(&task) {
        hls_concurrent_complete(segment, headers.clone(), Some(DownloadStatus::Downloading)).await;
        return;
    }

    task.priority += 1;
    let task_arc = Arc::new(Mutex::new(task));
    let segment_clone = segment.clone();
    let headers = headers.clone();
    let segment_url = segment_clone.url.clone();
    let mut rx = dm.subscribe();
    dm.execute_task(task_arc).await;
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + TASK_WAIT_TIMEOUT;
        while tokio::time::Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Ok(updated)) => {
                    let (task_url, status) = {
                        let updated = updated.lock();
                        (updated.url(), updated.status)
                    };
                    if task_url != segment_url {
                        continue;
                    }
                    match status {
                        DownloadStatus::Completed => {
                            log_d(&format!("Asynchronous download completed： {task_url}"));
                            tokio::spawn(async move {
                                hls_concurrent_complete(segment_clone, headers, None).await;
                            });
                            break;
                        }
                        DownloadStatus::Failed | DownloadStatus::Cancelled => {
                            log_w(&format!(
                                "Asynchronous download ended with {:?}: {task_url}",
                                status
                            ));
                            break;
                        }
                        _ => continue,
                    }
                }
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {
                    let url = segment_url.clone();
                    let completed = find_completed_task_data(&|t| t.url() == url).is_some()
                        || LruCacheSingleton::instance()
                            .memory_get(&generate_md5(&url))
                            .await
                            .is_some();
                    if completed {
                        log_d(&format!(
                            "Asynchronous download completed (recovered): {url}"
                        ));
                        let seg = segment_clone.clone();
                        let hdrs = headers.clone();
                        tokio::spawn(async move {
                            hls_concurrent_complete(seg, hdrs, None).await;
                        });
                        break;
                    }
                    continue;
                }
                Ok(Err(_)) => break,
                Err(_) => {
                    let url = segment_url.clone();
                    let recovered = find_completed_task_data(&|t| t.url() == url).is_some()
                        || LruCacheSingleton::instance()
                            .memory_get(&generate_md5(&url))
                            .await
                            .is_some();
                    if recovered {
                        let seg = segment_clone.clone();
                        let hdrs = headers.clone();
                        tokio::spawn(async move {
                            hls_concurrent_complete(seg, hdrs, None).await;
                        });
                        break;
                    }
                    log_w(&format!(
                        "Asynchronous download timed out waiting: {segment_url}"
                    ));
                    break;
                }
            }
        }
    });
}

#[async_recursion]
async fn hls_concurrent_complete(
    hls_segment: HlsSegment,
    headers: HashMap<String, String>,
    status: Option<DownloadStatus>,
) {
    {
        HLS_REGISTRY.lock().update_status(
            &hls_segment.url,
            status.unwrap_or(DownloadStatus::Completed),
        );
    }

    let next_segment = {
        let registry = HLS_REGISTRY.lock();
        if let Some(latest_url) = registry.latest_for(&hls_segment.key) {
            if let Some(latest) = registry.find_by_url(&latest_url) {
                let same_key = registry.segments_for_key(&latest.key);
                if let Some(idx) = same_key.iter().position(|e| e.url == latest.url) {
                    if idx + 1 < same_key.len() {
                        Some(same_key[idx + 1].clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    };
    if let Some(next) = next_segment {
        hls_concurrent_loop(Some(next), headers).await;
        return;
    }

    let keys = HLS_REGISTRY.lock().playlist_keys();
    if keys.is_empty() {
        return;
    }
    let idx = rand::rng().random_range(0..keys.len());
    let key = keys[idx].clone();
    let idle = HLS_REGISTRY.lock().find_idle(&key);
    if let Some(idle) = idle {
        hls_concurrent_loop(Some(idle), headers).await;
        return;
    }
    HLS_REGISTRY.lock().evict_playlist(&key);
    hls_concurrent_complete(hls_segment, headers, status).await;
}

fn find_segment_by_uri(uri: &Url) -> Option<HlsSegment> {
    HLS_REGISTRY.lock().find_by_url(&uri.to_string())
}

fn concurrent_add(hls_segment: HlsSegment) {
    HLS_REGISTRY.lock().add_segment(hls_segment);
}

/// Clears in-memory HLS prefetch state (e.g. on plugin dispose).
pub fn clear_hls_registry() {
    *HLS_REGISTRY.lock() = HlsRegistry::default();
}

fn segment_to_task(
    segment: &HlsSegment,
    hls_key: &str,
    headers: Option<&HashMap<String, String>>,
) -> DownloadTask {
    let mut task = DownloadTask::new(to_safe_uri(&segment.url), None);
    task.hls_key = Some(hls_key.to_string());
    task.headers = headers.cloned();
    task.start_range = segment.start_range;
    task.end_range = segment.end_range;
    task
}

fn modify_m3u8_file(uri: &Url, data: &Bytes, hls_key: &str, config: &Config) -> String {
    let lines = read_lines_from_bytes(data);
    let uri_re = Regex::new(r#"URI="([^"]+)""#).unwrap();
    let byterange_re = Regex::new(r"#EXT-X-BYTERANGE:(\d+)(?:@(\d+))?").unwrap();
    let mut buffer = String::new();
    let mut last_line = String::new();
    let mut last_end_range: i64 = 0;

    for line in lines {
        let mut line_out = line.clone();
        let hls_line = line.trim();
        let mut parse_uri: Option<String> = None;

        if hls_line.starts_with("#EXT-X-KEY")
            || hls_line.starts_with("#EXT-X-MEDIA")
            || hls_line.starts_with("#EXT-X-MAP")
        {
            if let Some(caps) = uri_re.captures(hls_line) {
                if let Some(m) = caps.get(1) {
                    parse_uri = Some(to_safe_url(m.as_str()));
                    let new_uri = rewrite_uri(&parse_uri.as_ref().unwrap(), uri, config);
                    line_out = hls_line.replace(m.as_str(), &new_uri);
                }
            }
        }

        if last_line.starts_with("#EXTINF")
            || last_line.starts_with("#EXT-X-BYTERANGE")
            || last_line.starts_with("#EXT-X-STREAM-INF")
        {
            if !line.starts_with('#') {
                let safe = to_safe_url(&line);
                line_out = rewrite_uri(&safe, uri, config);
            }
        }

        if hls_line.starts_with("#EXT-X-KEY")
            || hls_line.starts_with("#EXT-X-MEDIA")
            || hls_line.starts_with("#EXT-X-MAP")
        {
            if let Some(mut parse_uri) = parse_uri.take() {
                if !parse_uri.starts_with("http") {
                    let (relative_path, resolved) = resolve_relative_path(&parse_uri);
                    parse_uri = format!("{}/{}", uri_path_prefix(uri, relative_path), resolved);
                }
                concurrent_add(HlsSegment::new(hls_key.to_string(), parse_uri));
            }
        }

        if last_line.starts_with("#EXTINF")
            || last_line.starts_with("#EXT-X-BYTERANGE")
            || last_line.starts_with("#EXT-X-STREAM-INF")
        {
            if !line.starts_with('#') {
                let mut hls_line_resolved = to_safe_url(&line);
                if !hls_line_resolved.starts_with("http") {
                    let (relative_path, mut resolved) = resolve_relative_path(&hls_line_resolved);
                    let prefix = format!("{}/", uri_path_prefix(uri, relative_path));
                    if resolved.starts_with('/') {
                        let split: Vec<&str> = resolved.split('/').collect();
                        let mut result = Vec::new();
                        for item in split {
                            if prefix.contains(item) {
                                continue;
                            }
                            result.push(item);
                        }
                        resolved = result.join("/");
                    }
                    hls_line_resolved = format!("{prefix}{resolved}");
                }

                let mut start_range = 0i64;
                let mut end_range: Option<i64> = None;
                if last_line.starts_with("#EXT-X-BYTERANGE") {
                    if let Some(caps) = byterange_re.captures(&last_line) {
                        let length: i64 = caps
                            .get(1)
                            .and_then(|m| m.as_str().parse().ok())
                            .unwrap_or(0);
                        if let Some(offset_m) = caps.get(2) {
                            start_range = offset_m.as_str().parse().unwrap_or(0);
                            end_range = if length == 0 {
                                None
                            } else {
                                Some(start_range + length - 1)
                            };
                        } else {
                            start_range = last_end_range;
                            end_range = if length == 0 {
                                None
                            } else {
                                Some(start_range + length - 1)
                            };
                            last_end_range = end_range.unwrap_or(0) + 1;
                        }
                    }
                }

                line_out = rewrite_uri(&hls_line_resolved, uri, config);
                concurrent_add(HlsSegment::with_range(
                    hls_key.to_string(),
                    hls_line_resolved,
                    start_range,
                    end_range,
                ));
            }
        }

        buffer.push_str(&line_out);
        buffer.push_str("\r\n");
        last_line = line;
    }
    buffer
}

fn rewrite_uri(input: &str, origin_uri: &Url, config: &Config) -> String {
    if input.starts_with("http") {
        to_local_url(input, config)
    } else {
        let origin_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(uri_base(origin_uri).as_bytes());
        if input.contains('?') {
            format!("{input}&origin={origin_b64}")
        } else {
            format!("{input}?origin={origin_b64}")
        }
    }
}

fn resolve_relative_path(path: &str) -> (usize, String) {
    let mut relative_path = 0usize;
    let mut hls_line = path.to_string();
    while hls_line.starts_with("../") {
        hls_line = hls_line[3..].to_string();
        relative_path += 1;
    }
    (relative_path, hls_line)
}

fn resolve_relative_url(base_uri: Option<&Url>, uri: &Url, segment_url: &str) -> String {
    let (relative_path, mut segment_url) = resolve_relative_path(segment_url);
    let prefix = format!(
        "{}/",
        base_uri
            .map(|u| uri_path_prefix(u, relative_path))
            .unwrap_or_else(|| uri_path_prefix(uri, relative_path))
    );
    if segment_url.starts_with('/') {
        let split: Vec<&str> = segment_url.split('/').collect();
        let mut result = Vec::new();
        for item in split {
            if prefix.contains(item) {
                continue;
            }
            result.push(item);
        }
        segment_url = result.join("/");
    }
    format!("{prefix}{segment_url}")
}

fn read_lines_from_bytes(data: &Bytes) -> Vec<String> {
    let mut lines = Vec::new();
    let mut buffer = String::new();
    let mut is_cr = false;
    for &byte in data.iter() {
        if byte == b'\n' {
            if is_cr {
                buffer.push('\r');
                is_cr = false;
            }
            lines.push(buffer.clone());
            buffer.clear();
        } else if byte == b'\r' {
            is_cr = true;
        } else {
            if is_cr {
                buffer.push('\r');
                is_cr = false;
            }
            buffer.push(byte as char);
        }
    }
    if !buffer.is_empty() || is_cr {
        if is_cr {
            buffer.push('\r');
        }
        lines.push(buffer);
    }
    lines
}

#[cfg(test)]
mod hls_registry_tests {
    use super::*;

    #[test]
    fn registry_groups_segments_by_playlist_key() {
        let mut registry = HlsRegistry::default();
        registry.add_segment(HlsSegment::new(
            "key_a".to_string(),
            "https://a/1.ts".to_string(),
        ));
        registry.add_segment(HlsSegment::new(
            "key_a".to_string(),
            "https://a/2.ts".to_string(),
        ));
        registry.add_segment(HlsSegment::new(
            "key_b".to_string(),
            "https://b/1.ts".to_string(),
        ));
        assert_eq!(registry.segments_for_key("key_a").len(), 2);
        assert_eq!(registry.segments_for_key("key_b").len(), 1);
        assert_eq!(registry.playlist_keys().len(), 2);
    }

    #[test]
    fn registry_evict_playlist_removes_key_state() {
        let mut registry = HlsRegistry::default();
        registry.add_segment(HlsSegment::new(
            "key_a".to_string(),
            "https://a/1.ts".to_string(),
        ));
        registry.set_latest("key_a", "https://a/1.ts");
        registry.evict_playlist("key_a");
        assert!(registry.playlist_keys().is_empty());
        assert!(registry.latest_for("key_a").is_none());
    }
}
