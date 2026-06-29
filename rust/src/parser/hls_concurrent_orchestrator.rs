use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use async_recursion::async_recursion;
use parking_lot::Mutex;
use rand::RngExt;

use crate::cache::cache_key::{CacheKey, CacheKeyContext};
use crate::download::{DownloadStatus, DownloadTask};
use crate::ext::file_ext::FileExt;
use crate::ext::log_ext::{log_d, log_w};
use crate::ext::string_ext::to_safe_uri;
use crate::proxy::ProxyRuntime;

use super::download_wait::{TASK_WAIT_TIMEOUT, find_completed_task_data};
use super::hls_registry::{
    HlsSegment, prefetch_begin, prefetch_evict_playlist, prefetch_mark_status,
    query_active_segment, query_downloading_count, query_idle_segment, query_next_prefetch_segment,
    query_playlist_keys,
};

pub(crate) async fn hls_concurrent_loop(
    runtime: Arc<ProxyRuntime>,
    hls_segment: Option<HlsSegment>,
    headers: HashMap<String, String>,
) {
    let Some(hls_segment) = hls_segment else {
        return;
    };

    let evicted_urls = prefetch_begin(&hls_segment);
    if !evicted_urls.is_empty() {
        let pool = runtime.downloads().pool();
        let ids: Vec<String> = pool
            .task_list()
            .into_iter()
            .filter(|t| evicted_urls.contains(&t.lock().url()))
            .map(|t| t.lock().id.clone())
            .collect();
        pool.remove_tasks_by_ids(&ids);
    }

    let Some(segment) = query_active_segment(&hls_segment) else {
        return;
    };

    if query_downloading_count(&segment.key) >= 2 {
        return;
    }

    let config = runtime.ctx.config.read().clone();
    let ctx = CacheKeyContext::new(config.clone(), runtime.ctx.url_matcher.as_ref());
    let mut probe_task = DownloadTask::new(to_safe_uri(&segment.url), None);
    probe_task.headers = Some(headers.clone());
    probe_task.hls_key = Some(segment.key.clone());
    probe_task.start_range = segment.start_range;
    probe_task.end_range = segment.end_range;
    let key = CacheKey::for_task(&probe_task, &ctx);
    if runtime.cache.memory_get(&key.entry).await.is_some() {
        hls_concurrent_complete(runtime.clone(), segment, headers.clone(), None).await;
        return;
    }

    let mut task = probe_task;
    if let Ok(path) = FileExt::create_cache_path(Some(&key.directory)).await {
        task.cache_dir = path.clone();
        let save_path = CacheKey::for_task(&task, &ctx).save_path(&task);
        if Path::new(&save_path).exists() {
            hls_concurrent_complete(runtime.clone(), segment, headers.clone(), None).await;
            return;
        }
    }

    let dm = runtime.downloads();
    if dm.is_url_downloading(&task) {
        hls_concurrent_complete(
            runtime.clone(),
            segment,
            headers.clone(),
            Some(DownloadStatus::Downloading),
        )
        .await;
        return;
    }

    task.priority += 1;
    let task_arc = Arc::new(Mutex::new(task));
    let segment_clone = segment.clone();
    let headers = headers.clone();
    let wait_runtime = runtime.clone();
    let wait_entry = key.entry;
    let wait_config = config;
    let mut rx = dm.subscribe();
    dm.submit(task_arc).await;
    tokio::spawn(async move {
        let wait_ctx = CacheKeyContext::new(wait_config, wait_runtime.ctx.url_matcher.as_ref());
        let deadline = tokio::time::Instant::now() + TASK_WAIT_TIMEOUT;
        while tokio::time::Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Ok(updated)) => {
                    let status = {
                        let updated = updated.lock();
                        if !wait_ctx.entry_matches(&updated, &wait_entry) {
                            continue;
                        }
                        updated.status
                    };
                    match status {
                        DownloadStatus::Completed => {
                            log_d(&format!(
                                "Asynchronous download completed： {}",
                                segment_clone.url
                            ));
                            let rt = wait_runtime.clone();
                            tokio::spawn(async move {
                                hls_concurrent_complete(rt, segment_clone, headers, None).await;
                            });
                            break;
                        }
                        DownloadStatus::Failed | DownloadStatus::Cancelled => {
                            log_w(&format!(
                                "Asynchronous download ended with {:?}: {}",
                                status, segment_clone.url
                            ));
                            break;
                        }
                        _ => continue,
                    }
                }
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {
                    let completed = find_completed_task_data(&wait_runtime, &|t| {
                        wait_ctx.entry_matches(t, &wait_entry)
                    })
                    .is_some()
                        || wait_runtime.cache.memory_get(&wait_entry).await.is_some();
                    if completed {
                        log_d(&format!(
                            "Asynchronous download completed (recovered): {}",
                            segment_clone.url
                        ));
                        let seg = segment_clone.clone();
                        let hdrs = headers.clone();
                        let rt = wait_runtime.clone();
                        tokio::spawn(async move {
                            hls_concurrent_complete(rt, seg, hdrs, None).await;
                        });
                        break;
                    }
                    continue;
                }
                Ok(Err(_)) => break,
                Err(_) => {
                    let recovered = find_completed_task_data(&wait_runtime, &|t| {
                        wait_ctx.entry_matches(t, &wait_entry)
                    })
                    .is_some()
                        || wait_runtime.cache.memory_get(&wait_entry).await.is_some();
                    if recovered {
                        let seg = segment_clone.clone();
                        let hdrs = headers.clone();
                        let rt = wait_runtime.clone();
                        tokio::spawn(async move {
                            hls_concurrent_complete(rt, seg, hdrs, None).await;
                        });
                        break;
                    }
                    log_w(&format!(
                        "Asynchronous download timed out waiting: {}",
                        segment_clone.url
                    ));
                    break;
                }
            }
        }
    });
}

#[async_recursion]
async fn hls_concurrent_complete(
    runtime: Arc<ProxyRuntime>,
    hls_segment: HlsSegment,
    headers: HashMap<String, String>,
    status: Option<DownloadStatus>,
) {
    prefetch_mark_status(
        &hls_segment.url,
        status.unwrap_or(DownloadStatus::Completed),
    );

    if let Some(next) = query_next_prefetch_segment(&hls_segment.key) {
        hls_concurrent_loop(runtime.clone(), Some(next), headers).await;
        return;
    }

    let keys = query_playlist_keys();
    if keys.is_empty() {
        return;
    }
    let idx = rand::rng().random_range(0..keys.len());
    let key = keys[idx].clone();
    if let Some(idle) = query_idle_segment(&key) {
        hls_concurrent_loop(runtime.clone(), Some(idle), headers).await;
        return;
    }
    prefetch_evict_playlist(&key);
    hls_concurrent_complete(runtime, hls_segment, headers, status).await;
}
