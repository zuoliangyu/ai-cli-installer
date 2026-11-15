//! Unix PATH manipulation via shell rc file marker blocks.
//!
//! v0.0.2 limitation: writes to user-level rc files only. System-wide /etc edits
//! need sudo, which is awkward to elevate from a Tauri GUI without a custom
//! askpass — deferred to v0.0.3.
//!
//! Marker block makes the edit reversible and idempotent:
//!
//! ```text
//! # >>> ai-cli-installer (PATH) >>>
//! export PATH="$HOME/.local/bin:$PATH"
//! # <<< ai-cli-installer (PATH) <<<
//! ```
//!
//! The string logic (which rc files, how to add / strip the block) lives in
//! `rc_block.rs` so it's unit-tested on every platform.

use std::path::{Path, PathBuf};

use super::rc_block::{self, Strip};
use super::{PathScope, PathStatus};
use crate::error::{AppError, Result};

fn rc_files(shell: Option<&str>) -> Result<Vec<PathBuf>> {
    let home = dirs::home_dir().ok_or_else(|| AppError::Other("no home dir".into()))?;
    let xdg_config_home = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    Ok(rc_block::rc_targets(
        &home,
        shell,
        cfg!(target_os = "macos"),
        xdg_config_home.as_deref(),
        |p| p.exists(),
    ))
}

pub async fn status(dir: &Path) -> Result<PathStatus> {
    let dir_str = dir.to_string_lossy().to_string();

    let shell = std::env::var("SHELL").ok();
    // Fish only reads its own snippet; an old .profile marker doesn't count.
    let in_user_path = rc_files(shell.as_deref())?.iter().any(|p| {
        std::fs::read_to_string(p)
            .map(|s| matches!(rc_block::strip_marker_block(&s), Strip::Removed(_)))
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
    let shell = std::env::var("SHELL").ok();
    let block = rc_block::path_block(&dir.to_string_lossy(), shell.as_deref());
    for rc in rc_files(shell.as_deref())? {
        // Read + append + write happens under the config lock in one go.
        crate::config_file::update_with_backup(&rc, |existing| {
            rc_block::append_block(existing.unwrap_or(""), &block)
                .map(|updated| updated.map(String::into_bytes))
        })
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
    let mut incomplete: Vec<String> = Vec::new();
    let shell = std::env::var("SHELL").ok();
    for rc in rc_files(shell.as_deref())? {
        if !rc.exists() {
            continue;
        }
        crate::config_file::update_with_backup(&rc, |content| {
            Ok::<_, std::io::Error>(match content.map(rc_block::strip_marker_block) {
                Some(Strip::Removed(new)) => Some(new.into_bytes()),
                Some(Strip::Incomplete) => {
                    tracing::warn!(
                        "{}: begin marker without end marker; left untouched",
                        rc.display()
                    );
                    incomplete.push(rc.display().to_string());
                    None
                }
                Some(Strip::NotFound) | None => None,
            })
        })
        .map_err(|e| AppError::Other(format!("write {}: {}", rc.display(), e)))?;
    }
    if !incomplete.is_empty() {
        return Err(AppError::Other(format!(
            "以下文件中的 PATH 标记块缺少结束标记，为避免误删你的配置未做修改，请手动删除：{}",
            incomplete.join("、")
        )));
    }
    Ok(())
}
