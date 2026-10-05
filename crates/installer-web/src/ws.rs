//! WebSocket handler that forwards `DownloadProgress` broadcasts to every
//! connected client. The `/ws/progress` route mirrors the Tauri shell's
//! `download-progress` event so the front-end's API layer can subscribe to
//! the same logical stream regardless of transport.

use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tokio::sync::broadcast;
use tokio::time::{interval, Instant, MissedTickBehavior};

use crate::security::{token_matches, unauthorized};
use crate::{ProgressTx, ServerState};

const PING_INTERVAL: Duration = Duration::from_secs(20);
/// No frame (pong included) from the client for this long → drop it.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Deserialize)]
pub struct AuthQuery {
    token: Option<String>,
}

pub async fn progress_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<ServerState>,
    Query(auth): Query<AuthQuery>,
) -> Response {
    if !token_matches(&state.access_token, auth.token.as_deref()) {
        return unauthorized().into_response();
    }
    ws.on_upgrade(move |socket| handle_progress_socket(socket, state.progress_tx))
}

/// The broadcast receiver lives only as long as this function, so every exit
/// path (close, send failure, idle timeout) also drops the subscription.
async fn handle_progress_socket(mut socket: WebSocket, tx: ProgressTx) {
    let mut rx = tx.subscribe();
    let mut ping = interval(PING_INTERVAL);
    ping.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ping.tick().await;
    let mut last_seen = Instant::now();

    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(progress) => {
                        let Ok(json) = serde_json::to_string(&progress) else {
                            continue;
                        };
                        if socket.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            _ = ping.tick() => {
                if last_seen.elapsed() > IDLE_TIMEOUT {
                    break;
                }
                if socket.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => last_seen = Instant::now(),
                }
            }
        }
    }
    let _ = socket.send(Message::Close(None)).await;
}
