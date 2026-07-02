use std::collections::HashMap;
use std::sync::Arc;

use tokio::net::TcpStream;
use tokio::sync::mpsc;
use url::Url;

use crate::ext::string_ext::to_safe_uri;
use crate::ext::uri_ext::hls_key_for_url;
use crate::proxy::ProxyRuntime;

use super::hls_parser::HlsMasterPlaylist;
use super::hls_parser::HlsPlaylist;
use super::url_parser::PrecacheProgress;
use super::url_parser_factory::UrlParserFactory;
use super::url_parser_m3u8::UrlParserM3U8;

/// Video caching entry points mirroring Dart `VideoCaching`.
pub struct VideoCaching;

impl VideoCaching {
    pub async fn parse(
        runtime: Arc<ProxyRuntime>,
        stream: TcpStream,
        uri: Url,
        headers: HashMap<String, String>,
    ) -> bool {
        let parser = UrlParserFactory::create_parser(&uri, runtime);
        parser.parse(stream, uri, headers).await
    }

    pub async fn is_cached(
        runtime: Arc<ProxyRuntime>,
        url: &str,
        headers: Option<HashMap<String, String>>,
        cache_segments: usize,
    ) -> bool {
        let uri = to_safe_uri(url);
        let parser = UrlParserFactory::create_parser(&uri, runtime);
        parser.is_cached(url, headers, cache_segments).await
    }

    pub async fn precache(
        runtime: Arc<ProxyRuntime>,
        url: &str,
        headers: Option<HashMap<String, String>>,
        cache_segments: usize,
        download_now: bool,
        progress_tx: Option<mpsc::UnboundedSender<PrecacheProgress>>,
    ) -> Result<(), String> {
        let uri = to_safe_uri(url);
        let parser = UrlParserFactory::create_parser(&uri, runtime);
        parser
            .precache(url, headers, cache_segments, download_now, progress_tx)
            .await
    }

    pub async fn parse_hls_master_playlist(
        runtime: Arc<ProxyRuntime>,
        url: &str,
        headers: Option<HashMap<String, String>>,
    ) -> Option<HlsMasterPlaylist> {
        let uri = to_safe_uri(url);
        let parser = UrlParserFactory::create_parser(&uri, runtime.clone());
        let m3u8 = UrlParserM3U8::new(runtime.clone());
        if !runtime.ctx.url_matcher.match_m3u8(&uri)
            && !runtime.ctx.url_matcher.match_m3u8_key(&uri)
            && !runtime.ctx.url_matcher.match_m3u8_segment(&uri)
        {
            let _ = parser;
            return None;
        }
        let hls_key = hls_key_for_url(url);
        match m3u8
            .parse_playlist(&uri, headers.as_ref(), Some(&hls_key))
            .await?
        {
            HlsPlaylist::Master(master) => Some(master),
            _ => None,
        }
    }
}
