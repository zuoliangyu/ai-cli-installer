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
    pub binary: String,
    pub checksum: String,
    pub size: u64,
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
