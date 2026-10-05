mod config;
mod routes;
mod security;
mod static_files;
mod ws;

use std::net::IpAddr;
use std::sync::Arc;

use axum::middleware;
use axum::routing::{get, post};
use axum::Router;
use clap::Parser;
use tokio::sync::broadcast;

use config::Config;
use installer_core::progress::DownloadProgress;
use installer_core::AppState;
use security::HostPolicy;

/// Channel that fans out `DownloadProgress` events to every connected
/// `/ws/progress` subscriber. Capacity 256 is enough that slow WebSocket
/// clients (laggy network) don't backpressure the installer.
pub(crate) type ProgressTx = Arc<broadcast::Sender<DownloadProgress>>;

#[derive(Clone)]
pub(crate) struct ServerState {
    pub installer: Arc<AppState>,
    pub progress_tx: ProgressTx,
    pub access_token: Arc<str>,
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

    let configured_token = config.configured_token().map(str::to_owned);
    let token_generated = configured_token.is_none();
    let access_token: Arc<str> = configured_token
        .unwrap_or_else(security::generate_token)
        .into();

    let state = ServerState {
        installer: installer_core::app_state::shared(),
        progress_tx: progress_tx.clone(),
        access_token: access_token.clone(),
    };
    let host_policy = Arc::new(HostPolicy::new(config.allowed_hosts()));

    let api = Router::new()
        .route("/api/tools", get(routes::list_tools))
        .route("/api/tools/install", post(routes::install_tool))
        .route("/api/mirrors", get(routes::list_mirrors))
        .route(
            "/api/mirrors/probe",
            get(routes::probe_mirrors).post(routes::probe_mirrors),
        )
        .route("/api/node", get(routes::detect_node))
        .route("/api/fixes", get(routes::list_fixes))
        .route("/api/fixes/apply", post(routes::apply_fixes))
        .route("/api/fixes/remove", post(routes::remove_fixes))
        .route("/api/path/status", get(routes::check_path_status))
        .route("/api/path/add", post(routes::add_to_path))
        .route("/api/path/remove", post(routes::remove_from_path))
        .route("/api/open-path", post(routes::open_path))
        .route("/api/logs", get(routes::get_logs))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            security::require_auth,
        ))
        .with_state(state.clone());

    let ws_routes = Router::new()
        .route("/ws/progress", get(ws::progress_ws_handler))
        .with_state(state);

    let app = Router::new()
        .merge(api)
        .merge(ws_routes)
        .fallback(static_files::static_handler)
        .layer(middleware::from_fn_with_state(
            host_policy,
            security::guard_host_and_origin,
        ));

    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Failed to bind address");

    tracing::info!("ai-cli-installer Web Server listening on http://{}", addr);
    if config.remote_bind_without_allowed_hosts() {
        tracing::warn!(
            "当前监听 {}，但未配置 --allowed-host；远程访问时浏览器使用的主机名/IP 需通过 --allowed-host（或 INSTALLER_ALLOWED_HOSTS）加入白名单，否则会被拒绝",
            config.host
        );
    }

    let base = format!("http://{}:{}/", browser_host(&config.host), config.port);
    if token_generated {
        println!("已生成本次运行的访问令牌，请用浏览器打开：\n  {base}?token={access_token}");
    } else {
        println!("请用浏览器打开：{base}?token=<已配置的访问令牌>");
    }

    axum::serve(listener, app).await.expect("server error");
}

/// Host to put in the printed URL: unspecified binds are reachable via
/// loopback; IPv6 literals need brackets.
fn browser_host(bind: &str) -> String {
    let bare = bind.trim().trim_matches(['[', ']']);
    match bare.parse::<IpAddr>() {
        Ok(ip) if ip.is_unspecified() => "127.0.0.1".into(),
        Ok(IpAddr::V6(_)) => format!("[{bare}]"),
        _ => bare.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::browser_host;

    #[test]
    fn printed_url_host_is_browsable() {
        assert_eq!(browser_host("0.0.0.0"), "127.0.0.1");
        assert_eq!(browser_host("::"), "127.0.0.1");
        assert_eq!(browser_host("::1"), "[::1]");
        assert_eq!(browser_host("localhost"), "localhost");
        assert_eq!(browser_host("192.168.1.5"), "192.168.1.5");
    }
}
