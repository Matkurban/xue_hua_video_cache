use std::path::PathBuf;
use std::sync::Arc;

use bytes::Bytes;
use once_cell::sync::OnceCell;
use tokio::sync::OnceCell as AsyncOnceCell;

use crate::ext::file_ext::FileExt;
use crate::ext::int_ext::to_memory_size;
use crate::global::Config;

use super::lru_cache::LruCache;
use super::lru_cache_memory::LruCacheMemory;
use super::lru_cache_storage::LruCacheStorage;

static SINGLETON: OnceCell<Arc<LruCacheSingleton>> = OnceCell::new();

pub struct LruCacheSingleton {
    memory: LruCacheMemory,
    storage: LruCacheStorage,
    storage_ready: AsyncOnceCell<()>,
}

impl LruCacheSingleton {
    pub fn instance() -> Arc<LruCacheSingleton> {
        SINGLETON
            .get_or_init(|| {
                let config = Config::default();
                Arc::new(LruCacheSingleton {
                    memory: LruCacheMemory::new(config.memory_cache_size),
                    storage: LruCacheStorage::new(config.storage_cache_size),
                    storage_ready: AsyncOnceCell::new(),
                })
            })
            .clone()
    }

    pub fn reconfigure(memory_size: i64, storage_size: i64) {
        let s = Self::instance();
        tokio::spawn(async move {
            s.memory.resize(memory_size).await;
            s.storage.resize(storage_size).await;
        });
    }

    pub async fn memory_get(&self, key: &str) -> Option<Bytes> {
        self.memory.get(&key.to_string()).await
    }

    pub async fn memory_put(&self, key: &str, value: Bytes) {
        self.memory.put(key.to_string(), value).await;
    }

    pub async fn memory_remove(&self, key: &str) {
        self.memory.remove(&key.to_string()).await;
    }

    pub async fn memory_clear(&self) {
        self.memory.clear().await;
    }

    pub async fn memory_format_size(&self) -> String {
        to_memory_size(self.memory.size())
    }

    pub async fn storage_get(&self, key: &str) -> Option<Bytes> {
        self.storage_init().await;
        if let Some(path) = self.storage.get(&key.to_string()).await {
            return tokio::fs::read(&path).await.ok().map(Bytes::from);
        }
        None
    }

    pub async fn storage_put(&self, key: &str, path: PathBuf) {
        self.storage_init().await;
        self.storage.put(key.to_string(), path).await;
    }

    pub async fn storage_remove(&self, key: &str) {
        self.storage_init().await;
        self.storage.remove(&key.to_string()).await;
    }

    pub async fn storage_clear(&self) {
        self.storage_init().await;
        self.storage.clear().await;
        if let Ok(root) = FileExt::create_cache_path(None).await {
            let _ = tokio::fs::remove_dir_all(root).await;
        }
    }

    pub async fn storage_format_size(&self) -> String {
        self.storage_init().await;
        to_memory_size(self.storage.size())
    }

    pub async fn storage_size_in_bytes(&self) -> i64 {
        self.storage_init().await;
        self.storage.size()
    }

    async fn storage_init(&self) {
        self.storage_ready
            .get_or_init(|| async {
                let Ok(root) = FileExt::create_cache_path(None).await else {
                    return;
                };
                let mut stack = vec![PathBuf::from(root)];
                while let Some(dir) = stack.pop() {
                    let mut entries = match tokio::fs::read_dir(&dir).await {
                        Ok(e) => e,
                        Err(_) => continue,
                    };
                    while let Ok(Some(entry)) = entries.next_entry().await {
                        let path = entry.path();
                        if path.extension().and_then(|e| e.to_str()) == Some("tmp") {
                            let _ = tokio::fs::remove_file(&path).await;
                            continue;
                        }
                        let meta = match entry.metadata().await {
                            Ok(m) => m,
                            Err(_) => continue,
                        };
                        if meta.is_dir() {
                            stack.push(path);
                        } else if meta.is_file() {
                            let key = path
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("")
                                .to_string();
                            let _ = self.storage.restore(key, path, meta.len() as i64).await;
                        }
                    }
                }
            })
            .await;
    }
}

#[cfg(test)]
mod singleton_tests {
    use super::*;
    use bytes::Bytes;

    use crate::global::Config;

    #[tokio::test]
    async fn memory_put_get_remove() {
        let singleton = LruCacheSingleton::instance();
        singleton.memory_clear().await;
        let value = Bytes::from_static(&[1, 2, 3]);
        singleton.memory_put("test_key", value.clone()).await;
        assert_eq!(singleton.memory_get("test_key").await, Some(value));
        singleton.memory_remove("test_key").await;
        assert!(singleton.memory_get("test_key").await.is_none());
    }

    #[tokio::test]
    async fn reconfigure_applies_new_memory_limit() {
        LruCacheSingleton::reconfigure(50, 1_000_000_000);
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let singleton = LruCacheSingleton::instance();
        singleton.memory_clear().await;
        // Re-apply after clear: parallel init tests may call reconfigure on the global singleton.
        LruCacheSingleton::reconfigure(50, 1_000_000_000);
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        singleton
            .memory_put("a", Bytes::from_static(&[0; 30]))
            .await;
        singleton
            .memory_put("b", Bytes::from_static(&[0; 30]))
            .await;
        assert!(singleton.memory_get("a").await.is_none());
        assert_eq!(
            singleton.memory_get("b").await,
            Some(Bytes::from_static(&[0; 30]))
        );
        LruCacheSingleton::reconfigure(
            Config::default().memory_cache_size,
            Config::default().storage_cache_size,
        );
    }
}
