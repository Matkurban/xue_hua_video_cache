use std::collections::HashMap;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use url::Url;

use crate::download::DownloadStatus;

pub(crate) const MAX_HLS_PLAYLIST_KEYS: usize = 8;

static HLS_REGISTRY: Lazy<Mutex<HlsRegistry>> = Lazy::new(|| Mutex::new(HlsRegistry::default()));

pub(crate) struct HlsRegistry {
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
    pub(crate) fn playlist_keys(&self) -> Vec<String> {
        self.playlists.keys().cloned().collect()
    }

    pub(crate) fn set_latest(&mut self, key: &str, url: &str) {
        self.latest_url.insert(key.to_string(), url.to_string());
    }

    pub(crate) fn latest_for(&self, key: &str) -> Option<String> {
        self.latest_url.get(key).cloned()
    }

    pub(crate) fn add_segment(&mut self, segment: HlsSegment) {
        let list = self.playlists.entry(segment.key.clone()).or_default();
        if !list.iter().any(|e| e.url == segment.url) {
            list.push(segment);
        }
    }

    pub(crate) fn register_segments(
        &mut self,
        key: &str,
        segments: impl IntoIterator<Item = HlsSegment>,
    ) {
        for segment in segments {
            let mut segment = segment;
            if segment.key != key {
                segment.key = key.to_string();
            }
            self.add_segment(segment);
        }
    }

    pub(crate) fn find_by_url(&self, url: &str) -> Option<HlsSegment> {
        self.playlists
            .values()
            .flatten()
            .find(|s| s.url == url)
            .cloned()
    }

    pub(crate) fn segments_for_key(&self, key: &str) -> Vec<HlsSegment> {
        self.playlists.get(key).cloned().unwrap_or_default()
    }

    pub(crate) fn update_status(&mut self, url: &str, status: DownloadStatus) {
        for list in self.playlists.values_mut() {
            if let Some(seg) = list.iter_mut().find(|e| e.url == url) {
                seg.status = status;
                return;
            }
        }
    }

    pub(crate) fn downloading_count(&self, key: &str) -> usize {
        self.playlists
            .get(key)
            .map(|list| {
                list.iter()
                    .filter(|e| e.status == DownloadStatus::Downloading)
                    .count()
            })
            .unwrap_or(0)
    }

    pub(crate) fn find_idle(&self, key: &str) -> Option<HlsSegment> {
        self.playlists
            .get(key)?
            .iter()
            .find(|e| e.status == DownloadStatus::Idle)
            .cloned()
    }

    pub(crate) fn evict_playlist(&mut self, key: &str) {
        self.playlists.remove(key);
        self.latest_url.remove(key);
    }

    /// Playlist key to evict when over capacity, excluding `keep_key`.
    pub(crate) fn eviction_candidate(&self, keep_key: &str) -> Option<String> {
        let keys = self.playlist_keys();
        if keys.len() <= MAX_HLS_PLAYLIST_KEYS {
            return None;
        }
        keys.iter()
            .find(|k| *k != keep_key)
            .cloned()
            .or_else(|| keys.first().cloned())
    }

    /// Active segment for prefetch: latest URL in playlist, falling back to `hint`.
    pub(crate) fn active_segment(&self, hint: &HlsSegment) -> Option<HlsSegment> {
        let latest_url = self
            .latest_for(&hint.key)
            .unwrap_or_else(|| hint.url.clone());
        self.find_by_url(&latest_url)
    }

    /// Next segment after `current` within the same playlist key.
    pub(crate) fn next_segment_after(&self, current: &HlsSegment) -> Option<HlsSegment> {
        let same_key = self.segments_for_key(&current.key);
        let idx = same_key.iter().position(|e| e.url == current.url)?;
        same_key.get(idx + 1).cloned()
    }

    /// Next segment after the current latest playback position in a playlist.
    pub(crate) fn next_prefetch_segment(&self, playlist_key: &str) -> Option<HlsSegment> {
        let latest_url = self.latest_for(playlist_key)?;
        let latest = self.find_by_url(&latest_url)?;
        self.next_segment_after(&latest)
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

fn lock_registry() -> parking_lot::MutexGuard<'static, HlsRegistry> {
    HLS_REGISTRY.lock()
}

pub(crate) fn find_segment_by_uri(uri: &Url) -> Option<HlsSegment> {
    lock_registry().find_by_url(&uri.to_string())
}

pub(crate) fn query_segments_for_playlist(key: &str) -> Vec<HlsSegment> {
    lock_registry().segments_for_key(key)
}

pub(crate) fn query_segment_by_url(url: &str) -> Option<HlsSegment> {
    lock_registry().find_by_url(url)
}

pub(crate) fn query_active_segment(hint: &HlsSegment) -> Option<HlsSegment> {
    lock_registry().active_segment(hint)
}

pub(crate) fn query_downloading_count(key: &str) -> usize {
    lock_registry().downloading_count(key)
}

pub(crate) fn query_next_prefetch_segment(playlist_key: &str) -> Option<HlsSegment> {
    lock_registry().next_prefetch_segment(playlist_key)
}

pub(crate) fn query_idle_segment(key: &str) -> Option<HlsSegment> {
    lock_registry().find_idle(key)
}

pub(crate) fn query_playlist_keys() -> Vec<String> {
    lock_registry().playlist_keys()
}

/// Mark playback position and evict overflow playlist if needed. Returns URLs removed from registry.
pub(crate) fn prefetch_begin(segment: &HlsSegment) -> Vec<String> {
    let mut registry = lock_registry();
    registry.set_latest(&segment.key, &segment.url);
    let Some(evict_key) = registry.eviction_candidate(&segment.key) else {
        return Vec::new();
    };
    let urls: Vec<String> = registry
        .segments_for_key(&evict_key)
        .into_iter()
        .map(|e| e.url)
        .collect();
    registry.evict_playlist(&evict_key);
    urls
}

pub(crate) fn prefetch_mark_status(url: &str, status: DownloadStatus) {
    lock_registry().update_status(url, status);
}

pub(crate) fn prefetch_evict_playlist(key: &str) {
    lock_registry().evict_playlist(key);
}

pub(crate) fn register_playlist_segments(
    key: &str,
    segments: impl IntoIterator<Item = HlsSegment>,
) {
    lock_registry().register_segments(key, segments);
}

/// Clears in-memory HLS prefetch state (e.g. on plugin dispose).
pub(crate) fn clear_hls_registry() {
    *lock_registry() = HlsRegistry::default();
}

#[cfg(test)]
mod tests {
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
    fn register_playlist_segments_merges_and_dedupes_by_url() {
        let mut registry = HlsRegistry::default();
        registry.register_segments(
            "key_a",
            vec![
                HlsSegment::new("key_a".to_string(), "https://a/1.ts".to_string()),
                HlsSegment::new("key_a".to_string(), "https://a/2.ts".to_string()),
            ],
        );
        registry.register_segments(
            "key_a",
            vec![
                HlsSegment::new("key_a".to_string(), "https://a/2.ts".to_string()),
                HlsSegment::new("key_a".to_string(), "https://a/3.ts".to_string()),
            ],
        );
        assert_eq!(registry.segments_for_key("key_a").len(), 3);
        let urls: Vec<_> = registry
            .segments_for_key("key_a")
            .into_iter()
            .map(|s| s.url)
            .collect();
        assert_eq!(
            urls,
            vec![
                "https://a/1.ts".to_string(),
                "https://a/2.ts".to_string(),
                "https://a/3.ts".to_string(),
            ]
        );
    }

    #[test]
    fn prefetch_begin_evicts_when_over_capacity() {
        let mut registry = HlsRegistry::default();
        for i in 0..=MAX_HLS_PLAYLIST_KEYS {
            registry.add_segment(HlsSegment::new(
                format!("key_{i}"),
                format!("https://a/{i}.ts"),
            ));
            registry.set_latest(&format!("key_{i}"), &format!("https://a/{i}.ts"));
        }
        let hint = HlsSegment::new("new_key".to_string(), "https://a/new.ts".to_string());
        registry.set_latest(&hint.key, &hint.url);
        let evict = registry.eviction_candidate(&hint.key).unwrap();
        let urls: Vec<_> = registry
            .segments_for_key(&evict)
            .into_iter()
            .map(|s| s.url)
            .collect();
        registry.evict_playlist(&evict);
        assert!(!urls.is_empty());
        assert!(registry.playlist_keys().len() <= MAX_HLS_PLAYLIST_KEYS);
    }

    #[test]
    fn next_prefetch_segment_returns_following_entry() {
        let mut registry = HlsRegistry::default();
        registry.add_segment(HlsSegment::new(
            "k".to_string(),
            "https://a/1.ts".to_string(),
        ));
        registry.add_segment(HlsSegment::new(
            "k".to_string(),
            "https://a/2.ts".to_string(),
        ));
        registry.set_latest("k", "https://a/1.ts");
        let next = registry.next_prefetch_segment("k").unwrap();
        assert_eq!(next.url, "https://a/2.ts");
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
