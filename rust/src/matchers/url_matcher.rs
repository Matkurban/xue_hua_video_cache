use async_trait::async_trait;
use url::Url;

#[async_trait]
pub trait UrlMatcher: Send + Sync {
    fn match_m3u8(&self, uri: &Url) -> bool;
    fn match_m3u8_key(&self, uri: &Url) -> bool;
    fn match_m3u8_segment(&self, uri: &Url) -> bool;
    fn match_mp4(&self, uri: &Url) -> bool;
    fn match_cache_key(&self, uri: &Url) -> Url;
}
