//! Thin Tauri command shell. Every command delegates to `installer_core` and
//! merely adapts Tauri-specific concerns: `AppHandle::emit` for download
//! progress, IPC argument decoding (camelCase JSON ↔ Rust snake_case), and
//! `Result<T, AppError>` serialization.

use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

use installer_core::env_manager::{PathScope, PathStatus};
use installer_core::fixes::{ApplyReport, Fix, RemoveReport};
use installer_core::mirrors::{MirrorList, MirrorProbe};
use installer_core::npm_installer::NodeInfo;
use installer_core::presets::{ClaudePreset, ClaudeSettingsEnv};
use installer_core::progress::{DownloadProgress, ProgressCallback};
use installer_core::tools::{InstallMethod, InstallReport, ToolDescriptor};
use installer_core::{app_state, AppState, Result};

use crate::log_buffer::LogBuffer;

/// Wrap the Tauri `AppHandle` as a `ProgressCallback` that emits
/// `download-progress` events to the front-end window.
fn progress_for_app(app: AppHandle) -> ProgressCallback {
    Arc::new(move |p: DownloadProgress| {
        let _ = app.emit("download-progress", p);
    })
}

#[tauri::command]
pub async fn list_tools(state: State<'_, Arc<AppState>>) -> Result<Vec<ToolDescriptor>> {
    app_state::list_tools(&state).await
}

#[tauri::command]
pub async fn list_mirrors(state: State<'_, Arc<AppState>>) -> Result<MirrorList> {
    app_state::list_mirrors(&state).await
}

#[tauri::command]
pub async fn probe_mirrors(state: State<'_, Arc<AppState>>) -> Result<Vec<MirrorProbe>> {
    app_state::probe_mirrors(&state).await
}

#[tauri::command]
pub async fn install_tool(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    tool_id: String,
    channel: Option<String>,
    method: Option<InstallMethod>,
) -> Result<InstallReport> {
    let progress = progress_for_app(app);
    app_state::install_tool(&state, progress, &tool_id, channel, method).await
}

#[tauri::command]
pub async fn detect_node() -> Result<NodeInfo> {
    app_state::detect_node().await
}

#[tauri::command]
pub async fn list_fixes(state: State<'_, Arc<AppState>>) -> Result<Vec<Fix>> {
    app_state::list_fixes(&state).await
}

#[tauri::command]
pub async fn apply_fixes(
    state: State<'_, Arc<AppState>>,
    fix_ids: Vec<String>,
) -> Result<ApplyReport> {
    app_state::apply_fixes(&state, &fix_ids).await
}

#[tauri::command]
pub async fn remove_fixes(
    state: State<'_, Arc<AppState>>,
    fix_ids: Vec<String>,
) -> Result<RemoveReport> {
    app_state::remove_fixes(&state, &fix_ids).await
}

#[tauri::command]
pub async fn open_path(path: String) -> Result<()> {
    app_state::open_path(&path)
}

#[tauri::command]
pub async fn check_path_status(tool_id: String) -> Result<PathStatus> {
    app_state::check_path_status(&tool_id).await
}

#[tauri::command]
pub async fn add_to_path(tool_id: String, scope: Option<PathScope>) -> Result<()> {
    app_state::add_to_path(&tool_id, scope.unwrap_or(PathScope::System)).await
}

#[tauri::command]
pub async fn remove_from_path(tool_id: String, scope: Option<PathScope>) -> Result<()> {
    app_state::remove_from_path(&tool_id, scope.unwrap_or(PathScope::System)).await
}

#[tauri::command]
pub async fn list_claude_presets() -> Result<Vec<ClaudePreset>> {
    Ok(app_state::list_claude_presets())
}

#[tauri::command]
pub async fn get_claude_settings() -> Result<ClaudeSettingsEnv> {
    app_state::get_claude_settings()
}

#[tauri::command]
pub async fn apply_claude_preset(base_url: String, auth_token: String) -> Result<()> {
    app_state::apply_claude_preset(&base_url, &auth_token)
}

#[tauri::command]
pub async fn get_logs(state: State<'_, LogBuffer>) -> Result<Vec<String>> {
    Ok(state.lines())
}
