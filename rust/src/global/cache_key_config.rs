/// Cache key customization passed from Dart via FRB.
#[flutter_rust_bridge::frb]
#[derive(Debug, Clone, Default)]
pub struct CacheKeyConfig {
    /// Query parameter names to strip before computing the cache key (e.g. `token`).
    pub ignore_query_keys: Vec<String>,
}
