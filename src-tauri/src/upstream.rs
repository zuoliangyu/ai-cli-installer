use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::error::Result;

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

pub async fn fetch_text(client: &reqwest::Client, url: &str) -> Result<String> {
    let resp = client.get(url).send().await?.error_for_status()?;
    Ok(resp.text().await?.trim().to_string())
}

pub async fn fetch_manifest(client: &reqwest::Client, url: &str) -> Result<Manifest> {
    let resp = client.get(url).send().await?.error_for_status()?;
    let body = resp.bytes().await?;
    let m: Manifest = serde_json::from_slice(&body)?;
    Ok(m)
}
