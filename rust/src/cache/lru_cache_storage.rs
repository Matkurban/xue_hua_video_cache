use std::path::{Path, PathBuf};

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::ext::log_ext::log_e;

use super::lru_cache::LruCache;
use super::lru_cache_impl::LruCacheImpl;

pub struct LruCacheStorage {
    inner: LruCacheImpl<String, PathBuf>,
    entry_sizes: Mutex<std::collections::HashMap<String, i64>>,
}

impl LruCacheStorage {
    pub fn new(max_size: i64) -> Self {
        Self {
            inner: LruCacheImpl::new(max_size),
            entry_sizes: Mutex::new(std::collections::HashMap::new()),
        }
    }

    pub fn size(&self) -> i64 {
        *self.inner.size.lock()
    }

    pub async fn restore(&self, key: String, path: PathBuf, value_size: i64) {
        assert!(!key.is_empty());
        if self.inner.map.lock().contains_key(&key) {
            let _ = self.remove_entry_locked(&key, false).await;
        }
        self.inner.map.lock().insert(key.clone(), path);
        self.entry_sizes.lock().insert(key, value_size);
        *self.inner.size.lock() += value_size;
    }

    async fn safe_size(path: &Path) -> i64 {
        match tokio::fs::metadata(path).await {
            Ok(m) => m.len() as i64,
            Err(_) => 0,
        }
    }

    async fn remove_entry_locked(&self, key: &str, delete_file: bool) -> Option<PathBuf> {
        let previous = {
            let mut map = self.inner.map.lock();
            map.shift_remove(key)
        };
        let Some(path) = previous else {
            return None;
        };
        let removed_size = self.entry_sizes.lock().remove(key);
        *self.inner.size.lock() -= removed_size.unwrap_or_else(|| {
            // sync fallback
            0
        });
        if *self.inner.size.lock() < 0 {
            *self.inner.size.lock() = 0;
        }
        if delete_file {
            if tokio::fs::metadata(&path).await.is_ok() {
                if let Err(e) = tokio::fs::remove_file(&path).await {
                    log_e(&format!("[LruCacheStorage] Delete cache file failed: {e}"));
                }
            }
        }
        Some(path)
    }

    async fn repair_size_locked(&self) {
        let entries: Vec<(String, PathBuf)> = {
            let map = self.inner.map.lock();
            map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        };
        let mut recalculated = 0i64;
        let mut missing = Vec::new();
        for (key, path) in entries {
            if tokio::fs::metadata(&path).await.is_ok() {
                let sz = Self::safe_size(&path).await;
                self.entry_sizes.lock().insert(key.clone(), sz);
                recalculated += sz;
            } else {
                missing.push(key);
            }
        }
        for key in missing {
            self.inner.map.lock().shift_remove(&key);
            self.entry_sizes.lock().remove(&key);
        }
        *self.inner.size.lock() = recalculated;
    }

    async fn trim_to_size_locked(&self, max_size: i64) {
        if max_size < 0 {
            let keys: Vec<String> = self.inner.map.lock().keys().cloned().collect();
            for key in keys {
                let _ = self.remove_entry_locked(&key, true).await;
            }
            return;
        }
        self.repair_size_locked().await;
        while *self.inner.size.lock() > max_size {
            let eldest = {
                let map = self.inner.map.lock();
                map.iter().next().map(|(k, _)| k.clone())
            };
            let Some(key) = eldest else {
                *self.inner.size.lock() = 0;
                return;
            };
            let _ = self.remove_entry_locked(&key, true).await;
        }
    }
}

#[async_trait]
impl LruCache<String, PathBuf> for LruCacheStorage {
    async fn get(&self, key: &String) -> Option<PathBuf> {
        assert!(!key.is_empty());
        let entity = {
            let map = self.inner.map.lock();
            map.get(key).cloned()
        };
        if let Some(path) = entity {
            if tokio::fs::metadata(&path).await.is_ok() {
                {
                    let mut map = self.inner.map.lock();
                    if let Some(p) = map.shift_remove(key) {
                        map.insert(key.clone(), p);
                    }
                }
                return Some(path);
            }
            let _ = self.remove_entry_locked(key, false).await;
        }
        None
    }

    async fn put(&self, key: String, value: PathBuf) -> Option<PathBuf> {
        assert!(!key.is_empty());
        let value_size = Self::safe_size(&value).await;
        if self.inner.map.lock().contains_key(&key) {
            let _ = self.remove_entry_locked(&key, false).await;
        }
        *self.inner.size.lock() += value_size;
        self.entry_sizes.lock().insert(key.clone(), value_size);
        self.inner.map.lock().insert(key.clone(), value);
        let max_size = *self.inner.max_size.lock();
        self.trim_to_size_locked(max_size).await;
        None
    }

    async fn remove(&self, key: &String) -> Option<PathBuf> {
        assert!(!key.is_empty());
        self.remove_entry_locked(key, true).await
    }

    async fn clear(&self) {
        self.trim_to_size(-1).await;
    }

    async fn trim_to_size(&self, max_size: i64) {
        self.trim_to_size_locked(max_size).await;
    }

    async fn resize(&self, max_size: i64) {
        assert!(max_size > 0);
        *self.inner.max_size.lock() = max_size;
        self.trim_to_size(max_size).await;
    }
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::LruCacheStorage;
    use crate::cache::lru_cache::LruCache;

    #[tokio::test]
    async fn put_get_remove() {
        let dir = env::temp_dir().join(format!("lru_test_{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let cache = LruCacheStorage::new(1000);
        let file = dir.join("test.dat");
        tokio::fs::write(&file, b"hello").await.unwrap();
        cache.put("k".to_string(), file.clone()).await;
        assert_eq!(cache.get(&"k".to_string()).await, Some(file.clone()));
        assert!(cache.remove(&"k".to_string()).await.is_some());
        assert!(cache.get(&"k".to_string()).await.is_none());
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
