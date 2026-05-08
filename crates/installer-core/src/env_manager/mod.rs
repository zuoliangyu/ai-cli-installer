//! Cross-platform PATH manager.
//!
//! All write operations target the **system-wide** PATH:
//! - Windows: `HKLM\System\CurrentControlSet\Control\Session Manager\Environment\Path`
//!   (requires UAC elevation; we spawn an elevated PowerShell child process for the
//!   single write, broadcast `WM_SETTINGCHANGE` so new processes pick it up).
//! - Linux/macOS: a marker block in `/etc/profile.d/ai-cli-installer.sh` (requires sudo;
//!   v0.0.2 falls back to writing user-level `~/.profile` because non-interactive sudo
//!   from a Tauri GUI is fragile — system-wide deferred to v0.0.3).
//!
//! Read operations don't need elevation.

use std::path::Path;

use crate::error::Result;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as imp;

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathScope {
    /// System-wide PATH. Requires admin/sudo.
    System,
    /// User-only PATH. No elevation needed.
    User,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PathStatus {
    pub dir: String,
    pub in_user_path: bool,
    pub in_system_path: bool,
    /// Effective: any of the above OR currently visible to running process.
    pub effective: bool,
}

pub async fn status(dir: &Path) -> Result<PathStatus> {
    imp::status(dir).await
}

/// Add `dir` to PATH at the chosen scope.
/// On Windows + System scope, this triggers a UAC prompt.
pub async fn add(dir: &Path, scope: PathScope) -> Result<()> {
    imp::add(dir, scope).await
}

pub async fn remove(dir: &Path, scope: PathScope) -> Result<()> {
    imp::remove(dir, scope).await
}
