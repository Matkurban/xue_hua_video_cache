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
