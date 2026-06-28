use std::sync::Arc;

use crate::frb_generated::StreamSink;
use parking_lot::Mutex;

use crate::download::{DownloadStatus, DownloadTask};
use crate::proxy::require_state;

#[flutter_rust_bridge::frb]
#[derive(Debug, Clone)]
pub struct DownloadTaskInfo {
    pub id: String,
    pub url: String,
    pub priority: i32,
    pub progress: f64,
    pub cached_bytes: i64,
    pub downloaded_bytes: i64,
    pub total_bytes: i64,
    pub status: DownloadStatus,
    pub hls_key: Option<String>,
}

fn task_to_info(task: &DownloadTask) -> DownloadTaskInfo {
    DownloadTaskInfo {
        id: task.id.clone(),
        url: task.url(),
        priority: task.priority,
        progress: task.progress,
        cached_bytes: task.cached_bytes,
        downloaded_bytes: task.downloaded_bytes,
        total_bytes: task.total_bytes,
        status: task.status,
        hls_key: task.hls_key.clone(),
    }
}

fn tasks_to_info(tasks: Vec<Arc<Mutex<DownloadTask>>>) -> Vec<DownloadTaskInfo> {
    tasks.iter().map(|t| task_to_info(&t.lock())).collect()
}

#[flutter_rust_bridge::frb]
pub async fn download_manager_subscribe(sink: StreamSink<DownloadTaskInfo>) -> Result<(), String> {
    let state = require_state()?;
    let mut rx = state.download_manager().subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(task) => {
                    let info = task_to_info(&task.lock());
                    if sink.add(info).is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
    });
    Ok(())
}

#[flutter_rust_bridge::frb]
pub fn download_manager_all_tasks() -> Result<Vec<DownloadTaskInfo>, String> {
    let state = require_state()?;
    Ok(tasks_to_info(state.download_manager().task_list()))
}

#[flutter_rust_bridge::frb]
pub fn download_manager_downloading_tasks() -> Result<Vec<DownloadTaskInfo>, String> {
    let state = require_state()?;
    Ok(tasks_to_info(state.download_manager().downloading_tasks()))
}

#[flutter_rust_bridge::frb]
pub fn download_manager_pause_task_by_id(task_id: String) -> Result<(), String> {
    let state = require_state()?;
    state.download_manager().pause_task_by_id(&task_id);
    Ok(())
}

#[flutter_rust_bridge::frb]
pub fn download_manager_resume_task_by_id(task_id: String) -> Result<(), String> {
    let state = require_state()?;
    state.download_manager().resume_task_by_id(&task_id);
    Ok(())
}

#[flutter_rust_bridge::frb]
pub fn download_manager_cancel_task_by_id(task_id: String) -> Result<(), String> {
    let state = require_state()?;
    state.download_manager().cancel_task_by_id(&task_id);
    Ok(())
}

#[flutter_rust_bridge::frb]
pub fn download_manager_pause_all_tasks() -> Result<(), String> {
    let state = require_state()?;
    state.download_manager().pause_all_tasks();
    Ok(())
}

#[flutter_rust_bridge::frb]
pub fn download_manager_cancel_all_tasks() -> Result<(), String> {
    let state = require_state()?;
    state.download_manager().cancel_all_tasks();
    Ok(())
}

#[flutter_rust_bridge::frb]
pub fn download_manager_cancel_task_by_url(url: String) -> Result<(), String> {
    let state = require_state()?;
    state.download_manager().cancel_task_by_url(&url);
    Ok(())
}

#[flutter_rust_bridge::frb]
pub fn download_manager_pause_task_by_url(url: String) -> Result<(), String> {
    let state = require_state()?;
    state.download_manager().pause_task_by_url(&url);
    Ok(())
}

#[flutter_rust_bridge::frb]
pub fn download_manager_resume_task_by_url(url: String) -> Result<(), String> {
    let state = require_state()?;
    state.download_manager().resume_task_by_url(&url);
    Ok(())
}

#[flutter_rust_bridge::frb]
pub async fn download_manager_cancel_task_about_url(url: String) -> Result<(), String> {
    let state = require_state()?;
    state.download_manager().cancel_task_about_url(&url);
    Ok(())
}
