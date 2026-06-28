use std::collections::HashMap;

use crate::frb_generated::StreamSink;

use crate::cache::LruCacheSingleton;
use crate::parser::PrecacheProgress;
use crate::parser::video_caching::VideoCaching;

#[flutter_rust_bridge::frb]
#[derive(Debug, Clone)]
pub struct PrecacheProgressInfo {
    pub progress: f64,
    pub url: String,
    pub start_range: Option<i64>,
    pub end_range: Option<i64>,
    pub segment_url: Option<String>,
    pub parent_url: Option<String>,
    pub file_name: Option<String>,
    pub hls_key: Option<String>,
    pub total_segments: Option<i32>,
    pub current_segment_index: Option<i32>,
}

impl From<PrecacheProgress> for PrecacheProgressInfo {
    fn from(p: PrecacheProgress) -> Self {
        Self {
            progress: p.progress,
            url: p.url,
            start_range: p.start_range,
            end_range: p.end_range,
            segment_url: p.segment_url,
            parent_url: p.parent_url,
            file_name: p.file_name,
            hls_key: p.hls_key,
            total_segments: p.total_segments.map(|v| v as i32),
            current_segment_index: p.current_segment_index.map(|v| v as i32),
        }
    }
}

#[flutter_rust_bridge::frb]
#[derive(Debug, Clone)]
pub struct HlsMasterPlaylistInfo {
    pub media_playlist_urls: Vec<String>,
}

#[flutter_rust_bridge::frb]
pub async fn video_caching_precache(
    url: String,
    headers: Option<HashMap<String, String>>,
    cache_segments: i32,
    download_now: bool,
    progress_listen: bool,
    sink: Option<StreamSink<PrecacheProgressInfo>>,
) -> Result<(), String> {
    let _state = crate::proxy::require_state()?;
    let progress_tx = if progress_listen {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        if let Some(sink) = sink {
            tokio::spawn(async move {
                while let Some(p) = rx.recv().await {
                    if sink.add(PrecacheProgressInfo::from(p)).is_err() {
                        break;
                    }
                }
            });
        }
        Some(tx)
    } else {
        None
    };
    VideoCaching::precache(
        &url,
        headers,
        cache_segments.max(0) as usize,
        download_now,
        progress_tx,
    )
    .await
}

#[flutter_rust_bridge::frb]
pub async fn video_caching_is_cached(
    url: String,
    headers: Option<HashMap<String, String>>,
    cache_segments: i32,
) -> Result<bool, String> {
    let Some(state) = crate::proxy::video_proxy::VideoProxyState::get() else {
        return Ok(false);
    };
    if state.is_disposed() {
        return Ok(false);
    }
    Ok(VideoCaching::is_cached(&url, headers, cache_segments.max(0) as usize).await)
}

#[flutter_rust_bridge::frb]
pub async fn video_caching_parse_hls_master_playlist(
    url: String,
    headers: Option<HashMap<String, String>>,
) -> Result<Option<HlsMasterPlaylistInfo>, String> {
    let _state = crate::proxy::require_state()?;
    Ok(VideoCaching::parse_hls_master_playlist(&url, headers)
        .await
        .map(|m| HlsMasterPlaylistInfo {
            media_playlist_urls: m.media_playlist_urls,
        }))
}

#[flutter_rust_bridge::frb]
pub async fn lru_remove_cache_by_url(url: String, single_file: bool) -> Result<(), String> {
    LruCacheSingleton::instance()
        .remove_cache_by_url(&url, single_file)
        .await
}
