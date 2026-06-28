use async_trait::async_trait;
use bytes::Bytes;

use super::lru_cache::LruCache;
use super::lru_cache_impl::LruCacheImpl;

pub struct LruCacheMemory {
    inner: LruCacheImpl<String, Bytes>,
}

impl LruCacheMemory {
    pub fn new(max_size: i64) -> Self {
        Self {
            inner: LruCacheImpl::new(max_size),
        }
    }

    pub fn size(&self) -> i64 {
        *self.inner.size.lock()
    }

    fn trim_to_size_locked(&self, max_size: i64) {
        loop {
            if *self.inner.size.lock() < 0 {
                panic!("LruCacheMemory.sizeOf() is reporting inconsistent results!");
            }
            if *self.inner.size.lock() <= max_size {
                break;
            }
            let eldest = {
                let map = self.inner.map.lock();
                map.iter().next().map(|(k, v)| (k.clone(), v.clone()))
            };
            let Some((key, value)) = eldest else {
                break;
            };
            {
                let mut map = self.inner.map.lock();
                map.shift_remove(&key);
            }
            *self.inner.size.lock() -= value.len() as i64;
        }
    }
}

#[async_trait]
impl LruCache<String, Bytes> for LruCacheMemory {
    async fn get(&self, key: &String) -> Option<Bytes> {
        assert!(!key.is_empty(), "key must not be empty");
        let entity = {
            let map = self.inner.map.lock();
            map.get(key).cloned()
        };
        if let Some(value) = entity {
            let mut map = self.inner.map.lock();
            if let Some(v) = map.shift_remove(key) {
                map.insert(key.clone(), v);
            }
            return Some(value);
        }
        None
    }

    async fn put(&self, key: String, value: Bytes) -> Option<Bytes> {
        assert!(!key.is_empty(), "key must not be empty");
        *self.inner.size.lock() += value.len() as i64;
        let previous = {
            let mut map = self.inner.map.lock();
            let prev = map.swap_remove(&key);
            map.insert(key, value);
            prev
        };
        if let Some(ref prev) = previous {
            *self.inner.size.lock() -= prev.len() as i64;
        }
        let max = *self.inner.max_size.lock();
        self.trim_to_size_locked(max);
        previous
    }

    async fn remove(&self, key: &String) -> Option<Bytes> {
        assert!(!key.is_empty(), "key must not be empty");
        let previous = {
            let mut map = self.inner.map.lock();
            map.shift_remove(key)
        };
        if let Some(ref prev) = previous {
            *self.inner.size.lock() -= prev.len() as i64;
        }
        previous
    }

    async fn clear(&self) {
        self.trim_to_size(-1).await;
    }

    async fn trim_to_size(&self, max_size: i64) {
        if max_size < 0 {
            self.inner.map.lock().clear();
            *self.inner.size.lock() = 0;
            return;
        }
        self.trim_to_size_locked(max_size);
    }

    async fn resize(&self, max_size: i64) {
        assert!(max_size > 0);
        *self.inner.max_size.lock() = max_size;
        self.trim_to_size(max_size).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn put_and_get() {
        let cache = LruCacheMemory::new(10);
        let value = Bytes::from_static(&[1, 2, 3]);
        cache.put("a".to_string(), value.clone()).await;
        let result = cache.get(&"a".to_string()).await;
        assert_eq!(result, Some(value));
    }

    #[tokio::test]
    async fn get_promotes_entry_for_lru() {
        let cache = LruCacheMemory::new(10);
        cache
            .put("a".to_string(), Bytes::from_static(&[1, 2, 3, 4, 5]))
            .await;
        cache
            .put("b".to_string(), Bytes::from_static(&[6, 7, 8, 9, 10]))
            .await;
        cache.get(&"a".to_string()).await;
        cache
            .put("c".to_string(), Bytes::from_static(&[11, 12, 13]))
            .await;
        assert!(cache.get(&"b".to_string()).await.is_none());
        assert_eq!(
            cache.get(&"a".to_string()).await,
            Some(Bytes::from_static(&[1, 2, 3, 4, 5]))
        );
    }

    #[tokio::test]
    async fn eviction_when_over_max_size() {
        let cache = LruCacheMemory::new(10);
        cache
            .put("a".to_string(), Bytes::from_static(&[1, 2, 3, 4, 5]))
            .await;
        cache
            .put("b".to_string(), Bytes::from_static(&[6, 7, 8, 9, 10]))
            .await;
        cache
            .put("c".to_string(), Bytes::from_static(&[11, 12, 13]))
            .await;
        assert!(cache.get(&"a".to_string()).await.is_none());
        assert_eq!(
            cache.get(&"b".to_string()).await,
            Some(Bytes::from_static(&[6, 7, 8, 9, 10]))
        );
        assert_eq!(
            cache.get(&"c".to_string()).await,
            Some(Bytes::from_static(&[11, 12, 13]))
        );
    }

    #[tokio::test]
    async fn remove_and_clear() {
        let cache = LruCacheMemory::new(100);
        cache
            .put("a".to_string(), Bytes::from_static(&[1, 2, 3]))
            .await;
        assert!(cache.remove(&"a".to_string()).await.is_some());
        assert!(cache.get(&"a".to_string()).await.is_none());
        cache
            .put("b".to_string(), Bytes::from_static(&[4, 5, 6]))
            .await;
        cache.clear().await;
        assert!(cache.get(&"b".to_string()).await.is_none());
    }
}
