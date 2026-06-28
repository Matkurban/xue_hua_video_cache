/// Application-wide configuration (mirrors `global/config.dart`).
#[derive(Debug, Clone)]
pub struct Config {
    pub ip: String,
    pub port: u16,
    pub mb_size: i64,
    pub memory_cache_size: i64,
    pub storage_cache_size: i64,
    pub segment_size: i64,
    pub custom_cache_id: String,
}

impl Default for Config {
    fn default() -> Self {
        let mb_size = 1_000_000;
        Self {
            ip: "127.0.0.1".to_string(),
            port: 20250,
            mb_size,
            memory_cache_size: 100 * mb_size,
            storage_cache_size: 1_000 * mb_size,
            segment_size: 2_000_000,
            custom_cache_id: "Custom-Cache-ID".to_string(),
        }
    }
}

impl Config {
    pub fn server_url(&self) -> String {
        format!("{}:{}", self.ip, self.port)
    }
}
