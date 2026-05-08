//! Process-spawning helpers that work consistently on Windows.
//!
//! Rust's `Command::new("npm")` is supposed to fall back to `.cmd`/`.bat` when
//! resolving an executable through `PATH`, but in practice (Tauri-bundled apps
//! launched from Explorer, nvm-shimmed `npm`) it often fails. We always go
//! through `cmd.exe /c` on Windows to make node-ecosystem shims (`npm.cmd`,
//! `pnpm.cmd`, `codex.cmd`, ...) reliably runnable.

use std::path::{Path, PathBuf};
use tokio::process::Command;

/// Build a `Command` that spawns `program`. On Windows, wraps in `cmd /c` so
/// that `.cmd`/`.bat` shims resolve correctly even when launched from a
/// non-shell parent process.
pub fn shell_command(program: &str) -> Command {
    if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/c").arg(program);
        c
    } else {
        Command::new(program)
    }
}

/// Run an executable on disk and return stdout. On Windows, if the path looks
/// like a shim (`.cmd`/`.bat`/`.ps1`), routes through `cmd /c`.
pub async fn run_executable(path: &Path, args: &[&str]) -> Option<String> {
    let path_str = path.to_string_lossy();
    let lower = path_str.to_ascii_lowercase();
    let needs_cmd = cfg!(windows)
        && (lower.ends_with(".cmd") || lower.ends_with(".bat") || lower.ends_with(".ps1"));

    let output = if needs_cmd {
        Command::new("cmd")
            .arg("/c")
            .arg(path)
            .args(args)
            .output()
            .await
            .ok()?
    } else {
        Command::new(path).args(args).output().await.ok()?
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
        Command::new("where")
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
