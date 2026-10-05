//! Cross-platform PATH manager.
//!
//! User PATH is the default because it works without elevation on every platform.
//! Windows also supports explicit system-wide writes through elevated PowerShell;
//! Linux/macOS intentionally reject system scope because GUI sudo is unreliable.
//!
//! Read operations don't need elevation.

use std::path::Path;

use crate::error::Result;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;

#[cfg(any(unix, test))]
mod rc_block;
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
