use std::io::Write;
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
///
/// Bounded by `proc::INSTALL_TIMEOUT`: on a black-holed network the precheck
/// can hang instead of failing fast, and the caller holds the global tool lock.
pub async fn run_self_install(binary: &Path, target: Option<&str>) -> Result<String> {
    let mut cmd = Command::new(binary);
    cmd.arg("install");
    if let Some(t) = target {
        cmd.arg(t);
    }
    cmd.arg("--force");
    crate::proc::silence_windows(&mut cmd);
    let output = crate::proc::output_with_timeout(&mut cmd, crate::proc::INSTALL_TIMEOUT)
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
/// and chmod. Equivalent in spirit to what `codex.rs::install_native` does
/// for its decompressed binary. Goes through [`install_executable`], so a
/// running `claude` is replaced safely.
pub async fn deploy_binary_to_launcher(
    binary: &Path,
    launcher_dir: &Path,
    bin_name: &str,
) -> Result<PathBuf> {
    tokio::fs::create_dir_all(launcher_dir)
        .await
        .map_err(|e| AppError::Install(format!("create launcher dir: {}", e)))?;
    let final_path = launcher_dir.join(bin_name);
    let src = binary.to_path_buf();
    install_executable(final_path.clone(), move |out| {
        let mut input = std::fs::File::open(&src)?;
        std::io::copy(&mut input, out)?;
        Ok(())
    })
    .await
    .map_err(|e| AppError::Install(format!("copy bootstrap to launcher: {}", e)))?;
    Ok(final_path)
}

/// Produce the executable at `dest` via `write` without ever truncating the
/// file in place, on a blocking thread.
///
/// Writing straight into an existing binary fails while it runs (Windows
/// `os error 32`, Linux `ETXTBSY`), leaves a half-written file when
/// interrupted, and on macOS mutating a signed binary in place can get the
/// next launch `Killed: 9`. Instead we write a sibling temp file, fsync,
/// chmod, then rename it over `dest`. On Windows a running `dest` can't be
/// replaced but *can* be renamed, so it's moved aside to `<name>.old` first;
/// leftovers are cleaned on the next install.
pub async fn install_executable<F>(dest: PathBuf, write: F) -> Result<()>
where
    F: FnOnce(&mut std::fs::File) -> std::io::Result<()> + Send + 'static,
{
    tokio::task::spawn_blocking(move || replace_executable(&dest, write))
        .await
        .map_err(|e| AppError::Install(format!("install task failed: {}", e)))?
        .map_err(AppError::Io)
}

fn sibling(dest: &Path, prefix: &str, suffix: &str) -> PathBuf {
    let mut name = std::ffi::OsString::from(prefix);
    name.push(dest.file_name().unwrap_or_default());
    name.push(suffix);
    dest.with_file_name(name)
}

fn replace_executable<F>(dest: &Path, write: F) -> std::io::Result<()>
where
    F: FnOnce(&mut std::fs::File) -> std::io::Result<()>,
{
    if dest.file_name().is_none() {
        return Err(std::io::Error::other(format!(
            "invalid install path: {}",
            dest.display()
        )));
    }
    cleanup_stale_binaries(dest);
    let tmp = sibling(dest, ".", &format!(".new-{}", std::process::id()));
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        write(&mut f)?;
        f.flush()?;
        f.sync_all()?;
        drop(f);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        }
        swap_into_place(&tmp, dest)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn swap_into_place(tmp: &Path, dest: &Path) -> std::io::Result<()> {
    match std::fs::rename(tmp, dest) {
        Ok(()) => Ok(()),
        #[cfg(windows)]
        Err(first) if dest.exists() => {
            // Most likely the old exe is running. Move it aside, then retry.
            let old = free_old_path(dest);
            std::fs::rename(dest, &old).map_err(|e| {
                std::io::Error::new(
                    e.kind(),
                    format!(
                        "{} 正在被占用，无法替换（{}；改名旧文件也失败：{}）。请关闭正在运行的程序后重试。",
                        dest.display(),
                        first,
                        e
                    ),
                )
            })?;
            if let Err(e) = std::fs::rename(tmp, dest) {
                // Put the old binary back so the user isn't left with nothing.
                let _ = std::fs::rename(&old, dest);
                return Err(e);
            }
            tracing::info!(
                "{} was in use; moved it aside to {}",
                dest.display(),
                old.display()
            );
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// First `<name>.old`, `<name>.old.1`, ... that doesn't exist (after trying
/// to delete it — an old copy that's still running can't be deleted).
#[cfg(windows)]
fn free_old_path(dest: &Path) -> PathBuf {
    for i in 0..32 {
        let suffix = if i == 0 {
            ".old".to_string()
        } else {
            format!(".old.{}", i)
        };
        let candidate = sibling(dest, "", &suffix);
        if !candidate.exists() || std::fs::remove_file(&candidate).is_ok() {
            return candidate;
        }
    }
    sibling(dest, "", &format!(".old.{}", std::process::id()))
}

/// Best-effort removal of `<name>.old*` (moved-aside binaries from a
/// previous install) and orphaned `.<name>.new-*` temp files.
pub fn cleanup_stale_binaries(dest: &Path) {
    let (Some(dir), Some(name)) = (dest.parent(), dest.file_name().and_then(|n| n.to_str()))
    else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let old_prefix = format!("{}.old", name);
    let tmp_prefix = format!(".{}.new-", name);
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(n) = file_name.to_str() else {
            continue;
        };
        if is_stale_name(n, &old_prefix, &tmp_prefix) {
            // Fails harmlessly while a moved-aside exe is still running.
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn is_stale_name(n: &str, old_prefix: &str, tmp_prefix: &str) -> bool {
    if let Some(rest) = n.strip_prefix(old_prefix) {
        return rest.is_empty()
            || rest
                .strip_prefix('.')
                .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()));
    }
    n.starts_with(tmp_prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_name_matching() {
        let old = "codex.exe.old";
        let tmp = ".codex.exe.new-";
        assert!(is_stale_name("codex.exe.old", old, tmp));
        assert!(is_stale_name("codex.exe.old.3", old, tmp));
        assert!(is_stale_name(".codex.exe.new-1234", old, tmp));
        assert!(!is_stale_name("codex.exe", old, tmp));
        assert!(!is_stale_name("codex.exe.older", old, tmp));
        assert!(!is_stale_name("codex.exe.old.x", old, tmp));
    }

    #[test]
    fn replaces_existing_file_atomically() {
        let dir =
            std::env::temp_dir().join(format!("ai-cli-installer-exe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("tool.bin");
        std::fs::write(&dest, b"old").unwrap();
        std::fs::write(dir.join("tool.bin.old"), b"stale").unwrap();

        replace_executable(&dest, |f| f.write_all(b"new")).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new");
        assert!(!dir.join("tool.bin.old").exists());

        // A failing writer leaves the previous binary untouched and no temp.
        let err = replace_executable(&dest, |_| Err(std::io::Error::other("boom")));
        assert!(err.is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), b"new");
        assert_eq!(std::fs::read_dir(&dir).unwrap().flatten().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn replaces_running_executable_on_windows() {
        // Copy a real exe (cmd.exe), launch it, then replace it while it runs.
        let dir =
            std::env::temp_dir().join(format!("ai-cli-installer-busy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("busy.exe");
        let system_root = std::env::var_os("SystemRoot").expect("SystemRoot");
        let cmd_exe = PathBuf::from(system_root).join("System32").join("cmd.exe");
        std::fs::copy(&cmd_exe, &dest).unwrap();
        let mut child = std::process::Command::new(&dest)
            .args(["/c", "ping", "-n", "5", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();

        // Plain truncating write fails while the exe runs...
        assert!(std::fs::write(&dest, b"x").is_err());
        // ...but the rename-aside path succeeds.
        replace_executable(&dest, |f| f.write_all(b"new")).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new");
        assert!(dir.join("busy.exe.old").exists());

        let _ = child.kill();
        let _ = child.wait();
        cleanup_stale_binaries(&dest);
        assert!(!dir.join("busy.exe.old").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
