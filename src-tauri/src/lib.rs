use std::sync::Arc;

mod commands;
mod downloader;
mod error;
mod installer;
mod mirrors;
mod platform;
mod tools;
mod upstream;
mod verifier;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Arc::new(commands::AppState::new()))
        .invoke_handler(tauri::generate_handler![
            commands::list_tools,
            commands::list_mirrors,
            commands::probe_mirrors,
            commands::install_tool,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
