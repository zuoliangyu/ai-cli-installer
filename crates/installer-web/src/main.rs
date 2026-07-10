mod config;
mod routes;
mod static_files;
mod ws;

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use clap::Parser;
use tokio::sync::broadcast;

use config::Config;
use installer_core::progress::DownloadProgress;
use installer_core::AppState;

/// Channel that fans out `DownloadProgress` events to every connected
/// `/ws/progress` subscriber. Capacity 256 is enough that slow WebSocket
/// clients (laggy network) don't backpressure the installer.
pub(crate) type ProgressTx = Arc<broadcast::Sender<DownloadProgress>>;

#[derive(Clone)]
pub(crate) struct ServerState {
    pub installer: Arc<AppState>,
    pub progress_tx: ProgressTx,
    pub access_token: Option<Arc<str>>,
}

async fn require_auth(State(state): State<ServerState>, request: Request, next: Next) -> Response {
    let provided = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if token_matches(state.access_token.as_deref(), provided) {
        next.run(request).await
    } else {
        (StatusCode::UNAUTHORIZED, "缺少或无效的访问令牌").into_response()
    }
}

pub(crate) fn token_matches(expected: Option<&str>, provided: Option<&str>) -> bool {
    expected.is_none() || expected == provided
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = Config::parse();
    if let Err(message) = config.validate() {
        eprintln!("配置错误：{message}");
        std::process::exit(2);
    }

    let (tx, _) = broadcast::channel::<DownloadProgress>(256);
    let progress_tx: ProgressTx = Arc::new(tx);

    let state = ServerState {
        installer: installer_core::app_state::shared(),
        progress_tx: progress_tx.clone(),
        access_token: config
            .token
            .as_deref()
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .map(Arc::<str>::from),
    };

    let api = Router::new()
        .route("/api/tools", get(routes::list_tools))
        .route("/api/tools/install", post(routes::install_tool))
        .route("/api/mirrors", get(routes::list_mirrors))
        .route("/api/mirrors/probe", post(routes::probe_mirrors))
        .route("/api/node", get(routes::detect_node))
        .route("/api/fixes", get(routes::list_fixes))
        .route("/api/fixes/apply", post(routes::apply_fixes))
        .route("/api/fixes/remove", post(routes::remove_fixes))
        .route("/api/path/status", get(routes::check_path_status))
        .route("/api/path/add", post(routes::add_to_path))
        .route("/api/path/remove", post(routes::remove_from_path))
        .route("/api/presets", get(routes::list_claude_presets))
        .route("/api/presets/current", get(routes::get_claude_settings))
        .route("/api/presets/apply", post(routes::apply_claude_preset))
        .route("/api/open-path", post(routes::open_path))
        .route("/api/logs", get(routes::get_logs))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth))
        .with_state(state.clone());

    let ws_routes = Router::new()
        .route("/ws/progress", get(ws::progress_ws_handler))
        .with_state(state);

    let app = Router::new()
        .merge(api)
        .merge(ws_routes)
        .fallback(static_files::static_handler);

    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Failed to bind address");

    tracing::info!("ai-cli-installer Web Server listening on http://{}", addr);

    axum::serve(listener, app).await.expect("server error");
}

#[cfg(test)]
mod tests {
    use super::token_matches;

    #[test]
    fn access_token_must_match_when_configured() {
        assert!(token_matches(None, None));
        assert!(token_matches(Some("secret"), Some("secret")));
        assert!(!token_matches(Some("secret"), None));
        assert!(!token_matches(Some("secret"), Some("wrong")));
    }
}
