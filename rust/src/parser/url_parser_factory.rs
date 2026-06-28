use std::sync::Arc;

use url::Url;

use crate::proxy::app_context::AppContext;

use super::url_parser::UrlParser;
use super::url_parser_default::UrlParserDefault;
use super::url_parser_m3u8::UrlParserM3U8;
use super::url_parser_mp4::UrlParserMp4;

pub struct UrlParserFactory;

impl UrlParserFactory {
    pub fn create_parser(uri: &Url, ctx: &AppContext) -> Arc<dyn UrlParser> {
        let matcher = ctx.url_matcher.as_ref();
        if matcher.match_m3u8(uri) || matcher.match_m3u8_key(uri) || matcher.match_m3u8_segment(uri)
        {
            Arc::new(UrlParserM3U8)
        } else if matcher.match_mp4(uri) {
            Arc::new(UrlParserMp4)
        } else {
            Arc::new(UrlParserDefault)
        }
    }
}
