use crate::cache::cache_key::{CacheKey, CacheKeyContext};
use crate::download::DownloadTask;
use crate::ext::file_ext::FileExt;
use crate::ext::string_ext::{generate_md5, to_safe_uri};
use crate::global::Config;
use crate::matchers::UrlMatcher;
use crate::parser::hls_registry::{
    prefetch_evict_playlist, query_segment_by_url, query_segments_for_playlist,
};

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
    let task = DownloadTask::new(uri, None);
    let match_key = CacheKey::for_task(&task, &ctx).entry;
    cache.storage_remove(&match_key).await;
    cache.memory_remove(&match_key).await;

    let dir_key = generate_md5(url);
    if match_key != dir_key {
        cache.storage_remove(&dir_key).await;
        cache.memory_remove(&dir_key).await;
    }

    if let Some(segment) = query_segment_by_url(url) {
        let segment_key = segment_entry_key(&segment, &ctx);
        if segment_key != match_key {
            cache.storage_remove(&segment_key).await;
            cache.memory_remove(&segment_key).await;
        }
    }
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

    use super::*;

    fn test_matcher() -> UrlMatcherConfigurable {
        UrlMatcherConfigurable::new(&CacheKeyConfig::default())
    }

    #[tokio::test]
    async fn remove_single_entry_uses_entry_key() {
        let cache = LruCacheSingleton::instance();
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
}
