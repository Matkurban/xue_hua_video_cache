use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use once_cell::sync::Lazy;
use tokio::time;

static DEBOUNCE_TIMERS: Lazy<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub struct FunctionProxy;

impl FunctionProxy {
    pub fn debounce<F>(target: F, key: Option<&str>, timeout_ms: u64)
    where
        F: Fn() + Send + Sync + 'static,
    {
        let key = key
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("{:p}", Arc::as_ptr(&Arc::new(()))));
        let target = Arc::new(target);
        if let Ok(mut map) = DEBOUNCE_TIMERS.lock() {
            if let Some(handle) = map.remove(&key) {
                handle.abort();
            }
            let target_clone = target.clone();
            let key_for_task = key.clone();
            let handle = tokio::spawn(async move {
                time::sleep(Duration::from_millis(timeout_ms)).await;
                target_clone();
                if let Ok(mut map) = DEBOUNCE_TIMERS.lock() {
                    map.remove(&key_for_task);
                }
            });
            map.insert(key, handle);
        }
    }
}
