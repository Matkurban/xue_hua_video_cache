use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use parking_lot::Mutex;
use tokio::sync::broadcast;

use crate::download::{DownloadStatus, DownloadTask};
use crate::ext::log_ext::log_w;
use crate::proxy::require_state;

pub const TASK_WAIT_TIMEOUT: Duration = Duration::from_secs(120);
pub const CACHE_POLL_TIMEOUT: Duration = Duration::from_secs(30);

/// Scan the download pool for a matching task that already finished.
pub fn find_completed_task_data(task_matches: &impl Fn(&DownloadTask) -> bool) -> Option<Bytes> {
    let state = require_state().ok()?;
    for task in state.download_manager().task_list() {
        let task = task.lock();
        if task_matches(&*task) && task.status == DownloadStatus::Completed {
            return Some(task.data.clone());
        }
    }
    None
}

/// Wait for a download task to complete, fail, or cancel. Returns cached bytes on success.
pub async fn wait_for_task_completion(
    rx: &mut broadcast::Receiver<Arc<Mutex<DownloadTask>>>,
    task_matches: impl Fn(&DownloadTask) -> bool,
) -> Option<Bytes> {
    let deadline = tokio::time::Instant::now() + TASK_WAIT_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            log_w("[DownloadWait] timed out waiting for task completion");
            return None;
        }
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Ok(updated)) => {
                let updated = updated.lock();
                if !task_matches(&*updated) {
                    continue;
                }
                match updated.status {
                    DownloadStatus::Completed => return Some(updated.data.clone()),
                    DownloadStatus::Failed | DownloadStatus::Cancelled => {
                        log_w(&format!(
                            "[DownloadWait] task ended with {:?}: {}",
                            updated.status,
                            updated.url()
                        ));
                        return None;
                    }
                    _ => continue,
                }
            }
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => {
                if let Some(data) = find_completed_task_data(&task_matches) {
                    return Some(data);
                }
                continue;
            }
            Ok(Err(_)) => break,
            Err(_) => {
                log_w("[DownloadWait] timed out waiting for task completion");
                return None;
            }
        }
    }
    None
}

/// Poll a cache fetch until data is available or the timeout elapses.
pub async fn wait_for_cache<F, Fut>(mut fetch: F, timeout: Duration) -> Option<Bytes>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<Bytes>>,
{
    let deadline = tokio::time::Instant::now() + timeout;
    let mut data = fetch().await;
    while data.is_none() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
        data = fetch().await;
    }
    if data.is_none() {
        log_w("[DownloadWait] timed out waiting for cache");
    }
    data
}
