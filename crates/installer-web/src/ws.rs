//! WebSocket handler that forwards `DownloadProgress` broadcasts to every
//! connected client. The `/ws/progress` route mirrors the Tauri shell's
//! `download-progress` event so the front-end's API layer can subscribe to
//! the same logical stream regardless of transport.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tokio::sync::broadcast;

use crate::{token_matches, ProgressTx, ServerState};

#[derive(Deserialize)]
pub struct AuthQuery {
    token: Option<String>,
}

pub async fn progress_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<ServerState>,
    Query(auth): Query<AuthQuery>,
) -> Response {
    if !token_matches(state.access_token.as_deref(), auth.token.as_deref()) {
        return (StatusCode::UNAUTHORIZED, "缺少或无效的访问令牌").into_response();
    }
    ws.on_upgrade(move |socket| handle_progress_socket(socket, state.progress_tx))
}

async fn handle_progress_socket(mut socket: WebSocket, tx: ProgressTx) {
    let mut rx = tx.subscribe();

    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(progress) => {
                        let json = match serde_json::to_string(&progress) {
                            Ok(s) => s,
                            Err(_) => continue,
                        };
                        if socket.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }
}
