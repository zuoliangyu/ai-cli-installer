//! Transport-agnostic progress callback.
//!
//! Tauri shell adapts this to `AppHandle::emit("download-progress", ...)`.
//! Web shell adapts it to a `tokio::broadcast` that fans out to WebSocket
//! subscribers.

use serde::Serialize;
use std::sync::Arc;

#[derive(Clone, Debug, Serialize)]
pub struct DownloadProgress {
    pub tool_id: String,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub mirror: String,
}

pub type ProgressCallback = Arc<dyn Fn(DownloadProgress) + Send + Sync>;

/// Convenience callback that drops every event. Useful in tests and CLI use.
pub fn noop_progress() -> ProgressCallback {
    Arc::new(|_| {})
}
