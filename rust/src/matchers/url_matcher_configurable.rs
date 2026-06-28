use std::collections::HashSet;

use url::Url;

use crate::global::CacheKeyConfig;

use super::url_matcher::UrlMatcher;
use super::url_matcher_default::UrlMatcherDefault;

pub struct UrlMatcherConfigurable {
    ignore_query_keys: HashSet<String>,
}

impl UrlMatcherConfigurable {
    pub fn new(config: &CacheKeyConfig) -> Self {
        Self {
            ignore_query_keys: config
                .ignore_query_keys
                .iter()
                .map(|k| k.to_lowercase())
                .collect(),
        }
    }
}

impl UrlMatcher for UrlMatcherConfigurable {
    fn match_m3u8(&self, uri: &Url) -> bool {
        UrlMatcherDefault.match_m3u8(uri)
    }

    fn match_m3u8_key(&self, uri: &Url) -> bool {
        UrlMatcherDefault.match_m3u8_key(uri)
    }

    fn match_m3u8_segment(&self, uri: &Url) -> bool {
        UrlMatcherDefault.match_m3u8_segment(uri)
    }

    fn match_mp4(&self, uri: &Url) -> bool {
        UrlMatcherDefault.match_mp4(uri)
    }

    fn match_cache_key(&self, uri: &Url) -> Url {
        if self.ignore_query_keys.is_empty() {
            return UrlMatcherDefault.match_cache_key(uri);
        }
        let params: Vec<(String, String)> = uri
            .query_pairs()
            .filter(|(k, _)| !self.ignore_query_keys.contains(&k.to_lowercase()))
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        let mut new_uri = uri.clone();
        if params.is_empty() {
            new_uri.set_query(None);
        } else {
            let q: String = params
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("&");
            new_uri.set_query(Some(&q));
        }
        new_uri
    }
}

#[cfg(test)]
mod matcher_tests {
    use url::Url;

    use crate::global::CacheKeyConfig;
    use crate::matchers::{UrlMatcher, UrlMatcherConfigurable, UrlMatcherDefault};

    #[test]
    fn configurable_strips_ignore_query_keys() {
        let matcher = UrlMatcherConfigurable::new(&CacheKeyConfig {
            ignore_query_keys: vec!["token".to_string()],
        });
        let uri = Url::parse("https://example.com/v.mp4?token=abc&id=1").unwrap();
        let key = matcher.match_cache_key(&uri);
        assert!(!key.query().unwrap_or("").contains("token"));
        assert!(key.query().unwrap_or("").contains("id=1"));
    }

    #[test]
    fn default_keeps_only_range_query_keys() {
        let uri = Url::parse("https://example.com/v.mp4?token=abc&startRange=0").unwrap();
        let key = UrlMatcherDefault.match_cache_key(&uri);
        assert!(!key.query().unwrap_or("").contains("token"));
        assert!(key.query().unwrap_or("").contains("startRange=0"));
    }
}
