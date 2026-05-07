use std::sync::Arc;
use tauri::{AppHandle, State};
use tokio::sync::RwLock;

use crate::env_manager::{self, PathScope, PathStatus};
use crate::error::{AppError, Result};
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
pub async fn list_tools(_state: State<'_, Arc<AppState>>) -> Result<Vec<ToolDescriptor>> {
    let cc = ClaudeCode;
    let mut cd = cc.descriptor();
    cd.installed_version = cc.detect_installed().await;

    let cx = CodexCli;
    let mut xd = cx.descriptor();
    xd.installed_version = cx.detect_installed().await;

    Ok(vec![cd, xd])
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
    let channel = channel.unwrap_or_else(|| "latest".to_string());
    let method = method.unwrap_or_default();
    let client = state.client.clone();
    match tool_id.as_str() {
        ClaudeCode::ID => {
            let mirrors = ClaudeCode.mirror_list();
            ClaudeCode.install(method, app, client, mirrors, channel).await
        }
        CodexCli::ID => {
            let mirrors = CodexCli.mirror_list();
            CodexCli.install(method, app, client, mirrors, channel).await
        }
        other => Err(AppError::Other(format!("unknown tool: {}", other))),
    }
}

#[tauri::command]
pub async fn detect_node() -> Result<NodeInfo> {
    npm_installer::detect_node().await
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
