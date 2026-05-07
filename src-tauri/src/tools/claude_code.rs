use std::path::PathBuf;
use std::time::Instant;
use tauri::AppHandle;

use crate::downloader;
use crate::error::{AppError, Result};
use crate::installer;
use crate::mirrors::{self, MirrorList};
use crate::platform;
use crate::tools::{InstallReport, Tool, ToolDescriptor, ToolId};
use crate::verifier;

pub struct ClaudeCode;

impl ClaudeCode {
    pub const ID: ToolId = "claude-code";

    fn launcher_path(&self) -> Option<PathBuf> {
        let bin_name = if cfg!(target_os = "windows") {
            "claude.exe"
        } else {
            "claude"
        };
        Some(self.launcher_dir()?.join(bin_name))
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

    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: Self::ID.to_string(),
            name: "Claude Code".to_string(),
            description: "Anthropic 官方命令行工具".to_string(),
            installed_version: None,
            install_path: self.launcher_path().and_then(|p| p.to_str().map(String::from)),
        }
    }

    async fn detect_installed(&self) -> Option<String> {
        let p = self.launcher_path()?;
        if !p.exists() {
            return None;
        }
        let output = tokio::process::Command::new(&p)
            .arg("--version")
            .output()
            .await
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&output.stdout).to_string();
        // expected format: "2.1.132 (Claude Code)"
        s.split_whitespace().next().map(|s| s.to_string())
    }

    async fn install(
        &self,
        app: AppHandle,
        client: reqwest::Client,
        mirrors: MirrorList,
        channel: String,
    ) -> Result<InstallReport> {
        let started = Instant::now();
        let plat = platform::current()?;

        // 1. resolve version through mirror chain
        let (_chosen_mirror, version) = mirrors::fetch_version(&client, &mirrors, &channel).await?;
        tracing::info!("resolved {} -> {}", channel, version);

        // 2. fetch manifest (any working mirror)
        let (_, manifest) = mirrors::fetch_manifest(&client, &mirrors, &version).await?;
        let entry = manifest
            .platforms
            .get(plat)
            .ok_or_else(|| AppError::ManifestMissingPlatform(plat.to_string()))?
            .clone();

        // 3. download binary, falling through mirrors on failure
        let staging = installer::staging_dir()?;
        tokio::fs::create_dir_all(&staging).await?;
        let dest = staging.join(format!("claude-{}-{}", version, plat));

        let mut last_err: Option<AppError> = None;
        let mut downloaded_bytes: u64 = 0;
        for m in &mirrors.mirrors {
            let url = m.binary_url(&version, plat, &entry.binary);
            tracing::info!("download attempt: {}", url);
            match downloader::download_to_file(
                &client,
                &app,
                Self::ID,
                m.name(),
                &url,
                &dest,
            )
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

        // 4. checksum
        verifier::verify(&dest, &entry.checksum).await?;

        // 5. self-install
        installer::make_executable(&dest).await?;
        let install_target = if channel == "latest" || channel == "stable" {
            Some(channel.as_str())
        } else {
            // explicit version pin → don't pass anything special, binary will install itself
            None
        };
        let stdout = installer::run_self_install(&dest, install_target).await?;
        tracing::debug!("self-install stdout:\n{}", stdout);

        // 6. cleanup bootstrap
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
        })
    }
}
