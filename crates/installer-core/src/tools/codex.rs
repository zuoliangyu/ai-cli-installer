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
use crate::upstream::PlatformEntry;
use crate::validate;

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
        progress: ProgressCallback,
        client: reqwest::Client,
        mirrors: MirrorList,
        version: String,
        started: Instant,
    ) -> Result<InstallReport> {
        let plat = platform::current()?;
        validate::ensure_version(&version, "Codex 版本号")?;

        // Full list for the manifest so the trust policy can reach
        // github-direct even when the user pinned one proxy for downloads.
        let manifest_list = self.mirror_list();
        let (_, manifest) = mirrors::fetch_manifest(&client, &manifest_list, &version).await?;
        let (asset_plat, entry) = platform_entry(&manifest.platforms, plat)
            .ok_or_else(|| AppError::ManifestMissingPlatform(plat.to_string()))?;
        validate::ensure_file_name(&entry.binary, "manifest binary")?;
        let runtime_name =
            validate::ensure_file_name(entry.runtime_filename(), "manifest runtime_binary")?
                .to_string();

        let staging = installer::staging_dir()?;
        tokio::fs::create_dir_all(&staging).await?;
        let staged_name = format!("codex-{}-{}", version, plat);
        let zst_dest = staging.join(validate::ensure_file_name(&staged_name, "staging file")?);

        // Collected up front: a borrowed iterator held across `.await` makes
        // the install future non-`Send` (axum / tauri handlers need Send).
        let candidates: Vec<(String, String)> = mirrors
            .mirrors
            .iter()
            .map(|m| (m.name().to_string(), m.binary_url(&version, &asset_plat, &entry.binary)))
            .collect();
        downloader::download_verified(
            &client,
            &progress,
            Self::ID,
            candidates,
            &entry.checksum,
            Some(entry.size),
            &zst_dest,
        )
        .await?;

        let dest_dir = self
            .launcher_dir()
            .ok_or_else(|| AppError::Other("no home dir".into()))?;
        tokio::fs::create_dir_all(&dest_dir).await?;
        let final_path = dest_dir.join(&runtime_name);

        // Decompression is CPU-bound and synchronous: off the async runtime,
        // into a temp file that's renamed over the (possibly running) binary.
        let src = zst_dest.clone();
        installer::install_executable(final_path.clone(), move |out| {
            let input = std::fs::File::open(&src)?;
            let mut decoder = zstd::stream::Decoder::new(input)?;
            std::io::copy(&mut decoder, out)?;
            Ok(())
        })
        .await
        .map_err(|e| AppError::Other(format!("解压 Codex 失败：{}", e)))?;
        let _ = tokio::fs::remove_file(&zst_dest).await;

        Ok(InstallReport {
            tool_id: Self::ID.to_string(),
            version,
            install_path: final_path.to_string_lossy().to_string(),
            elapsed_secs: started.elapsed().as_secs(),
            method: InstallMethod::Native,
            auto_applied_fixes: Vec::new(),
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
                "Codex 通过 npm 安装需要 Node.js {}+，当前是 {}。请升级 Node。",
                min, info.node_version
            )));
        }

        let plat = platform::current()?;
        validate::ensure_version(&version, "Codex 版本号")?;
        tracing::info!("codex npm route version {}", version);

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
            Ok(()) => tracing::info!("Codex installed via mirror tarballs"),
            Err(e) => {
                tracing::warn!(
                    "codex mirror tarball install failed ({}), falling back to npmmirror",
                    e
                );
                let spec = format!("{}@{}", Self::NPM_PACKAGE, version);
                npm_installer::install_global(&spec, None).await?;
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
            auto_applied_fixes: Vec::new(),
        })
    }
}

/// Look up `plat` in the manifest. Codex's Linux builds are statically
/// linked against musl and published once as `linux-x64` / `linux-arm64`,
/// so a `*-musl` platform falls back to the plain key. Returns the key that
/// matched (it's part of the asset name) and the entry.
fn platform_entry(
    platforms: &std::collections::BTreeMap<String, PlatformEntry>,
    plat: &str,
) -> Option<(String, PlatformEntry)> {
    if let Some(e) = platforms.get(plat) {
        return Some((plat.to_string(), e.clone()));
    }
    let base = plat.strip_suffix("-musl")?;
    platforms.get(base).map(|e| (base.to_string(), e.clone()))
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
        if let Some(p) = self.launcher_path() {
            if p.exists() {
                if let Some(v) = run_version(&p).await {
                    return Some(v);
                }
            }
        }
        // 兜底：通过 `where`/`command -v` 解析 PATH 上的 codex（含 .cmd shim），
        // 再 cmd /c 跑 --version。这样 nvm-windows / npm-cmd 这类场景能命中。
        let resolved = crate::proc::resolve_command_path("codex").await?;
        run_version(&resolved).await
    }

    async fn install_version(
        &self,
        method: InstallMethod,
        progress: ProgressCallback,
        client: reqwest::Client,
        mirrors: MirrorList,
        _channel: String,
        version: String,
    ) -> Result<InstallReport> {
        let started = Instant::now();
        match method {
            InstallMethod::Native => {
                self.install_native(progress, client, mirrors, version, started)
                    .await
            }
            InstallMethod::Npm => self.install_npm(client, mirrors, version, started).await,
        }
    }
}

async fn run_version(path: &std::path::Path) -> Option<String> {
    let s = crate::proc::run_executable(path, &["--version"]).await?;
    // Format examples: "codex-cli 0.128.0" or "0.128.0"
    s.split_whitespace()
        .find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(binary: &str) -> PlatformEntry {
        PlatformEntry {
            binary: binary.into(),
            runtime_binary: None,
            checksum: "00".into(),
            size: 1,
        }
    }

    #[test]
    fn musl_falls_back_to_static_linux_build() {
        let mut platforms = std::collections::BTreeMap::new();
        platforms.insert("linux-x64".to_string(), entry("codex.zst"));
        let (key, _) = platform_entry(&platforms, "linux-x64-musl").unwrap();
        assert_eq!(key, "linux-x64");
        let (key, _) = platform_entry(&platforms, "linux-x64").unwrap();
        assert_eq!(key, "linux-x64");
        assert!(platform_entry(&platforms, "darwin-arm64").is_none());
    }
}
