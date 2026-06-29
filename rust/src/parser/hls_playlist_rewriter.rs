use base64::Engine;
use bytes::Bytes;
use regex::Regex;
use url::Url;

use crate::ext::string_ext::{to_local_url, to_safe_url};
use crate::ext::uri_ext::{uri_base, uri_path_prefix};
use crate::global::Config;

use super::hls_playlist_resolver::{read_lines_from_bytes, resolve_relative_path};
use super::hls_registry::HlsSegment;

/// Rewrite an M3U8 playlist for proxy serving and collect segment metadata (no registry I/O).
///
/// Includes auxiliary URIs (`#EXT-X-KEY`, `#EXT-X-MEDIA`, `#EXT-X-MAP`) in the segment list
/// for registry prefetch. Use [`segments_from_playlist_bytes`] for media-only extraction.
pub(crate) fn rewrite_m3u8_playlist(
    uri: &Url,
    data: &Bytes,
    hls_key: &str,
    config: &Config,
) -> (String, Vec<HlsSegment>) {
    rewrite_m3u8_playlist_inner(uri, data, hls_key, config, true)
}

/// Extract media segment metadata from playlist bytes (precache / segment listing).
pub(crate) fn segments_from_playlist_bytes(
    uri: &Url,
    data: &Bytes,
    hls_key: &str,
    config: &Config,
) -> Vec<HlsSegment> {
    rewrite_m3u8_playlist_inner(uri, data, hls_key, config, false).1
}

fn rewrite_m3u8_playlist_inner(
    uri: &Url,
    data: &Bytes,
    hls_key: &str,
    config: &Config,
    collect_aux_segments: bool,
) -> (String, Vec<HlsSegment>) {
    let lines = read_lines_from_bytes(data);
    let uri_re = Regex::new(r#"URI="([^"]+)""#).unwrap();
    let byterange_re = Regex::new(r"#EXT-X-BYTERANGE:(\d+)(?:@(\d+))?").unwrap();
    let mut buffer = String::new();
    let mut segments = Vec::new();
    let mut last_line = String::new();
    let mut last_end_range: i64 = 0;

    for line in lines {
        let mut line_out = line.clone();
        let hls_line = line.trim();
        let mut parse_uri: Option<String> = None;

        if hls_line.starts_with("#EXT-X-KEY")
            || hls_line.starts_with("#EXT-X-MEDIA")
            || hls_line.starts_with("#EXT-X-MAP")
        {
            if let Some(caps) = uri_re.captures(hls_line) {
                if let Some(m) = caps.get(1) {
                    parse_uri = Some(to_safe_url(m.as_str()));
                    let new_uri = rewrite_uri(&parse_uri.as_ref().unwrap(), uri, config);
                    line_out = hls_line.replace(m.as_str(), &new_uri);
                }
            }
        }

        if last_line.starts_with("#EXTINF")
            || last_line.starts_with("#EXT-X-BYTERANGE")
            || last_line.starts_with("#EXT-X-STREAM-INF")
        {
            if !line.starts_with('#') {
                let safe = to_safe_url(&line);
                line_out = rewrite_uri(&safe, uri, config);
            }
        }

        if collect_aux_segments
            && (hls_line.starts_with("#EXT-X-KEY")
                || hls_line.starts_with("#EXT-X-MEDIA")
                || hls_line.starts_with("#EXT-X-MAP"))
        {
            if let Some(mut parse_uri) = parse_uri.take() {
                if !parse_uri.starts_with("http") {
                    let (relative_path, resolved) = resolve_relative_path(&parse_uri);
                    parse_uri = format!("{}/{}", uri_path_prefix(uri, relative_path), resolved);
                }
                segments.push(HlsSegment::new(hls_key.to_string(), parse_uri));
            }
        }

        if last_line.starts_with("#EXTINF")
            || last_line.starts_with("#EXT-X-BYTERANGE")
            || last_line.starts_with("#EXT-X-STREAM-INF")
        {
            if !line.starts_with('#') {
                let mut hls_line_resolved = to_safe_url(&line);
                if !hls_line_resolved.starts_with("http") {
                    let (relative_path, resolved) = resolve_relative_path(&hls_line_resolved);
                    if resolved.starts_with('/') {
                        let origin = uri_base(uri);
                        hls_line_resolved =
                            if let Ok(origin_url) = Url::parse(&format!("{origin}/")) {
                                origin_url
                                    .join(resolved.trim_start_matches('/'))
                                    .map(|u| u.to_string())
                                    .unwrap_or_else(|_| {
                                        format!("{origin}/{}", resolved.trim_start_matches('/'))
                                    })
                            } else {
                                format!("{origin}/{}", resolved.trim_start_matches('/'))
                            };
                    } else {
                        let prefix = uri_path_prefix(uri, relative_path);
                        let trimmed = resolved.trim_start_matches('/');
                        hls_line_resolved = if let Ok(base_url) = Url::parse(&format!("{prefix}/"))
                        {
                            base_url
                                .join(trimmed)
                                .map(|u| u.to_string())
                                .unwrap_or_else(|_| format!("{prefix}/{trimmed}"))
                        } else {
                            format!("{prefix}/{trimmed}")
                        };
                    }
                }

                let mut start_range = 0i64;
                let mut end_range: Option<i64> = None;
                if last_line.starts_with("#EXT-X-BYTERANGE") {
                    if let Some(caps) = byterange_re.captures(&last_line) {
                        let length: i64 = caps
                            .get(1)
                            .and_then(|m| m.as_str().parse().ok())
                            .unwrap_or(0);
                        if let Some(offset_m) = caps.get(2) {
                            start_range = offset_m.as_str().parse().unwrap_or(0);
                            end_range = if length == 0 {
                                None
                            } else {
                                Some(start_range + length - 1)
                            };
                        } else {
                            start_range = last_end_range;
                            end_range = if length == 0 {
                                None
                            } else {
                                Some(start_range + length - 1)
                            };
                            last_end_range = end_range.unwrap_or(0) + 1;
                        }
                    }
                }

                line_out = rewrite_uri(&hls_line_resolved, uri, config);
                segments.push(HlsSegment::with_range(
                    hls_key.to_string(),
                    hls_line_resolved,
                    start_range,
                    end_range,
                ));
            }
        }

        buffer.push_str(&line_out);
        buffer.push_str("\r\n");
        last_line = line;
    }
    (buffer, segments)
}

fn rewrite_uri(input: &str, origin_uri: &Url, config: &Config) -> String {
    if input.starts_with("http") {
        to_local_url(input, config)
    } else {
        let origin_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(uri_base(origin_uri).as_bytes());
        if input.contains('?') {
            format!("{input}&origin={origin_b64}")
        } else {
            format!("{input}?origin={origin_b64}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::global::Config;
    use crate::parser::hls_registry::query_playlist_keys;

    fn test_config() -> Config {
        Config::default()
    }

    #[test]
    fn rewrite_m3u8_playlist_does_not_touch_registry() {
        let registry_len_before = query_playlist_keys().len();
        let uri = Url::parse("https://cdn.example.com/live/playlist.m3u8").unwrap();
        let data = Bytes::from_static(
            b"#EXTM3U\r\n#EXTINF:10.0,\r\nsegment0.ts\r\n#EXTINF:10.0,\r\nsegment1.ts\r\n",
        );
        let (body, segments) = rewrite_m3u8_playlist(&uri, &data, "playlist_key", &test_config());
        assert_eq!(query_playlist_keys().len(), registry_len_before);
        assert_eq!(segments.len(), 2);
        assert!(segments.iter().all(|s| s.key == "playlist_key"));
        assert!(body.contains("segment0.ts"));
        assert!(body.contains("\r\n"));
    }

    #[test]
    fn rewrite_m3u8_playlist_extracts_byterange_segment() {
        let uri = Url::parse("https://cdn.example.com/vod/playlist.m3u8").unwrap();
        let data = Bytes::from_static(b"#EXTM3U\r\n#EXT-X-BYTERANGE:1024@0\r\nsegment.bin\r\n");
        let (_, segments) = rewrite_m3u8_playlist(&uri, &data, "vod_key", &test_config());
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].start_range, 0);
        assert_eq!(segments[0].end_range, Some(1023));
    }

    #[test]
    fn segments_from_playlist_bytes_omits_aux_uris() {
        let uri = Url::parse("https://cdn.example.com/live/playlist.m3u8").unwrap();
        let data = Bytes::from_static(
            b"#EXTM3U\r\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\r\n#EXTINF:10.0,\r\nseg0.ts\r\n",
        );
        let all = rewrite_m3u8_playlist(&uri, &data, "key", &test_config()).1;
        let media_only = segments_from_playlist_bytes(&uri, &data, "key", &test_config());
        assert_eq!(all.len(), 2);
        assert_eq!(media_only.len(), 1);
        assert!(media_only[0].url.contains("seg0.ts"));
    }
}
