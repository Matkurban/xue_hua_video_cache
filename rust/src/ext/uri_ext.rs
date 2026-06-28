use url::Url;

use super::string_ext::generate_md5;

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
