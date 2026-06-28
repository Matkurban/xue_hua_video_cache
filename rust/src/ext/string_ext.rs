use base64::Engine;
use md5::{Digest, Md5};
use url::Url;

use crate::global::Config;

pub fn generate_md5(input: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(input.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>()
}

pub fn to_safe_url(input: &str) -> String {
    let trimmed = input.trim();
    let encoded = urlencoding::encode(trimmed).replace("%0D", "");
    urlencoding::decode(&encoded)
        .map(|s| s.into_owned())
        .unwrap_or_else(|_| trimmed.to_string())
}

pub fn to_safe_uri(input: &str) -> Url {
    Url::parse(&to_safe_url(input)).unwrap_or_else(|_| Url::parse("http://invalid").unwrap())
}

/// Parse a proxy request target: full URL, path+query from GET line, or percent-encoded URL path.
fn parse_request_uri(input: &str) -> Url {
    let trimmed = input.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return to_safe_uri(trimmed);
    }
    if trimmed.starts_with('/') {
        if let Ok(decoded) = urlencoding::decode(trimmed.trim_start_matches('/')) {
            if decoded.starts_with("http://") || decoded.starts_with("https://") {
                return to_safe_uri(&decoded);
            }
        }
        return Url::options()
            .base_url(Some(&Url::parse("http://proxy.local").unwrap()))
            .parse(trimmed)
            .unwrap_or_else(|_| Url::parse("http://invalid").unwrap());
    }
    to_safe_uri(trimmed)
}

pub fn to_local_url(input: &str, config: &Config) -> String {
    if !input.starts_with("http") {
        return input.to_string();
    }
    let uri = to_safe_uri(input);
    if uri.host_str() == Some(&config.ip) && uri.port_or_known_default() == Some(config.port) {
        return input.to_string();
    }
    let origin = uri.origin().ascii_serialization();
    let origin_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(origin.as_bytes());
    let mut local = Url::parse(&format!("http://{}:{}", config.ip, config.port)).unwrap();
    local.set_path(uri.path());
    {
        let mut pairs: Vec<(String, String)> = uri
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        if !pairs.iter().any(|(k, _)| k == "origin") {
            pairs.push(("origin".to_string(), origin_b64));
        }
        let query: String = pairs
            .iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        local.set_query(Some(&query));
    }
    local.to_string()
}

pub fn to_origin_url(input: &str) -> String {
    let uri = parse_request_uri(input);
    let mut params: Vec<(String, String)> = uri
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let origin_idx = params.iter().position(|(k, _)| k == "origin");
    let Some(idx) = origin_idx else {
        if uri.host_str() == Some("proxy.local") {
            return input.to_string();
        }
        if uri.scheme() == "http" || uri.scheme() == "https" {
            return uri.to_string();
        }
        return input.to_string();
    };
    let origin_b64 = params.remove(idx).1;
    let origin_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(origin_b64.as_bytes())
        .ok()
        .and_then(|b| String::from_utf8(b).ok());
    let Some(origin) = origin_bytes else {
        return input.to_string();
    };
    let mut origin_uri = Url::parse(&origin).unwrap_or(uri.clone());
    origin_uri.set_path(uri.path());
    if params.is_empty() {
        origin_uri.set_query(None);
    } else {
        let query: String = params
            .iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        origin_uri.set_query(Some(&query));
    }
    origin_uri.to_string()
}

#[cfg(test)]
mod tests {
    use base64::Engine;

    use super::*;

    #[test]
    fn to_origin_url_restores_from_proxy_path_with_origin_query() {
        let origin_b64 =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"https://flutter.github.io");
        let path = format!("/assets-for-api-docs/assets/videos/butterfly.mp4?origin={origin_b64}");
        let restored = to_origin_url(&path);
        assert_eq!(
            restored,
            "https://flutter.github.io/assets-for-api-docs/assets/videos/butterfly.mp4"
        );
    }

    #[test]
    fn to_origin_url_decodes_percent_encoded_full_url_path() {
        let encoded = urlencoding::encode("https://flutter.github.io/assets/videos/butterfly.mp4");
        let path = format!("/{encoded}");
        let restored = to_origin_url(&path);
        assert_eq!(
            restored,
            "https://flutter.github.io/assets/videos/butterfly.mp4"
        );
    }

    #[test]
    fn local_and_origin_url_round_trip() {
        let mut config = Config::default();
        config.ip = "127.0.0.1".to_string();
        config.port = 9999;
        let remote = "https://flutter.github.io/assets-for-api-docs/assets/videos/butterfly.mp4";
        let local = to_local_url(remote, &config);
        let uri = Url::parse(&local).unwrap();
        let proxy_path = format!("{}?{}", uri.path(), uri.query().unwrap_or(""));
        let restored = to_origin_url(&proxy_path);
        assert_eq!(restored, remote);
    }
}
