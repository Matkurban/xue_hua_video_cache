use crate::cache::cache_key::{CacheKey, CacheKeyContext};
use crate::download::DownloadTask;
use crate::ext::file_ext::FileExt;
use crate::ext::string_ext::{generate_md5, to_safe_uri};
use crate::global::Config;
use crate::matchers::UrlMatcher;
use crate::parser::hls_registry::{
    prefetch_evict_playlist, query_segment_by_url, query_segments_for_playlist,
};
use crate::parser::segment_resolver::SegmentResolver;

use super::LruCacheSingleton;

pub async fn remove_cache_by_url(
    cache: &LruCacheSingleton,
    url: &str,
    single_file: bool,
    config: &Config,
    matcher: &dyn UrlMatcher,
) -> Result<(), String> {
    cache.storage_size_in_bytes().await;
    let dir_key = generate_md5(url);

    if single_file {
        remove_single_entry(cache, url, config, matcher).await;
        return Ok(());
    }

    let ctx = CacheKeyContext::new(config.clone(), matcher);
    let segments = query_segments_for_playlist(&dir_key);
    for segment in segments {
        let match_key = segment_entry_key(&segment, &ctx);
        cache.storage_remove(&match_key).await;
        cache.memory_remove(&match_key).await;
    }
    prefetch_evict_playlist(&dir_key);

    let root = FileExt::create_cache_path(None)
        .await
        .map_err(|e| e.to_string())?;
    let mut entries = tokio::fs::read_dir(&root)
        .await
        .map_err(|e| e.to_string())?;
    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().and_then(|n| n.to_str()) == Some(&dir_key) {
                remove_dir_recursive(cache, &path).await;
            }
        }
    }
    Ok(())
}

async fn remove_single_entry(
    cache: &LruCacheSingleton,
    url: &str,
    config: &Config,
    matcher: &dyn UrlMatcher,
) {
    let ctx = CacheKeyContext::new(config.clone(), matcher);
    let uri = to_safe_uri(url);
    let mut task = DownloadTask::new(uri.clone(), None);
    let dir_key = generate_md5(url);

    let full_key = CacheKey::for_task(&task, &ctx).entry;
    let content_length =
        SegmentResolver::read_content_length_from_cache(cache, &task, &ctx).await;

    purge_cache_entry(cache, &full_key).await;

    let metadata_key = CacheKey::for_content_length(&task, &ctx).entry;
    purge_cache_entry(cache, &metadata_key).await;

    let segment_size = config.segment_size;
    if let Some(len) = content_length {
        let total_segments = len / segment_size
            + if len % segment_size > 0 { 1 } else { 0 };
        for count in 0..total_segments {
            task.start_range = segment_size * count;
            task.end_range = Some(task.start_range + segment_size - 1);
            let segment_key = CacheKey::for_task(&task, &ctx).entry;
            purge_cache_entry(cache, &segment_key).await;
        }
    }

    task.start_range = 0;
    task.end_range = Some(1);
    let legacy_probe_key = CacheKey::for_task(&task, &ctx).entry;
    purge_cache_entry(cache, &legacy_probe_key).await;

    if full_key != dir_key {
        purge_cache_entry(cache, &dir_key).await;
    }

    if let Some(segment) = query_segment_by_url(url) {
        let segment_key = segment_entry_key(&segment, &ctx);
        if segment_key != full_key {
            purge_cache_entry(cache, &segment_key).await;
        }
    }

    if let Ok(root) = FileExt::create_cache_path(None).await {
        let dir_path = std::path::PathBuf::from(root).join(&dir_key);
        if dir_path.is_dir() {
            remove_dir_recursive(cache, &dir_path).await;
        }
    }
}

async fn purge_cache_entry(cache: &LruCacheSingleton, entry: &str) {
    cache.storage_remove(entry).await;
    cache.memory_remove(entry).await;
}

fn segment_entry_key(segment: &crate::parser::HlsSegment, ctx: &CacheKeyContext<'_>) -> String {
    let mut task = DownloadTask::new(to_safe_uri(&segment.url), None);
    task.hls_key = Some(segment.key.clone());
    task.start_range = segment.start_range;
    task.end_range = segment.end_range;
    CacheKey::for_task(&task, ctx).entry
}

async fn remove_dir_recursive(cache: &LruCacheSingleton, dir: &std::path::PathBuf) {
    let mut stack = vec![dir.clone()];
    while let Some(d) = stack.pop() {
        if let Ok(mut entries) = tokio::fs::read_dir(&d).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    cache.storage_remove(stem).await;
                    cache.memory_remove(stem).await;
                }
            }
        }
        let _ = tokio::fs::remove_dir_all(&d).await;
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use crate::cache::LruCacheSingleton;
    use crate::cache::cache_key::{CacheKey, CacheKeyContext};
    use crate::download::DownloadTask;
    use crate::global::{CacheKeyConfig, Config};
    use crate::matchers::UrlMatcherConfigurable;

    use crate::test_urls::SAMPLE_MP4;

    use std::sync::{Mutex, MutexGuard};

    use super::*;

    fn cache_test_lock() -> MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn test_matcher() -> UrlMatcherConfigurable {
        UrlMatcherConfigurable::new(&CacheKeyConfig::default())
    }

    #[tokio::test]
    async fn remove_single_entry_uses_entry_key() {
        let _guard = cache_test_lock();
        let cache = LruCacheSingleton::instance();
        cache.memory_clear().await;
        let config = Config::default();
        let matcher = test_matcher();
        let ctx = CacheKeyContext::new(config.clone(), &matcher);
        let url = SAMPLE_MP4;
        let task = DownloadTask::new(to_safe_uri(url), None);
        let key = CacheKey::for_task(&task, &ctx).entry;
        cache.memory_put(&key, Bytes::from_static(b"cached")).await;
        remove_single_entry(&cache, url, &config, &matcher).await;
        assert!(cache.memory_get(&key).await.is_none());
    }

    #[tokio::test]
    async fn remove_single_entry_clears_mp4_range_segments() {
        let _guard = cache_test_lock();
        let cache = LruCacheSingleton::instance();
        cache.memory_clear().await;
        let config = Config::default();
        let matcher = test_matcher();
        let ctx = CacheKeyContext::new(config.clone(), &matcher);
        let url = "https://example.com/remove-ranges.mp4";
        let uri = to_safe_uri(url);
        let mut task = DownloadTask::new(uri, None);

        let full_key = CacheKey::for_task(&task, &ctx).entry;
        cache
            .memory_put(&full_key, Bytes::from_static(b"full"))
            .await;

        let metadata_key = CacheKey::for_content_length(&task, &ctx).entry;
        cache
            .memory_put(
                &metadata_key,
                Bytes::from(format!("{}", config.segment_size * 2)),
            )
            .await;

        for count in 0..2 {
            task.start_range = config.segment_size * count;
            task.end_range = Some(task.start_range + config.segment_size - 1);
            let segment_key = CacheKey::for_task(&task, &ctx).entry;
            cache
                .memory_put(&segment_key, Bytes::from_static(b"segment"))
                .await;
        }

        remove_single_entry(&cache, url, &config, &matcher).await;

        assert!(cache.memory_get(&full_key).await.is_none());
        assert!(cache.memory_get(&metadata_key).await.is_none());
        task.start_range = 0;
        task.end_range = Some(config.segment_size - 1);
        let seg0 = CacheKey::for_task(&task, &ctx).entry;
        assert!(cache.memory_get(&seg0).await.is_none());
        task.start_range = config.segment_size;
        task.end_range = Some(config.segment_size * 2 - 1);
        let seg1 = CacheKey::for_task(&task, &ctx).entry;
        assert!(cache.memory_get(&seg1).await.is_none());
    }
}
