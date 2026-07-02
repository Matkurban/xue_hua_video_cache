use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use url::Url;

use crate::cache::cache_key::{CacheKey, CacheKeyContext};
use crate::download::DownloadTask;
use crate::ext::file_ext::FileExt;
use crate::ext::log_ext::{log_d, log_w};
use crate::ext::socket_ext::{append_string, append_to_writer, write_bad_gateway};
use crate::ext::string_ext::to_safe_uri;
use crate::proxy::ProxyRuntime;

use super::content_length_probe::{ContentLengthProbe, cached_content_length};
use super::download_wait::{CACHE_POLL_TIMEOUT, wait_for_cache};
use super::range_response::{
    RangeSpec, clamped_range_end, effective_streaming_spec, format_content_range_for_file,
    parse_range_from_headers, streaming_body_length_from_spec, streaming_content_length,
    streaming_status_line,
};
use super::segment_fetcher::SegmentFetcher;
use super::segment_resolver::SegmentResolver;

/// Range-based parser behavior variant (default vs MP4-specific partial semantics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RangeParseMode {
    Default,
    Mp4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RespondPath {
    ProbeFast,
    BoundedRange,
    ProbeGet,
}

pub(crate) struct RangeResponder;

fn is_probe_range(range_spec: Option<&RangeSpec>) -> bool {
    matches!(
        range_spec,
        Some(RangeSpec {
            start: 0,
            end: Some(1)
        })
    )
}

fn is_bounded_range(range_spec: Option<&RangeSpec>) -> bool {
    range_spec.is_some_and(|spec| spec.end.is_some())
}

fn incoming_range_header(headers: &HashMap<String, String>) -> Option<String> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("range"))
        .map(|(_, value)| value.clone())
}

fn log_respond_path(path: RespondPath, range_header: Option<&str>) {
    let path_name = match path {
        RespondPath::ProbeFast => "probe_fast",
        RespondPath::BoundedRange => "bounded_range",
        RespondPath::ProbeGet => "probe_get",
    };
    log_d(&format!(
        "[RangeResponder] range={} path={path_name}",
        range_header.unwrap_or("(none)")
    ));
}

impl RangeResponder {
    pub(crate) async fn respond(
        runtime: &Arc<ProxyRuntime>,
        mut stream: TcpStream,
        uri: Url,
        headers: HashMap<String, String>,
        mode: RangeParseMode,
    ) -> bool {
        let label = match mode {
            RangeParseMode::Default => "UrlParserDefault",
            RangeParseMode::Mp4 => "UrlParserMp4",
        };
        let mp4_mode = mode == RangeParseMode::Mp4;
        let include_content_range = runtime.ctx.platform.is_android() || mp4_mode;

        let result = async {
            let range_spec = parse_range_from_headers(&headers);
            let range_header = incoming_range_header(&headers);
            let status_line = streaming_status_line(range_spec.as_ref(), mp4_mode);
            let mut response_headers = vec![
                status_line.to_string(),
                "Accept-Ranges: bytes".to_string(),
                "Content-Type: video/mp4".to_string(),
            ];

            parse_streaming(
                runtime,
                &mut stream,
                &uri,
                &mut response_headers,
                range_spec.as_ref(),
                range_header.as_deref(),
                &headers,
                mode,
                include_content_range,
            )
            .await?;
            stream.flush().await.ok();
            Ok::<(), String>(())
        }
        .await;

        if let Err(ref e) = result {
            log_w(&format!("[{label}] ⚠ ⚠ ⚠ parse error: {e}"));
        }
        let _ = stream.shutdown().await;
        log_d("Connection closed\n");
        result.is_ok()
    }
}

async fn parse_streaming(
    runtime: &Arc<ProxyRuntime>,
    stream: &mut TcpStream,
    uri: &Url,
    response_headers: &mut Vec<String>,
    range_spec: Option<&RangeSpec>,
    range_header: Option<&str>,
    headers: &HashMap<String, String>,
    mode: RangeParseMode,
    include_content_range: bool,
) -> Result<(), String> {
    let config = runtime.ctx.config.read().clone();
    let segment_size = config.segment_size;

    if is_probe_range(range_spec) {
        log_respond_path(RespondPath::ProbeFast, range_header);
        return respond_probe_fast(
            runtime,
            stream,
            uri,
            response_headers,
            range_spec.unwrap(),
            headers,
        )
        .await;
    }

    if is_bounded_range(range_spec) {
        log_respond_path(RespondPath::BoundedRange, range_header);
        let spec = range_spec.unwrap();
        let body_length = streaming_body_length_from_spec(spec).unwrap_or(0);
        let file_content_length =
            cached_content_length(runtime, uri, headers).await.unwrap_or(-1);
        let response_end = spec.end.unwrap();

        response_headers.push(format!("content-length: {body_length}"));
        if include_content_range {
            response_headers.push(format!(
                "content-range: {}",
                format_content_range_for_file(spec, response_end, file_content_length)
            ));
        }
        let header_block = response_headers.join("\r\n");
        if !append_string(stream, &header_block).await {
            return Err("write headers failed".into());
        }

        return serve_segments(
            runtime,
            stream,
            uri,
            headers,
            spec.start,
            response_end,
            segment_size,
            mode,
        )
        .await;
    }

    log_respond_path(RespondPath::ProbeGet, range_header);
    let content_length = match ContentLengthProbe::probe(runtime, uri, headers).await {
        Ok(len) => len,
        Err(e) => {
            log_w(&format!("[RangeResponder] probe_get failed: {e}"));
            if !write_bad_gateway(stream, &e).await {
                return Err("write 502 failed".into());
            }
            return Ok(());
        }
    };

    if content_length <= 0 {
        if !write_bad_gateway(stream, "origin content-length probe failed").await {
            return Err("write 502 failed".into());
        }
        return Ok(());
    }

    let range_spec = effective_streaming_spec(range_spec.copied());
    let request_range_end = clamped_range_end(&range_spec, content_length);
    response_headers.push(format!(
        "content-length: {}",
        streaming_content_length(&range_spec, content_length)
    ));
    if include_content_range {
        response_headers.push(format!(
            "content-range: {}",
            format_content_range_for_file(&range_spec, request_range_end, content_length)
        ));
    }
    let header_block = response_headers.join("\r\n");
    if !append_string(stream, &header_block).await {
        return Err("write headers failed".into());
    }

    serve_segments(
        runtime,
        stream,
        uri,
        headers,
        range_spec.start,
        request_range_end,
        segment_size,
        mode,
    )
    .await
}

async fn respond_probe_fast(
    runtime: &Arc<ProxyRuntime>,
    stream: &mut TcpStream,
    uri: &Url,
    response_headers: &mut Vec<String>,
    probe_spec: &RangeSpec,
    headers: &HashMap<String, String>,
) -> Result<(), String> {
    let file_content_length = cached_content_length(runtime, uri, headers)
        .await
        .unwrap_or(-1);
    response_headers.push("content-length: 2".to_string());
    response_headers.push(format!(
        "content-range: {}",
        format_content_range_for_file(probe_spec, 1, file_content_length)
    ));
    let header_block = response_headers.join("\r\n");
    if !append_string(stream, &header_block).await {
        return Err("write headers failed".into());
    }
    let probe_bytes = fetch_range_bytes(runtime, uri, headers, 0, 1).await;
    let _ = append_to_writer(stream, &probe_bytes).await;
    Ok(())
}

async fn concurrent(
    runtime: &Arc<ProxyRuntime>,
    task: &DownloadTask,
    headers: &HashMap<String, String>,
) {
    let config = runtime.ctx.config.read().clone();
    let segment_size = config.segment_size;
    let dm = runtime.downloads();
    let pool = dm.pool();
    let matcher = runtime.ctx.url_matcher.as_ref();

    let ctx = CacheKeyContext::new(config.clone(), matcher);

    let mut new_task = task.clone();
    let url = new_task.url();
    let uri = to_safe_uri(&url);
    let content_length = match ContentLengthProbe::probe(runtime, &uri, headers).await {
        Ok(len) => len,
        Err(_) => -1,
    };
    const MAX_LOOKAHEAD_SKIPS: i32 = 128;
    let mut skipped = 0i32;
    let mut active_size = pool
        .task_list()
        .iter()
        .filter(|t| t.lock().url() == url)
        .count();

    while active_size < 2 {
        new_task.start_range += segment_size;
        if content_length > 0 && new_task.start_range >= content_length {
            break;
        }
        new_task.end_range = Some((new_task.start_range + segment_size * 2 - 1).min(
            if content_length > 0 {
                content_length - 1
            } else {
                i64::MAX
            },
        ));
        new_task.headers = Some(headers.clone());

        let key = CacheKey::for_task(&new_task, &ctx);
        let mut is_exit = pool
            .task_list()
            .iter()
            .any(|t| ctx.entry_matches(&t.lock(), &key.entry));
        if runtime.cache.memory_get(&key.entry).await.is_some() {
            is_exit = true;
        }
        if let Ok(cache_path) = FileExt::create_cache_path(Some(&key.directory)).await {
            new_task.cache_dir = cache_path.clone();
            let save_path = CacheKey::for_task(&new_task, &ctx).save_path(&new_task);
            if Path::new(&save_path).exists() {
                is_exit = true;
            }
        }
        if is_exit {
            skipped += 1;
            if skipped >= MAX_LOOKAHEAD_SKIPS {
                break;
            }
            continue;
        }
        skipped = 0;
        log_d(&format!(
            "Asynchronous download start： {} {}-{:?}",
            new_task.url(),
            new_task.start_range,
            new_task.end_range
        ));
        let arc = Arc::new(Mutex::new(new_task.clone()));
        dm.submit(arc).await;
        active_size = pool
            .task_list()
            .iter()
            .filter(|t| t.lock().url() == url)
            .count();
    }
}

async fn fetch_range_bytes(
    runtime: &Arc<ProxyRuntime>,
    uri: &Url,
    headers: &HashMap<String, String>,
    start: i64,
    end: i64,
) -> bytes::Bytes {
    let mut task = DownloadTask::new(uri.clone(), None);
    task.headers = Some(headers.clone());
    task.start_range = start;
    task.end_range = Some(end);

    if let Some(data) = SegmentResolver::resolve(runtime, &task).await {
        let want = (end - start + 1).max(0) as usize;
        if data.len() >= want {
            return data.slice(0..want);
        }
        return data;
    }

    let arc = Arc::new(Mutex::new(task));
    if let Some(data) = SegmentFetcher::download(runtime, arc).await {
        let want = (end - start + 1).max(0) as usize;
        if data.len() >= want {
            return data.slice(0..want);
        }
        return data;
    }
    bytes::Bytes::new()
}

async fn serve_segments(
    runtime: &Arc<ProxyRuntime>,
    stream: &mut TcpStream,
    uri: &Url,
    headers: &HashMap<String, String>,
    request_range_start: i64,
    request_range_end: i64,
    segment_size: i64,
    mode: RangeParseMode,
) -> Result<(), String> {
    let dm = runtime.downloads();

    let mut downloading = true;
    let mut start_range = request_range_start - (request_range_start % segment_size);
    let mut end_range = start_range + segment_size - 1;
    let mut retry = 3;

    while downloading {
        if mode == RangeParseMode::Mp4 && end_range > request_range_end {
            end_range = request_range_end;
        }

        let mut task = DownloadTask::new(uri.clone(), None);
        task.headers = Some(headers.clone());
        task.start_range = start_range;
        task.end_range = Some(end_range);
        log_d(&format!(
            "Start {} Request range：{}-{}",
            task.url(),
            task.start_range,
            task.end_range.unwrap_or(-1)
        ));

        let mut data = SegmentResolver::resolve(runtime, &task).await;
        if data.is_none() && dm.is_task_exist(&task) {
            data = wait_for_cache(
                || SegmentResolver::resolve(runtime, &task),
                CACHE_POLL_TIMEOUT,
            )
            .await;
        }
        if data.is_none() {
            concurrent(runtime, &task, headers).await;
            let arc = Arc::new(Mutex::new(task.clone()));
            {
                arc.lock().priority += 2;
            }
            data = SegmentFetcher::download(runtime, arc).await;
        }
        if data.is_none() {
            retry -= 1;
            if retry == 0 {
                break;
            }
            continue;
        }

        let slice = data.unwrap();
        let mut start_index = 0usize;
        let mut end_index = slice.len();
        if start_range < request_range_start {
            start_index = (request_range_start - start_range) as usize;
        }
        if end_range > request_range_end {
            end_index = (request_range_end - start_range + 1) as usize;
        }
        let chunk = if start_index > 0 || end_index < slice.len() {
            slice.slice(start_index..end_index)
        } else {
            slice
        };

        if !append_to_writer(stream, &chunk).await {
            downloading = false;
        }
        start_range += segment_size;
        end_range = start_range + segment_size - 1;
        if start_range > request_range_end {
            downloading = false;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::range_response::{parse_range_header, streaming_body_length_from_spec};

    #[test]
    fn is_probe_range_matches_bytes_zero_one() {
        let spec = parse_range_header("bytes=0-1").unwrap();
        assert!(is_probe_range(Some(&spec)));
    }

    #[test]
    fn is_probe_range_rejects_other_ranges() {
        let spec = parse_range_header("bytes=0-").unwrap();
        assert!(!is_probe_range(Some(&spec)));
    }

    #[test]
    fn is_bounded_range_matches_closed_range() {
        let spec = parse_range_header("bytes=1048576-2097151").unwrap();
        assert!(is_bounded_range(Some(&spec)));
        assert_eq!(streaming_body_length_from_spec(&spec), Some(1_048_576));
    }

    #[test]
    fn is_bounded_range_rejects_open_ended() {
        let spec = parse_range_header("bytes=100-").unwrap();
        assert!(!is_bounded_range(Some(&spec)));
    }

    #[test]
    fn probe_fast_headers_include_content_length_two() {
        let spec = parse_range_header("bytes=0-1").unwrap();
        assert!(is_probe_range(Some(&spec)));
        assert_eq!(streaming_body_length_from_spec(&spec), Some(2));
    }
}
