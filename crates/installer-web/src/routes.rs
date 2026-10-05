//! HTTP route handlers. Each handler is a thin adapter around
//! `installer_core::app_state` — JSON in, JSON out, with the broadcast sink
//! adapting to a `ProgressCallback` for the long-running install route.

use std::future::Future;
use std::sync::Arc;

use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use installer_core::app_state;
use installer_core::env_manager::{PathScope, PathStatus};
use installer_core::fixes::{ApplyReport, Fix, RemoveReport};
use installer_core::mirrors::{MirrorList, MirrorProbe};
use installer_core::npm_installer::NodeInfo;
use installer_core::progress::{DownloadProgress, ProgressCallback};
use installer_core::tools::claude_code::ClaudeCode;
use installer_core::tools::codex::CodexCli;
use installer_core::tools::{InstallMethod, InstallReport, ToolDescriptor};
use installer_core::AppError;

use crate::ServerState;

/// Plain-text error response; the front-end shows `resp.text()` verbatim.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }
}

impl From<AppError> for ApiError {
    fn from(e: AppError) -> Self {
        let status = match &e {
            AppError::Other(msg) if is_input_error(msg) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            message: e.to_string(),
        }
    }
}

/// `AppError` has no dedicated "bad input" variant; these are the messages
/// core produces for caller-supplied values (unknown tool / mirror, bad path).
fn is_input_error(msg: &str) -> bool {
    const PREFIXES: [&str; 5] = [
        "unknown tool",
        "未知镜像",
        "path not found",
        "refusing to open",
        "empty patch path",
    ];
    PREFIXES.iter().any(|p| msg.starts_with(p))
}

impl From<JsonRejection> for ApiError {
    fn from(e: JsonRejection) -> Self {
        Self::bad_request(format!("请求参数无效：{}", e.body_text()))
    }
}

impl From<QueryRejection> for ApiError {
    fn from(e: QueryRejection) -> Self {
        Self::bad_request(format!("查询参数无效：{}", e.body_text()))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, self.message).into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;
type JsonBody<T> = Result<Json<T>, JsonRejection>;
type QueryParams<T> = Result<Query<T>, QueryRejection>;

/// Run `fut` on its own task so the operation finishes even if the client
/// disconnects (axum drops the handler future on disconnect).
async fn detached<T, F>(fut: F) -> ApiResult<T>
where
    F: Future<Output = installer_core::Result<T>> + Send + 'static,
    T: Send + 'static,
{
    match tokio::spawn(fut).await {
        Ok(result) => result.map_err(ApiError::from),
        Err(e) => Err(ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("后台任务异常终止：{e}"),
        }),
    }
}

fn ensure_known_tool(tool_id: &str) -> ApiResult<()> {
    if tool_id == ClaudeCode::ID || tool_id == CodexCli::ID {
        Ok(())
    } else {
        Err(ApiError::bad_request(format!("未知工具：{tool_id}")))
    }
}

pub async fn list_tools(State(state): State<ServerState>) -> ApiResult<Json<Vec<ToolDescriptor>>> {
    // Diagnostics hold the core's tool-operation lock; keep them detached.
    let installer = state.installer.clone();
    detached(async move { app_state::list_tools(&installer).await })
        .await
        .map(Json)
}

pub async fn list_mirrors(State(state): State<ServerState>) -> ApiResult<Json<MirrorList>> {
    Ok(Json(app_state::list_mirrors(&state.installer).await?))
}

pub async fn probe_mirrors(State(state): State<ServerState>) -> ApiResult<Json<Vec<MirrorProbe>>> {
    Ok(Json(app_state::probe_mirrors(&state.installer).await?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallToolBody {
    pub tool_id: String,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub method: Option<InstallMethod>,
    /// Optional mirror name to pin the install to a single source (e.g.
    /// `"github-direct"`). `None` = auto mode (race all mirrors).
    #[serde(default)]
    pub mirror: Option<String>,
}

pub async fn install_tool(
    State(state): State<ServerState>,
    body: JsonBody<InstallToolBody>,
) -> ApiResult<Json<InstallReport>> {
    let Json(body) = body?;
    ensure_known_tool(&body.tool_id)?;
    let tx = state.progress_tx.clone();
    let progress: ProgressCallback = Arc::new(move |p: DownloadProgress| {
        let _ = tx.send(p);
    });
    let installer = state.installer.clone();
    detached(async move {
        app_state::install_tool(
            &installer,
            progress,
            &body.tool_id,
            body.channel,
            body.method,
            body.mirror,
        )
        .await
    })
    .await
    .map(Json)
}

pub async fn detect_node() -> ApiResult<Json<NodeInfo>> {
    Ok(Json(app_state::detect_node().await?))
}

pub async fn list_fixes(State(state): State<ServerState>) -> ApiResult<Json<Vec<Fix>>> {
    Ok(Json(app_state::list_fixes(&state.installer).await?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixesBody {
    pub fix_ids: Vec<String>,
}

pub async fn apply_fixes(
    State(state): State<ServerState>,
    body: JsonBody<FixesBody>,
) -> ApiResult<Json<ApplyReport>> {
    let Json(body) = body?;
    let installer = state.installer.clone();
    detached(async move { app_state::apply_fixes(&installer, &body.fix_ids).await })
        .await
        .map(Json)
}

pub async fn remove_fixes(
    State(state): State<ServerState>,
    body: JsonBody<FixesBody>,
) -> ApiResult<Json<RemoveReport>> {
    let Json(body) = body?;
    let installer = state.installer.clone();
    detached(async move { app_state::remove_fixes(&installer, &body.fix_ids).await })
        .await
        .map(Json)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathToolQuery {
    pub tool_id: String,
}

pub async fn check_path_status(query: QueryParams<PathToolQuery>) -> ApiResult<Json<PathStatus>> {
    let Query(q) = query?;
    ensure_known_tool(&q.tool_id)?;
    Ok(Json(app_state::check_path_status(&q.tool_id).await?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathBody {
    pub tool_id: String,
    #[serde(default)]
    pub scope: Option<PathScope>,
}

impl PathBody {
    fn scope(&self) -> PathScope {
        self.scope.unwrap_or_default()
    }
}

pub async fn add_to_path(body: JsonBody<PathBody>) -> ApiResult<StatusCode> {
    let Json(body) = body?;
    ensure_known_tool(&body.tool_id)?;
    let scope = body.scope();
    detached(async move { app_state::add_to_path(&body.tool_id, scope).await }).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove_from_path(body: JsonBody<PathBody>) -> ApiResult<StatusCode> {
    let Json(body) = body?;
    ensure_known_tool(&body.tool_id)?;
    let scope = body.scope();
    detached(async move { app_state::remove_from_path(&body.tool_id, scope).await }).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct OpenPathBody {
    pub path: String,
}

pub async fn open_path(body: JsonBody<OpenPathBody>) -> ApiResult<StatusCode> {
    let Json(body) = body?;
    if body.path.trim().is_empty() {
        return Err(ApiError::bad_request("路径不能为空"));
    }
    app_state::open_path(&body.path)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct LogsQuery {
    /// Accepted (and validated) for parity with Tauri; unused without a log source.
    #[serde(default)]
    #[allow(dead_code)]
    pub since: Option<u64>,
}

/// Same shape as the Tauri `get_logs` command (`LogChunk`).
#[derive(Serialize)]
pub struct LogChunk {
    lines: Vec<String>,
    next: u64,
    reset: bool,
}

pub async fn get_logs(query: QueryParams<LogsQuery>) -> ApiResult<Json<LogChunk>> {
    query?;
    // Web mode has no in-process log buffer: always an empty, non-reset chunk.
    Ok(Json(LogChunk {
        lines: Vec::new(),
        next: 0,
        reset: false,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_errors_map_to_400() {
        let unknown = ApiError::from(AppError::Other("unknown tool: foo".into()));
        assert_eq!(unknown.status, StatusCode::BAD_REQUEST);
        let mirror = ApiError::from(AppError::Other("未知镜像: `x`。".into()));
        assert_eq!(mirror.status, StatusCode::BAD_REQUEST);
        let io = ApiError::from(AppError::Install("spawn failed".into()));
        assert_eq!(io.status, StatusCode::INTERNAL_SERVER_ERROR);
        let other = ApiError::from(AppError::Other("no home dir".into()));
        assert_eq!(other.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(ensure_known_tool(ClaudeCode::ID).is_ok());
        assert_eq!(
            ensure_known_tool("rm -rf").unwrap_err().status,
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn detached_task_survives_dropped_caller() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(detached(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let _ = tx.send(());
            Ok::<_, AppError>(())
        }));
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        handle.abort();
        assert!(
            rx.await.is_ok(),
            "detached work must finish after the caller is dropped"
        );
    }
}
