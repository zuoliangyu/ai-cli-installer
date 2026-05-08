//! npm-route installer for Claude Code / Codex.
//!
//! Two install modes:
//! 1. **Mirror tarballs (preferred, v0.0.11+)** — download the `.tgz` for the
//!    main package + the user's current platform from our mirror release (which
//!    has GH proxy fallback chain), then `npm cache add` the platform tarball
//!    and `npm install -g <main.tgz> --include=optional --prefer-offline`.
//!    No external registry needed, fastest in CN, version-locked to mirror.
//! 2. **Online registry (fallback)** — `npm install -g <pkg> --registry npmmirror`.
//!    Used if mirror tarballs fail to download or apply.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::error::{AppError, Result};
use crate::mirrors::{Mirror, MirrorList};
use crate::proc::shell_command;
use crate::verifier;

const DEFAULT_REGISTRY: &str = "https://registry.npmmirror.com";

#[derive(Debug, Clone, serde::Serialize)]
pub struct NodeInfo {
    pub node_version: String,   // e.g. "v22.16.0"
    pub node_major: u32,
    pub npm_version: Option<String>,
}

/// Detect Node + npm. Errors if Node not on PATH or unparseable.
pub async fn detect_node() -> Result<NodeInfo> {
    let node_out = shell_command("node")
        .arg("--version")
        .output()
        .await
        .map_err(|e| {
            AppError::Other(format!(
                "未检测到 Node.js（{}）。请先安装 Node 18+：https://nodejs.org",
                e
            ))
        })?;

    if !node_out.status.success() {
        return Err(AppError::Other(format!(
            "node --version 失败 (exit {:?})",
            node_out.status.code()
        )));
    }

    let node_version = String::from_utf8_lossy(&node_out.stdout).trim().to_string();
    let stripped = node_version.trim_start_matches('v');
    let node_major: u32 = stripped
        .split('.')
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| AppError::Other(format!("无法解析 Node 版本: {}", node_version)))?;

    let npm_version = shell_command("npm")
        .arg("--version")
        .output()
        .await
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());

    Ok(NodeInfo {
        node_version,
        node_major,
        npm_version,
    })
}

/// Run `npm install -g <package> --registry <r>` and return stdout on success.
/// Doesn't expose progress (npm install is opaque); UI shows a spinner.
pub async fn install_global(package: &str, registry: Option<&str>) -> Result<String> {
    let reg = registry.unwrap_or(DEFAULT_REGISTRY);

    let output = shell_command("npm")
        .args(["install", "-g", package, "--registry", reg])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| AppError::Other(format!("启动 npm 失败：{}", e)))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        return Err(AppError::Install(format!(
            "npm install -g {} 失败 (exit {:?})\n--- stderr ---\n{}\n--- stdout ---\n{}",
            package,
            output.status.code(),
            stderr.trim(),
            stdout.trim()
        )));
    }

    tracing::debug!("npm install stdout:\n{}", stdout);
    Ok(stdout)
}

// ---------- Mirror-tarball install path ----------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NpmManifest {
    pub version: String,
    #[serde(default)]
    pub registry: String,
    pub packages: Vec<NpmManifestEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NpmManifestEntry {
    pub name: String,
    pub version: String,
    /// Codex-style label ("main" / "linux-x64" / etc). Absent in Claude
    /// Code's manifest because each platform is its own scoped package.
    #[serde(default)]
    pub label: Option<String>,
    pub tgz: String,
    pub checksum: String,
    pub size: u64,
}

impl NpmManifestEntry {
    /// Best-effort role detection: is this the "main" wrapper or a platform sub-package?
    /// Returns Some(platform_key) for sub-packages, None for the main wrapper.
    pub fn detect_platform(&self) -> Option<String> {
        // Codex variant: label is set explicitly
        if let Some(label) = &self.label {
            if label != "main" {
                return Some(label.clone());
            }
            return None;
        }
        // Claude variant: package name has the platform suffix, e.g.
        //   "@anthropic-ai/claude-code-linux-x64" → "linux-x64"
        // The main wrapper is just "@anthropic-ai/claude-code".
        for plat in [
            "darwin-arm64",
            "darwin-x64",
            "linux-arm64",
            "linux-x64",
            "win32-arm64",
            "win32-x64",
        ] {
            if self.name.ends_with(plat) {
                return Some(plat.to_string());
            }
        }
        None
    }
}

/// Try mirrors in order to download `npm-manifest.json` of a given version.
async fn fetch_npm_manifest(
    client: &reqwest::Client,
    mirrors: &MirrorList,
    version: &str,
) -> Result<NpmManifest> {
    for m in &mirrors.mirrors {
        let url = match m {
            Mirror::GhRelease { .. } => m.asset_url(version, "npm-manifest.json"),
            Mirror::Upstream { .. } => continue,
        };
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => match resp.bytes().await {
                Ok(bytes) => {
                    if let Ok(manifest) = serde_json::from_slice::<NpmManifest>(&bytes) {
                        tracing::info!("npm-manifest.json from {}", m.name());
                        return Ok(manifest);
                    }
                }
                Err(e) => tracing::warn!("npm-manifest read failed via {}: {}", m.name(), e),
            },
            Ok(r) => tracing::warn!("npm-manifest {} returned {}", m.name(), r.status()),
            Err(e) => tracing::warn!("npm-manifest fetch via {}: {}", m.name(), e),
        }
    }
    Err(AppError::AllMirrorsFailed)
}

/// Download an asset via the mirror chain (first success wins) and verify SHA256.
async fn download_asset(
    client: &reqwest::Client,
    mirrors: &MirrorList,
    version: &str,
    asset: &str,
    expected_sha256: &str,
    dest: &Path,
) -> Result<()> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    for m in &mirrors.mirrors {
        let url = match m {
            Mirror::GhRelease { .. } => m.asset_url(version, asset),
            Mirror::Upstream { .. } => continue,
        };
        tracing::info!("download {} via {}: {}", asset, m.name(), url);
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => match resp.bytes().await {
                Ok(bytes) => {
                    if let Err(e) = tokio::fs::write(dest, &bytes).await {
                        tracing::warn!("write {} failed: {}", dest.display(), e);
                        continue;
                    }
                    if verifier::verify(dest, expected_sha256).await.is_ok() {
                        return Ok(());
                    }
                    tracing::warn!("checksum mismatch for {} via {}", asset, m.name());
                    let _ = tokio::fs::remove_file(dest).await;
                }
                Err(e) => tracing::warn!("read body {} via {}: {}", asset, m.name(), e),
            },
            Ok(r) => tracing::warn!("{} via {}: {}", asset, m.name(), r.status()),
            Err(e) => tracing::warn!("fetch {} via {}: {}", asset, m.name(), e),
        }
    }
    Err(AppError::AllMirrorsFailed)
}

/// Install a tool's npm package by downloading 2 .tgz files from our mirror
/// (main wrapper + current platform sub-package) and feeding them to npm with
/// `--prefer-offline`. Returns Ok on success; caller can fall back to online
/// install on Err.
pub async fn install_via_mirror_tarballs(
    client: &reqwest::Client,
    mirrors: &MirrorList,
    version: &str,
    platform: &str,
) -> Result<()> {
    let manifest = fetch_npm_manifest(client, mirrors, version).await?;

    // Find main wrapper (no platform) and the entry matching current platform.
    let mut main: Option<&NpmManifestEntry> = None;
    let mut plat_entry: Option<&NpmManifestEntry> = None;
    for entry in &manifest.packages {
        match entry.detect_platform() {
            None => main = Some(entry),
            Some(p) if p == platform => plat_entry = Some(entry),
            Some(_) => {}
        }
    }
    let main = main
        .ok_or_else(|| AppError::Other("npm-manifest missing main wrapper entry".into()))?;
    let plat_entry = plat_entry.ok_or_else(|| {
        AppError::Other(format!(
            "npm-manifest has no entry for platform `{}`",
            platform
        ))
    })?;

    // Stage to a tool-and-version-keyed cache dir
    let cache = npm_stage_dir(version)?;
    let main_path = cache.join(&main.tgz);
    let plat_path = cache.join(&plat_entry.tgz);

    download_asset(
        client,
        mirrors,
        version,
        &main.tgz,
        &main.checksum,
        &main_path,
    )
    .await?;
    download_asset(
        client,
        mirrors,
        version,
        &plat_entry.tgz,
        &plat_entry.checksum,
        &plat_path,
    )
    .await?;

    // Cache the platform tarball (works for both Claude's separate-package
    // scheme and Codex's version-alias scheme).
    let cache_out = shell_command("npm")
        .args(["cache", "add", plat_path.to_str().unwrap_or("")])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| AppError::Other(format!("npm cache add failed to spawn: {}", e)))?;
    if !cache_out.status.success() {
        return Err(AppError::Install(format!(
            "npm cache add 失败: {}",
            String::from_utf8_lossy(&cache_out.stderr).trim()
        )));
    }

    // Install main, optionalDeps resolved from our cache
    let install_out = shell_command("npm")
        .args([
            "install",
            "-g",
            main_path.to_str().unwrap_or(""),
            "--include=optional",
            "--prefer-offline",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| AppError::Other(format!("npm install failed to spawn: {}", e)))?;
    if !install_out.status.success() {
        return Err(AppError::Install(format!(
            "npm install -g via mirror 失败:\n{}",
            String::from_utf8_lossy(&install_out.stderr).trim()
        )));
    }

    // Cleanup staged tarballs (cache copy is what matters now)
    let _ = tokio::fs::remove_file(&main_path).await;
    let _ = tokio::fs::remove_file(&plat_path).await;

    Ok(())
}

fn npm_stage_dir(version: &str) -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| AppError::Other("no home dir".into()))?;
    Ok(home
        .join(".cache")
        .join("ai-cli-installer")
        .join("npm")
        .join(version))
}

/// `npm bin -g` returns the global bin directory. Useful for verifying
/// where the installed binary lives (so we can show a sensible install_path).
pub async fn npm_global_bin() -> Result<String> {
    // npm 9+ removed `npm bin -g`. Use `npm prefix -g` + /bin (Unix) or root (Win).
    let prefix_out = shell_command("npm")
        .args(["prefix", "-g"])
        .output()
        .await
        .map_err(|e| AppError::Other(format!("npm prefix -g failed: {}", e)))?;

    if !prefix_out.status.success() {
        return Err(AppError::Other("npm prefix -g failed".into()));
    }
    let prefix = String::from_utf8_lossy(&prefix_out.stdout).trim().to_string();

    // Windows: prefix IS the bin dir; Unix: prefix/bin
    let bin = if cfg!(target_os = "windows") {
        prefix
    } else {
        format!("{}/bin", prefix)
    };
    Ok(bin)
}
