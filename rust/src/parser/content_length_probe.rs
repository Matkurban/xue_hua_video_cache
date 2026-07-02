use std::collections::HashMap;
use std::sync::Arc;

use regex::Regex;
use tokio::sync::OnceCell;
use url::Url;

use crate::download::DownloadTask;
use crate::ext::log_ext::log_d;
use crate::proxy::ProxyRuntime;

use super::segment_resolver::SegmentResolver;

/// Deduplicated origin content-length probe (cache → single in-flight GET per URI).
pub(crate) struct ContentLengthProbe;

impl ContentLengthProbe {
    pub(crate) async fn probe(
        runtime: &Arc<ProxyRuntime>,
        uri: &Url,
        headers: &HashMap<String, String>,
    ) -> Result<i64, String> {
        if let Some(len) = read_cached_content_length(runtime, uri, headers).await {
            if len > 0 {
                return Ok(len);
            }
        }

        let key = uri.as_str().to_string();
        let cell = {
            let mut inflight = runtime.content_length_inflight.lock();
            inflight
                .entry(key.clone())
                .or_insert_with(|| Arc::new(OnceCell::new()))
                .clone()
        };

        let outcome = cell
            .get_or_init(|| async {
                let len = origin_probe_via_get(runtime, uri, Some(headers)).await?;
                if len > 0 {
                    let mut probe = DownloadTask::new(uri.clone(), None);
                    probe.headers = Some(headers.clone());
                    SegmentResolver::store_content_length(runtime, &probe, len).await;
                    Ok(len)
                } else {
                    Err("origin content-length probe failed".to_string())
                }
            })
            .await
            .clone();

        runtime.content_length_inflight.lock().remove(&key);
        outcome
    }
}

/// GET with `Range: bytes=0-0` — matches browser/download path (follows redirects).
pub(crate) async fn origin_probe_via_get(
    runtime: &Arc<ProxyRuntime>,
    uri: &Url,
    headers: Option<&HashMap<String, String>>,
) -> Result<i64, String> {
    let config = runtime.ctx.config.read().clone();
    let mut request = runtime
        .ctx
        .http_client
        .get(uri.as_str())
        .header("Range", "bytes=0-0");
    if let Some(h) = headers {
        for (k, v) in h {
            let kl = k.to_lowercase();
            if kl == "host" && v == &config.server_url() {
                continue;
            }
            if kl == "range" {
                continue;
            }
            request = request.header(k.as_str(), v.as_str());
        }
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("origin GET probe failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("origin GET probe status {status}"));
    }

    if let Some(content_range) = response.headers().get("content-range") {
        if let Ok(s) = content_range.to_str() {
            if let Some(total) = parse_content_range_total(s) {
                log_d(&format!(
                    "[ContentLengthProbe] GET probe content-range total={total} uri={uri}"
                ));
                return Ok(total);
            }
        }
    }

    if let Some(len) = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok())
    {
        if len > 0 {
            log_d(&format!(
                "[ContentLengthProbe] GET probe content-length={len} uri={uri}"
            ));
            return Ok(len);
        }
    }

    Err("origin GET probe: no content-length or content-range".to_string())
}

fn parse_content_range_total(content_range: &str) -> Option<i64> {
    let re = Regex::new(r"bytes (\d+)-(\d+)/(\d+)").ok()?;
    let caps = re.captures(content_range)?;
    let total = caps.get(3)?.as_str();
    if total.is_empty() || total == "0" {
        return None;
    }
    total.parse::<i64>().ok()
}

/// Returns a positive cached content length when already stored; does not contact origin.
pub(crate) async fn cached_content_length(
    runtime: &Arc<ProxyRuntime>,
    uri: &Url,
    headers: &HashMap<String, String>,
) -> Option<i64> {
    read_cached_content_length(runtime, uri, headers).await
}

async fn read_cached_content_length(
    runtime: &Arc<ProxyRuntime>,
    uri: &Url,
    headers: &std::collections::HashMap<String, String>,
) -> Option<i64> {
    let mut probe = DownloadTask::new(uri.clone(), None);
    probe.headers = Some(headers.clone());
    SegmentResolver::read_content_length(runtime, &probe).await
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use url::Url;

    use crate::cache::LruCacheSingleton;
    use crate::download::DownloadManager;
    use crate::global::CacheKeyConfig;
    use crate::proxy::platform_kind::PlatformKind;
    use crate::proxy::app_context::AppContext;
    use crate::proxy::{ProxyRuntime, build_test_runtime};
    use crate::test_urls::SAMPLE_MP4;

    use super::*;

    fn build_android_runtime() -> Arc<ProxyRuntime> {
        let ctx = Arc::new(AppContext::new(
            PlatformKind::Android,
            CacheKeyConfig::default(),
        ));
        let cache = LruCacheSingleton::instance();
        let downloads = Arc::new(DownloadManager::new(2, ctx.clone(), cache.clone()));
        Arc::new(ProxyRuntime::new(ctx, downloads, cache))
    }

    async fn read_http_request(stream: &mut tokio::net::TcpStream) -> String {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 512];
        loop {
            let n = stream.read(&mut chunk).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        String::from_utf8_lossy(&buf).into_owned()
    }

    async fn write_http_response(stream: &mut tokio::net::TcpStream, body: &str) {
        let _ = stream.write_all(body.as_bytes()).await;
    }

    #[tokio::test]
    async fn get_probe_parses_content_range_from_redirect_target() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let api_url = format!("http://{addr}/api/video.mp4");
        let target_url = format!("http://{addr}/target/video.mp4");
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();

        let server = tokio::spawn(async move {
            let _ = ready_tx.send(());
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let req = read_http_request(&mut stream).await;
                if req.starts_with("GET /api/video.mp4") {
                    assert!(
                        req.to_lowercase().contains("range: bytes=0-0"),
                        "expected Range header in request: {req}"
                    );
                    write_http_response(
                        &mut stream,
                        &format!(
                            "HTTP/1.1 302 Found\r\nLocation: {target_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        ),
                    )
                    .await;
                    continue;
                }
                if req.starts_with("GET /target/video.mp4") {
                    write_http_response(
                        &mut stream,
                        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-0/1000\r\nContent-Length: 1\r\nConnection: close\r\n\r\nx",
                    )
                    .await;
                    break;
                }
            }
        });

        ready_rx.await.unwrap();
        let runtime = build_test_runtime();
        let uri = Url::parse(&api_url).unwrap();
        let len = origin_probe_via_get(&runtime, &uri, None)
            .await
            .expect("probe");
        assert_eq!(len, 1000);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn get_probe_rejects_head_404_style_body_length() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}/video.mp4");

        let server = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let req = read_http_request(&mut stream).await;
                if req.starts_with("HEAD ") {
                    write_http_response(
                        &mut stream,
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 18\r\n\r\n",
                    )
                    .await;
                    continue;
                }
                if req.starts_with("GET ") {
                    write_http_response(
                        &mut stream,
                        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-0/5000\r\nContent-Length: 1\r\n\r\nx",
                    )
                    .await;
                    break;
                }
            }
        });

        let runtime = build_test_runtime();
        let uri = Url::parse(&url).unwrap();
        let len = origin_probe_via_get(&runtime, &uri, None)
            .await
            .expect("GET probe");
        assert_eq!(len, 5000);
        assert_ne!(len, 18);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn get_probe_fails_on_404() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}/missing.mp4");

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _req = read_http_request(&mut stream).await;
            write_http_response(
                &mut stream,
                "HTTP/1.1 404 Not Found\r\nContent-Length: 18\r\n\r\n",
            )
            .await;
        });

        let runtime = build_test_runtime();
        let uri = Url::parse(&url).unwrap();
        let err = origin_probe_via_get(&runtime, &uri, None)
            .await
            .unwrap_err();
        assert!(err.contains("404"));
        server.await.unwrap();
    }

    #[tokio::test]
    #[ignore = "network GET — run with: cargo test --ignored inflight_probe"]
    async fn inflight_map_entry_removed_after_probe_attempt() {
        let runtime = build_android_runtime();
        let uri = Url::parse(SAMPLE_MP4).unwrap();
        let headers = HashMap::new();

        let _ = ContentLengthProbe::probe(&runtime, &uri, &headers).await;
        assert!(runtime.content_length_inflight.lock().is_empty());
    }

    #[test]
    fn parse_content_range_total_extracts_file_size() {
        assert_eq!(
            parse_content_range_total("bytes 0-0/12345"),
            Some(12345)
        );
        assert_eq!(parse_content_range_total("bytes 0-0/*"), None);
        assert_eq!(parse_content_range_total("bytes 0-0/0"), None);
    }
}
