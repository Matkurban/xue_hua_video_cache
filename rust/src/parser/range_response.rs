use std::collections::HashMap;

use bytes::Bytes;

/// Parsed HTTP `Range` header for byte sequences (`bytes=start-end`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RangeSpec {
    pub start: i64,
    /// Inclusive end byte index, or `None` when the request runs through EOF.
    pub end: Option<i64>,
}

/// Buffered HTTP response metadata for a static byte body (M3U8 / full-object serve).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferRangeResponse {
    pub status_line: &'static str,
    pub body: Bytes,
    pub content_length: usize,
    pub content_range: Option<String>,
}

pub fn status_line(spec: Option<&RangeSpec>) -> &'static str {
    if spec.is_some() {
        "HTTP/1.1 206 Partial Content"
    } else {
        "HTTP/1.1 200 OK"
    }
}

/// Parse a `Range` header value such as `bytes=0-1` or `bytes=100-`.
pub fn parse_range_header(value: &str) -> Option<RangeSpec> {
    let mut value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Some(stripped) = value.strip_prefix("bytes=") {
        value = stripped.trim();
    }
    let (start_part, end_part) = value.split_once('-')?;
    let start = start_part.trim().parse::<i64>().ok()?;
    if start < 0 {
        return None;
    }
    let end = if end_part.trim().is_empty() {
        None
    } else {
        Some(end_part.trim().parse::<i64>().ok()?)
    };
    if let Some(end) = end {
        if end < start {
            return None;
        }
    }
    Some(RangeSpec { start, end })
}

/// Case-insensitive lookup of the `Range` request header.
pub fn parse_range_from_headers(headers: &HashMap<String, String>) -> Option<RangeSpec> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("range"))
        .and_then(|(_, value)| parse_range_header(value))
}

pub fn apply_range_to_task(task: &mut crate::download::DownloadTask, spec: &RangeSpec) {
    task.start_range = spec.start;
    task.end_range = spec.end;
}

/// Slice a buffered body and build range response headers (`total` = body length).
pub fn build_buffer_response(data: Bytes, spec: Option<&RangeSpec>) -> BufferRangeResponse {
    let status_line = status_line(spec);
    let Some(spec) = spec else {
        return BufferRangeResponse {
            status_line,
            content_length: data.len(),
            content_range: None,
            body: data,
        };
    };

    let total = data.len();
    if total == 0 {
        return BufferRangeResponse {
            status_line,
            body: data,
            content_length: 0,
            content_range: Some("bytes */0".to_string()),
        };
    }

    let max_index = total as i64 - 1;
    let start = spec.start.clamp(0, max_index);
    let end = spec.end.unwrap_or(max_index).clamp(start, max_index);
    let start_u = start as usize;
    let end_u = end as usize;
    let body = data.slice(start_u..=end_u);
    BufferRangeResponse {
        status_line,
        content_length: body.len(),
        content_range: Some(format_content_range_for_buffer(start, end, total)),
        body,
    }
}

/// `Content-Range` for a buffered object where total is the body size.
pub fn format_content_range_for_buffer(start: i64, end: i64, buffer_total: usize) -> String {
    format!("bytes {start}-{end}/{buffer_total}")
}

/// `Content-Range` for streaming/file serve where total is the full asset size.
pub fn format_content_range_for_file(
    spec: &RangeSpec,
    response_end: i64,
    file_total: i64,
) -> String {
    format!("bytes {}-{}/{}", spec.start, response_end, file_total)
}

/// Whether a streaming MP4/default parser should answer with **206**.
///
/// Default mode treats `bytes=0-` (open-ended from start) as a full-file **200**;
/// MP4 mode treats any present `Range` header as partial.
pub fn is_streaming_partial(spec: Option<&RangeSpec>, mp4_mode: bool) -> bool {
    let Some(spec) = spec else {
        return false;
    };
    if mp4_mode {
        return true;
    }
    spec.start > 0 || spec.end.is_some_and(|end| end > 0)
}

pub fn streaming_status_line(spec: Option<&RangeSpec>, mp4_mode: bool) -> &'static str {
    if is_streaming_partial(spec, mp4_mode) {
        "HTTP/1.1 206 Partial Content"
    } else {
        "HTTP/1.1 200 OK"
    }
}

/// Clamp an requested range to `[0, file_total - 1]`; open-ended uses EOF.
pub fn clamped_range_end(spec: &RangeSpec, file_total: i64) -> i64 {
    if file_total <= 0 {
        return spec.start.max(0);
    }
    let max_end = file_total - 1;
    let end = spec.end.unwrap_or(max_end);
    end.min(max_end).max(spec.start.min(max_end))
}

pub fn streaming_content_length(spec: &RangeSpec, file_total: i64) -> i64 {
    let end = clamped_range_end(spec, file_total);
    (end - spec.start + 1).max(0)
}

/// Effective range for streaming body serve (full file when no `Range` header).
pub fn effective_streaming_spec(spec: Option<RangeSpec>) -> RangeSpec {
    spec.unwrap_or(RangeSpec {
        start: 0,
        end: None,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn parse_closed_range() {
        let spec = parse_range_header("bytes=1048576-2097151").unwrap();
        assert_eq!(spec.start, 1_048_576);
        assert_eq!(spec.end, Some(2_097_151));
    }

    #[test]
    fn parse_open_ended_range() {
        let spec = parse_range_header("bytes=100-").unwrap();
        assert_eq!(spec.start, 100);
        assert_eq!(spec.end, None);
    }

    #[test]
    fn parse_probe_range() {
        let spec = parse_range_header("bytes=0-1").unwrap();
        assert_eq!(spec.start, 0);
        assert_eq!(spec.end, Some(1));
    }

    #[test]
    fn parse_range_from_headers_is_case_insensitive() {
        let mut headers = HashMap::new();
        headers.insert("Range".to_string(), "bytes=0-1".to_string());
        let spec = parse_range_from_headers(&headers).unwrap();
        assert_eq!(spec.end, Some(1));
    }

    #[test]
    fn open_ended_range_uses_206_status() {
        let spec = parse_range_header("bytes=100-").unwrap();
        assert_eq!(status_line(Some(&spec)), "HTTP/1.1 206 Partial Content");
    }

    #[test]
    fn no_range_uses_200_status() {
        assert_eq!(status_line(None), "HTTP/1.1 200 OK");
    }

    #[test]
    fn build_buffer_response_slices_closed_range() {
        let data = Bytes::from_static(b"0123456789");
        let spec = RangeSpec {
            start: 2,
            end: Some(5),
        };
        let response = build_buffer_response(data, Some(&spec));
        assert_eq!(response.status_line, "HTTP/1.1 206 Partial Content");
        assert_eq!(response.body.as_ref(), b"2345");
        assert_eq!(response.content_length, 4);
        assert_eq!(response.content_range.as_deref(), Some("bytes 2-5/10"));
    }

    #[test]
    fn build_buffer_response_open_ended_slices_to_eof() {
        let data = Bytes::from_static(b"0123456789");
        let spec = RangeSpec {
            start: 8,
            end: None,
        };
        let response = build_buffer_response(data, Some(&spec));
        assert_eq!(response.body.as_ref(), b"89");
        assert_eq!(response.content_range.as_deref(), Some("bytes 8-9/10"));
    }

    #[test]
    fn build_buffer_response_without_range_returns_full_body() {
        let data = Bytes::from_static(b"hello");
        let response = build_buffer_response(data, None);
        assert_eq!(response.status_line, "HTTP/1.1 200 OK");
        assert_eq!(response.body.as_ref(), b"hello");
        assert!(response.content_range.is_none());
    }

    #[test]
    fn format_content_range_for_file_uses_asset_total() {
        let spec = RangeSpec {
            start: 1_048_576,
            end: Some(2_097_151),
        };
        assert_eq!(
            format_content_range_for_file(&spec, 2_097_151, 50_000_000),
            "bytes 1048576-2097151/50000000"
        );
    }

    #[test]
    fn streaming_default_open_from_zero_is_not_partial() {
        let spec = RangeSpec {
            start: 0,
            end: None,
        };
        assert!(!is_streaming_partial(Some(&spec), false));
        assert_eq!(streaming_status_line(Some(&spec), false), "HTTP/1.1 200 OK");
    }

    #[test]
    fn streaming_mp4_any_range_is_partial() {
        let spec = RangeSpec {
            start: 0,
            end: None,
        };
        assert!(is_streaming_partial(Some(&spec), true));
    }

    #[test]
    fn clamped_range_end_open_ended_uses_file_total() {
        let spec = RangeSpec {
            start: 100,
            end: None,
        };
        assert_eq!(clamped_range_end(&spec, 1000), 999);
        assert_eq!(streaming_content_length(&spec, 1000), 900);
    }

    #[test]
    fn effective_streaming_spec_defaults_to_full_file() {
        let spec = effective_streaming_spec(None);
        assert_eq!(spec.start, 0);
        assert_eq!(spec.end, None);
    }
}
