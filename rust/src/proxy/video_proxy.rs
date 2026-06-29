use std::sync::Arc;

use once_cell::sync::OnceCell;
use parking_lot::Mutex;

use crate::cache::LruCacheSingleton;
use crate::download::DownloadManager;
use crate::ext::file_ext::FileExt;
use crate::ext::log_ext::set_log_enabled;
use crate::global::CacheKeyConfig;
use crate::parser::hls_registry::clear_hls_registry;

use super::app_context::AppContext;
use super::local_proxy_server::LocalProxyServer;
use super::platform_kind::PlatformKind;
use super::proxy_runtime::ProxyRuntime;

static STATE: OnceCell<Arc<VideoProxyState>> = OnceCell::new();

pub struct VideoProxyState {
    pub runtime: Arc<ProxyRuntime>,
    local_proxy: Mutex<Option<LocalProxyServer>>,
    max_concurrent: Mutex<usize>,
    initialized: Mutex<bool>,
    disposed: Mutex<bool>,
}

impl VideoProxyState {
    pub fn get() -> Option<Arc<VideoProxyState>> {
        STATE.get().cloned()
    }

    pub fn ctx(&self) -> Arc<AppContext> {
        self.runtime.ctx.clone()
    }

    pub fn download_manager(&self) -> Arc<DownloadManager> {
        self.runtime.downloads()
    }

    pub async fn init(
        ip: Option<String>,
        port: Option<u16>,
        max_memory_cache_size: i64,
        max_storage_cache_size: i64,
        cache_dir: String,
        log_print: bool,
        segment_size: i64,
        max_concurrent_downloads: usize,
        platform: PlatformKind,
        cache_key_config: CacheKeyConfig,
    ) -> Result<(), String> {
        if let Some(state) = STATE.get() {
            if state.is_disposed() {
                return Err("XueHUAEVideoCache has been disposed".to_string());
            }
            apply_init_config(
                &state.runtime.ctx,
                &state.max_concurrent,
                ip,
                port,
                max_memory_cache_size,
                max_storage_cache_size,
                cache_dir,
                log_print,
                segment_size,
                max_concurrent_downloads,
            );
            return state.restart().await;
        }

        let ctx = Arc::new(AppContext::new(platform, cache_key_config));
        let max_concurrent = Mutex::new(max_concurrent_downloads);
        apply_init_config(
            &ctx,
            &max_concurrent,
            ip,
            port,
            max_memory_cache_size,
            max_storage_cache_size,
            cache_dir,
            log_print,
            segment_size,
            max_concurrent_downloads,
        );
        let cache = LruCacheSingleton::instance();
        let dm = Arc::new(DownloadManager::new(
            max_concurrent_downloads,
            ctx.clone(),
            cache.clone(),
        ));
        let runtime = Arc::new(ProxyRuntime::new(ctx, dm, cache));
        let mut proxy = LocalProxyServer::new(runtime.clone());
        proxy.start().await.map_err(|e| e.to_string())?;
        let state = Arc::new(VideoProxyState {
            runtime,
            local_proxy: Mutex::new(Some(proxy)),
            max_concurrent,
            initialized: Mutex::new(true),
            disposed: Mutex::new(false),
        });
        STATE
            .set(state.clone())
            .map_err(|_| "XueHUAEVideoCache already initialized".to_string())?;
        spawn_health_monitor(state);
        Ok(())
    }

    pub async fn restart(&self) -> Result<(), String> {
        if !*self.initialized.lock() {
            return Err(
                "XueHUAEVideoCache.initialize() must be called before restart()".to_string(),
            );
        }
        self.runtime.downloads().dispose();
        clear_hls_registry();
        let max = *self.max_concurrent.lock();
        let new_dm = Arc::new(DownloadManager::new(
            max,
            self.runtime.ctx.clone(),
            self.runtime.cache.clone(),
        ));
        self.runtime.replace_downloads(new_dm);
        let mut proxy_opt = self.local_proxy.lock().take();
        if let Some(ref mut proxy) = proxy_opt {
            proxy.restart().await.map_err(|e| e.to_string())?;
        }
        *self.local_proxy.lock() = proxy_opt;
        Ok(())
    }

    pub async fn is_running(&self) -> bool {
        let (ip, port) = {
            let config = self.runtime.ctx.config.read();
            (config.ip.clone(), config.port)
        };
        for _ in 0..3 {
            let ok = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                tokio::net::TcpStream::connect(format!("{ip}:{port}")),
            )
            .await
            .map(|r| r.is_ok())
            .unwrap_or(false);
            if ok {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        false
    }

    pub async fn restart_proxy_if_unhealthy(&self) {
        if self.is_running().await {
            return;
        }
        let mut proxy_opt = self.local_proxy.lock().take();
        if let Some(ref mut proxy) = proxy_opt {
            let _ = proxy.restart().await;
        }
        *self.local_proxy.lock() = proxy_opt;
    }

    pub fn dispose(&self) {
        if *self.disposed.lock() {
            return;
        }
        *self.disposed.lock() = true;
        *self.initialized.lock() = false;
        self.runtime.downloads().dispose();
        if let Some(mut proxy) = self.local_proxy.lock().take() {
            proxy.shutdown_listener();
        }
        clear_hls_registry();
    }

    pub fn is_disposed(&self) -> bool {
        *self.disposed.lock()
    }
}

fn apply_init_config(
    ctx: &Arc<AppContext>,
    max_concurrent: &Mutex<usize>,
    ip: Option<String>,
    port: Option<u16>,
    max_memory_cache_size: i64,
    max_storage_cache_size: i64,
    cache_dir: String,
    log_print: bool,
    segment_size: i64,
    max_concurrent_downloads: usize,
) {
    let memory_size;
    let storage_size;
    {
        let mut config = ctx.config.write();
        config.memory_cache_size = max_memory_cache_size * config.mb_size;
        config.storage_cache_size = max_storage_cache_size * config.mb_size;
        config.segment_size = segment_size * config.mb_size;
        if let Some(ip) = ip {
            config.ip = ip;
        }
        if let Some(port) = port {
            config.port = port;
        }
        memory_size = config.memory_cache_size;
        storage_size = config.storage_cache_size;
    }
    *max_concurrent.lock() = max_concurrent_downloads;
    LruCacheSingleton::reconfigure(memory_size, storage_size);
    set_log_enabled(log_print);
    if !cache_dir.is_empty() {
        FileExt::set_cache_root_path(cache_dir);
    }
}

fn spawn_health_monitor(state: Arc<VideoProxyState>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(10));
        loop {
            ticker.tick().await;
            if !*state.initialized.lock() || *state.disposed.lock() {
                break;
            }
            state.restart_proxy_if_unhealthy().await;
        }
    });
}

pub fn require_runtime() -> Result<Arc<ProxyRuntime>, String> {
    let state = VideoProxyState::get()
        .ok_or_else(|| "XueHUAEVideoCache.initialize() must be called first".to_string())?;
    if state.is_disposed() {
        return Err("XueHUAEVideoCache has been disposed".to_string());
    }
    Ok(state.runtime.clone())
}

#[cfg(test)]
mod video_proxy_tests {
    use super::*;
    use crate::global::CacheKeyConfig;

    async fn init_defaults() -> Result<(), String> {
        VideoProxyState::init(
            None,
            None,
            100,
            1024,
            String::new(),
            false,
            2,
            4,
            PlatformKind::Other,
            CacheKeyConfig::default(),
        )
        .await
    }

    #[tokio::test]
    async fn init_twice_is_idempotent() {
        init_defaults().await.expect("first init");
        init_defaults().await.expect("second init should not fail");
        let state = VideoProxyState::get().expect("state");
        assert!(state.is_running().await);
    }
}
