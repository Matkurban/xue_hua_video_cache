use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use bytes::Bytes;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::ext::string_ext::{generate_md5, to_safe_uri};
use crate::global::Config;
use crate::matchers::UrlMatcher;

use super::download_status::DownloadStatus;

static AUTO_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub struct DownloadTask {
    pub id: String,
    pub uri: Url,
    pub priority: i32,
    pub progress: f64,
    pub cached_bytes: i64,
    pub downloaded_bytes: i64,
    pub total_bytes: i64,
    pub status: DownloadStatus,
    pub start_range: i64,
    pub end_range: Option<i64>,
    pub headers: Option<HashMap<String, String>>,
    pub hls_key: Option<String>,
    pub retry_times: i32,
    pub cancel_token: Option<CancellationToken>,
    pub last_progress_at: Option<Instant>,
    pub data: Bytes,
    pub cache_dir: String,
    pub file_name: String,
}

impl DownloadTask {
    pub fn new(uri: Url, file_name: Option<String>) -> Self {
        let id = AUTO_ID.fetch_add(1, Ordering::SeqCst).to_string();
        let file_name = file_name.unwrap_or_else(|| uri.to_string());
        Self {
            id,
            uri,
            priority: 1,
            progress: 0.0,
            cached_bytes: 0,
            downloaded_bytes: 0,
            total_bytes: 0,
            status: DownloadStatus::Idle,
            start_range: 0,
            end_range: None,
            headers: None,
            hls_key: None,
            retry_times: 0,
            cancel_token: None,
            last_progress_at: None,
            data: Bytes::new(),
            cache_dir: String::new(),
            file_name,
        }
    }

    pub fn reset_id() {
        AUTO_ID.store(1, Ordering::SeqCst);
    }

    pub fn url(&self) -> String {
        self.uri.to_string()
    }

    pub fn match_url(&self, config: &Config, matcher: &dyn UrlMatcher) -> String {
        let cache_key = config.custom_cache_id.to_lowercase();
        let headers = self.headers.clone().unwrap_or_default();
        let headers: HashMap<String, String> = headers
            .into_iter()
            .map(|(k, v)| (k.to_lowercase(), v))
            .collect();
        let mut safe_uri = to_safe_uri(&self.file_name);
        if let Some(host) = headers.get(&cache_key) {
            safe_uri.set_host(Some(host)).ok();
        }
        let mut query: HashMap<String, String> = safe_uri
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        if self.start_range > 0 {
            query
                .entry("startRange".to_string())
                .or_insert_with(|| self.start_range.to_string());
        }
        if let Some(end) = self.end_range {
            query
                .entry("startRange".to_string())
                .or_insert_with(|| "0".to_string());
            query.insert("endRange".to_string(), end.to_string());
        }
        let q: String = query
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");
        safe_uri.set_query(if q.is_empty() { None } else { Some(&q) });
        let cache_uri = matcher.match_cache_key(&safe_uri);
        generate_md5(&cache_uri.to_string())
    }

    pub fn save_file_name(&self, config: &Config, matcher: &dyn UrlMatcher) -> String {
        let match_url = self.match_url(config, matcher);
        let extension = self.file_name.rsplit('.').next().unwrap_or("bin");
        if let Ok(uri) = Url::parse(&self.file_name) {
            if let Some(last) = uri.path_segments().and_then(|mut s| s.next_back()) {
                if let Some(ext) = last.rsplit('.').next() {
                    return format!("{match_url}.{ext}");
                }
            }
        }
        format!("{match_url}.{extension}")
    }

    pub fn save_path(&self, config: &Config, matcher: &dyn UrlMatcher) -> String {
        format!(
            "{}/{}",
            self.cache_dir,
            self.save_file_name(config, matcher)
        )
    }

    pub fn reset(&mut self) {
        self.downloaded_bytes = 0;
        self.total_bytes = 0;
        self.progress = 0.0;
        self.start_range = 0;
        self.end_range = None;
        self.data = Bytes::new();
    }
}
