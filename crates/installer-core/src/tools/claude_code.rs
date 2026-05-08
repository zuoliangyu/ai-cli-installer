use std::path::PathBuf;
use std::time::Instant;

use crate::downloader;
use crate::error::{AppError, Result};
use crate::installer;
use crate::mirrors::{self, MirrorList};
use crate::npm_installer;
use crate::platform;
use crate::progress::ProgressCallback;
use crate::tools::{InstallMethod, InstallReport, Tool, ToolDescriptor, ToolId};
use crate::verifier;

pub struct ClaudeCode;

impl ClaudeCode {
    pub const ID: ToolId = "claude-code";
    pub const NPM_PACKAGE: &'static str = "@anthropic-ai/claude-code";

    fn launcher_path(&self) -> Option<PathBuf> {
        let bin_name = if cfg!(target_os = "windows") {
            "claude.exe"
        } else {
            "claude"
        };
        Some(self.launcher_dir()?.join(bin_name))
    }

    async fn install_native(
        &self,
        progress: ProgressCallback,
        client: reqwest::Client,
        mirrors: MirrorList,
        channel: String,
        started: Instant,
    ) -> Result<InstallReport> {
        let plat = platform::current()?;

        let (_, version) = mirrors::fetch_version(&client, &mirrors, &channel).await?;
        tracing::info!("resolved {} -> {}", channel, version);

        let (_, manifest) = mirrors::fetch_manifest(&client, &mirrors, &version).await?;
        let entry = manifest
            .platforms
            .get(plat)
            .ok_or_else(|| AppError::ManifestMissingPlatform(plat.to_string()))?
            .clone();

        let staging = installer::staging_dir()?;
        tokio::fs::create_dir_all(&staging).await?;
        let dest = staging.join(format!("claude-{}-{}", version, plat));

        let mut last_err: Option<AppError> = None;
        let mut downloaded_bytes: u64 = 0;
        for m in &mirrors.mirrors {
            let url = m.binary_url(&version, plat, &entry.binary);
            tracing::info!("download attempt: {}", url);
            match downloader::download_to_file(&client, &progress, Self::ID, m.name(), &url, &dest)
                .await
            {
                Ok(bytes) => {
                    downloaded_bytes = bytes;
                    last_err = None;
                    break;
                }
                Err(e) => {
                    tracing::warn!("mirror {} failed: {}", m.name(), e);
                    last_err = Some(e);
                    let _ = tokio::fs::remove_file(&dest).await;
                }
            }
        }
        if let Some(e) = last_err {
            return Err(e);
        }
        if downloaded_bytes == 0 {
            return Err(AppError::AllMirrorsFailed);
        }

        verifier::verify(&dest, &entry.checksum).await?;

        installer::make_executable(&dest).await?;
        let install_target = if channel == "latest" || channel == "stable" {
            Some(channel.as_str())
        } else {
            None
        };
        let stdout = installer::run_self_install(&dest, install_target).await?;
        tracing::debug!("self-install stdout:\n{}", stdout);

        let _ = tokio::fs::remove_file(&dest).await;

        let install_path = self
            .launcher_path()
            .and_then(|p| p.to_str().map(String::from))
            .unwrap_or_default();

        Ok(InstallReport {
            tool_id: Self::ID.to_string(),
            version,
            install_path,
            elapsed_secs: started.elapsed().as_secs(),
            method: InstallMethod::Native,
        })
    }

    async fn install_npm(
        &self,
        client: reqwest::Client,
        mirrors: MirrorList,
        channel: String,
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

        // Resolve version through mirror chain (same logic as native install)
        let plat = platform::current()?;
        let (_, version) = mirrors::fetch_version(&client, &mirrors, &channel).await?;
        tracing::info!("npm route resolved version {}", version);

        // Try mirror tarballs first; fallback to npmmirror on any error.
        match npm_installer::install_via_mirror_tarballs(&client, &mirrors, &version, plat).await {
            Ok(()) => tracing::info!("Claude Code installed via mirror tarballs"),
            Err(e) => {
                tracing::warn!(
                    "mirror tarball install failed ({}), falling back to npmmirror",
                    e
                );
                npm_installer::install_global(Self::NPM_PACKAGE, None).await?;
            }
        }

        let installed_version = self
            .detect_installed()
            .await
            .unwrap_or_else(|| version.clone());
        let install_path = npm_installer::npm_global_bin().await.unwrap_or_default();

        Ok(InstallReport {
            tool_id: Self::ID.to_string(),
            version: installed_version,
            install_path,
            elapsed_secs: started.elapsed().as_secs(),
            method: InstallMethod::Npm,
        })
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

    async fn install(
        &self,
        method: InstallMethod,
        progress: ProgressCallback,
        client: reqwest::Client,
        mirrors: MirrorList,
        channel: String,
    ) -> Result<InstallReport> {
        let started = Instant::now();
        match method {
            InstallMethod::Native => {
                self.install_native(progress, client, mirrors, channel, started)
                    .await
            }
            InstallMethod::Npm => self.install_npm(client, mirrors, channel, started).await,
        }
    }
}

async fn run_version(path: &std::path::Path) -> Option<String> {
    let s = crate::proc::run_executable(path, &["--version"]).await?;
    // Format examples: "2.1.132 (Claude Code)"
    s.split_whitespace().next().map(String::from)
}
