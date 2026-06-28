use async_trait::async_trait;

#[async_trait]
pub trait LruCache<K, V>: Send + Sync
where
    K: Send + Sync,
    V: Send + Sync,
{
    async fn get(&self, key: &K) -> Option<V>;
    async fn put(&self, key: K, value: V) -> Option<V>;
    async fn remove(&self, key: &K) -> Option<V>;
    async fn clear(&self);
    async fn trim_to_size(&self, max_size: i64);
    async fn resize(&self, max_size: i64);
}
