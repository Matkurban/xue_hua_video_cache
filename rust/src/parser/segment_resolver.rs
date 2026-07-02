use std::path::Path;
use std::sync::Arc;

use bytes::Bytes;

use crate::cache::cache_key::{CacheKey, CacheKeyContext};
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

        if let Some(data) = runtime.cache.memory_get(&key.entry).await {
            log_d(&format!(
                "From memory: {}, total memory size: {}, Request range：{}-{:?}",
                to_memory_size(data.len() as i64),
                runtime.cache.memory_format_size().await,
                task.start_range,
                task.end_range
            ));
            return Some(data);
        }
        if let Some(data) = runtime.cache.storage_get(&key.entry).await {
            log_d(&format!(
                "From file: {} Request range：{}-{:?}",
                key.entry, task.start_range, task.end_range
            ));
            runtime.cache.memory_put(&key.entry, data.clone()).await;
            return Some(data);
        }
        None
    }

    pub(crate) async fn store_content_length(
        runtime: &Arc<ProxyRuntime>,
        task: &DownloadTask,
        content_length: i64,
    ) {
        let ctx = CacheKeyContext::from_runtime(runtime);
        let key = CacheKey::for_task(task, &ctx);
        let Ok(cache_path) = FileExt::create_cache_path(Some(&key.directory)).await else {
            return;
        };
        let save_name = key.file_name(task);
        let file_path = format!("{cache_path}/{save_name}");
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
}

#[cfg(test)]
mod tests {
    use url::Url;

    use crate::download::DownloadTask;
    use crate::proxy::build_test_runtime;

    use crate::test_urls::SAMPLE_MP4;

    use super::*;

    #[tokio::test]
    async fn resolve_returns_none_when_cache_empty() {
        let runtime = build_test_runtime();
        let uri = Url::parse(SAMPLE_MP4).unwrap();
        let task = DownloadTask::new(uri, None);
        assert!(SegmentResolver::resolve(&runtime, &task).await.is_none());
    }
}
