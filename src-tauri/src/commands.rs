use std::sync::Arc;
use tauri::{AppHandle, State};
use tokio::sync::RwLock;

use crate::error::Result;
use crate::mirrors::{self, MirrorList, MirrorProbe};
use crate::tools::{claude_code::ClaudeCode, InstallReport, Tool, ToolDescriptor};

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
        other => Err(crate::error::AppError::Other(format!(
            "unknown tool: {}",
            other
        ))),
    }
}
