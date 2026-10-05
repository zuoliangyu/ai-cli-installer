use crate::error::{AppError, Result};

pub fn current() -> Result<&'static str> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        return Ok("win32-x64");
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        return Ok("win32-arm64");
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        return Ok("darwin-x64");
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return Ok("darwin-arm64");
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return Ok(if linux::is_musl() { "linux-x64-musl" } else { "linux-x64" });
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        return Ok(if linux::is_musl() { "linux-arm64-musl" } else { "linux-arm64" });
    }
    #[allow(unreachable_code)]
    Err(AppError::UnsupportedPlatform(format!(
        "{}-{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    )))
}

#[cfg(target_os = "linux")]
mod linux {
    use std::path::Path;

    /// musl systems ship their dynamic loader as `/lib/ld-musl-<arch>.so.1`
    /// (Alpine, Void-musl, ...); glibc systems never have it. The
    /// `libc.musl-*` names are Alpine's compat symlinks, kept as a fallback.
    pub fn is_musl() -> bool {
        let has_loader = std::fs::read_dir("/lib")
            .map(|entries| {
                entries
                    .flatten()
                    .any(|e| e.file_name().to_string_lossy().starts_with("ld-musl-"))
            })
            .unwrap_or(false);
        has_loader
            || Path::new("/lib/libc.musl-x86_64.so.1").exists()
            || Path::new("/lib/libc.musl-aarch64.so.1").exists()
    }
}

/// Markers wrapped around the login shell's `$PATH`, so whatever its rc
/// files print (banners, `nvm` notices, fortune) can be discarded.
#[cfg(any(unix, test))]
const PATH_BEGIN: &str = "__AI_CLI_INSTALLER_PATH_BEGIN__";
#[cfg(any(unix, test))]
const PATH_END: &str = "__AI_CLI_INSTALLER_PATH_END__";

/// Make the process `PATH` match what the user's login shell sees. Runs at
/// most once per process; a no-op on Windows.
///
/// Apps launched from Finder / Dock (and many Linux desktop launchers) inherit
/// launchd's minimal `PATH=/usr/bin:/bin:/usr/sbin:/sbin`, so Homebrew / nvm
/// installs of `node`, `npm`, `claude`, `codex` are invisible and the UI
/// reports "not installed" / "Node.js missing". We ask `$SHELL -l -i` for its
/// `$PATH` (bounded by a timeout so a hanging rc file can't stall startup)
/// and append every directory we don't already have; `/opt/homebrew/bin` and
/// `/usr/local/bin` are appended as a fallback when the shell gives nothing.
pub fn ensure_login_shell_path() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        #[cfg(unix)]
        login_path::apply();
    });
}

#[cfg(unix)]
mod login_path {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    const LOGIN_SHELL_TIMEOUT: Duration = Duration::from_secs(5);
    const FALLBACK_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin"];

    pub fn apply() {
        let current = std::env::var("PATH").unwrap_or_default();
        let mut extra: Vec<String> = Vec::new();
        match login_shell_path() {
            Some(p) => extra.extend(p.split(':').map(str::to_string)),
            None => tracing::info!("login shell PATH unavailable; using fallback dirs only"),
        }
        extra.extend(
            FALLBACK_DIRS
                .iter()
                .filter(|d| std::path::Path::new(d).is_dir())
                .map(|d| d.to_string()),
        );
        let refs: Vec<&str> = extra.iter().map(String::as_str).collect();
        if let Some(merged) = super::merge_path_lists(&current, &refs) {
            tracing::info!("PATH extended from login shell: {}", merged);
            // Runs once, from `AppState::new`, before any of our own worker
            // tasks spawn children or read PATH.
            std::env::set_var("PATH", merged);
        }
    }

    fn login_shell_path() -> Option<String> {
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| {
                if cfg!(target_os = "macos") {
                    "/bin/zsh".to_string()
                } else {
                    "/bin/sh".to_string()
                }
            });
        let script = format!(
            "printf '%s%s%s' '{}' \"$PATH\" '{}'",
            super::PATH_BEGIN,
            super::PATH_END
        );
        let mut child = Command::new(&shell)
            .args(["-l", "-i", "-c", &script])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| tracing::warn!("spawn login shell {}: {}", shell, e))
            .ok()?;

        // Read on a helper thread: an rc file that backgrounds a process can
        // keep the pipe open after the shell exits, so never block on EOF.
        let mut stdout = child.stdout.take()?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stdout.read_to_end(&mut buf);
            let _ = tx.send(buf);
        });

        let deadline = Instant::now() + LOGIN_SHELL_TIMEOUT;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(25))
                }
                _ => {
                    tracing::warn!(
                        "login shell {} did not finish within {}s; killed",
                        shell,
                        LOGIN_SHELL_TIMEOUT.as_secs()
                    );
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        let buf = rx
            .recv_timeout(remaining.max(Duration::from_millis(200)))
            .ok()?;
        super::extract_marked_path(&String::from_utf8_lossy(&buf)).map(str::to_string)
    }
}

/// The text between [`PATH_BEGIN`] and [`PATH_END`] (last occurrence wins,
/// in case an rc file echoes the command line). `None` when absent / empty.
#[cfg(any(unix, test))]
fn extract_marked_path(output: &str) -> Option<&str> {
    let start = output.rfind(PATH_BEGIN)? + PATH_BEGIN.len();
    let len = output[start..].find(PATH_END)?;
    let path = output[start..start + len].trim();
    (!path.is_empty()).then_some(path)
}

/// Append every entry of `extra` that's not already in the `:`-separated
/// `current`, keeping order. `None` when nothing would change.
#[cfg(any(unix, test))]
fn merge_path_lists(current: &str, extra: &[&str]) -> Option<String> {
    let mut entries: Vec<&str> = current.split(':').filter(|s| !s.is_empty()).collect();
    let before = entries.len();
    for dir in extra {
        let dir = dir.trim();
        // Only absolute dirs: a relative entry from an rc file would make
        // lookups depend on our working directory.
        if dir.starts_with('/') && !entries.contains(&dir) {
            entries.push(dir);
        }
    }
    (entries.len() != before).then(|| entries.join(":"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_path_between_markers_ignoring_rc_noise() {
        let out = format!(
            "Welcome!\nnvm: using node v22\n{}/opt/homebrew/bin:/usr/bin{}",
            PATH_BEGIN, PATH_END
        );
        assert_eq!(extract_marked_path(&out), Some("/opt/homebrew/bin:/usr/bin"));
        assert_eq!(extract_marked_path("no markers"), None);
        assert_eq!(extract_marked_path(&format!("{}{}", PATH_BEGIN, PATH_END)), None);
    }

    #[test]
    fn merges_only_new_absolute_dirs() {
        let merged = merge_path_lists(
            "/usr/bin:/bin",
            &["/opt/homebrew/bin", "/usr/bin", "relative/bin", "/Users/a/.nvm/bin"],
        );
        assert_eq!(
            merged.as_deref(),
            Some("/usr/bin:/bin:/opt/homebrew/bin:/Users/a/.nvm/bin")
        );
        assert_eq!(merge_path_lists("/usr/bin:/bin", &["/bin"]), None);
    }
}
