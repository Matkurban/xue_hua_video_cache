//! Network integration test against the official Flutter butterfly.mp4 sample.
//! Opt-in only: `cargo test --ignored butterfly`

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::ext::file_ext::FileExt;
    use crate::global::CacheKeyConfig;
    use crate::parser::video_caching::VideoCaching;
    use crate::proxy::platform_kind::PlatformKind;
    use crate::proxy::video_proxy::VideoProxyState;

    const BUTTERFLY_MP4: &str =
        "https://flutter.github.io/assets-for-api-docs/assets/videos/butterfly.mp4";

    const WAIT_CACHED_TIMEOUT: Duration = Duration::from_secs(45);

    async fn wait_cached(segments: usize) -> bool {
        let deadline = tokio::time::Instant::now() + WAIT_CACHED_TIMEOUT;
        while tokio::time::Instant::now() < deadline {
            if VideoCaching::is_cached(BUTTERFLY_MP4, None, segments).await {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
        false
    }

    #[tokio::test]
    #[ignore = "network E2E — run with: cargo test --ignored butterfly"]
    async fn butterfly_mp4_precache_is_cached_and_proxy_serves_range() {
        let temp = std::env::temp_dir().join(format!("xue_hua_butterfly_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();
        FileExt::set_cache_root_path(temp.to_string_lossy().to_string());

        VideoProxyState::init(
            None,
            None,
            100,
            1024,
            String::new(),
            false,
            1,
            2,
            PlatformKind::Other,
            CacheKeyConfig::default(),
        )
        .await
        .expect("init");

        let state = VideoProxyState::get().expect("state");
        assert!(state.is_running().await);

        assert!(
            !VideoCaching::is_cached(BUTTERFLY_MP4, None, 2).await,
            "should not be cached before precache"
        );

        VideoCaching::precache(BUTTERFLY_MP4, None, 2, true, None)
            .await
            .expect("precache");

        assert!(
            wait_cached(2).await,
            "timed out waiting for butterfly.mp4 segments"
        );

        let (ip, port) = {
            let cfg = state.ctx.config.read();
            (cfg.ip.clone(), cfg.port)
        };

        let local_path = urlencoding::encode(BUTTERFLY_MP4);
        let addr = format!("{ip}:{port}");
        let mut stream = tokio::net::TcpStream::connect(&addr)
            .await
            .expect("connect proxy");
        let request = format!(
            "GET /{local_path} HTTP/1.1\r\nHost: {addr}\r\nRange: bytes=0-4095\r\nConnection: close\r\n\r\n"
        );
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut buf = vec![0u8; 16384];
        let n = stream.read(&mut buf).await.unwrap();
        let response = String::from_utf8_lossy(&buf[..n]);
        assert!(
            response.contains("200 OK") || response.contains("206 Partial Content"),
            "unexpected proxy response: {}",
            &response[..response.len().min(200)]
        );

        let _ = std::fs::remove_dir_all(&temp);
    }
}
