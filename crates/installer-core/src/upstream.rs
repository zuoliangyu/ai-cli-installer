use futures_util::StreamExt;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

use crate::error::{AppError, Result};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Manifest {
    pub version: String,
    #[serde(default)]
    pub commit: String,
    #[serde(default, rename = "buildDate")]
    pub build_date: String,
    pub platforms: BTreeMap<String, PlatformEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlatformEntry {
    /// File name on our mirror release (after `{platform}-` prefix in flat
    /// asset name). For Claude Code: `claude` / `claude.exe`. For Codex:
    /// `codex.zst` / `codex.exe.zst` (still compressed at this point).
    pub binary: String,

    /// Set when `binary` is an archive that decompresses to a different
    /// filename (Codex: `binary=codex.zst`, `runtime_binary=codex`).
    /// `None` for Claude Code where `binary` IS the executable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_binary: Option<String>,

    pub checksum: String,
    pub size: u64,
}

impl PlatformEntry {
    /// Final executable name on disk after any extraction. Falls back to
    /// `binary` when `runtime_binary` isn't set (e.g. Claude Code).
    pub fn runtime_filename(&self) -> &str {
        self.runtime_binary.as_deref().unwrap_or(&self.binary)
    }
}

/// Hard cap for small metadata responses (version pointers, manifests,
/// fixes.json). Real payloads are a few KB; a proxy that streams more than
/// this is broken or hostile, and we must not buffer it all in memory.
pub const MAX_METADATA_BYTES: usize = 1024 * 1024;
const METADATA_TIMEOUT: Duration = Duration::from_secs(60);

/// Read a response body into memory, failing once it exceeds `max` bytes.
pub async fn read_capped(resp: reqwest::Response, max: usize) -> Result<Vec<u8>> {
    if resp.content_length().is_some_and(|len| len > max as u64) {
        return Err(AppError::Other(format!("响应体过大（超过 {} 字节）", max)));
    }
    let mut stream = resp.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if body.len() + chunk.len() > max {
            return Err(AppError::Other(format!("响应体过大（超过 {} 字节）", max)));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub async fn fetch_text(client: &reqwest::Client, url: &str) -> Result<String> {
    let body = fetch_metadata(client, url, METADATA_TIMEOUT).await?;
    Ok(String::from_utf8_lossy(&body).trim().to_string())
}

/// GET `url` and parse it as JSON (size-capped like [`fetch_text`]).
pub async fn fetch_json<T: DeserializeOwned>(client: &reqwest::Client, url: &str) -> Result<T> {
    let body = fetch_metadata(client, url, METADATA_TIMEOUT).await?;
    Ok(serde_json::from_slice(&body)?)
}

async fn fetch_metadata(client: &reqwest::Client, url: &str, timeout: Duration) -> Result<Vec<u8>> {
    let resp = client
        .get(url)
        .timeout(timeout)
        .send()
        .await?
        .error_for_status()?;
    read_capped(resp, MAX_METADATA_BYTES).await
}

pub async fn fetch_manifest(client: &reqwest::Client, url: &str) -> Result<Manifest> {
    fetch_json(client, url).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn metadata_keeps_total_deadline_during_continuous_body() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let read = socket.read(&mut buffer).await.unwrap();
                assert!(read > 0);
                request.extend_from_slice(&buffer[..read]);
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            for _ in 0..20 {
                tokio::time::sleep(Duration::from_millis(30)).await;
                if socket.write_all(b"x").await.is_err() {
                    return;
                }
            }
        });
        let client = crate::app_state::build_http_client(Duration::from_secs(1));
        let result = fetch_metadata(
            &client,
            &format!("http://{addr}/metadata"),
            Duration::from_millis(100),
        )
        .await;
        server.abort();
        match result {
            Err(AppError::Network(error)) => assert!(error.is_timeout(), "{error}"),
            other => panic!("expected metadata total timeout, got {other:?}"),
        }
    }
}
