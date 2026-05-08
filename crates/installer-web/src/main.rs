mod config;
mod routes;
mod static_files;
mod ws;

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use clap::Parser;
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;

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
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = Config::parse();

    let (tx, _) = broadcast::channel::<DownloadProgress>(256);
    let progress_tx: ProgressTx = Arc::new(tx);

    let state = ServerState {
        installer: installer_core::app_state::shared(),
        progress_tx: progress_tx.clone(),
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
        .with_state(state);

    let ws_routes = Router::new()
        .route("/ws/progress", get(ws::progress_ws_handler))
        .with_state(progress_tx);

    let app = Router::new()
        .merge(api)
        .merge(ws_routes)
        .fallback(static_files::static_handler)
        .layer(CorsLayer::permissive());

    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Failed to bind address");

    tracing::info!("ai-cli-installer Web Server listening on http://{}", addr);

    axum::serve(listener, app).await.expect("server error");
}
