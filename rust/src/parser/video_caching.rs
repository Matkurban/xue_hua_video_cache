use std::collections::HashMap;

use tokio::net::TcpStream;
use tokio::sync::mpsc;
use url::Url;

use crate::ext::string_ext::to_safe_uri;
use crate::proxy::require_state;

use super::hls_parser::HlsMasterPlaylist;
use super::hls_parser::HlsPlaylist;
use super::url_parser::PrecacheProgress;
use super::url_parser_factory::UrlParserFactory;
use super::url_parser_m3u8::UrlParserM3U8;

/// Video caching entry points mirroring Dart `VideoCaching`.
pub struct VideoCaching;

impl VideoCaching {
    pub async fn parse(stream: TcpStream, uri: Url, headers: HashMap<String, String>) -> bool {
        let state = match require_state() {
            Ok(s) => s,
            Err(_) => return false,
        };
        let parser = UrlParserFactory::create_parser(&uri, &state.ctx);
        parser.parse(stream, uri, headers).await
    }

    pub async fn is_cached(
        url: &str,
        headers: Option<HashMap<String, String>>,
        cache_segments: usize,
    ) -> bool {
        let uri = to_safe_uri(url);
        let state = match require_state() {
            Ok(s) => s,
            Err(_) => return false,
        };
        UrlParserFactory::create_parser(&uri, &state.ctx)
            .is_cached(url, headers, cache_segments)
            .await
    }

    pub async fn precache(
        url: &str,
        headers: Option<HashMap<String, String>>,
        cache_segments: usize,
        download_now: bool,
        progress_tx: Option<mpsc::UnboundedSender<PrecacheProgress>>,
    ) -> Result<(), String> {
        let uri = to_safe_uri(url);
        let state = require_state()?;
        UrlParserFactory::create_parser(&uri, &state.ctx)
            .precache(url, headers, cache_segments, download_now, progress_tx)
            .await
    }

    pub async fn parse_hls_master_playlist(
        url: &str,
        headers: Option<HashMap<String, String>>,
    ) -> Option<HlsMasterPlaylist> {
        let uri = to_safe_uri(url);
        let state = require_state().ok()?;
        let parser = UrlParserFactory::create_parser(&uri, &state.ctx);
        let m3u8 = UrlParserM3U8;
        if !state.ctx.url_matcher.match_m3u8(&uri)
            && !state.ctx.url_matcher.match_m3u8_key(&uri)
            && !state.ctx.url_matcher.match_m3u8_segment(&uri)
        {
            let _ = parser;
            return None;
        }
        let hls_key = crate::ext::string_ext::generate_md5(url);
        match m3u8
            .parse_playlist(&uri, headers.as_ref(), Some(&hls_key))
            .await?
        {
            HlsPlaylist::Master(master) => Some(master),
            _ => None,
        }
    }
}
