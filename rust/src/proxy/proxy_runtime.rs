use std::sync::Arc;

use parking_lot::RwLock;

use crate::cache::LruCacheSingleton;
use crate::download::DownloadManager;

use super::app_context::AppContext;

/// Runtime dependencies shared by the local proxy, parsers, and download pool.
pub struct ProxyRuntime {
    pub ctx: Arc<AppContext>,
    downloads: RwLock<Arc<DownloadManager>>,
    pub cache: Arc<LruCacheSingleton>,
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

    use super::*;

    #[tokio::test]
    async fn parser_works_with_injected_runtime_without_global_init() {
        let runtime = build_test_runtime();
        let uri = Url::parse("https://example.com/video.mp4").unwrap();
        let parser = UrlParserFactory::create_parser(&uri, runtime);
        assert!(
            !parser
                .is_cached("https://example.com/video.mp4", None, 1)
                .await
        );
    }
}
