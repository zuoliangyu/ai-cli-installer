use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, State};
use tokio::sync::RwLock;

use crate::env_manager::{self, PathScope, PathStatus};
use crate::error::{AppError, Result};
use crate::fixes::{self, ApplyReport, Fix, RemoveReport};
use crate::install_diagnostics;
use crate::mirrors::{self, MirrorList, MirrorProbe};
use crate::npm_installer::{self, NodeInfo};
use crate::presets::{self, ClaudePreset, ClaudeSettingsEnv};
use crate::tools::{
    claude_code::ClaudeCode, codex::CodexCli, InstallMethod, InstallReport, Tool, ToolDescriptor,
};

/// Resolve a tool_id to its launcher dir.
fn launcher_dir_for(tool_id: &str) -> Result<std::path::PathBuf> {
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

pub struct AppState {
    pub client: reqwest::Client,
    pub mirrors: RwLock<MirrorList>,
}

impl AppState {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(concat!("ai-cli-installer/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .expect("build reqwest client");
        Self {
            client,
            mirrors: RwLock::new(MirrorList::builtin()),
        }
    }
}

#[tauri::command]
pub async fn list_tools(state: State<'_, Arc<AppState>>) -> Result<Vec<ToolDescriptor>> {
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
        channel_version(&state.client, &cc, "latest"),
        channel_version(&state.client, &cc, "stable"),
        install_diagnostics::diagnose(
            "claude",
            native_launcher_path(&cc, "claude"),
            cc.launcher_dir(),
            cc.npm_package(),
        ),
        cx.detect_installed(),
        channel_version(&state.client, &cx, "latest"),
        channel_version(&state.client, &cx, "stable"),
        install_diagnostics::diagnose(
            "codex",
            native_launcher_path(&cx, "codex"),
            cx.launcher_dir(),
            cx.npm_package(),
        )
    );

    cd.installed_version = cc_installed;
    cd.latest_version = cc_latest;
    cd.stable_version = cc_stable;
    cd.installations = cc_installations;

    xd.installed_version = cx_installed;
    xd.latest_version = cx_latest;
    xd.stable_version = cx_stable;
    xd.installations = cx_installations;

    Ok(vec![cd, xd])
}

async fn channel_version<T: Tool>(
    client: &reqwest::Client,
    tool: &T,
    channel: &str,
) -> Option<String> {
    let version = fetch_channel_version(client, tool, channel).await;
    if version.is_some() || channel != "stable" {
        return version;
    }
    fetch_channel_version(client, tool, "latest").await
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

fn native_launcher_path<T: Tool>(tool: &T, command_name: &str) -> Option<std::path::PathBuf> {
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

#[tauri::command]
pub async fn list_mirrors(state: State<'_, Arc<AppState>>) -> Result<MirrorList> {
    Ok(state.mirrors.read().await.clone())
}

#[tauri::command]
pub async fn probe_mirrors(state: State<'_, Arc<AppState>>) -> Result<Vec<MirrorProbe>> {
    let list = state.mirrors.read().await.clone();
    Ok(mirrors::probe_all(&state.client, &list).await)
}

#[tauri::command]
pub async fn install_tool(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    tool_id: String,
    channel: Option<String>,
    method: Option<InstallMethod>,
) -> Result<InstallReport> {
    let requested_channel = channel.unwrap_or_else(|| "latest".to_string());
    let method = method.unwrap_or_default();
    let client = state.client.clone();
    match tool_id.as_str() {
        ClaudeCode::ID => {
            let mirrors = ClaudeCode.mirror_list();
            let channel = install_channel(&client, &ClaudeCode, requested_channel).await;
            ClaudeCode
                .install(method, app, client, mirrors, channel)
                .await
        }
        CodexCli::ID => {
            let mirrors = CodexCli.mirror_list();
            let channel = install_channel(&client, &CodexCli, requested_channel).await;
            CodexCli
                .install(method, app, client, mirrors, channel)
                .await
        }
        other => Err(AppError::Other(format!("unknown tool: {}", other))),
    }
}

#[tauri::command]
pub async fn detect_node() -> Result<NodeInfo> {
    npm_installer::detect_node().await
}

#[tauri::command]
pub async fn list_fixes(state: State<'_, Arc<AppState>>) -> Result<Vec<Fix>> {
    fixes::list_fixes(&state.client).await
}

#[tauri::command]
pub async fn apply_fixes(
    state: State<'_, Arc<AppState>>,
    fix_ids: Vec<String>,
) -> Result<ApplyReport> {
    fixes::apply_selected(&state.client, &fix_ids).await
}

#[tauri::command]
pub async fn remove_fixes(
    state: State<'_, Arc<AppState>>,
    fix_ids: Vec<String>,
) -> Result<RemoveReport> {
    fixes::remove_selected(&state.client, &fix_ids).await
}

#[tauri::command]
pub async fn open_path(path: String) -> Result<()> {
    let path = std::path::PathBuf::from(path);
    if !path.exists() {
        return Err(AppError::Other(format!(
            "path not found: {}",
            path.display()
        )));
    }

    open_path_with_system(&path)
}

fn open_path_with_system(path: &std::path::Path) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer.exe")
            .arg(path)
            .spawn()?;
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

#[tauri::command]
pub async fn check_path_status(tool_id: String) -> Result<PathStatus> {
    let dir = launcher_dir_for(&tool_id)?;
    env_manager::status(&dir).await
}

#[tauri::command]
pub async fn add_to_path(tool_id: String, scope: Option<String>) -> Result<()> {
    let dir = launcher_dir_for(&tool_id)?;
    let scope = parse_scope(scope.as_deref())?;
    env_manager::add(&dir, scope).await
}

#[tauri::command]
pub async fn remove_from_path(tool_id: String, scope: Option<String>) -> Result<()> {
    let dir = launcher_dir_for(&tool_id)?;
    let scope = parse_scope(scope.as_deref())?;
    env_manager::remove(&dir, scope).await
}

fn parse_scope(s: Option<&str>) -> Result<PathScope> {
    Ok(match s.unwrap_or("system") {
        "system" => PathScope::System,
        "user" => PathScope::User,
        other => return Err(AppError::Other(format!("unknown scope: {}", other))),
    })
}

#[tauri::command]
pub async fn list_claude_presets() -> Result<Vec<ClaudePreset>> {
    Ok(presets::list_all_presets())
}

#[tauri::command]
pub async fn get_claude_settings() -> Result<ClaudeSettingsEnv> {
    presets::read_current_env()
}

#[tauri::command]
pub async fn apply_claude_preset(base_url: String, auth_token: String) -> Result<()> {
    presets::apply_env(&base_url, &auth_token)
}
