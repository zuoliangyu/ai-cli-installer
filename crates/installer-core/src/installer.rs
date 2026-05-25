use std::path::{Path, PathBuf};
use tokio::process::Command;

use crate::error::{AppError, Result};

/// Where the bootstrap binary is staged before invoking its self-install.
pub fn staging_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| AppError::Other("no home dir".into()))?;
    Ok(home.join(".claude").join("downloads"))
}

/// Make a file executable (no-op on Windows).
#[cfg(unix)]
pub async fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = tokio::fs::metadata(path).await?.permissions();
    perms.set_mode(0o755);
    tokio::fs::set_permissions(path, perms).await?;
    Ok(())
}

#[cfg(not(unix))]
pub async fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

/// Run `<binary> install [target] --force` and capture output. Mirrors official install.sh behavior.
///
/// Historically `--force` was meant to bypass the bootstrap's own version check against
/// `downloads.claude.ai`. As of recent claude.exe builds it no longer does — the precheck
/// fires unconditionally and returns ECONNREFUSED on networks where the official endpoint
/// is unreachable (the error message paradoxically still tells you to "Try running with
/// --force to override checks"). See claude-code issues #13498 / #13981 / #51733.
///
/// We keep calling self-install as the preferred path so that it remains a no-op on
/// networks that work, but the caller MUST handle errors here by falling back to a direct
/// deploy via `deploy_binary_to_launcher` — the downloaded bootstrap IS the final
/// executable, so a plain copy is functionally equivalent for Claude Code.
pub async fn run_self_install(binary: &Path, target: Option<&str>) -> Result<String> {
    let mut cmd = Command::new(binary);
    cmd.arg("install");
    if let Some(t) = target {
        cmd.arg(t);
    }
    cmd.arg("--force");
    crate::proc::silence_windows(&mut cmd);
    let output = cmd
        .output()
        .await
        .map_err(|e| AppError::Install(format!("spawn failed: {}", e)))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        return Err(AppError::Install(format!(
            "exit {:?}\nstdout:\n{}\nstderr:\n{}",
            output.status.code(),
            stdout,
            stderr
        )));
    }
    Ok(stdout)
}

/// Fallback for when `claude install` self-install fails — e.g. the bootstrap
/// can't reach `downloads.claude.ai` for its (now non-skippable) version precheck.
///
/// For Claude Code the downloaded binary IS the final executable (see
/// `upstream::PlatformEntry::binary` docstring), so deploying is just a copy
/// + chmod. Equivalent in spirit to what `codex.rs::install_native` does for
/// its decompressed binary.
pub async fn deploy_binary_to_launcher(
    binary: &Path,
    launcher_dir: &Path,
    bin_name: &str,
) -> Result<PathBuf> {
    tokio::fs::create_dir_all(launcher_dir)
        .await
        .map_err(|e| AppError::Install(format!("create launcher dir: {}", e)))?;
    let final_path = launcher_dir.join(bin_name);
    tokio::fs::copy(binary, &final_path)
        .await
        .map_err(|e| AppError::Install(format!("copy bootstrap to launcher: {}", e)))?;
    make_executable(&final_path).await?;
    Ok(final_path)
}
