use std::sync::Arc;

use once_cell::sync::OnceCell;
use parking_lot::Mutex;

use crate::cache::LruCacheSingleton;
use crate::download::DownloadManager;
use crate::ext::file_ext::FileExt;
use crate::ext::log_ext::set_log_enabled;
use crate::global::CacheKeyConfig;
use crate::parser::url_parser_m3u8::clear_hls_registry;

use super::app_context::AppContext;
use super::local_proxy_server::LocalProxyServer;
use super::platform_kind::PlatformKind;

static STATE: OnceCell<Arc<VideoProxyState>> = OnceCell::new();

pub struct VideoProxyState {
    pub ctx: Arc<AppContext>,
    download_manager: Mutex<Option<Arc<DownloadManager>>>,
    local_proxy: Mutex<Option<LocalProxyServer>>,
    max_concurrent: Mutex<usize>,
    initialized: Mutex<bool>,
    disposed: Mutex<bool>,
}

impl VideoProxyState {
    pub fn get() -> Option<Arc<VideoProxyState>> {
        STATE.get().cloned()
    }

    pub fn download_manager(&self) -> Arc<DownloadManager> {
        self.download_manager
            .lock()
            .as_ref()
            .expect("XueHUAEVideoCache not initialized")
            .clone()
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
        let ctx = Arc::new(AppContext::new(platform, cache_key_config));
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
        LruCacheSingleton::reconfigure(memory_size, storage_size);
        set_log_enabled(log_print);
        if !cache_dir.is_empty() {
            FileExt::set_cache_root_path(cache_dir);
        }
        let dm = Arc::new(DownloadManager::new(max_concurrent_downloads, ctx.clone()));
        let mut proxy = LocalProxyServer::new(ctx.clone());
        proxy.start().await.map_err(|e| e.to_string())?;
        let state = Arc::new(VideoProxyState {
            ctx,
            download_manager: Mutex::new(Some(dm)),
            local_proxy: Mutex::new(Some(proxy)),
            max_concurrent: Mutex::new(max_concurrent_downloads),
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
        if let Some(dm) = self.download_manager.lock().take() {
            dm.dispose();
        }
        let max = *self.max_concurrent.lock();
        let new_dm = Arc::new(DownloadManager::new(max, self.ctx.clone()));
        *self.download_manager.lock() = Some(new_dm);
        let mut proxy_opt = self.local_proxy.lock().take();
        if let Some(ref mut proxy) = proxy_opt {
            proxy.restart().await.map_err(|e| e.to_string())?;
        }
        *self.local_proxy.lock() = proxy_opt;
        Ok(())
    }

    pub async fn is_running(&self) -> bool {
        let (ip, port) = {
            let config = self.ctx.config.read();
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
        if let Some(dm) = self.download_manager.lock().take() {
            dm.dispose();
        }
        if let Some(mut proxy) = self.local_proxy.lock().take() {
            proxy.shutdown_listener();
        }
        clear_hls_registry();
    }

    pub fn is_disposed(&self) -> bool {
        *self.disposed.lock()
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

pub fn require_state() -> Result<Arc<VideoProxyState>, String> {
    let state = VideoProxyState::get()
        .ok_or_else(|| "XueHUAEVideoCache.initialize() must be called first".to_string())?;
    if *state.disposed.lock() {
        return Err("XueHUAEVideoCache has been disposed".to_string());
    }
    Ok(state)
}
