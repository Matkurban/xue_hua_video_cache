use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use bytes::Bytes;
use parking_lot::Mutex;
use url::Url;

use crate::cache::cache_key::{CacheKey, CacheKeyContext};
use crate::download::DownloadTask;
use crate::ext::file_ext::FileExt;
use crate::ext::log_ext::log_d;
use crate::ext::string_ext::to_safe_uri;
use crate::ext::uri_ext::{uri_base, uri_generate_md5, uri_path_prefix};
use crate::proxy::ProxyRuntime;

use super::download_wait::wait_for_task_completion;
use super::hls_parser::{HlsMediaPlaylist, HlsPlaylist, parse_playlist};
use super::hls_playlist_rewriter::segments_from_playlist_bytes;
use super::hls_registry::HlsSegment;
use super::segment_resolver::SegmentResolver;

pub(crate) struct HlsPlaylistResolver {
    runtime: Arc<ProxyRuntime>,
}

impl HlsPlaylistResolver {
    pub(crate) fn new(runtime: Arc<ProxyRuntime>) -> Self {
        Self { runtime }
    }

    pub(crate) async fn parse_playlist(
        &self,
        uri: &Url,
        headers: Option<&HashMap<String, String>>,
        hls_key: Option<&str>,
    ) -> Option<HlsPlaylist> {
        let key = hls_key.unwrap_or(&uri_generate_md5(uri)).to_string();
        let data = self.fetch_playlist_bytes(uri, headers, &key).await?;
        let lines = read_lines_from_bytes(&data);
        parse_playlist(uri, &lines)
    }

    async fn fetch_playlist_bytes(
        &self,
        uri: &Url,
        headers: Option<&HashMap<String, String>>,
        hls_key: &str,
    ) -> Option<Bytes> {
        let mut task = DownloadTask::new(uri.clone(), None);
        task.headers = headers.cloned();
        task.hls_key = Some(hls_key.to_string());
        if let Some(cached) = SegmentResolver::resolve(&self.runtime, &task).await {
            return Some(cached);
        }
        hls_download(&self.runtime, Arc::new(Mutex::new(task))).await
    }

    async fn resolve_media_playlist_uri(
        &self,
        uri: &Url,
        headers: Option<&HashMap<String, String>>,
        hls_key: &str,
    ) -> Option<Url> {
        match self.parse_playlist(uri, headers, Some(hls_key)).await? {
            HlsPlaylist::Master(master) => {
                for media_url in master.media_playlist_urls {
                    let media_uri = to_safe_uri(&format!("{}{}", uri_base(uri), media_url));
                    if let Some(HlsPlaylist::Media(_)) = self
                        .parse_playlist(&media_uri, headers, Some(hls_key))
                        .await
                    {
                        return Some(media_uri);
                    }
                }
                None
            }
            HlsPlaylist::Media(_) => Some(uri.clone()),
        }
    }

    pub(crate) async fn parse_media_playlist(
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

    pub(crate) async fn parse_segment(
        &self,
        uri: &Url,
        headers: Option<&HashMap<String, String>>,
    ) -> Vec<HlsSegment> {
        let hls_key = uri_generate_md5(uri);
        let Some(media_uri) = self
            .resolve_media_playlist_uri(uri, headers, &hls_key)
            .await
        else {
            return Vec::new();
        };
        let Some(data) = self
            .fetch_playlist_bytes(&media_uri, headers, &hls_key)
            .await
        else {
            return Vec::new();
        };
        let config = self.runtime.ctx.config.read().clone();
        segments_from_playlist_bytes(&media_uri, &data, &hls_key, &config)
    }
}

pub(crate) async fn hls_download(
    runtime: &Arc<ProxyRuntime>,
    task: Arc<Mutex<DownloadTask>>,
) -> Option<Bytes> {
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

    let dm = runtime.downloads();
    let mut rx = dm.subscribe();
    dm.submit(task.clone()).await;

    let ctx = CacheKeyContext::from_runtime(runtime);
    let entry = CacheKey::for_task(&task.lock(), &ctx).entry;
    wait_for_task_completion(runtime, &mut rx, |t| ctx.entry_matches(t, &entry)).await
}

pub(crate) async fn hls_push_task(
    runtime: &Arc<ProxyRuntime>,
    task: Arc<Mutex<DownloadTask>>,
) -> Result<(), String> {
    let ctx = CacheKeyContext::from_runtime(runtime);
    let key = CacheKey::for_task(&task.lock(), &ctx);
    if runtime.cache.memory_get(&key.entry).await.is_some() {
        return Ok(());
    }
    let cache_path = FileExt::create_cache_path(Some(&key.directory))
        .await
        .map_err(|e| e.to_string())?;
    {
        let mut t = task.lock();
        t.cache_dir = cache_path;
    }
    let save_path = CacheKey::for_task(&task.lock(), &ctx).save_path(&task.lock());
    if Path::new(&save_path).exists() {
        return Ok(());
    }
    let dm = runtime.downloads();
    dm.submit(task).await;
    Ok(())
}

pub(crate) fn read_lines_from_bytes(data: &Bytes) -> Vec<String> {
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

pub(crate) fn resolve_relative_path(path: &str) -> (usize, String) {
    let mut relative_path = 0usize;
    let mut hls_line = path.to_string();
    while hls_line.starts_with("../") {
        hls_line = hls_line[3..].to_string();
        relative_path += 1;
    }
    (relative_path, hls_line)
}

/// Resolve a segment URL against a playlist base (used in resolver tests; segment lists use rewriter).
#[allow(dead_code)]
pub(crate) fn resolve_relative_url(base_uri: Option<&Url>, uri: &Url, segment_url: &str) -> String {
    let (relative_path, segment_path) = resolve_relative_path(segment_url);
    let base = base_uri.unwrap_or(uri);
    let trimmed = segment_path.trim_start_matches('/');

    if segment_path.starts_with('/') {
        let origin = uri_base(base);
        if let Ok(origin_url) = Url::parse(&format!("{origin}/")) {
            if let Ok(resolved) = origin_url.join(trimmed) {
                return resolved.to_string();
            }
        }
        return format!("{origin}/{trimmed}");
    }

    let prefix = uri_path_prefix(base, relative_path);
    if let Ok(base_url) = Url::parse(&format!("{prefix}/")) {
        if let Ok(resolved) = base_url.join(trimmed) {
            return resolved.to_string();
        }
    }
    format!("{prefix}/{trimmed}")
}

pub(crate) fn segment_to_task(
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

#[cfg(test)]
mod tests {
    use url::Url;

    use super::*;

    #[test]
    fn resolve_relative_url_joins_absolute_path_without_substring_stripping() {
        let uri = Url::parse("https://cdn.example.com/videos/v1/master.m3u8").unwrap();
        let resolved = resolve_relative_url(None, &uri, "/videos/v1/segment.ts");
        assert_eq!(resolved, "https://cdn.example.com/videos/v1/segment.ts");
    }

    #[test]
    fn resolve_relative_url_resolves_parent_relative_path() {
        let uri = Url::parse("https://cdn.example.com/videos/v1/master.m3u8").unwrap();
        let resolved = resolve_relative_url(None, &uri, "../v2/segment.ts");
        assert_eq!(resolved, "https://cdn.example.com/videos/v2/segment.ts");
    }
}
