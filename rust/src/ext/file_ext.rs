use std::sync::RwLock;

use once_cell::sync::Lazy;

static CACHE_ROOT: Lazy<RwLock<String>> = Lazy::new(|| RwLock::new(String::new()));

pub struct FileExt;

impl FileExt {
    pub fn set_cache_root_path(path: String) {
        if let Ok(mut g) = CACHE_ROOT.write() {
            *g = path;
        }
    }

    pub fn cache_root_path() -> String {
        CACHE_ROOT.read().map(|g| g.clone()).unwrap_or_default()
    }

    pub async fn create_cache_path(cache_dir: Option<&str>) -> Result<String, std::io::Error> {
        let mut root = Self::cache_root_path();
        if root.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "cache root not set; call XueHUAEVideoCache.initialize with cacheDir from path_provider",
            ));
        }
        if !root.ends_with("/videos") && !root.contains("/videos") {
            root = format!("{root}/videos");
            Self::set_cache_root_path(root.clone());
        }
        if let Some(dir) = cache_dir {
            if !dir.is_empty() {
                root = format!("{root}/{dir}");
            }
        }
        tokio::fs::create_dir_all(&root).await?;
        Ok(root)
    }
}
