use url::Url;

/// Parsed HLS playlist (master or media).
#[derive(Debug, Clone)]
pub enum HlsPlaylist {
    Master(HlsMasterPlaylist),
    Media(HlsMediaPlaylist),
}

#[derive(Debug, Clone)]
pub struct HlsMasterPlaylist {
    pub media_playlist_urls: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct HlsMediaPlaylist {
    pub base_uri: Option<Url>,
    pub segments: Vec<HlsMediaSegment>,
}

#[derive(Debug, Clone)]
pub struct HlsMediaSegment {
    pub url: Option<String>,
    pub byterange_offset: Option<i64>,
    pub byterange_length: Option<i64>,
}

/// Minimal HLS playlist parser supporting master playlists and media segments with
/// `#EXT-X-BYTERANGE`.
pub fn parse_playlist(uri: &Url, lines: &[String]) -> Option<HlsPlaylist> {
    if lines.is_empty() {
        return None;
    }

    let is_master = lines
        .iter()
        .any(|l| l.trim().starts_with("#EXT-X-STREAM-INF"));
    if is_master {
        return Some(HlsPlaylist::Master(parse_master(lines)));
    }

    let is_media = lines.iter().any(|l| l.trim().starts_with("#EXTINF"));
    if is_media {
        return Some(HlsPlaylist::Media(parse_media(uri, lines)));
    }

    None
}

fn parse_master(lines: &[String]) -> HlsMasterPlaylist {
    let mut urls = Vec::new();
    let mut last_is_stream_inf = false;
    for line in lines {
        let trimmed = line.trim();
        if trimmed.starts_with("#EXT-X-STREAM-INF") {
            last_is_stream_inf = true;
            continue;
        }
        if last_is_stream_inf && !trimmed.starts_with('#') && !trimmed.is_empty() {
            urls.push(trimmed.to_string());
            last_is_stream_inf = false;
        } else if trimmed.starts_with('#') {
            last_is_stream_inf = false;
        }
    }
    HlsMasterPlaylist {
        media_playlist_urls: urls,
    }
}

fn parse_media(uri: &Url, lines: &[String]) -> HlsMediaPlaylist {
    let mut segments = Vec::new();
    let mut last_end_range: i64 = 0;
    let mut pending_byterange: Option<(i64, Option<i64>)> = None;

    for line in lines {
        let trimmed = line.trim();
        if trimmed.starts_with("#EXT-X-BYTERANGE:") {
            pending_byterange = parse_byterange_tag(trimmed, last_end_range);
            if let Some((_start, Some(end))) = pending_byterange {
                last_end_range = end;
            }
            continue;
        }
        if trimmed.starts_with("#EXTINF") {
            continue;
        }
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }

        let (start, length) = pending_byterange.take().unwrap_or((0, None));
        let end_range = length
            .map(|len| {
                if len == 0 {
                    None
                } else {
                    Some(start + len - 1)
                }
            })
            .flatten();

        segments.push(HlsMediaSegment {
            url: Some(trimmed.to_string()),
            byterange_offset: if start > 0 || length.is_some() {
                Some(start)
            } else {
                None
            },
            byterange_length: length,
        });
        if let Some(end) = end_range {
            last_end_range = end + 1;
        }
    }

    HlsMediaPlaylist {
        base_uri: Some(uri.clone()),
        segments,
    }
}

fn parse_byterange_tag(line: &str, last_end: i64) -> Option<(i64, Option<i64>)> {
    let payload = line.trim_start_matches("#EXT-X-BYTERANGE:");
    let parts: Vec<&str> = payload.split('@').collect();
    match parts.as_slice() {
        [length] => {
            let len = length.parse::<i64>().ok()?;
            Some((last_end, Some(len)))
        }
        [length, offset] => {
            let len = length.parse::<i64>().ok()?;
            let start = offset.parse::<i64>().ok()?;
            Some((start, Some(len)))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ext::string_ext::to_safe_uri;

    #[test]
    fn parses_master_playlist() {
        let uri = to_safe_uri("https://example.com/master.m3u8");
        let lines = vec![
            "#EXTM3U".to_string(),
            "#EXT-X-STREAM-INF:BANDWIDTH=1280000".to_string(),
            "720p/playlist.m3u8".to_string(),
        ];
        let playlist = parse_playlist(&uri, &lines).unwrap();
        match playlist {
            HlsPlaylist::Master(m) => {
                assert_eq!(m.media_playlist_urls, vec!["720p/playlist.m3u8"]);
            }
            _ => panic!("expected master"),
        }
    }

    #[test]
    fn parses_media_byterange() {
        let uri = to_safe_uri("https://example.com/media.m3u8");
        let lines = vec![
            "#EXTM3U".to_string(),
            "#EXT-X-BYTERANGE:1000@5000".to_string(),
            "#EXTINF:10,".to_string(),
            "segment.ts".to_string(),
        ];
        let playlist = parse_playlist(&uri, &lines).unwrap();
        match playlist {
            HlsPlaylist::Media(m) => {
                assert_eq!(m.segments.len(), 1);
                assert_eq!(m.segments[0].byterange_offset, Some(5000));
                assert_eq!(m.segments[0].byterange_length, Some(1000));
            }
            _ => panic!("expected media"),
        }
    }
}
