//! Pure-logic core for ai-cli-installer.
//!
//! This crate contains everything that is independent of the runtime shell
//! (Tauri desktop or Axum web server): mirror chain, downloader, verifier,
//! tool specs (Claude Code / Codex), npm-route installer, fixes recipes,
//! claude presets, env-manager, install diagnostics. Both `src-tauri` and
//! `installer-web` consume this crate.
//!
//! Progress events are abstracted through [`progress::ProgressCallback`] so
//! the core never references Tauri's `AppHandle::emit` or any HTTP/WS sink
//! directly — each shell adapts the callback to its own transport.

pub mod app_state;
pub mod downloader;
pub mod env_manager;
pub mod error;
pub mod fixes;
pub mod install_diagnostics;
pub mod installer;
pub mod mirrors;
pub mod npm_installer;
pub mod platform;
pub mod presets;
pub mod progress;
pub mod tools;
pub mod upstream;
pub mod verifier;

pub use app_state::AppState;
pub use error::{AppError, Result};
pub use progress::{noop_progress, DownloadProgress, ProgressCallback};
