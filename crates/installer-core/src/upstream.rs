use futures_util::StreamExt;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    let resp = client.get(url).send().await?.error_for_status()?;
    let body = read_capped(resp, MAX_METADATA_BYTES).await?;
    Ok(String::from_utf8_lossy(&body).trim().to_string())
}

/// GET `url` and parse it as JSON (size-capped like [`fetch_text`]).
pub async fn fetch_json<T: DeserializeOwned>(client: &reqwest::Client, url: &str) -> Result<T> {
    let resp = client.get(url).send().await?.error_for_status()?;
    let body = read_capped(resp, MAX_METADATA_BYTES).await?;
    Ok(serde_json::from_slice(&body)?)
}

pub async fn fetch_manifest(client: &reqwest::Client, url: &str) -> Result<Manifest> {
    fetch_json(client, url).await
}
