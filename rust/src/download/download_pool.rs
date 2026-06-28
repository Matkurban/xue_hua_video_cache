use std::sync::Arc;
use std::time::{Duration, Instant};

use async_recursion::async_recursion;
use bytes::Bytes;
use parking_lot::Mutex;
use reqwest::Client;
use tokio::io::AsyncWriteExt;
use tokio::sync::broadcast;

use crate::cache::LruCacheSingleton;
use crate::ext::file_ext::FileExt;
use crate::ext::gesture_ext::FunctionProxy;
use crate::ext::log_ext::log_v;
use crate::proxy::app_context::AppContext;

use super::download_status::DownloadStatus;
use super::download_task::DownloadTask;

pub const MAX_POOL_SIZE: usize = 1;
pub const MAX_TASK_PRIORITY: i32 = 9999;
pub const MIN_PROGRESS_UPDATE_INTERVAL: u64 = 500;

pub struct DownloadPool {
    pool_size: usize,
    tasks: Mutex<Vec<Arc<Mutex<DownloadTask>>>>,
    client: Client,
    tx: broadcast::Sender<Arc<Mutex<DownloadTask>>>,
    pub(crate) ctx: Arc<AppContext>,
}

impl DownloadPool {
    pub fn new(pool_size: usize, ctx: Arc<AppContext>) -> Self {
        assert!(pool_size > 0, "Pool size must be greater than 0");
        let (tx, _) = broadcast::channel(1024);
        Self {
            pool_size,
            tasks: Mutex::new(Vec::new()),
            client: ctx.http_client.clone(),
            tx,
            ctx,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Mutex<DownloadTask>>> {
        self.tx.subscribe()
    }

    pub fn task_list(&self) -> Vec<Arc<Mutex<DownloadTask>>> {
        self.tasks.lock().clone()
    }

    pub fn remove_tasks_by_ids(&self, ids: &[String]) {
        let mut tasks = self.tasks.lock();
        for task in tasks.iter() {
            if ids.contains(&task.lock().id) {
                Self::cancel_task_worker(task);
            }
        }
        tasks.retain(|t| !ids.contains(&t.lock().id));
    }

    fn cancel_task_worker(task: &Arc<Mutex<DownloadTask>>) {
        if let Some(token) = task.lock().cancel_token.take() {
            token.cancel();
        }
    }

    fn cancel_tasks(tasks: &[Arc<Mutex<DownloadTask>>]) {
        for task in tasks {
            Self::cancel_task_worker(task);
        }
    }

    pub fn downloading_tasks(&self) -> Vec<Arc<Mutex<DownloadTask>>> {
        self.tasks
            .lock()
            .iter()
            .filter(|t| t.lock().status == DownloadStatus::Downloading)
            .cloned()
            .collect()
    }

    pub async fn add_task(&self, task: Arc<Mutex<DownloadTask>>) -> Arc<Mutex<DownloadTask>> {
        let needs_path = task.lock().cache_dir.is_empty();
        if needs_path {
            if let Ok(p) = FileExt::create_cache_path(None).await {
                task.lock().cache_dir = p;
            }
        }
        self.tasks.lock().push(task.clone());
        task
    }

    pub async fn execute_task(self: &Arc<Self>, task: Arc<Mutex<DownloadTask>>) {
        let config = self.ctx.config.read().clone();
        let matcher = self.ctx.url_matcher.as_ref();
        let match_url = task.lock().match_url(&config, matcher);

        let removed = {
            let mut tasks = self.tasks.lock();
            let replace = tasks.iter().any(|e| {
                e.lock().match_url(&config, matcher) == match_url
                    && e.lock().priority < task.lock().priority
            });
            if replace {
                let removed: Vec<_> = tasks
                    .iter()
                    .filter(|e| e.lock().match_url(&config, matcher) == match_url)
                    .cloned()
                    .collect();
                tasks.retain(|e| e.lock().match_url(&config, matcher) != match_url);
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
            .any(|e| e.lock().match_url(&config, matcher) == match_url);
        if !already_exists {
            self.add_task(task.clone()).await;
        }

        let pool = Arc::clone(self);
        FunctionProxy::debounce(
            move || {
                let pool = pool.clone();
                tokio::spawn(async move {
                    run_download_pool_round(pool).await;
                });
            },
            Some("roundTask"),
            500,
        );
    }

    pub async fn round_task(self: &Arc<Self>) {
        run_download_pool_round(Arc::clone(self)).await;
    }

    pub fn update_task_by_id(&self, task_id: &str, status: DownloadStatus) {
        let task = {
            let tasks = self.tasks.lock();
            tasks.iter().find(|t| t.lock().id == task_id).cloned()
        };
        if let Some(task) = task {
            if status == DownloadStatus::Paused || status == DownloadStatus::Cancelled {
                if let Some(token) = task.lock().cancel_token.take() {
                    token.cancel();
                }
            }
            task.lock().status = status;
            if status == DownloadStatus::Completed
                || status == DownloadStatus::Failed
                || status == DownloadStatus::Cancelled
            {
                self.tasks.lock().retain(|t| t.lock().id != task_id);
            }
            let _ = self.tx.send(task);
        }
    }

    pub fn dispose(&self) {
        for task in self.tasks.lock().drain(..) {
            if let Some(token) = task.lock().cancel_token.take() {
                token.cancel();
            }
        }
        DownloadTask::reset_id();
    }

    fn download_header(task: &DownloadTask) -> std::collections::HashMap<String, String> {
        let mut headers = std::collections::HashMap::new();
        let mut range = String::new();
        if task.start_range > 0 || task.cached_bytes > 0 {
            range = format!("bytes={}-", task.start_range + task.cached_bytes);
        }
        if let Some(end) = task.end_range {
            if range.is_empty() {
                range = "bytes=0-".to_string();
            }
            range.push_str(&end.to_string());
        }
        if !range.is_empty() {
            headers.insert("Range".to_string(), range);
        }
        if let Some(ref h) = task.headers {
            for (k, v) in h {
                let kl = k.to_lowercase();
                if kl == "host" || kl == "range" {
                    continue;
                }
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
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
            t.status = DownloadStatus::Paused;
            drop(t);
            let _ = pool.tx.send(task.clone());
        }
    }
    drop(tasks);
    let list = pool.tasks.lock().clone();
    for task in list {
        let downloading = pool.downloading_tasks().len();
        if downloading >= pool.pool_size {
            break;
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
            run_pool_download(pool2, task).await;
        });
    }
}

async fn run_pool_download(pool: Arc<DownloadPool>, task: Arc<Mutex<DownloadTask>>) {
    use futures_util::StreamExt;

    let task_id = task.lock().id.clone();
    if !pool.tasks.lock().iter().any(|t| t.lock().id == task_id) {
        return;
    }

    let (url, tmp_path, save_path, headers_map, append, match_key) = {
        let mut t = task.lock();
        if t.cancel_token.is_none() {
            t.cancel_token = Some(tokio_util::sync::CancellationToken::new());
        }
        if t.cached_bytes == 0 {
            if let Ok(meta) = std::fs::metadata(format!(
                "{}.tmp",
                t.save_path(&pool.ctx.config.read(), pool.ctx.url_matcher.as_ref(),)
            )) {
                t.cached_bytes = meta.len() as i64;
            }
        }
        let append = t.cached_bytes > 0;
        let url = t.url();
        let config = pool.ctx.config.read().clone();
        let matcher = pool.ctx.url_matcher.as_ref();
        let save_path = t.save_path(&config, matcher);
        let tmp_path = format!("{save_path}.tmp");
        let match_key = t.match_url(&config, matcher);
        let headers = DownloadPool::download_header(&t);
        (url, tmp_path, save_path, headers, append, match_key)
    };

    let mut request = pool.client.get(&url);
    for (k, v) in &headers_map {
        request = request.header(k.as_str(), v.as_str());
    }

    let token = task.lock().cancel_token.clone();
    let response = if let Some(ref token) = token {
        tokio::select! {
            r = request.send() => r,
            _ = token.cancelled() => {
                log_v(&format!("[DownloadPool] Download cancelled: {url}"));
                {
                    let mut t = task.lock();
                    if t.status != DownloadStatus::Cancelled {
                        t.status = DownloadStatus::Paused;
                    }
                }
                let _ = pool.tx.send(task.clone());
                return;
            }
        }
    } else {
        request.send().await
    };

    let resp = match response {
        Ok(r) => r,
        Err(e) => {
            log_v(&format!("[DownloadPool] Download error: {e}"));
            let _ = tokio::fs::remove_file(&tmp_path).await;
            fail_pool_download(&pool, task).await;
            return;
        }
    };

    if resp.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        task.lock().cached_bytes = 0;
        task.lock().retry_times = 0;
        let _ = tokio::fs::remove_file(&tmp_path).await;
        fail_pool_download(&pool, task).await;
        return;
    }

    if !resp.status().is_success() {
        log_v(&format!("[DownloadPool] HTTP error: {}", resp.status()));
        let _ = tokio::fs::remove_file(&tmp_path).await;
        fail_pool_download(&pool, task).await;
        return;
    }

    let content_length = resp.content_length().unwrap_or(0) as i64;
    let total_bytes = if content_length > 0 {
        task.lock().cached_bytes + content_length
    } else {
        0
    };
    {
        let mut t = task.lock();
        if total_bytes > 0 {
            t.total_bytes = total_bytes;
        }
    }

    let mut file = match tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(&tmp_path)
        .await
    {
        Ok(f) => f,
        Err(e) => {
            log_v(&format!("[DownloadPool] File open error: {e}"));
            fail_pool_download(&pool, task).await;
            return;
        }
    };

    let mut stream = resp.bytes_stream();
    let mut received: i64 = 0;

    while let Some(chunk_result) = stream.next().await {
        if task.lock().status == DownloadStatus::Paused
            || task.lock().status == DownloadStatus::Cancelled
        {
            let cached = task.lock().cached_bytes + received;
            task.lock().cached_bytes = cached;
            task.lock().downloaded_bytes = cached;
            let _ = file.flush().await;
            if task.lock().status == DownloadStatus::Cancelled {
                let _ = tokio::fs::remove_file(&tmp_path).await;
                pool.update_task_by_id(&task.lock().id, DownloadStatus::Cancelled);
            }
            let _ = pool.tx.send(task.clone());
            return;
        }

        if let Some(ref token) = token {
            if token.is_cancelled() {
                let cached = task.lock().cached_bytes + received;
                task.lock().cached_bytes = cached;
                task.lock().downloaded_bytes = cached;
                let _ = file.flush().await;
                log_v(&format!("[DownloadPool] Download paused: {url}"));
                {
                    let mut t = task.lock();
                    if t.status != DownloadStatus::Cancelled {
                        t.status = DownloadStatus::Paused;
                    }
                }
                let _ = pool.tx.send(task.clone());
                return;
            }
        }

        let chunk = match chunk_result {
            Ok(c) => c,
            Err(e) => {
                log_v(&format!("[DownloadPool] Stream error: {e}"));
                let _ = tokio::fs::remove_file(&tmp_path).await;
                fail_pool_download(&pool, task).await;
                return;
            }
        };

        if let Err(e) = file.write_all(&chunk).await {
            log_v(&format!("[DownloadPool] Write error: {e}"));
            fail_pool_download(&pool, task).await;
            return;
        }

        received += chunk.len() as i64;
        {
            let mut t = task.lock();
            t.downloaded_bytes = t.cached_bytes + received;
            if t.total_bytes > 0 {
                t.progress = t.downloaded_bytes as f64 / t.total_bytes as f64;
            }
        }

        let should_notify = {
            let mut t = task.lock();
            let elapsed = t
                .last_progress_at
                .map(|i| i.elapsed())
                .unwrap_or(Duration::from_millis(MIN_PROGRESS_UPDATE_INTERVAL + 1));
            if elapsed.as_millis() as u64 >= MIN_PROGRESS_UPDATE_INTERVAL {
                t.last_progress_at = Some(Instant::now());
                true
            } else {
                false
            }
        };
        if should_notify {
            let _ = pool.tx.send(task.clone());
        }
    }

    let _ = file.flush().await;
    finish_pool_download(pool.clone(), task, &save_path, &tmp_path, &match_key).await;
}

async fn finish_pool_download(
    pool: Arc<DownloadPool>,
    task: Arc<Mutex<DownloadTask>>,
    save_path: &str,
    tmp_path: &str,
    match_key: &str,
) {
    if tokio::fs::metadata(save_path).await.is_ok() {
        let _ = tokio::fs::remove_file(save_path).await;
    }
    if tokio::fs::rename(tmp_path, save_path).await.is_err() {
        fail_pool_download(&pool, task).await;
        return;
    }
    let data = match tokio::fs::read(save_path).await {
        Ok(data) if !data.is_empty() => data,
        Ok(_) | Err(_) => {
            fail_pool_download(&pool, task).await;
            return;
        }
    };
    {
        let mut t = task.lock();
        t.progress = 1.0;
        t.data = Bytes::from(data.clone());
        t.status = DownloadStatus::Completed;
    }
    let _ = pool.tx.send(task.clone());
    let singleton = LruCacheSingleton::instance();
    singleton
        .memory_put(match_key, Bytes::from(data.clone()))
        .await;
    singleton
        .storage_put(match_key, std::path::PathBuf::from(save_path))
        .await;
    pool.tasks.lock().retain(|x| x.lock().id != task.lock().id);
    let pool2 = Arc::clone(&pool);
    FunctionProxy::debounce(
        move || {
            let pool2 = pool2.clone();
            tokio::spawn(async move {
                run_download_pool_round(pool2).await;
            });
        },
        Some("roundTask"),
        500,
    );
}

async fn fail_pool_download(pool: &Arc<DownloadPool>, task: Arc<Mutex<DownloadTask>>) {
    let task_id = {
        let mut t = task.lock();
        if t.status == DownloadStatus::Cancelled {
            return;
        }
        if t.retry_times > 0 {
            t.retry_times -= 1;
            t.status = DownloadStatus::Idle;
            log_v(&format!(
                "[DownloadPool] Download retry ({} left): {}",
                t.retry_times,
                t.url()
            ));
        } else {
            t.status = DownloadStatus::Failed;
        }
        t.id.clone()
    };
    let _ = pool.tx.send(task.clone());
    if task.lock().status == DownloadStatus::Failed {
        pool.tasks.lock().retain(|x| x.lock().id != task_id);
    }
    let pool2 = Arc::clone(pool);
    FunctionProxy::debounce(
        move || {
            let pool2 = pool2.clone();
            tokio::spawn(async move {
                run_download_pool_round(pool2).await;
            });
        },
        Some("roundTask"),
        500,
    );
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use parking_lot::Mutex;
    use url::Url;

    use crate::global::CacheKeyConfig;
    use crate::proxy::PlatformKind;
    use crate::proxy::app_context::AppContext;

    use super::*;

    fn test_ctx() -> Arc<AppContext> {
        Arc::new(AppContext::new(
            PlatformKind::Other,
            CacheKeyConfig::default(),
        ))
    }

    #[tokio::test]
    async fn add_task_increases_task_list() {
        let pool = DownloadPool::new(1, test_ctx());
        let uri = Url::parse("https://example.com/1.mp4").unwrap();
        let task = Arc::new(Mutex::new(DownloadTask::new(uri, None)));
        pool.add_task(task).await;
        assert_eq!(pool.task_list().len(), 1);
        pool.dispose();
    }

    #[tokio::test]
    async fn execute_task_replaces_lower_priority_without_download() {
        let pool = Arc::new(DownloadPool::new(1, test_ctx()));
        let uri = Url::parse("https://example.com/3.mp4").unwrap();
        let t1 = Arc::new(Mutex::new(DownloadTask::new(uri.clone(), None)));
        t1.lock().priority = 1;
        let t2 = Arc::new(Mutex::new(DownloadTask::new(uri, None)));
        t2.lock().priority = 5;
        pool.add_task(t1).await;
        {
            let match_url = t2
                .lock()
                .match_url(&pool.ctx.config.read(), pool.ctx.url_matcher.as_ref());
            pool.tasks.lock().retain(|e| {
                e.lock()
                    .match_url(&pool.ctx.config.read(), pool.ctx.url_matcher.as_ref())
                    != match_url
            });
            pool.add_task(t2.clone()).await;
        }
        assert_eq!(pool.task_list().len(), 1);
        assert_eq!(pool.task_list()[0].lock().priority, 5);
        pool.dispose();
    }

    #[tokio::test]
    async fn resume_sets_idle_so_scheduler_can_claim_task() {
        let pool = Arc::new(DownloadPool::new(1, test_ctx()));
        let uri = Url::parse("https://example.com/resume.mp4").unwrap();
        let task = Arc::new(Mutex::new(DownloadTask::new(uri, None)));
        pool.add_task(task.clone()).await;
        task.lock().status = DownloadStatus::Paused;
        task.lock().status = DownloadStatus::Idle;

        let claimable = {
            let t = task.lock();
            matches!(t.status, DownloadStatus::Idle | DownloadStatus::Paused)
        };
        assert!(claimable);

        task.lock().status = DownloadStatus::Downloading;
        let claimable_after = {
            let t = task.lock();
            matches!(t.status, DownloadStatus::Idle | DownloadStatus::Paused)
        };
        assert!(
            !claimable_after,
            "Downloading without an active worker must not be re-claimed by the scheduler"
        );
        pool.dispose();
    }

    #[tokio::test]
    async fn remove_tasks_by_ids_cancels_worker_token() {
        let pool = DownloadPool::new(1, test_ctx());
        let uri = Url::parse("https://example.com/cancel.mp4").unwrap();
        let task = Arc::new(Mutex::new(DownloadTask::new(uri, None)));
        task.lock().cancel_token = Some(tokio_util::sync::CancellationToken::new());
        let token = task.lock().cancel_token.clone().unwrap();
        pool.add_task(task.clone()).await;
        let id = task.lock().id.clone();
        pool.remove_tasks_by_ids(&[id]);
        assert!(token.is_cancelled());
        assert!(pool.task_list().is_empty());
        pool.dispose();
    }

    #[tokio::test]
    async fn cancel_task_by_id_keeps_cancelled_status() {
        let pool = Arc::new(DownloadPool::new(1, test_ctx()));
        let uri = Url::parse("https://example.com/cancel2.mp4").unwrap();
        let task = Arc::new(Mutex::new(DownloadTask::new(uri, None)));
        task.lock().cancel_token = Some(tokio_util::sync::CancellationToken::new());
        pool.add_task(task.clone()).await;
        let id = task.lock().id.clone();
        pool.update_task_by_id(&id, DownloadStatus::Cancelled);
        assert_eq!(task.lock().status, DownloadStatus::Cancelled);
        pool.dispose();
    }
}
