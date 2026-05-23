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
/// `--force` bypasses the bootstrap's own version check against `downloads.claude.ai`,
/// which is unreachable from networks where the official endpoint is blocked. The binary
/// we just downloaded from a mirror is already the right version, so the check is redundant.
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
