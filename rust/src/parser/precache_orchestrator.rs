use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use url::Url;

use crate::download::DownloadTask;
use crate::ext::string_ext::to_safe_uri;
use crate::proxy::ProxyRuntime;

use super::content_length_probe::ContentLengthProbe;
use super::segment_fetcher::SegmentFetcher;
use super::segment_resolver::SegmentResolver;
use super::url_parser::PrecacheProgress;

const QUEUED_PRECACHE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

pub(crate) struct PrecacheOrchestrator;

impl PrecacheOrchestrator {
    async fn segments_cached(
        runtime: &Arc<ProxyRuntime>,
        uri: &Url,
        headers: Option<HashMap<String, String>>,
        content_length: i64,
        mut cache_segments: usize,
    ) -> bool {
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

    pub(crate) async fn is_cached(
        runtime: &Arc<ProxyRuntime>,
        url: &str,
        headers: Option<HashMap<String, String>>,
        cache_segments: usize,
    ) -> bool {
        let uri = to_safe_uri(url);
        let header_map = headers.clone().unwrap_or_default();
        let content_length = ContentLengthProbe::probe(runtime, &uri, &header_map)
            .await
            .unwrap_or(-1);
        Self::segments_cached(runtime, &uri, headers, content_length, cache_segments).await
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
        let header_map = headers.clone().unwrap_or_default();
        let content_length = ContentLengthProbe::probe(runtime, &uri, &header_map)
            .await
            .unwrap_or(-1);
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
                        if !ok {
                            return false;
                        }
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
                        true
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
                if Self::segments_cached(
                    runtime,
                    &uri,
                    headers_for_wait.clone(),
                    content_length,
                    cache_segments,
                )
                .await
                {
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

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard};

    use bytes::Bytes;
    use url::Url;

    use crate::cache::cache_key::{CacheKey, CacheKeyContext};
    use crate::ext::string_ext::to_safe_uri;
    use crate::global::CacheKeyConfig;
    use crate::matchers::UrlMatcherConfigurable;
    use crate::proxy::build_test_runtime;

    use super::*;

    fn cache_test_lock() -> MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[tokio::test]
    async fn segments_cached_returns_true_when_segments_in_memory() {
        let _guard = cache_test_lock();
        let runtime = build_test_runtime();
        runtime.cache.memory_clear().await;
        let config = runtime.ctx.config.read().clone();
        let matcher = UrlMatcherConfigurable::new(&CacheKeyConfig::default());
        let ctx = CacheKeyContext::new(config.clone(), &matcher);
        let url = "https://example.com/segments-cached.mp4";
        let uri = to_safe_uri(url);
        let content_length = config.segment_size * 2;

        SegmentResolver::store_content_length(&runtime, &DownloadTask::new(uri.clone(), None), content_length)
            .await;

        for count in 0..1 {
            let mut task = DownloadTask::new(uri.clone(), None);
            task.start_range = config.segment_size * count;
            task.end_range = Some(task.start_range + config.segment_size - 1);
            let key = CacheKey::for_task(&task, &ctx);
            runtime
                .cache
                .memory_put(&key.entry, Bytes::from_static(b"segment"))
                .await;
        }

        assert!(
            PrecacheOrchestrator::segments_cached(&runtime, &uri, None, content_length, 1).await
        );
        assert!(
            !PrecacheOrchestrator::segments_cached(&runtime, &uri, None, content_length, 2).await
        );
    }

    #[tokio::test]
    async fn is_cached_matches_segments_cached_after_manual_cache_fill() {
        let _guard = cache_test_lock();
        let runtime = build_test_runtime();
        runtime.cache.memory_clear().await;
        let config = runtime.ctx.config.read().clone();
        let matcher = UrlMatcherConfigurable::new(&CacheKeyConfig::default());
        let ctx = CacheKeyContext::new(config.clone(), &matcher);
        let url = "https://example.com/is-cached-fill.mp4";
        let uri = Url::parse(url).unwrap();
        let content_length = config.segment_size;

        SegmentResolver::store_content_length(
            &runtime,
            &DownloadTask::new(uri.clone(), None),
            content_length,
        )
        .await;

        let mut task = DownloadTask::new(uri, None);
        task.start_range = 0;
        task.end_range = Some(config.segment_size - 1);
        let key = CacheKey::for_task(&task, &ctx);
        runtime
            .cache
            .memory_put(&key.entry, Bytes::from_static(b"segment"))
            .await;

        assert!(PrecacheOrchestrator::is_cached(&runtime, url, None, 1).await);
    }
}
