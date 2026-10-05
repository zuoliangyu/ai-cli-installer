use std::path::PathBuf;
use std::time::Instant;

use crate::downloader;
use crate::error::{AppError, Result};
use crate::fixes;
use crate::installer;
use crate::mirrors::{self, MirrorList};
use crate::npm_installer;
use crate::platform;
use crate::progress::ProgressCallback;
use crate::tools::{InstallMethod, InstallReport, Tool, ToolDescriptor, ToolId};
use crate::validate;

/// Fix IDs that get auto-applied right after a successful Claude Code
/// install. cc-005 writes `hasCompletedOnboarding: true` into
/// `~/.claude.json`, which lets users `claude login` without being
/// blocked by Claude Code's first-launch reachability check against
/// `api.anthropic.com` — the check ignores configured base_url
/// overrides, so CN / proxy users are otherwise stuck before login.
///
/// Keep this list small and clearly safe — anything more invasive
/// should remain user-driven via the 「配置修复」 panel. Definitions are
/// always taken from the embedded `fixes.json`, never the remote copy.
const AUTO_APPLY_ON_INSTALL: &[&str] = &["cc-005-onboarding-done"];

pub struct ClaudeCode;

impl ClaudeCode {
    pub const ID: ToolId = "claude-code";
    pub const NPM_PACKAGE: &'static str = "@anthropic-ai/claude-code";

    fn bin_name() -> &'static str {
        if cfg!(target_os = "windows") {
            "claude.exe"
        } else {
            "claude"
        }
    }

    fn launcher_path(&self) -> Option<PathBuf> {
        Some(self.launcher_dir()?.join(Self::bin_name()))
    }

    async fn install_native(
        &self,
        progress: ProgressCallback,
        client: reqwest::Client,
        mirrors: MirrorList,
        channel: String,
        version: String,
        started: Instant,
    ) -> Result<InstallReport> {
        let plat = platform::current()?;
        validate::ensure_version(&version, "Claude Code 版本号")?;

        // Manifest comes from the tool's full list (not a user-pinned subset)
        // so the trust policy can still reach official / github-direct.
        let manifest_list = self.mirror_list();
        let (_, manifest) = mirrors::fetch_manifest(&client, &manifest_list, &version).await?;
        let entry = manifest
            .platforms
            .get(plat)
            .ok_or_else(|| AppError::ManifestMissingPlatform(plat.to_string()))?
            .clone();
        validate::ensure_file_name(&entry.binary, "manifest binary")?;

        let staging = installer::staging_dir()?;
        tokio::fs::create_dir_all(&staging).await?;
        let staged_name = format!("claude-{}-{}", version, plat);
        let dest = staging.join(validate::ensure_file_name(&staged_name, "staging file")?);

        // Collected up front: a borrowed iterator held across `.await` makes
        // the install future non-`Send` (axum / tauri handlers need Send).
        let candidates: Vec<(String, String)> = mirrors
            .mirrors
            .iter()
            .map(|m| (m.name().to_string(), m.binary_url(&version, plat, &entry.binary)))
            .collect();
        downloader::download_verified(
            &client,
            &progress,
            Self::ID,
            candidates,
            &entry.checksum,
            Some(entry.size),
            &dest,
        )
        .await?;

        installer::make_executable(&dest).await?;
        let install_target = if channel == "latest" || channel == "stable" {
            Some(channel.as_str())
        } else {
            None
        };
        // Try the official self-install first; if it fails (typically because
        // claude.exe's own version precheck against downloads.claude.ai gets
        // ECONNREFUSED — see installer::run_self_install docstring), fall back
        // to a direct copy. The binary we just verified IS the final executable.
        match installer::run_self_install(&dest, install_target).await {
            Ok(out) => {
                tracing::debug!("self-install stdout:\n{}", out);
            }
            Err(e) => {
                tracing::warn!(
                    "claude self-install failed ({}); falling back to direct deploy. \
                     Expected when downloads.claude.ai is unreachable.",
                    e
                );
                let launcher_dir = self
                    .launcher_dir()
                    .ok_or_else(|| AppError::Other("no home dir".into()))?;
                let deployed =
                    installer::deploy_binary_to_launcher(&dest, &launcher_dir, Self::bin_name())
                        .await?;
                tracing::info!("deployed bootstrap to {}", deployed.display());
            }
        }

        let _ = tokio::fs::remove_file(&dest).await;

        let install_path = self
            .launcher_path()
            .and_then(|p| p.to_str().map(String::from))
            .unwrap_or_default();

        let auto_applied_fixes = auto_apply_install_fixes();

        Ok(InstallReport {
            tool_id: Self::ID.to_string(),
            version,
            install_path,
            elapsed_secs: started.elapsed().as_secs(),
            method: InstallMethod::Native,
            auto_applied_fixes,
        })
    }

    async fn install_npm(
        &self,
        client: reqwest::Client,
        mirrors: MirrorList,
        version: String,
        started: Instant,
    ) -> Result<InstallReport> {
        let info = npm_installer::detect_node().await?;
        let min = self.npm_min_node();
        if info.node_major < min {
            return Err(AppError::Other(format!(
                "Claude Code 通过 npm 安装需要 Node.js {}+，当前是 {}。请升级 Node。",
                min, info.node_version
            )));
        }

        let plat = platform::current()?;
        validate::ensure_version(&version, "Claude Code 版本号")?;
        tracing::info!("npm route version {}", version);

        // Try mirror tarballs first; fallback to npmmirror on any error.
        match npm_installer::install_via_mirror_tarballs(
            &client,
            &self.mirror_list(),
            &mirrors,
            Self::NPM_PACKAGE,
            &version,
            plat,
        )
        .await
        {
            Ok(()) => tracing::info!("Claude Code installed via mirror tarballs"),
            Err(e) => {
                tracing::warn!(
                    "mirror tarball install failed ({}), falling back to npmmirror",
                    e
                );
                // Pin the version we resolved — a bare package name would
                // silently install whatever npmmirror calls `latest`.
                let spec = format!("{}@{}", Self::NPM_PACKAGE, version);
                npm_installer::install_global(&spec, None).await?;
            }
        }

        let installed_version = self
            .detect_installed()
            .await
            .unwrap_or_else(|| version.clone());
        let install_path = npm_installer::npm_global_bin().await.unwrap_or_default();

        let auto_applied_fixes = auto_apply_install_fixes();

        Ok(InstallReport {
            tool_id: Self::ID.to_string(),
            version: installed_version,
            install_path,
            elapsed_secs: started.elapsed().as_secs(),
            method: InstallMethod::Npm,
            auto_applied_fixes,
        })
    }
}

/// Apply the small set of "open the box" fixes right after install, using
/// only the build-time embedded definitions (a remote `fixes.json` arrives
/// through third-party proxies and must never be applied unattended).
/// Failures are logged but never escalated — the install itself succeeded,
/// so the user shouldn't see an error just because we couldn't touch the
/// settings file.
fn auto_apply_install_fixes() -> Vec<String> {
    let ids: Vec<String> = AUTO_APPLY_ON_INSTALL.iter().map(|s| s.to_string()).collect();
    match fixes::apply_builtin_selected(&ids) {
        Ok(report) if report.applied_count > 0 => {
            tracing::info!(
                "auto-applied {} fix(es) post-install: {:?}",
                report.applied_count,
                ids
            );
            ids
        }
        Ok(_) => {
            tracing::info!("auto-apply: no matching fix definitions found");
            Vec::new()
        }
        Err(e) => {
            tracing::warn!("auto-apply install fixes failed: {} (install still succeeded)", e);
            Vec::new()
        }
    }
}

impl Tool for ClaudeCode {
    fn id(&self) -> ToolId {
        Self::ID
    }

    /// Same dir on all three platforms — Claude Code uses Unix-style layout
    /// even on Windows (`%USERPROFILE%\.local\bin\claude.exe`).
    fn launcher_dir(&self) -> Option<PathBuf> {
        Some(dirs::home_dir()?.join(".local").join("bin"))
    }

    fn npm_package(&self) -> Option<&'static str> {
        Some(Self::NPM_PACKAGE)
    }

    fn npm_min_node(&self) -> u32 {
        18
    }

    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: Self::ID.to_string(),
            name: "Claude Code".to_string(),
            description: "Anthropic 官方命令行工具".to_string(),
            installed_version: None,
            latest_version: None,
            stable_version: None,
            stable_falls_back_to_latest: false,
            latest_version_stale: false,
            stable_version_stale: false,
            installations: Vec::new(),
            install_path: self
                .launcher_path()
                .and_then(|p| p.to_str().map(String::from)),
            supports_npm: true,
            npm_package: Some(Self::NPM_PACKAGE.to_string()),
            npm_min_node: Some(self.npm_min_node()),
        }
    }

    async fn detect_installed(&self) -> Option<String> {
        // 1. Try our managed launcher path first
        if let Some(p) = self.launcher_path() {
            if p.exists() {
                if let Some(v) = run_version(&p).await {
                    return Some(v);
                }
            }
        }
        // 2. Resolve via `where`/`command -v` and run `--version` (handles
        // .cmd shims on Windows that bare Command::new("claude") misses).
        let resolved = crate::proc::resolve_command_path("claude").await?;
        run_version(&resolved).await
    }

    async fn install_version(
        &self,
        method: InstallMethod,
        progress: ProgressCallback,
        client: reqwest::Client,
        mirrors: MirrorList,
        channel: String,
        version: String,
    ) -> Result<InstallReport> {
        let started = Instant::now();
        match method {
            InstallMethod::Native => {
                self.install_native(progress, client, mirrors, channel, version, started)
                    .await
            }
            InstallMethod::Npm => self.install_npm(client, mirrors, version, started).await,
        }
    }
}

async fn run_version(path: &std::path::Path) -> Option<String> {
    let s = crate::proc::run_executable(path, &["--version"]).await?;
    // Format examples: "2.1.132 (Claude Code)"
    s.split_whitespace().next().map(String::from)
}
