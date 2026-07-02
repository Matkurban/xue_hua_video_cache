use std::path::Path;
use std::sync::Arc;

use bytes::Bytes;

use crate::cache::cache_key::{CacheKey, CacheKeyContext};
use crate::cache::LruCacheSingleton;
use crate::download::DownloadTask;
use crate::ext::file_ext::FileExt;
use crate::ext::int_ext::to_memory_size;
use crate::ext::log_ext::log_d;
use crate::proxy::ProxyRuntime;

pub(crate) struct SegmentResolver;

impl SegmentResolver {
    pub(crate) async fn resolve(runtime: &Arc<ProxyRuntime>, task: &DownloadTask) -> Option<Bytes> {
        let ctx = CacheKeyContext::from_runtime(runtime);
        let key = CacheKey::for_task(task, &ctx);
        read_bytes_from_cache(&runtime.cache, &key.entry).await
    }

    pub(crate) async fn store_content_length(
        runtime: &Arc<ProxyRuntime>,
        task: &DownloadTask,
        content_length: i64,
    ) {
        let ctx = CacheKeyContext::from_runtime(runtime);
        let key = CacheKey::for_content_length(task, &ctx);
        let payload = Bytes::from(content_length.to_string());
        runtime.cache.memory_put(&key.entry, payload).await;
        let Ok(cache_path) = FileExt::create_cache_path(Some(&key.directory)).await else {
            return;
        };
        let file_path = format!("{cache_path}/{}", key.content_length_file_name());
        if tokio::fs::write(&file_path, content_length.to_string())
            .await
            .is_err()
        {
            return;
        }
        runtime
            .cache
            .storage_put(&key.entry, Path::new(&file_path).to_path_buf())
            .await;
    }

    pub(crate) async fn read_content_length(
        runtime: &Arc<ProxyRuntime>,
        task: &DownloadTask,
    ) -> Option<i64> {
        let ctx = CacheKeyContext::from_runtime(runtime);
        Self::read_content_length_from_cache(&runtime.cache, task, &ctx).await
    }

    pub(crate) async fn read_content_length_from_cache(
        cache: &LruCacheSingleton,
        task: &DownloadTask,
        ctx: &CacheKeyContext<'_>,
    ) -> Option<i64> {
        let key = CacheKey::for_content_length(task, ctx);
        if let Some(len) = read_bytes_from_cache(cache, &key.entry)
            .await
            .and_then(|data| parse_content_length_bytes(&data))
        {
            return Some(len);
        }

        let mut legacy = task.clone();
        legacy.start_range = 0;
        legacy.end_range = Some(1);
        let legacy_key = CacheKey::for_task(&legacy, ctx);
        let Some(data) = read_bytes_from_cache(cache, &legacy_key.entry).await else {
            return None;
        };
        let len = parse_legacy_metadata_bytes(&data)?;
        purge_cache_entry(cache, &legacy_key.entry).await;
        let metadata_key = CacheKey::for_content_length(task, ctx);
        cache
            .memory_put(
                &metadata_key.entry,
                Bytes::from(len.to_string()),
            )
            .await;
        Some(len)
    }
}

async fn read_bytes_from_cache(cache: &LruCacheSingleton, entry: &str) -> Option<Bytes> {
    if let Some(data) = cache.memory_get(entry).await {
        log_d(&format!(
            "From memory: {}, total memory size: {}",
            to_memory_size(data.len() as i64),
            cache.memory_format_size().await
        ));
        return Some(data);
    }
    if let Some(data) = cache.storage_get(entry).await {
        log_d(&format!("From file: {entry}"));
        cache.memory_put(entry, data.clone()).await;
        return Some(data);
    }
    None
}

async fn purge_cache_entry(cache: &LruCacheSingleton, entry: &str) {
    cache.storage_remove(entry).await;
    cache.memory_remove(entry).await;
}

fn parse_content_length_bytes(data: &Bytes) -> Option<i64> {
    let len = String::from_utf8_lossy(data).parse::<i64>().ok()?;
    if len > 0 { Some(len) } else { None }
}

fn parse_legacy_metadata_bytes(data: &Bytes) -> Option<i64> {
    if data.len() >= 20 {
        return None;
    }
    let text = std::str::from_utf8(data).ok()?;
    if !text.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    text.parse::<i64>().ok().filter(|&n| n > 0)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Mutex, MutexGuard};

    use url::Url;

    use crate::cache::cache_key::{CacheKey, CacheKeyContext};
    use crate::download::DownloadTask;
    use crate::global::{CacheKeyConfig, Config};
    use crate::matchers::UrlMatcherConfigurable;
    use crate::proxy::build_test_runtime;

    use crate::test_urls::SAMPLE_MP4;

    use super::*;

    fn cache_test_lock() -> MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[tokio::test]
    async fn resolve_returns_none_when_cache_empty() {
        let _guard = cache_test_lock();
        let runtime = build_test_runtime();
        let uri = Url::parse(SAMPLE_MP4).unwrap();
        let task = DownloadTask::new(uri, None);
        assert!(SegmentResolver::resolve(&runtime, &task).await.is_none());
    }

    #[tokio::test]
    async fn store_content_length_does_not_occupy_bytes_0_1_slot() {
        let _guard = cache_test_lock();
        let runtime = build_test_runtime();
        runtime.cache.memory_clear().await;
        let uri = Url::parse(SAMPLE_MP4).unwrap();
        let mut task = DownloadTask::new(uri, None);
        task.headers = Some(HashMap::new());

        SegmentResolver::store_content_length(&runtime, &task, 5_000_000).await;

        let mut probe = task.clone();
        probe.start_range = 0;
        probe.end_range = Some(1);
        assert!(
            SegmentResolver::resolve(&runtime, &probe).await.is_none(),
            "bytes=0-1 slot must stay free for real probe data"
        );

        assert_eq!(
            SegmentResolver::read_content_length(&runtime, &task).await,
            Some(5_000_000)
        );
    }

    #[tokio::test]
    async fn read_content_length_migrates_legacy_bytes_0_1_metadata() {
        let _guard = cache_test_lock();
        let runtime = build_test_runtime();
        runtime.cache.memory_clear().await;
        let cache = runtime.cache.clone();
        let config = Config::default();
        let matcher = UrlMatcherConfigurable::new(&CacheKeyConfig::default());
        let ctx = CacheKeyContext::new(config, &matcher);
        let uri = Url::parse("https://example.com/legacy-migrate.mp4").unwrap();
        let task = DownloadTask::new(uri, None);

        let mut legacy = task.clone();
        legacy.start_range = 0;
        legacy.end_range = Some(1);
        let legacy_key = CacheKey::for_task(&legacy, &ctx);
        cache
            .memory_put(&legacy_key.entry, Bytes::from_static(b"12345"))
            .await;

        assert_eq!(
            SegmentResolver::read_content_length_from_cache(&cache, &task, &ctx).await,
            Some(12345)
        );
        assert!(cache.memory_get(&legacy_key.entry).await.is_none());

        let metadata_key = CacheKey::for_content_length(&task, &ctx);
        assert_eq!(
            SegmentResolver::read_content_length_from_cache(&cache, &task, &ctx).await,
            Some(12345)
        );
        assert!(cache.memory_get(&metadata_key.entry).await.is_some());
    }
}
