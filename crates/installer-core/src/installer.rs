use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::process::Command;

use crate::error::{AppError, Result};

static NEXT_STAGE: AtomicU64 = AtomicU64::new(0);

/// One install's private files; clones keep them alive across blocking work.
#[derive(Clone)]
pub struct TempStage(Arc<StageDirectory>);

struct StageDirectory(PathBuf);

impl TempStage {
    pub fn new() -> Result<Self> {
        let home = dirs::home_dir().ok_or_else(|| AppError::Other("no home dir".into()))?;
        Self::in_dir(&home.join(".cache").join("ai-cli-installer").join("staging"))
            .map_err(AppError::Io)
    }

    fn in_dir(root: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(root)?;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for _ in 0..16 {
            let sequence = NEXT_STAGE.fetch_add(1, Ordering::Relaxed);
            let dir = root.join(format!("{}-{nonce}-{sequence}", std::process::id()));
            match std::fs::create_dir(&dir) {
                Ok(()) => return Ok(Self(Arc::new(StageDirectory(dir)))),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "cannot allocate a unique staging directory",
        ))
    }

    pub fn path(&self) -> &Path {
        &self.0 .0
    }
}

impl Drop for StageDirectory {
    fn drop(&mut self) {
        match std::fs::remove_dir_all(&self.0) {
            Ok(()) => return,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(_) => {}
        }
        // Cancelled Windows file I/O/process termination may release handles shortly later.
        // Retry only this uniquely-created directory, never scan or prune existing caches.
        let path = self.0.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("installer-stage-cleanup".into())
            .spawn(move || {
                for delay_ms in [20, 50, 100, 200, 400, 800] {
                    std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                    match std::fs::remove_dir_all(&path) {
                        Ok(()) => return,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
                        Err(error) if delay_ms == 800 => {
                            tracing::warn!("cleanup stage {}: {error}", path.display())
                        }
                        Err(_) => {}
                    }
                }
            })
        {
            tracing::warn!("start staging cleanup for {}: {error}", self.0.display());
        }
    }
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
    stage: TempStage,
) -> Result<PathBuf> {
    tokio::fs::create_dir_all(launcher_dir)
        .await
        .map_err(|e| AppError::Install(format!("create launcher dir: {}", e)))?;
    let final_path = launcher_dir.join(bin_name);
    let src = binary.to_path_buf();
    install_executable(final_path.clone(), move |out| {
        let _stage = stage;
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
            rename_retrying(dest, &old).map_err(|e| {
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
            if let Err(e) = rename_retrying(tmp, dest) {
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

/// Renaming a running exe is allowed, but antivirus scanners (Defender) and
/// the loader briefly open fresh executables without FILE_SHARE_DELETE, which
/// surfaces as a sharing violation (os error 32). Retry for up to ~1.5s.
#[cfg(windows)]
fn rename_retrying(from: &Path, to: &Path) -> std::io::Result<()> {
    const ERROR_SHARING_VIOLATION: i32 = 32;
    let mut delay = std::time::Duration::from_millis(50);
    for _ in 0..6 {
        match std::fs::rename(from, to) {
            Err(e) if e.raw_os_error() == Some(ERROR_SHARING_VIOLATION) => {
                std::thread::sleep(delay);
                delay *= 2;
            }
            other => return other,
        }
    }
    std::fs::rename(from, to)
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
    let (Some(dir), Some(name)) = (dest.parent(), dest.file_name().and_then(|n| n.to_str())) else {
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
    fn temp_stages_are_unique_and_cleanup_only_their_own_files() {
        let root = TempStage::in_dir(&std::env::temp_dir()).unwrap();
        let legacy = root.path().join("existing-cache.tgz");
        std::fs::write(&legacy, b"keep").unwrap();
        let first = TempStage::in_dir(root.path()).unwrap();
        let second = TempStage::in_dir(root.path()).unwrap();
        let first_path = first.path().to_path_buf();
        let second_path = second.path().to_path_buf();
        assert_ne!(first_path, second_path);
        std::fs::write(first.path().join("partial.zst.part"), b"partial").unwrap();
        std::fs::write(second.path().join("package.tgz"), b"package").unwrap();

        let failure = (|| -> std::io::Result<()> {
            let _stage = first;
            Err(std::io::Error::other("install failed"))
        })();
        assert!(failure.is_err());
        assert!(!first_path.exists());
        assert!(second_path.join("package.tgz").exists());
        assert_eq!(std::fs::read(&legacy).unwrap(), b"keep");
        drop(second);
        assert!(!second_path.exists());
    }

    #[tokio::test]
    async fn temp_stage_cleans_up_when_install_future_is_cancelled() {
        let root = TempStage::in_dir(&std::env::temp_dir()).unwrap();
        let stage = TempStage::in_dir(root.path()).unwrap();
        let path = stage.path().to_path_buf();
        std::fs::write(path.join("download.part"), b"partial").unwrap();
        let (ready, started) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _stage = stage;
            ready.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        started.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn temp_stage_survives_cancelled_wait_until_blocking_reader_finishes() {
        let root = TempStage::in_dir(&std::env::temp_dir()).unwrap();
        let stage = TempStage::in_dir(root.path()).unwrap();
        let path = stage.path().to_path_buf();
        std::fs::write(path.join("source"), b"verified bytes").unwrap();
        let (ready, started) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let (done, finished) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let blocking_stage = stage.clone();
            tokio::task::spawn_blocking(move || {
                ready.send(()).unwrap();
                wait.recv_timeout(std::time::Duration::from_secs(3))
                    .unwrap();
                let contents = std::fs::read(blocking_stage.path().join("source")).unwrap();
                drop(blocking_stage);
                let _ = done.send(contents);
            })
            .await
            .unwrap();
            drop(stage);
        });
        started.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(path.join("source").exists());
        release.send(()).unwrap();
        assert_eq!(finished.await.unwrap(), b"verified bytes");
        assert!(!path.exists());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn temp_stage_retries_cleanup_after_windows_handle_is_released() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = TempStage::in_dir(&std::env::temp_dir()).unwrap();
        let stage = TempStage::in_dir(root.path()).unwrap();
        let path = stage.path().to_path_buf();
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(0)
            .open(path.join("busy.exe"))
            .unwrap();
        drop(stage);
        assert!(path.exists());
        drop(file);
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while path.exists() {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("stage remained after its file handle was released");
    }

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
        let dir = std::env::temp_dir().join(format!("ai-cli-installer-exe-{}", std::process::id()));
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
