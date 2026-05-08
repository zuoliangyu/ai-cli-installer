//! Shared runtime state — held by both the Tauri shell and the Web shell.
//!
//! Holds the `reqwest::Client` (so connection pools are reused across
//! commands) and the in-memory `MirrorList`. Higher-level service helpers
//! consume `&AppState` and a `ProgressCallback`, then call into the rest
//! of the core crate.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use crate::env_manager::{self, PathScope, PathStatus};
use crate::error::{AppError, Result};
use crate::fixes::{self, ApplyReport, Fix, RemoveReport};
use crate::install_diagnostics;
use crate::mirrors::{self, MirrorList, MirrorProbe};
use crate::npm_installer::{self, NodeInfo};
use crate::presets::{self, ClaudePreset, ClaudeSettingsEnv};
use crate::progress::ProgressCallback;
use crate::tools::{
    claude_code::ClaudeCode, codex::CodexCli, InstallMethod, InstallReport, Tool, ToolDescriptor,
};

pub struct AppState {
    pub client: reqwest::Client,
    pub mirrors: RwLock<MirrorList>,
}

impl AppState {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(concat!("ai-cli-installer/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(60))
            .build()
            .expect("build reqwest client");
        Self {
            client,
            mirrors: RwLock::new(MirrorList::builtin()),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------- High-level service API ----------
//
// Each function below replaces one of the previous Tauri commands. Both
// `src-tauri/commands.rs` and `installer-web/routes.rs` call into these so
// the two shells share a single source of truth for behavior.

pub async fn list_tools(state: &AppState) -> Result<Vec<ToolDescriptor>> {
    let cc = ClaudeCode;
    let mut cd = cc.descriptor();

    let cx = CodexCli;
    let mut xd = cx.descriptor();

    let (
        cc_installed,
        cc_latest,
        cc_stable,
        cc_installations,
        cx_installed,
        cx_latest,
        cx_stable,
        cx_installations,
    ) = tokio::join!(
        cc.detect_installed(),
        fetch_channel_version(&state.client, &cc, "latest"),
        fetch_channel_version(&state.client, &cc, "stable"),
        install_diagnostics::diagnose(
            "claude",
            native_launcher_path(&cc, "claude"),
            cc.launcher_dir(),
            cc.npm_package(),
        ),
        cx.detect_installed(),
        fetch_channel_version(&state.client, &cx, "latest"),
        fetch_channel_version(&state.client, &cx, "stable"),
        install_diagnostics::diagnose(
            "codex",
            native_launcher_path(&cx, "codex"),
            cx.launcher_dir(),
            cx.npm_package(),
        )
    );

    let (cc_stable, cc_falls_back) = resolve_stable(cc_stable, &cc_latest);
    let (cx_stable, cx_falls_back) = resolve_stable(cx_stable, &cx_latest);

    cd.installed_version = cc_installed.or_else(|| installed_from(&cc_installations));
    cd.latest_version = cc_latest;
    cd.stable_version = cc_stable;
    cd.stable_falls_back_to_latest = cc_falls_back;
    cd.installations = cc_installations;

    xd.installed_version = cx_installed.or_else(|| installed_from(&cx_installations));
    xd.latest_version = cx_latest;
    xd.stable_version = cx_stable;
    xd.stable_falls_back_to_latest = cx_falls_back;
    xd.installations = cx_installations;

    Ok(vec![cd, xd])
}

/// `Tool::detect_installed` 通过 `where`/`command -v` 跑 --version；遇到桌面进程
/// PATH 不全或 .cmd shim 解析失败时会返空。诊断流程已经扫到了 install，就以
/// 「current_path → 任一带版本号」的优先级反推一个版本号填进顶部"已安装"。
fn installed_from(installs: &[install_diagnostics::ToolInstallation]) -> Option<String> {
    installs
        .iter()
        .find(|i| i.current_path && i.version.is_some())
        .or_else(|| installs.iter().find(|i| i.version.is_some()))
        .and_then(|i| i.version.clone())
}

pub async fn list_mirrors(state: &AppState) -> Result<MirrorList> {
    Ok(state.mirrors.read().await.clone())
}

pub async fn probe_mirrors(state: &AppState) -> Result<Vec<MirrorProbe>> {
    let list = state.mirrors.read().await.clone();
    Ok(mirrors::probe_all(&state.client, &list).await)
}

pub async fn install_tool(
    state: &AppState,
    progress: ProgressCallback,
    tool_id: &str,
    channel: Option<String>,
    method: Option<InstallMethod>,
) -> Result<InstallReport> {
    let requested_channel = channel.unwrap_or_else(|| "latest".to_string());
    let method = method.unwrap_or_default();
    let client = state.client.clone();
    match tool_id {
        ClaudeCode::ID => {
            let mirrors = ClaudeCode.mirror_list();
            let channel = install_channel(&client, &ClaudeCode, requested_channel).await;
            ClaudeCode
                .install(method, progress, client, mirrors, channel)
                .await
        }
        CodexCli::ID => {
            let mirrors = CodexCli.mirror_list();
            let channel = install_channel(&client, &CodexCli, requested_channel).await;
            CodexCli
                .install(method, progress, client, mirrors, channel)
                .await
        }
        other => Err(AppError::Other(format!("unknown tool: {}", other))),
    }
}

pub async fn detect_node() -> Result<NodeInfo> {
    npm_installer::detect_node().await
}

pub async fn list_fixes(state: &AppState) -> Result<Vec<Fix>> {
    fixes::list_fixes(&state.client).await
}

pub async fn apply_fixes(state: &AppState, fix_ids: &[String]) -> Result<ApplyReport> {
    fixes::apply_selected(&state.client, fix_ids).await
}

pub async fn remove_fixes(state: &AppState, fix_ids: &[String]) -> Result<RemoveReport> {
    fixes::remove_selected(&state.client, fix_ids).await
}

pub async fn check_path_status(tool_id: &str) -> Result<PathStatus> {
    let dir = launcher_dir_for(tool_id)?;
    env_manager::status(&dir).await
}

pub async fn add_to_path(tool_id: &str, scope: PathScope) -> Result<()> {
    let dir = launcher_dir_for(tool_id)?;
    env_manager::add(&dir, scope).await
}

pub async fn remove_from_path(tool_id: &str, scope: PathScope) -> Result<()> {
    let dir = launcher_dir_for(tool_id)?;
    env_manager::remove(&dir, scope).await
}

pub fn list_claude_presets() -> Vec<ClaudePreset> {
    presets::list_all_presets()
}

pub fn get_claude_settings() -> Result<ClaudeSettingsEnv> {
    presets::read_current_env()
}

pub fn apply_claude_preset(base_url: &str, auth_token: &str) -> Result<()> {
    presets::apply_env(base_url, auth_token)
}

/// Open a JSON config file with the system's default associated app.
/// Whitelisted to `.json` files only — the UI only ever clicks paths produced
/// by `apply_fixes` / `remove_fixes`, which always write JSON.
pub fn open_path(path: &str) -> Result<()> {
    let raw = PathBuf::from(path);
    let canonical = raw
        .canonicalize()
        .map_err(|e| AppError::Other(format!("path not found: {} ({})", raw.display(), e)))?;

    let metadata = std::fs::metadata(&canonical)
        .map_err(|e| AppError::Other(format!("stat {}: {}", canonical.display(), e)))?;
    if !metadata.is_file() {
        return Err(AppError::Other(format!(
            "refusing to open non-file path: {}",
            canonical.display()
        )));
    }

    let ext_ok = canonical
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"));
    if !ext_ok {
        return Err(AppError::Other(format!(
            "refusing to open non-json path: {}",
            canonical.display()
        )));
    }

    open_path_with_system(&canonical)
}

fn open_path_with_system(path: &std::path::Path) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(["/c", "start", ""]).arg(path);
        crate::proc::silence_windows_std(&mut cmd);
        cmd.spawn()?;
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(path).spawn()?;
        return Ok(());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(path).spawn()?;
        return Ok(());
    }
}

// ---------- Helpers ----------

fn launcher_dir_for(tool_id: &str) -> Result<PathBuf> {
    match tool_id {
        ClaudeCode::ID => ClaudeCode
            .launcher_dir()
            .ok_or_else(|| AppError::Other("home dir not available".into())),
        CodexCli::ID => CodexCli
            .launcher_dir()
            .ok_or_else(|| AppError::Other("home dir not available".into())),
        other => Err(AppError::Other(format!("unknown tool: {}", other))),
    }
}

fn resolve_stable(stable: Option<String>, latest: &Option<String>) -> (Option<String>, bool) {
    match stable {
        Some(v) => (Some(v), false),
        None => (latest.clone(), latest.is_some()),
    }
}

async fn fetch_channel_version<T: Tool>(
    client: &reqwest::Client,
    tool: &T,
    channel: &str,
) -> Option<String> {
    let mirrors = tool.mirror_list();
    tokio::time::timeout(
        Duration::from_secs(5),
        mirrors::fetch_version(client, &mirrors, channel),
    )
    .await
    .ok()
    .and_then(|result| result.ok())
    .map(|(_, version)| version)
}

fn native_launcher_path<T: Tool>(tool: &T, command_name: &str) -> Option<PathBuf> {
    let file_name = if cfg!(target_os = "windows") {
        format!("{}.exe", command_name)
    } else {
        command_name.to_string()
    };
    Some(tool.launcher_dir()?.join(file_name))
}

async fn install_channel<T: Tool>(client: &reqwest::Client, tool: &T, channel: String) -> String {
    if channel != "stable"
        || fetch_channel_version(client, tool, "stable")
            .await
            .is_some()
    {
        return channel;
    }
    "latest".to_string()
}

/// Helper for Tauri / Axum shells to wrap their Arc-based state without each
/// having to know how the inner type is constructed.
pub fn shared() -> Arc<AppState> {
    Arc::new(AppState::new())
}
