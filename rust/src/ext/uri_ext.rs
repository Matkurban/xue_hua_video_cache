use url::Url;

use super::string_ext::{generate_md5, to_safe_uri};

pub fn uri_path_prefix(uri: &Url, relative_path: usize) -> String {
    let segments: Vec<&str> = uri.path_segments().map(|s| s.collect()).unwrap_or_default();
    if segments.is_empty() {
        panic!("Path segments are empty");
    }
    let end = segments.len().saturating_sub(1 + relative_path);
    let truncated: Vec<&str> = segments[..end].to_vec();
    let mut new_uri = uri.clone();
    new_uri.set_path(&format!("/{}", truncated.join("/")));
    new_uri.set_query(None);
    new_uri.to_string().trim_end_matches('?').to_string()
}

pub fn uri_base(uri: &Url) -> String {
    match uri.port() {
        Some(port) => format!(
            "{}://{}:{}",
            uri.scheme(),
            uri.host_str().unwrap_or(""),
            port
        ),
        None => format!("{}://{}", uri.scheme(), uri.host_str().unwrap_or("")),
    }
}

pub fn uri_generate_md5(uri: &Url) -> String {
    generate_md5(&uri.to_string())
}

/// Canonical HLS playlist directory key: always derived from normalized URL.
pub fn hls_key_for_url(url: &str) -> String {
    uri_generate_md5(&to_safe_uri(url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hls_key_for_url_trims_and_normalizes() {
        let raw = "  https://cdn.example.com/master.m3u8  ";
        let normalized = to_safe_uri(raw);
        assert_eq!(hls_key_for_url(raw), uri_generate_md5(&normalized));
    }

    #[test]
    fn hls_key_for_url_matches_uri_generate_md5_after_safe_parse() {
        let url = "https://cdn.example.com/videos/master.m3u8";
        assert_eq!(hls_key_for_url(url), uri_generate_md5(&to_safe_uri(url)));
    }
}
