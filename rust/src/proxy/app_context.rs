use std::sync::Arc;

use parking_lot::RwLock;
use reqwest::Client;

use crate::global::{CacheKeyConfig, Config};
use crate::http::HttpClientBuilder;
use crate::http::HttpClientDefault;
use crate::matchers::{UrlMatcher, UrlMatcherConfigurable};

use super::platform_kind::PlatformKind;

pub struct AppContext {
    pub config: RwLock<Config>,
    pub url_matcher: Arc<dyn UrlMatcher>,
    pub http_client: Client,
    pub platform: PlatformKind,
}

impl AppContext {
    pub fn new(platform: PlatformKind, cache_key_config: CacheKeyConfig) -> Self {
        let builder = HttpClientDefault;
        Self {
            config: RwLock::new(Config::default()),
            url_matcher: Arc::new(UrlMatcherConfigurable::new(&cache_key_config)),
            http_client: builder.create(),
            platform,
        }
    }
}
