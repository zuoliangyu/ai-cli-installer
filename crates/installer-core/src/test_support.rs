//! Test-only helpers shared by module tests.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Tiny one-response-per-connection HTTP server. `respond(request_index,
/// range_start)` returns the raw response bytes; the socket is closed
/// right after, so truncated bodies look like real connection drops.
pub async fn serve<F>(respond: F) -> String
where
    F: Fn(usize, Option<u64>) -> Vec<u8> + Send + Sync + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let respond = Arc::new(respond);
    let counter = Arc::new(AtomicUsize::new(0));
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let respond = respond.clone();
            let counter = counter.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 1024];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                }
                let head = String::from_utf8_lossy(&buf).to_ascii_lowercase();
                let range = head
                    .lines()
                    .find_map(|l| l.strip_prefix("range: bytes="))
                    .and_then(|r| r.trim().trim_end_matches('-').parse().ok());
                let idx = counter.fetch_add(1, Ordering::SeqCst);
                let _ = sock.write_all(&respond(idx, range)).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    format!("http://{}/file", addr)
}

pub fn response(status: &str, headers: &[String], body: &[u8]) -> Vec<u8> {
    let mut out = format!("HTTP/1.1 {}\r\nConnection: close\r\n", status).into_bytes();
    for h in headers {
        out.extend_from_slice(h.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(body);
    out
}
