use std::path::Path;
use std::sync::Arc;

use bytes::Bytes;
use parking_lot::Mutex;

use crate::cache::cache_key::{CacheKey, CacheKeyContext};
use crate::download::DownloadTask;
use crate::ext::file_ext::FileExt;
use crate::ext::log_ext::log_d;
use crate::proxy::ProxyRuntime;

use super::download_wait::wait_for_task_completion;

pub(crate) struct SegmentFetcher;

impl SegmentFetcher {
    pub(crate) async fn download(
        runtime: &Arc<ProxyRuntime>,
        task: Arc<Mutex<DownloadTask>>,
    ) -> Option<Bytes> {
        let ctx = CacheKeyContext::from_runtime(runtime);
        let entry = {
            let t = task.lock();
            let url = t.url();
            log_d(&format!("From network: {url}"));
            CacheKey::for_task(&t, &ctx).entry
        };

        {
            let directory = CacheKey::directory_for(&task.lock());
            if let Ok(path) = FileExt::create_cache_path(Some(&directory)).await {
                task.lock().cache_dir = path;
            }
        }

        let dm = runtime.downloads();
        let mut rx = dm.subscribe();
        dm.submit(task.clone()).await;

        wait_for_task_completion(runtime, &mut rx, |t| ctx.entry_matches(t, &entry)).await
    }

    pub(crate) async fn push(
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
        dm.submit(task.clone()).await;
        Ok(())
    }
}
