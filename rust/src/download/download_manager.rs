use std::sync::Arc;

use parking_lot::Mutex;

use crate::ext::string_ext::generate_md5;
use crate::proxy::app_context::AppContext;

use super::download_pool::DownloadPool;
use super::download_status::DownloadStatus;
use super::download_task::DownloadTask;

pub struct DownloadManager {
    pool: Arc<DownloadPool>,
}

impl DownloadManager {
    pub fn new(max_concurrent: usize, ctx: Arc<AppContext>) -> Self {
        Self {
            pool: Arc::new(DownloadPool::new(max_concurrent.max(1), ctx)),
        }
    }

    pub fn pool(&self) -> Arc<DownloadPool> {
        self.pool.clone()
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<Arc<Mutex<DownloadTask>>> {
        self.pool.subscribe()
    }

    pub async fn add_task(&self, task: Arc<Mutex<DownloadTask>>) {
        self.pool.add_task(task).await;
    }

    pub async fn execute_task(&self, task: Arc<Mutex<DownloadTask>>) {
        self.pool.execute_task(task).await;
    }

    pub async fn round_task(&self) {
        self.pool.round_task().await;
    }

    pub fn pause_task_by_id(&self, id: &str) {
        self.pool.update_task_by_id(id, DownloadStatus::Paused);
    }

    pub fn cancel_task_by_id(&self, id: &str) {
        self.pool.update_task_by_id(id, DownloadStatus::Cancelled);
    }

    pub fn cancel_task_by_url(&self, url: &str) {
        let ids: Vec<String> = self
            .pool
            .task_list()
            .into_iter()
            .filter(|t| t.lock().url() == url)
            .map(|t| t.lock().id.clone())
            .collect();
        for id in &ids {
            self.cancel_task_by_id(id);
        }
    }

    pub fn cancel_task_about_url(&self, url: &str) {
        let url_md5 = generate_md5(url);
        let ids: Vec<String> = self
            .pool
            .task_list()
            .into_iter()
            .filter(|t| {
                let t = t.lock();
                t.url() == url || t.hls_key.as_deref() == Some(url_md5.as_str())
            })
            .map(|t| t.lock().id.clone())
            .collect();
        for id in &ids {
            self.cancel_task_by_id(id);
        }
    }

    pub fn pause_task_by_url(&self, url: &str) {
        for task in self.pool.task_list() {
            if task.lock().url() == url {
                self.pause_task_by_id(&task.lock().id);
            }
        }
    }

    pub fn resume_task_by_url(&self, url: &str) {
        if let Some(task) = self
            .pool
            .task_list()
            .into_iter()
            .find(|t| t.lock().url() == url)
        {
            self.resume_task_by_id(&task.lock().id);
        }
    }

    pub fn is_task_exist(&self, task: &DownloadTask) -> bool {
        let match_url = task.match_url(
            &self.pool.ctx.config.read(),
            self.pool.ctx.url_matcher.as_ref(),
        );
        self.pool.task_list().iter().any(|t| {
            t.lock().match_url(
                &self.pool.ctx.config.read(),
                self.pool.ctx.url_matcher.as_ref(),
            ) == match_url
        })
    }

    pub fn is_url_downloading(&self, task: &DownloadTask) -> bool {
        let match_url = task.match_url(
            &self.pool.ctx.config.read(),
            self.pool.ctx.url_matcher.as_ref(),
        );
        self.pool.downloading_tasks().iter().any(|t| {
            t.lock().match_url(
                &self.pool.ctx.config.read(),
                self.pool.ctx.url_matcher.as_ref(),
            ) == match_url
        })
    }

    pub fn resume_task_by_id(&self, id: &str) {
        if let Some(task) = self
            .pool
            .task_list()
            .into_iter()
            .find(|t| t.lock().id == id)
        {
            task.lock().status = DownloadStatus::Idle;
            let pool = self.pool.clone();
            tokio::spawn(async move {
                pool.execute_task(task).await;
            });
        }
    }

    pub fn pause_all_tasks(&self) {
        for task in self.pool.downloading_tasks() {
            self.pause_task_by_id(&task.lock().id);
        }
    }

    pub fn cancel_all_tasks(&self) {
        let ids: Vec<String> = self
            .pool
            .task_list()
            .into_iter()
            .map(|t| t.lock().id.clone())
            .collect();
        for id in &ids {
            self.cancel_task_by_id(id);
        }
    }

    pub fn task_list(&self) -> Vec<Arc<Mutex<DownloadTask>>> {
        self.pool.task_list()
    }

    pub fn downloading_tasks(&self) -> Vec<Arc<Mutex<DownloadTask>>> {
        self.pool.downloading_tasks()
    }

    pub fn dispose(&self) {
        self.pool.dispose();
    }
}
