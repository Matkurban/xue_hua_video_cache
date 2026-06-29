use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::download::DownloadTask;
use crate::ext::string_ext::to_safe_uri;
use crate::proxy::ProxyRuntime;

use super::range_responder::RangeResponder;
use super::segment_fetcher::SegmentFetcher;
use super::segment_resolver::SegmentResolver;
use super::url_parser::PrecacheProgress;

const QUEUED_PRECACHE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

pub(crate) struct PrecacheOrchestrator;

impl PrecacheOrchestrator {
    pub(crate) async fn is_cached(
        runtime: &Arc<ProxyRuntime>,
        url: &str,
        headers: Option<HashMap<String, String>>,
        mut cache_segments: usize,
    ) -> bool {
        let uri = to_safe_uri(url);
        let content_length = RangeResponder::head(runtime, &uri, headers.as_ref()).await;
        let segment_size = runtime.ctx.config.read().segment_size;

        if content_length > 0 {
            let total_segments = content_length / segment_size
                + if content_length % segment_size > 0 {
                    1
                } else {
                    0
                };
            if cache_segments > total_segments as usize {
                cache_segments = total_segments as usize;
            }
        }

        let mut count = 0;
        while count < cache_segments {
            let mut task = DownloadTask::new(uri.clone(), None);
            task.headers = headers.clone();
            task.start_range = segment_size * count as i64;
            task.end_range = Some(task.start_range + segment_size - 1);
            count += 1;
            if SegmentResolver::resolve(runtime, &task).await.is_none() {
                return false;
            }
        }
        true
    }

    pub(crate) async fn precache(
        runtime: &Arc<ProxyRuntime>,
        url: &str,
        headers: Option<HashMap<String, String>>,
        mut cache_segments: usize,
        download_now: bool,
        progress_tx: Option<tokio::sync::mpsc::UnboundedSender<PrecacheProgress>>,
    ) -> Result<(), String> {
        let tx = progress_tx;

        let uri = to_safe_uri(url);
        let content_length = RangeResponder::head(runtime, &uri, headers.as_ref()).await;
        let config = runtime.ctx.config.read().clone();
        let segment_size = config.segment_size;

        if content_length > 0 {
            let total_segments = content_length / segment_size
                + if content_length % segment_size > 0 {
                    1
                } else {
                    0
                };
            if cache_segments > total_segments as usize {
                cache_segments = total_segments as usize;
            }
        }

        if cache_segments == 0 {
            return Ok(());
        }

        let downloaded = Arc::new(Mutex::new(0usize));
        let total_size = cache_segments;
        let mut failures = 0usize;
        let mut handles = Vec::new();

        for count in 0..cache_segments {
            let mut task = DownloadTask::new(uri.clone(), None);
            task.headers = headers.clone();
            task.start_range = segment_size * count as i64;
            task.end_range = Some(task.start_range + segment_size - 1);
            let task_arc = Arc::new(Mutex::new(task));
            let tx_clone = tx.clone();
            let downloaded = downloaded.clone();

            if download_now {
                let snapshot = task_arc.lock().clone();
                let cached = SegmentResolver::resolve(runtime, &snapshot).await;
                if cached.is_some() {
                    let mut n = downloaded.lock();
                    *n += 1;
                    if let Some(ref sender) = tx_clone {
                        let t = task_arc.lock();
                        let _ = sender.send(PrecacheProgress {
                            progress: *n as f64 / total_size as f64,
                            url: t.url(),
                            start_range: Some(t.start_range),
                            end_range: t.end_range,
                            segment_url: None,
                            parent_url: None,
                            file_name: None,
                            hls_key: None,
                            total_segments: None,
                            current_segment_index: None,
                        });
                    }
                    continue;
                }
                handles.push({
                    let runtime = runtime.clone();
                    tokio::spawn(async move {
                        let ok = SegmentFetcher::download(&runtime, task_arc.clone())
                            .await
                            .is_some();
                        let mut n = downloaded.lock();
                        *n += 1;
                        if let Some(sender) = tx_clone {
                            let t = task_arc.lock();
                            let _ = sender.send(PrecacheProgress {
                                progress: *n as f64 / total_size as f64,
                                url: t.url(),
                                start_range: Some(t.start_range),
                                end_range: t.end_range,
                                segment_url: None,
                                parent_url: None,
                                file_name: None,
                                hls_key: None,
                                total_segments: None,
                                current_segment_index: None,
                            });
                        }
                        ok
                    })
                });
            } else {
                SegmentFetcher::push(runtime, task_arc).await?;
            }
        }

        if !download_now {
            let headers_for_wait = headers.clone();
            let deadline = tokio::time::Instant::now() + QUEUED_PRECACHE_TIMEOUT;
            while tokio::time::Instant::now() < deadline {
                if Self::is_cached(runtime, url, headers_for_wait.clone(), cache_segments).await {
                    return Ok(());
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
            return Err(format!("Queued precache timed out for {url}"));
        }
        for handle in handles {
            match handle.await {
                Ok(true) => {}
                Ok(false) => failures += 1,
                Err(_) => failures += 1,
            }
        }
        if failures > 0 {
            return Err(format!(
                "Failed to precache {failures} of {total_size} segment(s) for {url}"
            ));
        }

        Ok(())
    }
}
