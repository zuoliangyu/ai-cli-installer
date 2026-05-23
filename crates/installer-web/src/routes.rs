//! HTTP route handlers. Each handler is a thin adapter around
//! `installer_core::app_state` — JSON in, JSON out, with the broadcast sink
//! adapting to a `ProgressCallback` for the long-running install route.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use installer_core::app_state;
use installer_core::env_manager::PathScope;
use installer_core::progress::{DownloadProgress, ProgressCallback};
use installer_core::tools::InstallMethod;

use crate::ServerState;

fn err(e: installer_core::AppError) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

pub async fn list_tools(State(state): State<ServerState>) -> impl IntoResponse {
    match app_state::list_tools(&state.installer).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(e).into_response(),
    }
}

pub async fn list_mirrors(State(state): State<ServerState>) -> impl IntoResponse {
    match app_state::list_mirrors(&state.installer).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(e).into_response(),
    }
}

pub async fn probe_mirrors(State(state): State<ServerState>) -> impl IntoResponse {
    match app_state::probe_mirrors(&state.installer).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(e).into_response(),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallToolBody {
    pub tool_id: String,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub method: Option<InstallMethod>,
}

pub async fn install_tool(
    State(state): State<ServerState>,
    Json(body): Json<InstallToolBody>,
) -> impl IntoResponse {
    let tx = state.progress_tx.clone();
    let progress: ProgressCallback = Arc::new(move |p: DownloadProgress| {
        let _ = tx.send(p);
    });
    match app_state::install_tool(&state.installer, progress, &body.tool_id, body.channel, body.method).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(e).into_response(),
    }
}

pub async fn detect_node() -> impl IntoResponse {
    match app_state::detect_node().await {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(e).into_response(),
    }
}

pub async fn list_fixes(State(state): State<ServerState>) -> impl IntoResponse {
    match app_state::list_fixes(&state.installer).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(e).into_response(),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixesBody {
    pub fix_ids: Vec<String>,
}

pub async fn apply_fixes(
    State(state): State<ServerState>,
    Json(body): Json<FixesBody>,
) -> impl IntoResponse {
    match app_state::apply_fixes(&state.installer, &body.fix_ids).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(e).into_response(),
    }
}

pub async fn remove_fixes(
    State(state): State<ServerState>,
    Json(body): Json<FixesBody>,
) -> impl IntoResponse {
    match app_state::remove_fixes(&state.installer, &body.fix_ids).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(e).into_response(),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathToolQuery {
    pub tool_id: String,
}

pub async fn check_path_status(
    axum::extract::Query(q): axum::extract::Query<PathToolQuery>,
) -> impl IntoResponse {
    match app_state::check_path_status(&q.tool_id).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(e).into_response(),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathBody {
    pub tool_id: String,
    #[serde(default)]
    pub scope: Option<PathScope>,
}

pub async fn add_to_path(Json(body): Json<PathBody>) -> impl IntoResponse {
    match app_state::add_to_path(&body.tool_id, body.scope.unwrap_or(PathScope::System)).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err(e).into_response(),
    }
}

pub async fn remove_from_path(Json(body): Json<PathBody>) -> impl IntoResponse {
    match app_state::remove_from_path(&body.tool_id, body.scope.unwrap_or(PathScope::System)).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err(e).into_response(),
    }
}

pub async fn list_claude_presets() -> impl IntoResponse {
    Json(app_state::list_claude_presets())
}

pub async fn get_claude_settings() -> impl IntoResponse {
    match app_state::get_claude_settings() {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(e).into_response(),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyPresetBody {
    pub base_url: String,
    pub auth_token: String,
}

pub async fn apply_claude_preset(Json(body): Json<ApplyPresetBody>) -> impl IntoResponse {
    match app_state::apply_claude_preset(&body.base_url, &body.auth_token) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err(e).into_response(),
    }
}

#[derive(Deserialize)]
pub struct OpenPathBody {
    pub path: String,
}

pub async fn open_path(Json(body): Json<OpenPathBody>) -> impl IntoResponse {
    match app_state::open_path(&body.path) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err(e).into_response(),
    }
}

pub async fn get_logs() -> impl IntoResponse {
    // Web mode has no in-process log buffer; return empty.
    Json(Vec::<String>::new())
}
