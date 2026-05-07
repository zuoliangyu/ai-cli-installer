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

pub struct CodexCli;

impl CodexCli {
    pub const ID: ToolId = "codex-cli";

    fn launcher_path(&self) -> Option<PathBuf> {
        let bin_name = if cfg!(target_os = "windows") {
            "codex.exe"
        } else {
            "codex"
        };
        Some(self.launcher_dir()?.join(bin_name))
    }
}

impl Tool for CodexCli {
    fn id(&self) -> ToolId {
        Self::ID
    }

    fn launcher_dir(&self) -> Option<PathBuf> {
        // Same convention as Codex's official install.sh: ~/.local/bin
        Some(dirs::home_dir()?.join(".local").join("bin"))
    }

    fn mirror_list(&self) -> MirrorList {
        // Codex doesn't have a stable upstream URL we can mirror format —
        // we go GitHub-Releases-only via codex-mirror.
        MirrorList::builtin_for("codex-mirror", false)
    }

    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: Self::ID.to_string(),
            name: "Codex".to_string(),
            description: "OpenAI 官方命令行编码代理".to_string(),
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
        // codex --version output: "codex-cli 0.128.0" or similar
        s.split_whitespace()
            .find(|w| w.chars().next().map_or(false, |c| c.is_ascii_digit()))
            .map(String::from)
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

        // 1. version
        let (_chosen, version) = mirrors::fetch_version(&client, &mirrors, &channel).await?;
        tracing::info!("codex resolved {} -> {}", channel, version);

        // 2. manifest
        let (_, manifest) = mirrors::fetch_manifest(&client, &mirrors, &version).await?;
        let entry = manifest
            .platforms
            .get(plat)
            .ok_or_else(|| AppError::ManifestMissingPlatform(plat.to_string()))?
            .clone();

        // 3. download .zst
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

        // 4. checksum (against the .zst, which is what the manifest hashes)
        verifier::verify(&zst_dest, &entry.checksum).await?;

        // 5. zstd decompress → final binary
        let runtime_name = entry.runtime_filename();
        let dest_dir = self
            .launcher_dir()
            .ok_or_else(|| AppError::Other("no home dir".into()))?;
        tokio::fs::create_dir_all(&dest_dir).await?;
        let final_path = dest_dir.join(runtime_name);

        decompress_zst(&zst_dest, &final_path)?;

        // 6. chmod +x (Unix) — Codex's binary is the standalone executable
        installer::make_executable(&final_path).await?;

        // 7. cleanup the .zst
        let _ = tokio::fs::remove_file(&zst_dest).await;

        let install_path = final_path.to_string_lossy().to_string();
        Ok(InstallReport {
            tool_id: Self::ID.to_string(),
            version,
            install_path,
            elapsed_secs: started.elapsed().as_secs(),
        })
    }
}

fn decompress_zst(src: &std::path::Path, dst: &std::path::Path) -> Result<()> {
    use std::fs::File;
    use std::io;

    let input = File::open(src)
        .map_err(|e| AppError::Other(format!("open {}: {}", src.display(), e)))?;
    let mut decoder = zstd::stream::Decoder::new(input)
        .map_err(|e| AppError::Other(format!("zstd init: {}", e)))?;

    let mut output = File::create(dst)
        .map_err(|e| AppError::Other(format!("create {}: {}", dst.display(), e)))?;

    io::copy(&mut decoder, &mut output)
        .map_err(|e| AppError::Other(format!("zstd copy: {}", e)))?;
    Ok(())
}
