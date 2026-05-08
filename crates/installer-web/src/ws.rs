//! WebSocket handler that forwards `DownloadProgress` broadcasts to every
//! connected client. The `/ws/progress` route mirrors the Tauri shell's
//! `download-progress` event so the front-end's API layer can subscribe to
//! the same logical stream regardless of transport.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use tokio::sync::broadcast;

use crate::ProgressTx;

pub async fn progress_ws_handler(
    ws: WebSocketUpgrade,
    State(tx): State<ProgressTx>,
) -> Response {
    ws.on_upgrade(move |socket| handle_progress_socket(socket, tx))
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
