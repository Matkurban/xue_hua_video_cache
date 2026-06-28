use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use bytes::Bytes;
use parking_lot::Mutex;
use regex::Regex;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use url::Url;

use crate::cache::LruCacheSingleton;
use crate::download::DownloadTask;
use crate::ext::file_ext::FileExt;
use crate::ext::int_ext::to_memory_size;
use crate::ext::log_ext::{log_d, log_w};
use crate::ext::socket_ext::{append_string, append_to_writer};
use crate::ext::string_ext::{generate_md5, to_safe_uri};
use crate::global::Config;
use crate::proxy::require_state;

use super::download_wait::{CACHE_POLL_TIMEOUT, wait_for_cache, wait_for_task_completion};
use super::url_parser::PrecacheProgress;

const QUEUED_PRECACHE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Range-based parser behavior variant (default vs MP4-specific partial semantics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeParseMode {
    Default,
    Mp4,
}

pub struct RangeParserCommon;

impl RangeParserCommon {
    pub async fn cache(task: &DownloadTask) -> Option<Bytes> {
        let singleton = LruCacheSingleton::instance();
        let state = require_state().ok()?;
        let config = state.ctx.config.read().clone();
        let matcher = state.ctx.url_matcher.as_ref();
        let match_url = task.match_url(&config, matcher);

        if let Some(data) = singleton.memory_get(&match_url).await {
            log_d(&format!(
                "From memory: {}, total memory size: {}, Request range：{}-{:?}",
                to_memory_size(data.len() as i64),
                singleton.memory_format_size().await,
                task.start_range,
                task.end_range
            ));
            return Some(data);
        }
        if let Some(data) = singleton.storage_get(&match_url).await {
            log_d(&format!(
                "From file: {match_url} Request range：{}-{:?}",
                task.start_range, task.end_range
            ));
            singleton.memory_put(&match_url, data.clone()).await;
            return Some(data);
        }
        None
    }

    pub async fn download(task: Arc<Mutex<DownloadTask>>) -> Option<Bytes> {
        let state = require_state().ok()?;
        let config = state.ctx.config.read().clone();
        let matcher = state.ctx.url_matcher.as_ref();
        let match_url = task.lock().match_url(&config, matcher);
        let url = task.lock().url();
        log_d(&format!("From network: {url}"));

        {
            let cache_key = {
                let t = task.lock();
                task_cache_key(&t)
            };
            if let Ok(path) = FileExt::create_cache_path(Some(&cache_key)).await {
                task.lock().cache_dir = path;
            }
        }

        let dm = state.download_manager();
        let mut rx = dm.subscribe();
        dm.execute_task(task.clone()).await;

        wait_for_task_completion(&mut rx, |t| t.match_url(&config, matcher) == match_url).await
    }

    pub async fn push(task: Arc<Mutex<DownloadTask>>) -> Result<(), String> {
        let state = require_state()?;
        let config = state.ctx.config.read().clone();
        let matcher = state.ctx.url_matcher.as_ref();
        let match_url = task.lock().match_url(&config, matcher);
        let singleton = LruCacheSingleton::instance();
        if singleton.memory_get(&match_url).await.is_some() {
            return Ok(());
        }
        let md5_key = task_cache_key(&task.lock());
        let cache_path = FileExt::create_cache_path(Some(&md5_key))
            .await
            .map_err(|e| e.to_string())?;
        let save_path = task.lock().save_path(&config, matcher);
        if Path::new(&save_path).exists() {
            return Ok(());
        }
        {
            let mut t = task.lock();
            t.cache_dir = cache_path;
        }
        let dm = state.download_manager();
        dm.add_task(task.clone()).await;
        dm.round_task().await;
        Ok(())
    }

    pub async fn head(uri: &Url, headers: Option<&HashMap<String, String>>) -> i64 {
        let state = match require_state() {
            Ok(s) => s,
            Err(_) => return -1,
        };
        let config = state.ctx.config.read().clone();
        let mut request = state.ctx.http_client.head(uri.as_str());
        if let Some(h) = headers {
            for (k, v) in h {
                let kl = k.to_lowercase();
                if kl == "host" && v == &config.server_url() {
                    continue;
                }
                if kl == "range" {
                    continue;
                }
                request = request.header(k.as_str(), v.as_str());
            }
        }
        let response = match request.send().await {
            Ok(r) => r,
            Err(_) => return -1,
        };
        if let Some(content_range) = response.headers().get("content-range") {
            if let Ok(s) = content_range.to_str() {
                if let Some(caps) = Regex::new(r"bytes (\d+)-(\d+)/(\d+)")
                    .ok()
                    .and_then(|re| re.captures(s))
                {
                    if let Some(total) = caps.get(3) {
                        let total = total.as_str();
                        if !total.is_empty() && total != "0" {
                            if let Ok(n) = total.parse::<i64>() {
                                return n;
                            }
                        }
                    }
                }
            }
        }
        response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(-1)
    }

    pub async fn cache_content_length(task: &DownloadTask, content_length: i64) {
        let state = match require_state() {
            Ok(s) => s,
            Err(_) => return,
        };
        let config = state.ctx.config.read().clone();
        let matcher = state.ctx.url_matcher.as_ref();
        let md5_key = task_cache_key(task);
        let Ok(cache_path) = FileExt::create_cache_path(Some(&md5_key)).await else {
            return;
        };
        let save_name = task.save_file_name(&config, matcher);
        let file_path = format!("{cache_path}/{save_name}");
        if tokio::fs::write(&file_path, content_length.to_string())
            .await
            .is_err()
        {
            return;
        }
        let match_url = task.match_url(&config, matcher);
        LruCacheSingleton::instance()
            .storage_put(&match_url, Path::new(&file_path).to_path_buf())
            .await;
    }

    pub async fn concurrent(task: &DownloadTask, headers: &HashMap<String, String>) {
        let state = match require_state() {
            Ok(s) => s,
            Err(_) => return,
        };
        let config = state.ctx.config.read().clone();
        let segment_size = config.segment_size;
        let dm = state.download_manager();
        let pool = dm.pool();
        let matcher = state.ctx.url_matcher.as_ref();

        let mut new_task = task.clone();
        let url = new_task.url();
        let uri = to_safe_uri(&url);
        let content_length = Self::head(&uri, Some(headers)).await;
        const MAX_LOOKAHEAD_SKIPS: i32 = 128;
        let mut skipped = 0i32;
        let mut active_size = pool
            .task_list()
            .iter()
            .filter(|t| t.lock().url() == url)
            .count();

        while active_size < 2 {
            new_task.start_range += segment_size;
            if content_length > 0 && new_task.start_range >= content_length {
                break;
            }
            new_task.end_range = Some(new_task.start_range + segment_size * 2 - 1);
            new_task.headers = Some(headers.clone());

            let match_url = new_task.match_url(&config, matcher);
            let mut is_exit = pool
                .task_list()
                .iter()
                .any(|t| t.lock().match_url(&config, matcher) == match_url);
            if LruCacheSingleton::instance()
                .memory_get(&match_url)
                .await
                .is_some()
            {
                is_exit = true;
            }
            let md5_key = task_cache_key(&new_task);
            if let Ok(cache_path) = FileExt::create_cache_path(Some(&md5_key)).await {
                new_task.cache_dir = cache_path.clone();
                let save_path = format!(
                    "{}/{}",
                    cache_path,
                    new_task.save_file_name(&config, matcher)
                );
                if Path::new(&save_path).exists() {
                    is_exit = true;
                }
            }
            if is_exit {
                skipped += 1;
                if skipped >= MAX_LOOKAHEAD_SKIPS {
                    break;
                }
                continue;
            }
            skipped = 0;
            log_d(&format!(
                "Asynchronous download start： {} {}-{:?}",
                new_task.url(),
                new_task.start_range,
                new_task.end_range
            ));
            let arc = Arc::new(Mutex::new(new_task.clone()));
            dm.execute_task(arc).await;
            active_size = pool
                .task_list()
                .iter()
                .filter(|t| t.lock().url() == url)
                .count();
        }
    }

    pub async fn is_cached(
        url: &str,
        headers: Option<HashMap<String, String>>,
        mut cache_segments: usize,
    ) -> bool {
        let uri = to_safe_uri(url);
        let content_length = Self::head(&uri, headers.as_ref()).await;
        let segment_size = require_state()
            .map(|s| s.ctx.config.read().segment_size)
            .unwrap_or(Config::default().segment_size);

        if content_length > 0 {
            let total_segments = content_length / segment_size
                + if content_length % segment_size > 0 {
                    1
                } else {
                    0
                };
            if cache_segments > total_segments as usize {
                cache_segments = total_segments as usize;
            }
        }

        let mut count = 0;
        while count < cache_segments {
            let mut task = DownloadTask::new(uri.clone(), None);
            task.headers = headers.clone();
            task.start_range = segment_size * count as i64;
            task.end_range = Some(task.start_range + segment_size - 1);
            count += 1;
            if Self::cache(&task).await.is_none() {
                return false;
            }
        }
        true
    }

    pub async fn precache(
        url: &str,
        headers: Option<HashMap<String, String>>,
        mut cache_segments: usize,
        download_now: bool,
        progress_tx: Option<tokio::sync::mpsc::UnboundedSender<PrecacheProgress>>,
    ) -> Result<(), String> {
        let tx = progress_tx;

        let uri = to_safe_uri(url);
        let content_length = Self::head(&uri, headers.as_ref()).await;
        let state = require_state()?;
        let config = state.ctx.config.read().clone();
        let segment_size = config.segment_size;

        if content_length > 0 {
            let total_segments = content_length / segment_size
                + if content_length % segment_size > 0 {
                    1
                } else {
                    0
                };
            if cache_segments > total_segments as usize {
                cache_segments = total_segments as usize;
            }
        }

        if cache_segments == 0 {
            return Ok(());
        }

        let downloaded = Arc::new(Mutex::new(0usize));
        let total_size = cache_segments;
        let mut failures = 0usize;
        let mut handles = Vec::new();

        for count in 0..cache_segments {
            let mut task = DownloadTask::new(uri.clone(), None);
            task.headers = headers.clone();
            task.start_range = segment_size * count as i64;
            task.end_range = Some(task.start_range + segment_size - 1);
            let task_arc = Arc::new(Mutex::new(task));
            let tx_clone = tx.clone();
            let downloaded = downloaded.clone();

            if download_now {
                let snapshot = task_arc.lock().clone();
                let cached = Self::cache(&snapshot).await;
                if cached.is_some() {
                    let mut n = downloaded.lock();
                    *n += 1;
                    if let Some(ref sender) = tx_clone {
                        let t = task_arc.lock();
                        let _ = sender.send(PrecacheProgress {
                            progress: *n as f64 / total_size as f64,
                            url: t.url(),
                            start_range: Some(t.start_range),
                            end_range: t.end_range,
                            segment_url: None,
                            parent_url: None,
                            file_name: None,
                            hls_key: None,
                            total_segments: None,
                            current_segment_index: None,
                        });
                    }
                    continue;
                }
                handles.push(tokio::spawn(async move {
                    let ok = RangeParserCommon::download(task_arc.clone())
                        .await
                        .is_some();
                    let mut n = downloaded.lock();
                    *n += 1;
                    if let Some(sender) = tx_clone {
                        let t = task_arc.lock();
                        let _ = sender.send(PrecacheProgress {
                            progress: *n as f64 / total_size as f64,
                            url: t.url(),
                            start_range: Some(t.start_range),
                            end_range: t.end_range,
                            segment_url: None,
                            parent_url: None,
                            file_name: None,
                            hls_key: None,
                            total_segments: None,
                            current_segment_index: None,
                        });
                    }
                    ok
                }));
            } else {
                Self::push(task_arc).await?;
            }
        }

        if !download_now {
            let headers_for_wait = headers.clone();
            let deadline = tokio::time::Instant::now() + QUEUED_PRECACHE_TIMEOUT;
            while tokio::time::Instant::now() < deadline {
                if Self::is_cached(url, headers_for_wait.clone(), cache_segments).await {
                    return Ok(());
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
            return Err(format!("Queued precache timed out for {url}"));
        }
        for handle in handles {
            match handle.await {
                Ok(true) => {}
                Ok(false) => failures += 1,
                Err(_) => failures += 1,
            }
        }
        if failures > 0 {
            return Err(format!(
                "Failed to precache {failures} of {total_size} segment(s) for {url}"
            ));
        }

        Ok(())
    }

    pub async fn parse(
        mut stream: TcpStream,
        uri: Url,
        headers: HashMap<String, String>,
        mode: RangeParseMode,
    ) -> bool {
        let state = match require_state() {
            Ok(s) => s,
            Err(e) => {
                log_w(&format!("[RangeParserCommon] parse error: {e}"));
                return false;
            }
        };
        let label = match mode {
            RangeParseMode::Default => "UrlParserDefault",
            RangeParseMode::Mp4 => "UrlParserMp4",
        };

        let result = async {
            let range_re = Regex::new(r"bytes=(\d+)-(\d*)").unwrap();
            let range_header = headers.get("range").map(String::as_str).unwrap_or("");
            let range_match = range_re.captures(range_header);
            let request_range_start = range_match
                .as_ref()
                .and_then(|c| c.get(1))
                .and_then(|m| m.as_str().parse::<i64>().ok())
                .unwrap_or(0);
            let request_range_end = range_match
                .as_ref()
                .and_then(|c| c.get(2))
                .and_then(|m| m.as_str().parse::<i64>().ok())
                .unwrap_or(-1);

            let partial = match mode {
                RangeParseMode::Default => request_range_start > 0 || request_range_end > 0,
                RangeParseMode::Mp4 => range_match.is_some(),
            };

            let status_line = if partial {
                "HTTP/1.1 206 Partial Content"
            } else {
                "HTTP/1.1 200 OK"
            };
            let mut response_headers = vec![
                status_line.to_string(),
                "Accept-Ranges: bytes".to_string(),
                "Content-Type: video/mp4".to_string(),
            ];

            if state.ctx.platform.is_android() {
                Self::parse_android(
                    &mut stream,
                    &uri,
                    &mut response_headers,
                    request_range_start,
                    request_range_end,
                    &headers,
                    mode,
                )
                .await?;
            } else {
                Self::parse_ios(
                    &mut stream,
                    &uri,
                    &mut response_headers,
                    request_range_start,
                    request_range_end,
                    &headers,
                    mode,
                )
                .await?;
            }
            stream.flush().await.ok();
            Ok::<(), String>(())
        }
        .await;

        if let Err(ref e) = result {
            log_w(&format!("[{label}] ⚠ ⚠ ⚠ parse error: {e}"));
        }
        let _ = stream.shutdown().await;
        log_d("Connection closed\n");
        result.is_ok()
    }

    async fn parse_android(
        stream: &mut TcpStream,
        uri: &Url,
        response_headers: &mut Vec<String>,
        request_range_start: i64,
        _request_range_end: i64,
        headers: &HashMap<String, String>,
        mode: RangeParseMode,
    ) -> Result<(), String> {
        let state = require_state().map_err(|e| e)?;
        let config = state.ctx.config.read().clone();
        let segment_size = config.segment_size;

        let mut probe = DownloadTask::new(uri.clone(), None);
        probe.headers = Some(headers.clone());
        probe.start_range = 0;
        probe.end_range = Some(1);

        let mut content_length = 0i64;
        if let Some(data) = Self::cache(&probe).await {
            content_length = String::from_utf8_lossy(&data).parse().unwrap_or(0);
        }
        if content_length == 0 {
            content_length = Self::head(uri, Some(headers)).await;
            Self::cache_content_length(&probe, content_length).await;
        }

        let request_range_end = content_length - 1;
        response_headers.push(format!(
            "content-length: {}",
            content_length - request_range_start
        ));
        response_headers.push(format!(
            "content-range: bytes {request_range_start}-{request_range_end}/{content_length}"
        ));
        let header_block = response_headers.join("\r\n");
        if !append_string(stream, &header_block).await {
            return Err("write headers failed".into());
        }

        Self::serve_segments(
            stream,
            uri,
            headers,
            request_range_start,
            request_range_end,
            segment_size,
            mode,
        )
        .await
    }

    async fn parse_ios(
        stream: &mut TcpStream,
        uri: &Url,
        response_headers: &mut Vec<String>,
        request_range_start: i64,
        mut request_range_end: i64,
        headers: &HashMap<String, String>,
        mode: RangeParseMode,
    ) -> Result<(), String> {
        let state = require_state().map_err(|e| e)?;
        let config = state.ctx.config.read().clone();
        let segment_size = config.segment_size;

        if (request_range_start == 0 && request_range_end == 1) || request_range_end == -1 {
            let mut probe = DownloadTask::new(uri.clone(), None);
            probe.headers = Some(headers.clone());
            probe.start_range = 0;
            probe.end_range = Some(1);

            let mut content_length = 0i64;
            if let Some(data) = Self::cache(&probe).await {
                content_length = String::from_utf8_lossy(&data).parse().unwrap_or(0);
            }
            if content_length == 0 {
                content_length = Self::head(uri, Some(headers)).await;
                Self::cache_content_length(&probe, content_length).await;
            }

            if request_range_start == 0 && request_range_end == 1 {
                response_headers.push(format!("content-range: bytes 0-1/{content_length}"));
                let header_block = response_headers.join("\r\n");
                if !append_string(stream, &header_block).await {
                    return Err("write headers failed".into());
                }
                let _ = append_to_writer(stream, &[0]).await;
                return Ok(());
            } else if request_range_end == -1 {
                request_range_end = content_length - 1;
            }
        }

        let content_length = request_range_end - request_range_start + 1;
        response_headers.push(format!("content-length: {content_length}"));
        if mode == RangeParseMode::Mp4 {
            response_headers.push(format!(
                "content-range: bytes {request_range_start}-{request_range_end}/{}",
                request_range_end + 1
            ));
        }
        let header_block = response_headers.join("\r\n");
        if !append_string(stream, &header_block).await {
            return Err("write headers failed".into());
        }

        Self::serve_segments(
            stream,
            uri,
            headers,
            request_range_start,
            request_range_end,
            segment_size,
            mode,
        )
        .await
    }

    async fn serve_segments(
        stream: &mut TcpStream,
        uri: &Url,
        headers: &HashMap<String, String>,
        request_range_start: i64,
        request_range_end: i64,
        segment_size: i64,
        mode: RangeParseMode,
    ) -> Result<(), String> {
        let state = require_state().map_err(|e| e)?;
        let dm = state.download_manager();

        let mut downloading = true;
        let mut start_range = request_range_start - (request_range_start % segment_size);
        let mut end_range = start_range + segment_size - 1;
        let mut retry = 3;

        while downloading {
            if mode == RangeParseMode::Mp4 && end_range > request_range_end {
                end_range = request_range_end;
            }

            let mut task = DownloadTask::new(uri.clone(), None);
            task.headers = Some(headers.clone());
            task.start_range = start_range;
            task.end_range = Some(end_range);
            log_d(&format!(
                "Start {} Request range：{}-{}",
                task.url(),
                task.start_range,
                task.end_range.unwrap_or(-1)
            ));

            let mut data = Self::cache(&task).await;
            if data.is_none() && dm.is_task_exist(&task) {
                data = wait_for_cache(|| Self::cache(&task), CACHE_POLL_TIMEOUT).await;
            }
            if data.is_none() {
                Self::concurrent(&task, headers).await;
                let arc = Arc::new(Mutex::new(task.clone()));
                {
                    arc.lock().priority += 2;
                }
                data = Self::download(arc).await;
            }
            if data.is_none() {
                retry -= 1;
                if retry == 0 {
                    break;
                }
                continue;
            }

            let slice = data.unwrap();
            let mut start_index = 0usize;
            let mut end_index = slice.len();
            if start_range < request_range_start {
                start_index = (request_range_start - start_range) as usize;
            }
            if end_range > request_range_end {
                end_index = (request_range_end - start_range + 1) as usize;
            }
            let chunk = if start_index > 0 || end_index < slice.len() {
                slice.slice(start_index..end_index)
            } else {
                slice
            };

            if !append_to_writer(stream, &chunk).await {
                downloading = false;
            }
            start_range += segment_size;
            end_range = start_range + segment_size - 1;
            if start_range > request_range_end {
                downloading = false;
            }
        }
        Ok(())
    }
}

fn task_cache_key(task: &DownloadTask) -> String {
    if let Some(ref key) = task.hls_key {
        key.clone()
    } else {
        generate_md5(&task.uri.to_string())
    }
}
