use tokio::io::{AsyncWrite, AsyncWriteExt};

const HTTP_TERMINAL: &str = "\r\n\r\n";

pub async fn append_to_writer(writer: &mut (impl AsyncWrite + Unpin), data: &[u8]) -> bool {
    writer.write_all(data).await.is_ok()
}

pub async fn append_string(writer: &mut (impl AsyncWrite + Unpin), s: &str) -> bool {
    let mut buf = s.as_bytes().to_vec();
    buf.extend_from_slice(HTTP_TERMINAL.as_bytes());
    append_to_writer(writer, &buf).await
}

pub async fn append_headers_and_body(
    writer: &mut (impl AsyncWrite + Unpin),
    headers: &str,
    body: &[u8],
) -> bool {
    let mut header_buf = headers.as_bytes().to_vec();
    header_buf.extend_from_slice(HTTP_TERMINAL.as_bytes());
    if !append_to_writer(writer, &header_buf).await {
        return false;
    }
    append_to_writer(writer, body).await
}

pub async fn write_bad_gateway(writer: &mut (impl AsyncWrite + Unpin), message: &str) -> bool {
    let body = message.as_bytes();
    let headers = format!(
        "HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/plain\r\nContent-Length: {}",
        body.len()
    );
    append_headers_and_body(writer, &headers, body).await
}

pub async fn write_bad_request(writer: &mut (impl AsyncWrite + Unpin), message: &str) -> bool {
    let body = message.as_bytes();
    let headers = format!(
        "HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nContent-Length: {}",
        body.len()
    );
    append_headers_and_body(writer, &headers, body).await
}
