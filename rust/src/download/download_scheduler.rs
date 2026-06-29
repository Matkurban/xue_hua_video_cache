use std::sync::Arc;

use async_recursion::async_recursion;
use parking_lot::Mutex;

use crate::cache::cache_key::{CacheKey, CacheKeyContext};
use crate::ext::gesture_ext::FunctionProxy;

use super::download_pool::DownloadPool;
use super::download_status::DownloadStatus;
use super::download_task::DownloadTask;

const ROUND_DEBOUNCE_MS: u64 = 500;
const ROUND_DEBOUNCE_KEY: &str = "roundTask";

impl DownloadPool {
    /// Enqueue (with dedupe/priority rules) and schedule a debounced pool round.
    pub async fn submit(self: &Arc<Self>, task: Arc<Mutex<DownloadTask>>) {
        let config = self.ctx.config.read().clone();
        let matcher = self.ctx.url_matcher.as_ref();
        let ctx = CacheKeyContext::new(config, matcher);
        let (entry, incoming_priority) = {
            let t = task.lock();
            (CacheKey::for_task(&t, &ctx).entry, t.priority)
        };

        let removed = {
            let mut tasks = self.tasks.lock();
            let replace = tasks.iter().any(|e| {
                let e = e.lock();
                ctx.entry_matches(&e, &entry) && e.priority < incoming_priority
            });
            if replace {
                let removed: Vec<_> = tasks
                    .iter()
                    .filter(|e| ctx.entry_matches(&e.lock(), &entry))
                    .cloned()
                    .collect();
                tasks.retain(|e| !ctx.entry_matches(&e.lock(), &entry));
                Some(removed)
            } else {
                None
            }
        };
        if let Some(removed) = removed {
            Self::cancel_tasks(&removed);
        }

        let already_exists = self
            .tasks
            .lock()
            .iter()
            .any(|e| ctx.entry_matches(&e.lock(), &entry));
        if !already_exists {
            self.add_task(task.clone()).await;
        }

        self.schedule_round_debounced();
    }

    pub(crate) fn schedule_round_debounced(self: &Arc<Self>) {
        let pool = Arc::clone(self);
        FunctionProxy::debounce(
            move || {
                let pool = pool.clone();
                tokio::spawn(async move {
                    pool.run_download_pool_round().await;
                });
            },
            Some(ROUND_DEBOUNCE_KEY),
            ROUND_DEBOUNCE_MS,
        );
    }

    pub(crate) async fn run_download_pool_round(self: &Arc<Self>) {
        run_download_pool_round(Arc::clone(self)).await;
    }
}

#[async_recursion]
async fn run_download_pool_round(pool: Arc<DownloadPool>) {
    let mut tasks = pool.tasks.lock();
    if tasks.is_empty() {
        return;
    }
    tasks.sort_by(|a, b| b.lock().priority.cmp(&a.lock().priority));
    for task in tasks.iter().skip(pool.pool_size) {
        let mut t = task.lock();
        if t.status == DownloadStatus::Downloading {
            if let Some(token) = t.cancel_token.take() {
                token.cancel();
            }
            t.status = DownloadStatus::Paused;
            drop(t);
            let _ = pool.tx.send(task.clone());
        }
    }
    drop(tasks);
    let list = pool.tasks.lock().clone();
    let config = pool.ctx.config.read().clone();
    let matcher = pool.ctx.url_matcher.as_ref();
    let ctx = CacheKeyContext::new(config, matcher);
    for task in list {
        let downloading = pool.downloading_tasks().len();
        if downloading >= pool.pool_size {
            break;
        }
        let entry = CacheKey::for_task(&task.lock(), &ctx).entry;
        let already_active = pool
            .downloading_tasks()
            .iter()
            .any(|active| ctx.entry_matches(&active.lock(), &entry));
        if already_active {
            continue;
        }
        {
            let mut t = task.lock();
            match t.status {
                DownloadStatus::Idle | DownloadStatus::Paused => {}
                DownloadStatus::Downloading => continue,
                _ => continue,
            }
            if t.retry_times == 0 && t.status == DownloadStatus::Idle {
                t.retry_times = 3;
            }
            t.status = DownloadStatus::Downloading;
        }
        let _ = pool.tx.send(task.clone());
        let pool2 = pool.clone();
        tokio::spawn(async move {
            super::download_pool::run_pool_download(pool2, task).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use parking_lot::Mutex;
    use url::Url;

    use crate::cache::LruCacheSingleton;
    use crate::global::CacheKeyConfig;
    use crate::proxy::PlatformKind;
    use crate::proxy::app_context::AppContext;

    use super::*;
    use crate::download::download_pool::DownloadPool;

    fn test_ctx() -> Arc<AppContext> {
        Arc::new(AppContext::new(
            PlatformKind::Other,
            CacheKeyConfig::default(),
        ))
    }

    fn test_cache() -> Arc<LruCacheSingleton> {
        LruCacheSingleton::instance()
    }

    #[tokio::test]
    async fn submit_replaces_lower_priority_task() {
        let pool = Arc::new(DownloadPool::new(1, test_ctx(), test_cache()));
        let uri = Url::parse("https://example.com/priority.mp4").unwrap();
        let low = Arc::new(Mutex::new(DownloadTask::new(uri.clone(), None)));
        low.lock().priority = 1;
        let high = Arc::new(Mutex::new(DownloadTask::new(uri, None)));
        high.lock().priority = 9;
        pool.submit(low).await;
        pool.submit(high.clone()).await;
        assert_eq!(pool.task_list().len(), 1);
        assert_eq!(pool.task_list()[0].lock().priority, 9);
        pool.dispose();
    }

    #[tokio::test]
    async fn submit_skips_duplicate_entry() {
        let pool = Arc::new(DownloadPool::new(1, test_ctx(), test_cache()));
        let uri = Url::parse("https://example.com/dedupe.mp4").unwrap();
        let t1 = Arc::new(Mutex::new(DownloadTask::new(uri.clone(), None)));
        let t2 = Arc::new(Mutex::new(DownloadTask::new(uri, None)));
        pool.submit(t1).await;
        pool.submit(t2).await;
        assert_eq!(pool.task_list().len(), 1);
        pool.dispose();
    }

    #[tokio::test]
    async fn overflow_downloading_task_is_paused_and_worker_cancelled() {
        let pool = Arc::new(DownloadPool::new(1, test_ctx(), test_cache()));
        let uri1 = Url::parse("https://example.com/a.mp4").unwrap();
        let uri2 = Url::parse("https://example.com/b.mp4").unwrap();
        let t1 = Arc::new(Mutex::new(DownloadTask::new(uri1, None)));
        let t2 = Arc::new(Mutex::new(DownloadTask::new(uri2, None)));
        t1.lock().status = DownloadStatus::Downloading;
        t1.lock().cancel_token = Some(tokio_util::sync::CancellationToken::new());
        t2.lock().status = DownloadStatus::Downloading;
        let token2 = tokio_util::sync::CancellationToken::new();
        t2.lock().cancel_token = Some(token2.clone());
        pool.tasks.lock().push(t1);
        pool.tasks.lock().push(t2.clone());
        pool.run_download_pool_round().await;
        assert_eq!(t2.lock().status, DownloadStatus::Paused);
        assert!(token2.is_cancelled());
        pool.dispose();
    }

    #[tokio::test]
    async fn resume_via_submit_leaves_task_claimable() {
        let pool = Arc::new(DownloadPool::new(1, test_ctx(), test_cache()));
        let uri = Url::parse("https://example.com/resume-submit.mp4").unwrap();
        let task = Arc::new(Mutex::new(DownloadTask::new(uri, None)));
        pool.submit(task.clone()).await;
        task.lock().status = DownloadStatus::Paused;
        task.lock().status = DownloadStatus::Idle;
        pool.submit(task.clone()).await;
        let claimable = matches!(
            task.lock().status,
            DownloadStatus::Idle | DownloadStatus::Paused
        );
        assert!(claimable);
        pool.dispose();
    }
}
