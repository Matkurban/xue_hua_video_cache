use std::sync::Arc;

use parking_lot::Mutex;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};

use crate::ext::log_ext::{log_d, log_w};
use crate::ext::socket_ext::append_string;
use crate::ext::string_ext::to_origin_url;
use crate::parser::video_caching::VideoCaching;
use crate::proxy::proxy_runtime::ProxyRuntime;

pub struct LocalProxyServer {
    runtime: Arc<ProxyRuntime>,
    shutdown: tokio::sync::watch::Sender<bool>,
    handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl LocalProxyServer {
    pub fn new(runtime: Arc<ProxyRuntime>) -> Self {
        let (tx, _) = tokio::sync::watch::channel(false);
        Self {
            runtime,
            shutdown: tx,
            handle: Mutex::new(None),
        }
    }

    pub async fn start(&mut self) -> Result<(), std::io::Error> {
        let _ = self.shutdown.send(false);
        let (ip, port) = {
            let config = self.runtime.ctx.config.read();
            (config.ip.clone(), config.port)
        };
        let listener = self.bind_with_retry(&ip, port).await?;
        let bound_port = listener.local_addr()?.port();
        {
            let mut config = self.runtime.ctx.config.write();
            config.port = bound_port;
        }
        log_d(&format!("Proxy server started {ip}:{bound_port}"));
        let shutdown_rx = self.shutdown.subscribe();
        let runtime = self.runtime.clone();
        let handle = tokio::spawn(async move {
            Self::run_listener(listener, shutdown_rx, runtime).await;
        });
        *self.handle.lock() = Some(handle);
        Ok(())
    }

    async fn bind_with_retry(
        &self,
        ip: &str,
        mut port: u16,
    ) -> Result<TcpListener, std::io::Error> {
        loop {
            let addr = format!("{ip}:{port}");
            match TcpListener::bind(&addr).await {
                Ok(listener) => return Ok(listener),
                Err(e) if e.raw_os_error() == Some(48) || e.raw_os_error() == Some(98) => {
                    log_w(&format!("Port {port} in use, trying next port: {e}"));
                    port = port.saturating_add(1);
                    self.runtime.ctx.config.write().port = port;
                }
                Err(e) => return Err(e),
            }
        }
    }

    async fn run_listener(
        listener: TcpListener,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
        runtime: Arc<ProxyRuntime>,
    ) {
        loop {
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_ok() && *shutdown.borrow() {
                        break;
                    }
                }
                accept = listener.accept() => {
                    if let Ok((stream, _)) = accept {
                        let runtime = runtime.clone();
                        tokio::spawn(handle_connection(stream, runtime));
                    }
                }
            }
        }
    }

    pub async fn restart(&mut self) -> Result<(), std::io::Error> {
        log_d("Proxy server restart requested...");
        self.shutdown_listener();
        let (tx, _) = tokio::sync::watch::channel(false);
        self.shutdown = tx;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        self.start().await
    }

    pub fn shutdown_listener(&mut self) {
        let _ = self.shutdown.send(true);
        if let Some(h) = self.handle.lock().take() {
            h.abort();
        }
    }
}

async fn handle_connection(mut stream: TcpStream, runtime: Arc<ProxyRuntime>) {
    let mut buf = vec![0u8; 8192];
    let n = match stream.read(&mut buf).await {
        Ok(n) => n,
        Err(e) => {
            log_w(&format!("Socket read error: {e}"));
            return;
        }
    };
    let request = String::from_utf8_lossy(&buf[..n]);
    if !request.contains("\r\n\r\n") {
        let _ = append_string(&mut stream, "HTTP/1.1 400 Bad Request").await;
        return;
    }
    let (head, _) = request.split_once("\r\n\r\n").unwrap();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    let path = parts.get(1).copied().unwrap_or("/");
    let mut headers = std::collections::HashMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_lowercase(), v.trim().to_string());
        }
    }
    if headers.is_empty() {
        let _ = append_string(&mut stream, "HTTP/1.1 400 Bad Request").await;
        return;
    }
    let origin = to_origin_url(path);
    log_d(&format!("Proxy request path -> {origin}"));
    let uri =
        url::Url::parse(&origin).unwrap_or_else(|_| url::Url::parse("http://invalid").unwrap());
    let _ = VideoCaching::parse(runtime, stream, uri, headers).await;
}
