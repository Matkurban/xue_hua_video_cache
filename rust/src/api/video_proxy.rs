use crate::ext::file_ext::FileExt;
use crate::global::CacheKeyConfig;
use crate::proxy::platform_kind::PlatformKind;
use crate::proxy::video_proxy::VideoProxyState;

#[flutter_rust_bridge::frb]
pub async fn video_proxy_init(
    ip: Option<String>,
    port: Option<u16>,
    max_memory_cache_size: i64,
    max_storage_cache_size: i64,
    cache_dir: String,
    log_print: bool,
    segment_size: i64,
    max_concurrent_downloads: u32,
    platform: PlatformKind,
    cache_key_config: CacheKeyConfig,
) -> Result<(), String> {
    VideoProxyState::init(
        ip,
        port,
        max_memory_cache_size,
        max_storage_cache_size,
        cache_dir,
        log_print,
        segment_size,
        max_concurrent_downloads as usize,
        platform,
        cache_key_config,
    )
    .await
}

#[flutter_rust_bridge::frb]
pub async fn video_proxy_restart() -> Result<(), String> {
    let state = crate::proxy::require_state()?;
    state.restart().await
}

#[flutter_rust_bridge::frb]
pub async fn video_proxy_is_running() -> Result<bool, String> {
    let state = crate::proxy::require_state()?;
    Ok(state.is_running().await)
}

#[flutter_rust_bridge::frb]
pub fn video_proxy_dispose() -> Result<(), String> {
    let state = VideoProxyState::get()
        .ok_or_else(|| "XueHUAEVideoCache.initialize() must be called first".to_string())?;
    if state.is_disposed() {
        return Ok(());
    }
    state.dispose();
    Ok(())
}

#[flutter_rust_bridge::frb]
pub fn set_cache_root_path(path: String) {
    FileExt::set_cache_root_path(path);
}
