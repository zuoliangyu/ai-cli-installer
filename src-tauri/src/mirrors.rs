use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

use crate::error::{AppError, Result};
use crate::upstream::{self, Manifest};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Mirror {
    /// Mirror that exposes the same path layout as `downloads.claude.ai/claude-code-releases`.
    /// `base` should NOT have a trailing slash.
    Upstream { name: String, base: String },

    /// GitHub Release-based mirror. Optionally fronted by a GH proxy host.
    /// Releases must be tagged `v{VERSION}` and contain:
    ///   - manifest.json
    ///   - {platform}-{binary}   (e.g. win32-x64-claude.exe, linux-x64-claude)
    ///   - latest.txt / stable.txt (channel pointer files)
    GhRelease {
        name: String,
        owner: String,
        repo: String,
        proxy: Option<String>,
    },
}

impl Mirror {
    pub fn name(&self) -> &str {
        match self {
            Mirror::Upstream { name, .. } => name,
            Mirror::GhRelease { name, .. } => name,
        }
    }

    pub fn version_url(&self, channel: &str) -> String {
        match self {
            Mirror::Upstream { base, .. } => format!("{}/{}", base, channel),
            Mirror::GhRelease {
                owner, repo, proxy, ..
            } => {
                let raw = format!(
                    "https://raw.githubusercontent.com/{}/{}/main/channels/{}.txt",
                    owner, repo, channel
                );
                proxy
                    .as_deref()
                    .map(|p| format!("{}/{}", p.trim_end_matches('/'), raw))
                    .unwrap_or(raw)
            }
        }
    }

    pub fn manifest_url(&self, version: &str) -> String {
        match self {
            Mirror::Upstream { base, .. } => format!("{}/{}/manifest.json", base, version),
            Mirror::GhRelease {
                owner, repo, proxy, ..
            } => {
                let raw = format!(
                    "https://github.com/{}/{}/releases/download/v{}/manifest.json",
                    owner, repo, version
                );
                proxy
                    .as_deref()
                    .map(|p| format!("{}/{}", p.trim_end_matches('/'), raw))
                    .unwrap_or(raw)
            }
        }
    }

    pub fn binary_url(&self, version: &str, platform: &str, binary: &str) -> String {
        match self {
            Mirror::Upstream { base, .. } => {
                format!("{}/{}/{}/{}", base, version, platform, binary)
            }
            Mirror::GhRelease {
                owner, repo, proxy, ..
            } => {
                // GH Release assets are flat — encode platform into asset name
                let asset = format!("{}-{}", platform, binary);
                let raw = format!(
                    "https://github.com/{}/{}/releases/download/v{}/{}",
                    owner, repo, version, asset
                );
                proxy
                    .as_deref()
                    .map(|p| format!("{}/{}", p.trim_end_matches('/'), raw))
                    .unwrap_or(raw)
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MirrorList {
    pub mirrors: Vec<Mirror>,
}

impl MirrorList {
    /// Built-in fallback for the Claude Code mirror — kept for backwards compat /
    /// default UI display. Tools should call `builtin_for` with their own repo.
    pub fn builtin() -> Self {
        Self::builtin_for("claude-code-mirror", /* with_upstream */ true)
    }

    /// Built-in fallback parameterized by mirror repo name.
    /// `with_upstream`: include downloads.claude.ai (only valid for claude-code).
    pub fn builtin_for(repo: &str, with_upstream: bool) -> Self {
        let owner = "zuoliangyu";

        let gh = |name: &str, proxy: Option<&str>| Mirror::GhRelease {
            name: name.to_string(),
            owner: owner.to_string(),
            repo: repo.to_string(),
            proxy: proxy.map(String::from),
        };

        let mut mirrors = Vec::new();
        if with_upstream {
            mirrors.push(Mirror::Upstream {
                name: "official".to_string(),
                base: "https://downloads.claude.ai/claude-code-releases".to_string(),
            });
        }
        mirrors.extend([
            gh("github-direct", None),
            gh("gh-proxy", Some("https://gh-proxy.com")),
            gh("fastgit", Some("https://fastgit.cc")),
            gh("yylx", Some("https://git.yylx.win")),
            gh("chenc", Some("https://github.chenc.dev")),
            gh("ghproxy-net", Some("https://ghproxy.net")),
            gh("ghfast", Some("https://ghfast.top")),
        ]);
        Self { mirrors }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MirrorProbe {
    pub name: String,
    pub ok: bool,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
}

/// Probe each mirror with a quick HEAD on the version endpoint and return latency.
pub async fn probe_all(client: &reqwest::Client, list: &MirrorList) -> Vec<MirrorProbe> {
    let probes = list.mirrors.iter().map(|m| {
        let client = client.clone();
        let url = m.version_url("latest");
        let name = m.name().to_string();
        async move {
            let start = Instant::now();
            match tokio::time::timeout(Duration::from_secs(5), client.head(&url).send()).await {
                Ok(Ok(r)) if r.status().is_success() => MirrorProbe {
                    name,
                    ok: true,
                    latency_ms: Some(start.elapsed().as_millis() as u64),
                    error: None,
                },
                Ok(Ok(r)) => MirrorProbe {
                    name,
                    ok: false,
                    latency_ms: None,
                    error: Some(format!("status {}", r.status())),
                },
                Ok(Err(e)) => MirrorProbe {
                    name,
                    ok: false,
                    latency_ms: None,
                    error: Some(e.to_string()),
                },
                Err(_) => MirrorProbe {
                    name,
                    ok: false,
                    latency_ms: None,
                    error: Some("timeout".to_string()),
                },
            }
        }
    });
    futures_util::future::join_all(probes).await
}

/// Try mirrors in order, returning the first one that successfully fetches the version string.
pub async fn fetch_version<'a>(
    client: &reqwest::Client,
    list: &'a MirrorList,
    channel: &str,
) -> Result<(&'a Mirror, String)> {
    for m in &list.mirrors {
        let url = m.version_url(channel);
        match upstream::fetch_text(client, &url).await {
            Ok(v) if !v.is_empty() => return Ok((m, v)),
            _ => continue,
        }
    }
    Err(AppError::AllMirrorsFailed)
}

/// Try mirrors in order to fetch the manifest for a given version.
pub async fn fetch_manifest<'a>(
    client: &reqwest::Client,
    list: &'a MirrorList,
    version: &str,
) -> Result<(&'a Mirror, Manifest)> {
    for m in &list.mirrors {
        let url = m.manifest_url(version);
        if let Ok(manifest) = upstream::fetch_manifest(client, &url).await {
            return Ok((m, manifest));
        }
    }
    Err(AppError::AllMirrorsFailed)
}
