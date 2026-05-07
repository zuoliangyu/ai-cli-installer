//! npm-route installer for Claude Code / Codex.
//!
//! `npm install -g <pkg>` against `registry.npmmirror.com` for speed in CN.
//! Doesn't permanently change user's npm config — registry is passed per-install.

use std::process::Stdio;
use tokio::process::Command;

use crate::error::{AppError, Result};

const DEFAULT_REGISTRY: &str = "https://registry.npmmirror.com";

#[derive(Debug, Clone, serde::Serialize)]
pub struct NodeInfo {
    pub node_version: String,   // e.g. "v22.16.0"
    pub node_major: u32,
    pub npm_version: Option<String>,
}

/// Detect Node + npm. Errors if Node not on PATH or unparseable.
pub async fn detect_node() -> Result<NodeInfo> {
    let node_out = Command::new("node")
        .arg("--version")
        .output()
        .await
        .map_err(|e| {
            AppError::Other(format!(
                "未检测到 Node.js（{}）。请先安装 Node 18+：https://nodejs.org",
                e
            ))
        })?;

    if !node_out.status.success() {
        return Err(AppError::Other(format!(
            "node --version 失败 (exit {:?})",
            node_out.status.code()
        )));
    }

    let node_version = String::from_utf8_lossy(&node_out.stdout).trim().to_string();
    let stripped = node_version.trim_start_matches('v');
    let node_major: u32 = stripped
        .split('.')
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| AppError::Other(format!("无法解析 Node 版本: {}", node_version)))?;

    let npm_version = Command::new("npm")
        .arg("--version")
        .output()
        .await
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());

    Ok(NodeInfo {
        node_version,
        node_major,
        npm_version,
    })
}

/// Run `npm install -g <package> --registry <r>` and return stdout on success.
/// Doesn't expose progress (npm install is opaque); UI shows a spinner.
pub async fn install_global(package: &str, registry: Option<&str>) -> Result<String> {
    let reg = registry.unwrap_or(DEFAULT_REGISTRY);

    let output = Command::new("npm")
        .args(["install", "-g", package, "--registry", reg])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| AppError::Other(format!("启动 npm 失败：{}", e)))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        return Err(AppError::Install(format!(
            "npm install -g {} 失败 (exit {:?})\n--- stderr ---\n{}\n--- stdout ---\n{}",
            package,
            output.status.code(),
            stderr.trim(),
            stdout.trim()
        )));
    }

    tracing::debug!("npm install stdout:\n{}", stdout);
    Ok(stdout)
}

/// `npm bin -g` returns the global bin directory. Useful for verifying
/// where the installed binary lives (so we can show a sensible install_path).
pub async fn npm_global_bin() -> Result<String> {
    // npm 9+ removed `npm bin -g`. Use `npm prefix -g` + /bin (Unix) or root (Win).
    let prefix_out = Command::new("npm")
        .args(["prefix", "-g"])
        .output()
        .await
        .map_err(|e| AppError::Other(format!("npm prefix -g failed: {}", e)))?;

    if !prefix_out.status.success() {
        return Err(AppError::Other("npm prefix -g failed".into()));
    }
    let prefix = String::from_utf8_lossy(&prefix_out.stdout).trim().to_string();

    // Windows: prefix IS the bin dir; Unix: prefix/bin
    let bin = if cfg!(target_os = "windows") {
        prefix
    } else {
        format!("{}/bin", prefix)
    };
    Ok(bin)
}
