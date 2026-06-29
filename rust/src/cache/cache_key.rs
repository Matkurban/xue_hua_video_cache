use std::collections::HashMap;
use std::sync::Arc;

use url::Url;

use crate::download::DownloadTask;
use crate::ext::string_ext::{generate_md5, to_safe_uri};
use crate::global::Config;
use crate::matchers::UrlMatcher;
use crate::proxy::ProxyRuntime;

/// LRU memory/storage index for a cached byte range (formerly `match_url`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey {
    pub entry: String,
    pub directory: String,
}

/// Config + matcher bundle passed to [`CacheKey::for_task`].
pub struct CacheKeyContext<'a> {
    config: Config,
    matcher: &'a dyn UrlMatcher,
}

impl<'a> CacheKeyContext<'a> {
    pub fn new(config: Config, matcher: &'a dyn UrlMatcher) -> Self {
        Self { config, matcher }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn matcher(&self) -> &dyn UrlMatcher {
        self.matcher
    }

    pub fn key_for(&self, task: &DownloadTask) -> CacheKey {
        CacheKey::for_task(task, self)
    }

    pub fn entry_for(&self, task: &DownloadTask) -> String {
        self.key_for(task).entry
    }

    pub fn entry_matches(&self, task: &DownloadTask, entry: &str) -> bool {
        self.entry_for(task) == entry
    }
}

impl CacheKeyContext<'_> {
    pub fn from_runtime<'r>(runtime: &'r Arc<ProxyRuntime>) -> CacheKeyContext<'r> {
        let config = runtime.ctx.config.read().clone();
        CacheKeyContext {
            config,
            matcher: runtime.ctx.url_matcher.as_ref(),
        }
    }
}

impl CacheKey {
    pub fn for_task(task: &DownloadTask, ctx: &CacheKeyContext<'_>) -> Self {
        Self {
            entry: compute_entry(task, ctx),
            directory: Self::directory_for(task),
        }
    }

    /// On-disk directory grouping (formerly `task_cache_key`).
    ///
    /// Does not depend on `Config` or `UrlMatcher`. See ADR-0001 for HLS invariants.
    pub fn directory_for(task: &DownloadTask) -> String {
        if let Some(ref key) = task.hls_key {
            key.clone()
        } else {
            generate_md5(&task.uri.to_string())
        }
    }

    pub fn file_name(&self, task: &DownloadTask) -> String {
        let extension = task.file_name.rsplit('.').next().unwrap_or("bin");
        if let Ok(uri) = Url::parse(&task.file_name) {
            if let Some(last) = uri.path_segments().and_then(|mut s| s.next_back()) {
                if let Some(ext) = last.rsplit('.').next() {
                    return format!("{}.{}", self.entry, ext);
                }
            }
        }
        format!("{}.{}", self.entry, extension)
    }

    pub fn save_path(&self, task: &DownloadTask) -> String {
        format!("{}/{}", task.cache_dir, self.file_name(task))
    }
}

fn compute_entry(task: &DownloadTask, ctx: &CacheKeyContext<'_>) -> String {
    let config = ctx.config();
    let matcher = ctx.matcher();
    let cache_key = config.custom_cache_id.to_lowercase();
    let headers = task.headers.clone().unwrap_or_default();
    let headers: HashMap<String, String> = headers
        .into_iter()
        .map(|(k, v)| (k.to_lowercase(), v))
        .collect();
    let mut safe_uri = to_safe_uri(&task.file_name);
    if let Some(host) = headers.get(&cache_key) {
        safe_uri.set_host(Some(host)).ok();
    }
    let mut query: HashMap<String, String> = safe_uri
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if task.start_range > 0 {
        query
            .entry("startRange".to_string())
            .or_insert_with(|| task.start_range.to_string());
    }
    if let Some(end) = task.end_range {
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

#[cfg(test)]
mod golden_tests {
    use std::collections::HashMap;

    use url::Url;

    use crate::download::DownloadTask;
    use crate::ext::string_ext::generate_md5;
    use crate::global::{CacheKeyConfig, Config};
    use crate::matchers::UrlMatcherConfigurable;

    use super::*;

    fn ctx() -> (Config, UrlMatcherConfigurable) {
        (
            Config::default(),
            UrlMatcherConfigurable::new(&CacheKeyConfig::default()),
        )
    }

    fn ctx_with_config(config: Config) -> (Config, UrlMatcherConfigurable) {
        let matcher = UrlMatcherConfigurable::new(&CacheKeyConfig::default());
        (config, matcher)
    }

    #[test]
    fn mp4_full_file_entry_equals_directory() {
        let (config, matcher) = ctx();
        let ctx = CacheKeyContext::new(config, &matcher);
        let uri = Url::parse("https://cdn.example.com/video.mp4").unwrap();
        let task = DownloadTask::new(uri, None);
        let key = CacheKey::for_task(&task, &ctx);
        let expected = generate_md5("https://cdn.example.com/video.mp4");
        assert_eq!(key.entry, expected);
        assert_eq!(key.directory, expected);
        assert_eq!(key.entry, key.directory);
    }

    #[test]
    fn mp4_range_segment_entry_differs_from_directory() {
        let (config, matcher) = ctx();
        let ctx = CacheKeyContext::new(config, &matcher);
        let uri = Url::parse("https://cdn.example.com/video.mp4").unwrap();
        let mut task = DownloadTask::new(uri, None);
        task.start_range = 1_048_576;
        task.end_range = Some(2_097_151);
        let key = CacheKey::for_task(&task, &ctx);
        let dir = generate_md5("https://cdn.example.com/video.mp4");
        assert_eq!(key.directory, dir);
        assert_ne!(key.entry, dir);
        assert_ne!(
            key.entry,
            CacheKey::for_task(
                &DownloadTask::new(
                    Url::parse("https://cdn.example.com/video.mp4").unwrap(),
                    None
                ),
                &ctx
            )
            .entry
        );
    }

    #[test]
    fn hls_precache_uses_playlist_directory_key() {
        let (config, matcher) = ctx();
        let ctx = CacheKeyContext::new(config, &matcher);
        let master = "https://cdn.example.com/master.m3u8";
        let segment_url = "https://cdn.example.com/seg001.ts";
        let mut task = DownloadTask::new(to_safe_uri(segment_url), None);
        task.hls_key = Some(generate_md5(master));
        task.start_range = 0;
        task.end_range = Some(1023);
        let key = CacheKey::for_task(&task, &ctx);
        assert_eq!(key.directory, generate_md5(master));
        assert_ne!(key.entry, key.directory);
    }

    #[test]
    fn hls_proxy_segment_uses_segment_url_as_directory() {
        let (config, matcher) = ctx();
        let ctx = CacheKeyContext::new(config, &matcher);
        let segment_url = "https://cdn.example.com/seg001.ts";
        let mut task = DownloadTask::new(to_safe_uri(segment_url), None);
        task.hls_key = Some(generate_md5(segment_url));
        let key = CacheKey::for_task(&task, &ctx);
        assert_eq!(key.directory, generate_md5(segment_url));
    }

    #[test]
    fn custom_cache_id_host_override_changes_entry_only() {
        let mut config = Config::default();
        config.custom_cache_id = "Custom-Cache-ID".to_string();
        let (config, matcher) = ctx_with_config(config);
        let ctx = CacheKeyContext::new(config, &matcher);
        let uri = Url::parse("https://cdn.example.com/video.mp4").unwrap();
        let mut task = DownloadTask::new(uri, None);
        let mut headers = HashMap::new();
        headers.insert(
            "Custom-Cache-ID".to_string(),
            "cache-host.example.com".to_string(),
        );
        task.headers = Some(headers);
        let key = CacheKey::for_task(&task, &ctx);
        let baseline = CacheKey::for_task(
            &DownloadTask::new(
                Url::parse("https://cdn.example.com/video.mp4").unwrap(),
                None,
            ),
            &ctx,
        );
        assert_ne!(key.entry, baseline.entry);
        assert_eq!(key.directory, baseline.directory);
    }

    #[test]
    fn file_name_and_save_path_use_entry_stem() {
        let (config, matcher) = ctx();
        let ctx = CacheKeyContext::new(config, &matcher);
        let uri = Url::parse("https://cdn.example.com/video.mp4").unwrap();
        let mut task = DownloadTask::new(uri, None);
        task.cache_dir = "/cache/root".to_string();
        let key = CacheKey::for_task(&task, &ctx);
        assert!(key.file_name(&task).ends_with(".mp4"));
        assert!(key.file_name(&task).starts_with(&key.entry));
        assert_eq!(
            key.save_path(&task),
            format!("/cache/root/{}", key.file_name(&task))
        );
    }

    #[test]
    fn ranged_segment_file_name_uses_ranged_entry() {
        let (config, matcher) = ctx();
        let ctx = CacheKeyContext::new(config, &matcher);
        let uri = Url::parse("https://cdn.example.com/video.mp4").unwrap();
        let mut task = DownloadTask::new(uri, None);
        task.start_range = 1024;
        task.end_range = Some(2047);
        let key = CacheKey::for_task(&task, &ctx);
        let baseline = CacheKey::for_task(
            &DownloadTask::new(
                Url::parse("https://cdn.example.com/video.mp4").unwrap(),
                None,
            ),
            &ctx,
        );
        assert_ne!(key.file_name(&task), baseline.file_name(&task));
    }
}
