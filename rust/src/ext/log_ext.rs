use std::sync::RwLock;

use once_cell::sync::Lazy;

static LOG_ENABLED: Lazy<RwLock<bool>> = Lazy::new(|| RwLock::new(false));

pub fn set_log_enabled(enabled: bool) {
    if let Ok(mut g) = LOG_ENABLED.write() {
        *g = enabled;
    }
}

fn enabled() -> bool {
    LOG_ENABLED.read().map(|g| *g).unwrap_or(false)
}

pub fn log_v(msg: &str) {
    if enabled() {
        log::debug!("[V] {msg}");
    }
}

pub fn log_d(msg: &str) {
    if enabled() {
        log::debug!("[D] {msg}");
    }
}

pub fn log_w(msg: &str) {
    if enabled() {
        log::warn!("[W] {msg}");
    }
}

pub fn log_e(msg: &str) {
    if enabled() {
        log::error!("[E] {msg}");
    }
}
