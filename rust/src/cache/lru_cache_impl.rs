use async_trait::async_trait;
use indexmap::IndexMap;
use parking_lot::Mutex;

use super::lru_cache::LruCache;

pub struct LruCacheImpl<K, V> {
    pub map: Mutex<IndexMap<K, V>>,
    pub max_size: Mutex<i64>,
    pub size: Mutex<i64>,
}

impl<K, V> LruCacheImpl<K, V>
where
    K: std::hash::Hash + Eq + Clone + Send + Sync,
    V: Send + Sync,
{
    pub fn new(max_size: i64) -> Self {
        assert!(max_size > 0, "maxSize must be greater than 0");
        Self {
            map: Mutex::new(IndexMap::new()),
            max_size: Mutex::new(max_size),
            size: Mutex::new(0),
        }
    }
}

#[async_trait]
impl<K, V> LruCache<K, V> for LruCacheImpl<K, V>
where
    K: std::hash::Hash + Eq + Clone + Send + Sync,
    V: Send + Sync,
{
    async fn get(&self, _key: &K) -> Option<V> {
        unimplemented!("subclass implements get")
    }

    async fn put(&self, _key: K, _value: V) -> Option<V> {
        unimplemented!("subclass implements put")
    }

    async fn remove(&self, _key: &K) -> Option<V> {
        unimplemented!("subclass implements remove")
    }

    async fn clear(&self) {
        self.trim_to_size(-1).await;
    }

    async fn trim_to_size(&self, _max_size: i64) {
        unimplemented!("subclass implements trim_to_size")
    }

    async fn resize(&self, max_size: i64) {
        assert!(max_size > 0, "maxSize must be greater than 0");
        *self.max_size.lock() = max_size;
        self.trim_to_size(max_size).await;
    }
}
