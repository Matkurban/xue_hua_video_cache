use url::Url;

use super::url_matcher::UrlMatcher;

pub struct UrlMatcherDefault;

impl UrlMatcher for UrlMatcherDefault {
    fn match_m3u8(&self, uri: &Url) -> bool {
        uri.path().to_lowercase().ends_with(".m3u8")
    }

    fn match_m3u8_key(&self, uri: &Url) -> bool {
        uri.path().to_lowercase().ends_with(".key")
    }

    fn match_m3u8_segment(&self, uri: &Url) -> bool {
        uri.path().to_lowercase().ends_with(".ts")
    }

    fn match_mp4(&self, uri: &Url) -> bool {
        uri.path().to_lowercase().ends_with(".mp4")
    }

    fn match_cache_key(&self, uri: &Url) -> Url {
        let mut params: Vec<(String, String)> = uri
            .query_pairs()
            .filter(|(k, _)| k == "startRange" || k == "endRange")
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        let mut new_uri = uri.clone();
        if params.is_empty() {
            new_uri.set_query(None);
        } else {
            let q: String = params
                .drain(..)
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("&");
            new_uri.set_query(Some(&q));
        }
        new_uri
    }
}
