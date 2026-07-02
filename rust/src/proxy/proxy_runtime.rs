use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use tokio::sync::OnceCell;

use crate::cache::LruCacheSingleton;
use crate::download::DownloadManager;

use super::app_context::AppContext;

/// Runtime dependencies shared by the local proxy, parsers, and download pool.
pub struct ProxyRuntime {
    pub ctx: Arc<AppContext>,
    downloads: RwLock<Arc<DownloadManager>>,
    pub cache: Arc<LruCacheSingleton>,
    /// In-flight content-length GET probes keyed by origin URI (dedupes concurrent ExoPlayer opens).
    pub(crate) content_length_inflight:
        Mutex<HashMap<String, Arc<OnceCell<Result<i64, String>>>>>,
}

impl ProxyRuntime {
    pub fn new(
        ctx: Arc<AppContext>,
        downloads: Arc<DownloadManager>,
        cache: Arc<LruCacheSingleton>,
    ) -> Self {
        Self {
            ctx,
            downloads: RwLock::new(downloads),
            cache,
            content_length_inflight: Mutex::new(HashMap::new()),
        }
    }

    pub fn downloads(&self) -> Arc<DownloadManager> {
        self.downloads.read().clone()
    }

    pub fn replace_downloads(&self, downloads: Arc<DownloadManager>) {
        *self.downloads.write() = downloads;
    }
}

/// Constructs a [ProxyRuntime] for unit tests without global [super::video_proxy::VideoProxyState].
pub fn build_test_runtime() -> Arc<ProxyRuntime> {
    use super::platform_kind::PlatformKind;
    use crate::global::CacheKeyConfig;

    let ctx = Arc::new(AppContext::new(
        PlatformKind::Other,
        CacheKeyConfig::default(),
    ));
    let cache = LruCacheSingleton::instance();
    let downloads = Arc::new(DownloadManager::new(2, ctx.clone(), cache.clone()));
    Arc::new(ProxyRuntime::new(ctx, downloads, cache))
}

#[cfg(test)]
mod tests {
    use url::Url;

    use crate::parser::url_parser::UrlParser;
    use crate::parser::url_parser_factory::UrlParserFactory;

    use crate::test_urls::SAMPLE_MP4;

    use super::*;

    #[tokio::test]
    async fn parser_works_with_injected_runtime_without_global_init() {
        let runtime = build_test_runtime();
        let uri = Url::parse(SAMPLE_MP4).unwrap();
        let parser = UrlParserFactory::create_parser(&uri, runtime);
        assert!(!parser.is_cached(SAMPLE_MP4, None, 1).await);
    }
}
