use crate::ext::string_ext::{generate_md5, to_local_url, to_origin_url, to_safe_url};
use crate::global::Config;
use crate::proxy::video_proxy::VideoProxyState;

#[flutter_rust_bridge::frb(sync)]
pub fn to_local_url_str(url: String) -> String {
    if let Some(state) = VideoProxyState::get() {
        if !state.is_disposed() {
            let config = state.runtime.ctx.config.read().clone();
            return to_local_url(&url, &config);
        }
    }
    to_local_url(&url, &Config::default())
}

#[flutter_rust_bridge::frb(sync)]
pub fn to_origin_url_str(url: String) -> String {
    to_origin_url(&url)
}

#[flutter_rust_bridge::frb(sync)]
pub fn to_safe_url_str(url: String) -> String {
    to_safe_url(&url)
}

#[flutter_rust_bridge::frb(sync)]
pub fn generate_md5_str(input: String) -> String {
    generate_md5(&input)
}
