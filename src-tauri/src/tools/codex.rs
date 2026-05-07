use std::path::PathBuf;
use std::time::Instant;
use tauri::AppHandle;

use crate::downloader;
use crate::error::{AppError, Result};
use crate::installer;
use crate::mirrors::{self, MirrorList};
use crate::npm_installer;
use crate::platform;
use crate::tools::{InstallMethod, InstallReport, Tool, ToolDescriptor, ToolId};
use crate::verifier;

pub struct CodexCli;

impl CodexCli {
    pub const ID: ToolId = "codex-cli";
    pub const NPM_PACKAGE: &'static str = "@openai/codex";

    fn launcher_path(&self) -> Option<PathBuf> {
        let bin_name = if cfg!(target_os = "windows") {
            "codex.exe"
        } else {
            "codex"
        };
        Some(self.launcher_dir()?.join(bin_name))
    }

    async fn install_native(
        &self,
        app: AppHandle,
        client: reqwest::Client,
        mirrors: MirrorList,
        channel: String,
        started: Instant,
    ) -> Result<InstallReport> {
        let plat = platform::current()?;

        let (_, version) = mirrors::fetch_version(&client, &mirrors, &channel).await?;
        tracing::info!("codex resolved {} -> {}", channel, version);

        let (_, manifest) = mirrors::fetch_manifest(&client, &mirrors, &version).await?;
        let entry = manifest
            .platforms
            .get(plat)
            .ok_or_else(|| AppError::ManifestMissingPlatform(plat.to_string()))?
            .clone();

        let staging = installer::staging_dir()?;
        tokio::fs::create_dir_all(&staging).await?;
        let zst_dest = staging.join(format!("codex-{}-{}", version, plat));

        let mut last_err: Option<AppError> = None;
        let mut downloaded_bytes: u64 = 0;
        for m in &mirrors.mirrors {
            let url = m.binary_url(&version, plat, &entry.binary);
            tracing::info!("codex download attempt: {}", url);
            match downloader::download_to_file(&client, &app, Self::ID, m.name(), &url, &zst_dest)
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
                    let _ = tokio::fs::remove_file(&zst_dest).await;
                }
            }
        }
        if let Some(e) = last_err {
            return Err(e);
        }
        if downloaded_bytes == 0 {
            return Err(AppError::AllMirrorsFailed);
        }

        verifier::verify(&zst_dest, &entry.checksum).await?;

        let runtime_name = entry.runtime_filename();
        let dest_dir = self
            .launcher_dir()
            .ok_or_else(|| AppError::Other("no home dir".into()))?;
        tokio::fs::create_dir_all(&dest_dir).await?;
        let final_path = dest_dir.join(runtime_name);

        decompress_zst(&zst_dest, &final_path)?;
        installer::make_executable(&final_path).await?;
        let _ = tokio::fs::remove_file(&zst_dest).await;

        Ok(InstallReport {
            tool_id: Self::ID.to_string(),
            version,
            install_path: final_path.to_string_lossy().to_string(),
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
                "Codex 通过 npm 安装需要 Node.js {}+，当前是 {}。请升级 Node。",
                min, info.node_version
            )));
        }

        let plat = platform::current()?;
        let (_, version) = mirrors::fetch_version(&client, &mirrors, &channel).await?;
        tracing::info!("codex npm route resolved version {}", version);

        match npm_installer::install_via_mirror_tarballs(&client, &mirrors, &version, plat).await {
            Ok(()) => tracing::info!("Codex installed via mirror tarballs"),
            Err(e) => {
                tracing::warn!("codex mirror tarball install failed ({}), falling back to npmmirror", e);
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

impl Tool for CodexCli {
    fn id(&self) -> ToolId {
        Self::ID
    }

    fn launcher_dir(&self) -> Option<PathBuf> {
        Some(dirs::home_dir()?.join(".local").join("bin"))
    }

    fn mirror_list(&self) -> MirrorList {
        MirrorList::builtin_for("codex-mirror", false)
    }

    fn npm_package(&self) -> Option<&'static str> {
        Some(Self::NPM_PACKAGE)
    }

    fn npm_min_node(&self) -> u32 {
        16
    }

    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: Self::ID.to_string(),
            name: "Codex".to_string(),
            description: "OpenAI 官方命令行编码代理".to_string(),
            installed_version: None,
            install_path: self.launcher_path().and_then(|p| p.to_str().map(String::from)),
            supports_npm: true,
            npm_package: Some(Self::NPM_PACKAGE.to_string()),
            npm_min_node: Some(self.npm_min_node()),
        }
    }

    async fn detect_installed(&self) -> Option<String> {
        if let Some(p) = self.launcher_path() {
            if p.exists() {
                if let Some(v) = run_version(&p).await {
                    return Some(v);
                }
            }
        }
        run_version(std::path::Path::new("codex")).await
    }

    async fn install(
        &self,
        method: InstallMethod,
        app: AppHandle,
        client: reqwest::Client,
        mirrors: MirrorList,
        channel: String,
    ) -> Result<InstallReport> {
        let started = Instant::now();
        match method {
            InstallMethod::Native => {
                self.install_native(app, client, mirrors, channel, started).await
            }
            InstallMethod::Npm => self.install_npm(client, mirrors, channel, started).await,
        }
    }
}

async fn run_version(path: &std::path::Path) -> Option<String> {
    let output = tokio::process::Command::new(path)
        .arg("--version")
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout).to_string();
    // Format examples: "codex-cli 0.128.0" or "0.128.0"
    s.split_whitespace()
        .find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(String::from)
}

fn decompress_zst(src: &std::path::Path, dst: &std::path::Path) -> Result<()> {
    use std::fs::File;
    use std::io;

    let input =
        File::open(src).map_err(|e| AppError::Other(format!("open {}: {}", src.display(), e)))?;
    let mut decoder =
        zstd::stream::Decoder::new(input).map_err(|e| AppError::Other(format!("zstd init: {}", e)))?;
    let mut output = File::create(dst)
        .map_err(|e| AppError::Other(format!("create {}: {}", dst.display(), e)))?;
    io::copy(&mut decoder, &mut output)
        .map_err(|e| AppError::Other(format!("zstd copy: {}", e)))?;
    Ok(())
}
