use std::sync::Arc;
use tauri::{AppHandle, State};
use tokio::sync::RwLock;

use crate::env_manager::{self, PathScope, PathStatus};
use crate::error::{AppError, Result};
use crate::mirrors::{self, MirrorList, MirrorProbe};
use crate::presets::{self, ClaudePreset, ClaudeSettingsEnv};
use crate::tools::{claude_code::ClaudeCode, InstallReport, Tool, ToolDescriptor};

/// Resolve a tool_id to its launcher dir, or return Other error.
fn launcher_dir_for(tool_id: &str) -> Result<std::path::PathBuf> {
    match tool_id {
        ClaudeCode::ID => ClaudeCode
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
    let mut d = cc.descriptor();
    d.installed_version = cc.detect_installed().await;
    Ok(vec![d])
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
) -> Result<InstallReport> {
    let channel = channel.unwrap_or_else(|| "latest".to_string());
    let mirrors = state.mirrors.read().await.clone();
    match tool_id.as_str() {
        ClaudeCode::ID => {
            ClaudeCode
                .install(app, state.client.clone(), mirrors, channel)
                .await
        }
        other => Err(AppError::Other(format!("unknown tool: {}", other))),
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
