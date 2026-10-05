//! npm-route installer for Claude Code / Codex.
//!
//! Two install modes:
//! 1. **Mirror tarballs (preferred, v0.0.11+)** — download the `.tgz` for the
//!    main package + the user's current platform from our mirror release (which
//!    has GH proxy fallback chain), then `npm cache add` the platform tarball
//!    and `npm install -g <main.tgz> --include=optional --prefer-offline`.
//!    No external registry needed, fastest in CN, version-locked to mirror.
//! 2. **Online registry (fallback)** — `npm install -g <pkg>@<version> --registry npmmirror`.
//!    Used if mirror tarballs fail to download or apply.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::downloader;
use crate::error::{AppError, Result};
use crate::mirrors::{self, Mirror, MirrorList};
use crate::proc::{output_with_timeout, shell_command, INSTALL_TIMEOUT, PROBE_TIMEOUT};
use crate::validate;

const DEFAULT_REGISTRY: &str = "https://registry.npmmirror.com";

/// Platform keys we know how to map to npm sub-packages. Longest first so
/// suffix matching never mistakes `linux-x64-musl` for `linux-x64`.
const NPM_PLATFORMS: &[&str] = &[
    "linux-arm64-musl",
    "linux-x64-musl",
    "darwin-arm64",
    "darwin-x64",
    "linux-arm64",
    "linux-x64",
    "win32-arm64",
    "win32-x64",
];

#[derive(Debug, Clone, serde::Serialize)]
pub struct NodeInfo {
    pub node_version: String,   // e.g. "v22.16.0"
    pub node_major: u32,
    pub npm_version: Option<String>,
}

/// Detect Node + npm. Errors if Node not on PATH or unparseable.
pub async fn detect_node() -> Result<NodeInfo> {
    let mut node_cmd = shell_command("node");
    node_cmd.arg("--version");
    let node_out = output_with_timeout(&mut node_cmd, PROBE_TIMEOUT)
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

    let mut npm_cmd = shell_command("npm");
    npm_cmd.arg("--version");
    let npm_version = output_with_timeout(&mut npm_cmd, PROBE_TIMEOUT)
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
/// `package` may carry a version spec (`@scope/name@1.2.3`) — callers pin the
/// version they resolved so the fallback doesn't silently install `latest`.
/// Doesn't expose progress (npm install is opaque); UI shows a spinner.
pub async fn install_global(package: &str, registry: Option<&str>) -> Result<String> {
    let reg = registry.unwrap_or(DEFAULT_REGISTRY);

    let mut cmd = shell_command("npm");
    cmd.args(["install", "-g", package, "--registry", reg]);
    let output = output_with_timeout(&mut cmd, INSTALL_TIMEOUT)
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

/// What an npm-manifest entry is, relative to the package being installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NpmEntryRole {
    /// The wrapper package itself (`@anthropic-ai/claude-code`, Codex `main`).
    Main,
    /// A platform sub-package; carries the platform key (`linux-x64-musl`, ...).
    Platform(String),
    /// Anything else — ignored rather than guessed at.
    Unknown,
}

impl NpmManifestEntry {
    /// Classify this entry for `package` (the main npm package name).
    ///
    /// Codex: `label` is explicit (`main` or a platform key; every entry has
    /// the same `name`). Claude Code: no label; the main wrapper's name is
    /// exactly `package`, platform packages are exactly `{package}-{plat}`.
    /// Exact matching means an unexpected entry (e.g. a new
    /// `*-linux-x64-musl` package we don't list) can never replace the main
    /// wrapper.
    pub fn role(&self, package: &str) -> NpmEntryRole {
        if let Some(label) = &self.label {
            return if label == "main" {
                NpmEntryRole::Main
            } else {
                NpmEntryRole::Platform(label.clone())
            };
        }
        if self.name == package {
            return NpmEntryRole::Main;
        }
        match self
            .name
            .strip_prefix(package)
            .and_then(|rest| rest.strip_prefix('-'))
        {
            Some(plat) if NPM_PLATFORMS.contains(&plat) => NpmEntryRole::Platform(plat.to_string()),
            _ => NpmEntryRole::Unknown,
        }
    }

    /// Best-effort role detection without knowing the main package name:
    /// Some(platform_key) for sub-packages, None for the main wrapper.
    /// Prefer [`Self::role`], which matches names exactly.
    pub fn detect_platform(&self) -> Option<String> {
        if let Some(label) = &self.label {
            if label != "main" {
                return Some(label.clone());
            }
            return None;
        }
        NPM_PLATFORMS
            .iter()
            .find(|plat| self.name.ends_with(&format!("-{}", plat)))
            .map(|plat| plat.to_string())
    }
}

/// Pick the main wrapper and the entry for `platform` out of `manifest`.
///
/// Codex publishes one static (musl) Linux build labelled `linux-x64`, so on
/// a musl system the label scheme falls back from `linux-x64-musl` to
/// `linux-x64`. Claude Code ships distinct musl packages, so no fallback
/// there — the npmmirror route handles musl correctly via optionalDeps.
fn select_entries<'a>(
    manifest: &'a NpmManifest,
    package: &str,
    platform: &str,
) -> Result<(&'a NpmManifestEntry, &'a NpmManifestEntry)> {
    let mut main: Option<&NpmManifestEntry> = None;
    let mut exact: Option<&NpmManifestEntry> = None;
    let mut musl_fallback: Option<&NpmManifestEntry> = None;
    let glibc_key = platform.strip_suffix("-musl");
    for entry in &manifest.packages {
        match entry.role(package) {
            NpmEntryRole::Main if main.is_none() => main = Some(entry),
            NpmEntryRole::Main => {
                return Err(AppError::Other(
                    "npm-manifest has more than one main wrapper entry".into(),
                ))
            }
            NpmEntryRole::Platform(p) if p == platform => exact = Some(entry),
            NpmEntryRole::Platform(p)
                if entry.label.is_some() && glibc_key == Some(p.as_str()) =>
            {
                musl_fallback = Some(entry)
            }
            NpmEntryRole::Platform(_) => {}
            NpmEntryRole::Unknown => {
                tracing::warn!("npm-manifest: ignoring unrecognized entry {}", entry.name)
            }
        }
    }
    let main =
        main.ok_or_else(|| AppError::Other("npm-manifest missing main wrapper entry".into()))?;
    let plat = exact.or(musl_fallback).ok_or_else(|| {
        AppError::Other(format!(
            "npm-manifest has no entry for platform `{}`",
            platform
        ))
    })?;
    Ok((main, plat))
}

/// Fetch `npm-manifest.json` of a given version through the mirror chain.
///
/// Same trust policy as the native manifest (`mirrors::fetch_json_trusted`):
/// github-direct wins outright; otherwise two proxies must agree on every
/// package's tarball name + checksum + size. Upstream mirrors don't host npm
/// assets and are skipped.
async fn fetch_npm_manifest(
    client: &reqwest::Client,
    mirrors: &MirrorList,
    version: &str,
) -> Result<NpmManifest> {
    let (m, manifest) = mirrors::fetch_json_trusted(
        client,
        mirrors,
        "npm-manifest.json",
        |m| match m {
            Mirror::GhRelease { .. } => Some(m.asset_url(version, "npm-manifest.json")),
            Mirror::Upstream { .. } => None,
        },
        npm_manifest_fingerprint,
    )
    .await?;
    tracing::info!("npm-manifest.json from {}", m.name());
    Ok(manifest)
}

type NpmFingerprint = Vec<(String, String, Option<String>, String, String, u64)>;

fn npm_manifest_fingerprint(m: &NpmManifest) -> Option<NpmFingerprint> {
    if m.packages.is_empty() {
        return None;
    }
    let mut fp: NpmFingerprint = m
        .packages
        .iter()
        .map(|p| {
            (
                p.name.clone(),
                p.version.clone(),
                p.label.clone(),
                p.tgz.clone(),
                p.checksum.to_ascii_lowercase(),
                p.size,
            )
        })
        .collect();
    fp.sort();
    Some(fp)
}

/// Install a tool's npm package by downloading 2 .tgz files from our mirror
/// (main wrapper + current platform sub-package) and feeding them to npm with
/// `--prefer-offline`. Returns Ok on success; caller can fall back to online
/// install on Err.
///
/// `package` is the main npm package name (e.g. `@anthropic-ai/claude-code`);
/// `manifest_mirrors` is where `npm-manifest.json` comes from (the tool's
/// full list, so the trust policy can reach github-direct even when the user
/// pinned a single proxy for downloads), `mirrors` is where tarballs come from.
pub async fn install_via_mirror_tarballs(
    client: &reqwest::Client,
    manifest_mirrors: &MirrorList,
    mirrors: &MirrorList,
    package: &str,
    version: &str,
    platform: &str,
) -> Result<()> {
    let manifest = fetch_npm_manifest(client, manifest_mirrors, version).await?;
    let (main, plat_entry) = select_entries(&manifest, package, platform)?;

    // Both names come off the network and are joined onto our cache dir.
    let main_tgz = validate::ensure_file_name(&main.tgz, "npm-manifest tgz")?;
    let plat_tgz = validate::ensure_file_name(&plat_entry.tgz, "npm-manifest tgz")?;

    // Stage to a tool-and-version-keyed cache dir
    let cache = npm_stage_dir(version)?;
    let main_path = cache.join(main_tgz);
    let plat_path = cache.join(plat_tgz);

    let progress = crate::progress::noop_progress();
    for (entry, tgz, dest) in [
        (main, main_tgz, &main_path),
        (plat_entry, plat_tgz, &plat_path),
    ] {
        let candidates: Vec<(String, String)> = mirrors
            .mirrors
            .iter()
            .filter_map(|m| match m {
                Mirror::GhRelease { .. } => {
                    Some((m.name().to_string(), m.asset_url(version, tgz)))
                }
                Mirror::Upstream { .. } => None,
            })
            .collect();
        downloader::download_verified(
            client,
            &progress,
            "npm",
            candidates,
            &entry.checksum,
            Some(entry.size),
            dest,
        )
        .await?;
    }

    // Cache the platform tarball (works for both Claude's separate-package
    // scheme and Codex's version-alias scheme). Paths are passed as OsStr,
    // so non-UTF-8 home directories aren't silently turned into "".
    let mut cache_cmd = shell_command("npm");
    cache_cmd.args(["cache", "add"]).arg(&plat_path);
    let cache_out = output_with_timeout(&mut cache_cmd, INSTALL_TIMEOUT)
        .await
        .map_err(|e| AppError::Other(format!("npm cache add failed to spawn: {}", e)))?;
    if !cache_out.status.success() {
        return Err(AppError::Install(format!(
            "npm cache add 失败: {}",
            String::from_utf8_lossy(&cache_out.stderr).trim()
        )));
    }

    // Install main, optionalDeps resolved from our cache
    let mut install_cmd = shell_command("npm");
    install_cmd
        .args(["install", "-g"])
        .arg(&main_path)
        .args(["--include=optional", "--prefer-offline"]);
    let install_out = output_with_timeout(&mut install_cmd, INSTALL_TIMEOUT)
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
    let version = validate::ensure_version(version, "npm 版本")?;
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
    let mut cmd = shell_command("npm");
    cmd.args(["prefix", "-g"]);
    let prefix_out = output_with_timeout(&mut cmd, PROBE_TIMEOUT)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, label: Option<&str>, tgz: &str) -> NpmManifestEntry {
        NpmManifestEntry {
            name: name.into(),
            version: "1.0.0".into(),
            label: label.map(String::from),
            tgz: tgz.into(),
            checksum: "00".into(),
            size: 1,
        }
    }

    const CC: &str = "@anthropic-ai/claude-code";

    fn claude_manifest() -> NpmManifest {
        NpmManifest {
            version: "1.0.0".into(),
            registry: String::new(),
            packages: vec![
                entry(CC, None, "main.tgz"),
                entry(&format!("{CC}-linux-x64"), None, "linux.tgz"),
                entry(&format!("{CC}-linux-x64-musl"), None, "musl.tgz"),
                entry(&format!("{CC}-win32-x64"), None, "win.tgz"),
                entry("@anthropic-ai/claude-code-something-new", None, "odd.tgz"),
            ],
        }
    }

    #[test]
    fn claude_roles_use_exact_names() {
        assert_eq!(entry(CC, None, "x").role(CC), NpmEntryRole::Main);
        assert_eq!(
            entry(&format!("{CC}-linux-x64-musl"), None, "x").role(CC),
            NpmEntryRole::Platform("linux-x64-musl".into())
        );
        assert_eq!(
            entry(&format!("{CC}-something-new"), None, "x").role(CC),
            NpmEntryRole::Unknown
        );
        assert_eq!(
            entry("@other/claude-code-linux-x64", None, "x").role(CC),
            NpmEntryRole::Unknown
        );
    }

    #[test]
    fn unknown_entries_never_replace_main() {
        let m = claude_manifest();
        let (main, plat) = select_entries(&m, CC, "linux-x64").unwrap();
        assert_eq!(main.tgz, "main.tgz");
        assert_eq!(plat.tgz, "linux.tgz");
        let (_, plat) = select_entries(&m, CC, "linux-x64-musl").unwrap();
        assert_eq!(plat.tgz, "musl.tgz");
        assert!(select_entries(&m, CC, "darwin-arm64").is_err());
    }

    #[test]
    fn codex_labels_with_musl_fallback() {
        let pkg = "@openai/codex";
        let m = NpmManifest {
            version: "0.1.0".into(),
            registry: String::new(),
            packages: vec![
                entry(pkg, Some("main"), "main.tgz"),
                entry(pkg, Some("linux-x64"), "linux.tgz"),
                entry(pkg, Some("win32-x64"), "win.tgz"),
            ],
        };
        let (main, plat) = select_entries(&m, pkg, "win32-x64").unwrap();
        assert_eq!((main.tgz.as_str(), plat.tgz.as_str()), ("main.tgz", "win.tgz"));
        let (_, plat) = select_entries(&m, pkg, "linux-x64-musl").unwrap();
        assert_eq!(plat.tgz, "linux.tgz");
    }

    #[test]
    fn legacy_detect_platform_prefers_musl_suffix() {
        assert_eq!(
            entry(&format!("{CC}-linux-x64-musl"), None, "x").detect_platform(),
            Some("linux-x64-musl".into())
        );
        assert_eq!(entry(CC, None, "x").detect_platform(), None);
    }

    #[test]
    fn stage_dir_rejects_bad_versions() {
        assert!(npm_stage_dir("../../etc").is_err());
        assert!(npm_stage_dir("1.2.3").is_ok());
    }

    #[test]
    fn npm_fingerprint_ignores_order() {
        let a = claude_manifest();
        let mut b = claude_manifest();
        b.packages.reverse();
        assert_eq!(npm_manifest_fingerprint(&a), npm_manifest_fingerprint(&b));
    }
}
