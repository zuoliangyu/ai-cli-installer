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

use std::path::{Path, PathBuf};
use tokio::process::Command;

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

/// Run an executable on disk and return stdout. On Windows, if the path looks
/// like a shim (`.cmd`/`.bat`/`.ps1`), routes through `cmd /c`. No console
/// window appears.
pub async fn run_executable(path: &Path, args: &[&str]) -> Option<String> {
    let path_str = path.to_string_lossy();
    let lower = path_str.to_ascii_lowercase();
    let needs_cmd = cfg!(windows)
        && (lower.ends_with(".cmd") || lower.ends_with(".bat") || lower.ends_with(".ps1"));

    let output = if needs_cmd {
        let mut c = silent_command("cmd");
        c.arg("/c").arg(path).args(args).output().await.ok()?
    } else {
        silent_command_path(path).args(args).output().await.ok()?
    };

    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Resolve a bare command name to an absolute path via `where` (Windows) or
/// `command -v` (Unix). Returns the first match.
pub async fn resolve_command_path(command_name: &str) -> Option<PathBuf> {
    let output = if cfg!(windows) {
        silent_command("where")
            .arg(command_name)
            .output()
            .await
            .ok()?
    } else {
        Command::new("sh")
            .args(["-c", &format!("command -v {}", command_name)])
            .output()
            .await
            .ok()?
    };
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
