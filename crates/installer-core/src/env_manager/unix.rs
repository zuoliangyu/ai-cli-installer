//! Unix PATH manipulation via shell rc file marker blocks.
//!
//! v0.0.2 limitation: writes to user-level rc files only. System-wide /etc edits
//! need sudo, which is awkward to elevate from a Tauri GUI without a custom
//! askpass — deferred to v0.0.3.
//!
//! Marker block makes the edit reversible and idempotent:
//!
//!     # >>> ai-cli-installer (PATH) >>>
//!     export PATH="$HOME/.local/bin:$PATH"
//!     # <<< ai-cli-installer (PATH) <<<

use std::path::{Path, PathBuf};

use super::{PathScope, PathStatus};
use crate::error::{AppError, Result};

const MARKER_BEGIN: &str = "# >>> ai-cli-installer (PATH) >>>";
const MARKER_END: &str = "# <<< ai-cli-installer (PATH) <<<";

fn rc_files() -> Vec<PathBuf> {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return vec![],
    };
    // Edit all that exist; create the canonical .profile if none do.
    let candidates = [".zshrc", ".bashrc", ".profile"];
    let existing: Vec<PathBuf> = candidates
        .iter()
        .map(|n| home.join(n))
        .filter(|p| p.exists())
        .collect();
    if existing.is_empty() {
        vec![home.join(".profile")]
    } else {
        existing
    }
}

pub async fn status(dir: &Path) -> Result<PathStatus> {
    let dir_str = dir.to_string_lossy().to_string();

    // "in_user_path" = our marker block exists in any rc file
    let in_user_path = rc_files().iter().any(|p| {
        std::fs::read_to_string(p)
            .map(|s| s.contains(MARKER_BEGIN))
            .unwrap_or(false)
    });

    // We don't try to detect /etc edits for v0.0.2. Mark system_path = false
    // so UI can prompt for system add (deferred) without misleading "already in".
    let in_system_path = false;

    // Effective: in current process PATH
    let effective = in_user_path
        || std::env::var("PATH")
            .map(|p| p.split(':').any(|s| s.trim() == dir_str))
            .unwrap_or(false);

    Ok(PathStatus {
        dir: dir_str,
        in_user_path,
        in_system_path,
        effective,
    })
}

pub async fn add(dir: &Path, scope: PathScope) -> Result<()> {
    if matches!(scope, PathScope::System) {
        return Err(AppError::Other(
            "Linux/macOS 系统 PATH 写入需要 sudo，v0.0.2 暂未实现。请手动编辑 /etc/profile.d/。"
                .into(),
        ));
    }
    let dir_str = dir.to_string_lossy().to_string();
    let block = format!(
        "\n{}\nexport PATH=\"{}:$PATH\"\n{}\n",
        MARKER_BEGIN, dir_str, MARKER_END
    );
    for rc in rc_files() {
        let existing = std::fs::read_to_string(&rc).unwrap_or_default();
        if existing.contains(MARKER_BEGIN) {
            // Already present — no-op
            continue;
        }
        let mut new_content = existing;
        if !new_content.is_empty() && !new_content.ends_with('\n') {
            new_content.push('\n');
        }
        new_content.push_str(&block);
        std::fs::write(&rc, new_content)
            .map_err(|e| AppError::Other(format!("write {}: {}", rc.display(), e)))?;
    }
    Ok(())
}

pub async fn remove(dir: &Path, scope: PathScope) -> Result<()> {
    if matches!(scope, PathScope::System) {
        return Err(AppError::Other(
            "Linux/macOS 系统 PATH 移除需要 sudo，v0.0.2 暂未实现。".into(),
        ));
    }
    let _ = dir; // signature parity with windows
    for rc in rc_files() {
        let content = match std::fs::read_to_string(&rc) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if !content.contains(MARKER_BEGIN) {
            continue;
        }
        let new = strip_marker_block(&content);
        std::fs::write(&rc, new)
            .map_err(|e| AppError::Other(format!("write {}: {}", rc.display(), e)))?;
    }
    Ok(())
}

fn strip_marker_block(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut skipping = false;
    for line in s.lines() {
        if line.trim() == MARKER_BEGIN {
            skipping = true;
            continue;
        }
        if line.trim() == MARKER_END {
            skipping = false;
            continue;
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}
