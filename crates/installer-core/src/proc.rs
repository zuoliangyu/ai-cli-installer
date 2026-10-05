//! Process-spawning helpers that work consistently on Windows.
//!
//! Rust's `Command::new("npm")` is supposed to fall back to `.cmd`/`.bat` when
//! resolving an executable through `PATH`, but in practice (Tauri-bundled apps
//! launched from Explorer, nvm-shimmed `npm`) it often fails. We always go
//! through `cmd.exe /c` on Windows to make node-ecosystem shims (`npm.cmd`,
//! `pnpm.cmd`, `codex.cmd`, ...) reliably runnable.
//!
//! All Windows spawns set `CREATE_NO_WINDOW (0x08000000)` so console children
//! (`cmd.exe`, `where.exe`, `npm.cmd` shim, ...) don't flash a black box from a
//! GUI-subsystem Tauri host.
//!
//! Every child we wait on goes through [`output_with_timeout`]: a hung
//! `npm install` / `claude install` / rc-file-heavy `--version` used to block
//! the global tool-operation lock forever. Children are `kill_on_drop`, so a
//! timed-out (or cancelled) wait also terminates the process.

use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::time::Duration;
use tokio::process::Command;

/// Budget for quick probes: `--version`, `where`, `npm prefix -g`, ...
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// Budget for real work: `npm install -g`, `claude install`, UAC prompts.
pub const INSTALL_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// CreationFlags bit that suppresses the per-child console window.
/// <https://learn.microsoft.com/en-us/windows/win32/procthread/process-creation-flags>
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Apply Windows-only flags (suppress console window). No-op on Unix.
#[cfg(windows)]
fn silence(cmd: &mut Command) {
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn silence(_cmd: &mut Command) {}

/// Public re-export: hide the spawned console window on Windows. Use for
/// `tokio::process::Command` instances built outside this module.
pub fn silence_windows(cmd: &mut Command) {
    silence(cmd);
}

/// Same as [`silence_windows`] but for `std::process::Command`.
pub fn silence_windows_std(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

/// Allocate a `Command` that won't flash a console window on Windows.
fn silent_command(program: &str) -> Command {
    let mut c = Command::new(program);
    silence(&mut c);
    c
}

fn silent_command_path(program: &Path) -> Command {
    let mut c = Command::new(program);
    silence(&mut c);
    c
}

/// Build a `Command` that spawns `program`. On Windows, wraps in `cmd /c` so
/// that `.cmd`/`.bat` shims resolve correctly even when launched from a
/// non-shell parent process. The console window is suppressed.
pub fn shell_command(program: &str) -> Command {
    if cfg!(windows) {
        let mut c = silent_command("cmd");
        c.arg("/c").arg(program);
        c
    } else {
        Command::new(program)
    }
}

/// Run `cmd` to completion, capturing stdout/stderr, bounded by `timeout`.
///
/// - stdin is closed so a child waiting for input (npm prompts, a shell rc
///   doing `read`) fails fast instead of hanging;
/// - `kill_on_drop(true)` means the timeout (or the caller's future being
///   dropped) kills the child. On Windows the whole tree is also taken down
///   with `taskkill /T`, because killing the `cmd /c` wrapper alone would
///   leave `node.exe` running and still holding files open.
///
/// A timeout is reported as `io::ErrorKind::TimedOut` with a Chinese
/// message, so callers' existing `map_err(|e| format!(..., e))` stays useful.
pub async fn output_with_timeout(cmd: &mut Command, timeout: Duration) -> std::io::Result<Output> {
    cmd.kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = cmd.spawn()?;
    let pid = child.id();
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(res) => res,
        Err(_) => {
            kill_tree(pid).await;
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("命令执行超时（超过 {} 秒），已强制结束", timeout.as_secs()),
            ))
        }
    }
}

/// Best-effort kill of a timed-out child's process tree. The direct child is
/// already killed by `kill_on_drop`; this catches grandchildren on Windows.
async fn kill_tree(pid: Option<u32>) {
    #[cfg(windows)]
    if let Some(pid) = pid {
        let mut c = silent_command("taskkill");
        c.args(["/T", "/F", "/PID", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let _ = tokio::time::timeout(Duration::from_secs(5), c.status()).await;
    }
    #[cfg(not(windows))]
    let _ = pid;
}

/// Run an executable on disk and return stdout. On Windows, `.cmd`/`.bat`
/// shims are routed through `cmd /c` and `.ps1` shims through
/// `powershell -File` (going through `cmd /c` would hit the file association
/// and open the script in Notepad). No console window appears. Bounded by
/// [`PROBE_TIMEOUT`].
pub async fn run_executable(path: &Path, args: &[&str]) -> Option<String> {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    let mut cmd = if cfg!(windows) && (lower.ends_with(".cmd") || lower.ends_with(".bat")) {
        let mut c = silent_command("cmd");
        c.arg("/c").arg(path);
        c
    } else if cfg!(windows) && lower.ends_with(".ps1") {
        let mut c = silent_command("powershell");
        c.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(path);
        c
    } else {
        silent_command_path(path)
    };
    cmd.args(args);

    let output = output_with_timeout(&mut cmd, PROBE_TIMEOUT).await.ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Resolve a bare command name to an absolute path via `where` (Windows) or
/// `command -v` (Unix). Returns the first match.
pub async fn resolve_command_path(command_name: &str) -> Option<PathBuf> {
    let mut cmd = if cfg!(windows) {
        let mut c = silent_command("where");
        c.arg(command_name);
        c
    } else {
        // Pass the name as a positional parameter instead of splicing it
        // into the script, so it's never interpreted by the shell.
        let mut c = Command::new("sh");
        c.args(["-c", "command -v \"$1\"", "sh", command_name]);
        c
    };
    let output = output_with_timeout(&mut cmd, PROBE_TIMEOUT).await.ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(PathBuf::from)
}

/// Resolve `command_name` then run `<resolved> --version` (or any args). On
/// Windows this dodges Rust's flaky `.cmd` PATH-extension lookup.
pub async fn run_version_by_name(command_name: &str, args: &[&str]) -> Option<String> {
    let path = resolve_command_path(command_name).await?;
    run_executable(&path, args).await
}
