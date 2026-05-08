use tauri::{Manager, PhysicalPosition, PhysicalSize};

use installer_core::app_state;

mod commands;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(app_state::shared())
        .setup(|app| {
            configure_main_window(app);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_tools,
            commands::list_mirrors,
            commands::probe_mirrors,
            commands::install_tool,
            commands::check_path_status,
            commands::add_to_path,
            commands::remove_from_path,
            commands::list_claude_presets,
            commands::get_claude_settings,
            commands::apply_claude_preset,
            commands::detect_node,
            commands::list_fixes,
            commands::apply_fixes,
            commands::remove_fixes,
            commands::open_path,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn configure_main_window(app: &tauri::App) {
    // Only auto-size and center on the very first launch. Subsequent launches
    // respect whatever size/position the user (or Tauri's saved state) chose.
    let marker = first_run_marker_path(app);
    if let Some(ref path) = marker {
        if path.exists() {
            return;
        }
    }

    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        return;
    };

    let monitor_size = monitor.size();
    let monitor_position = monitor.position();
    let width = ((monitor_size.width as f64) * 0.6).round().max(600.0) as u32;
    let height = ((monitor_size.height as f64) * 0.6).round().max(480.0) as u32;

    let _ = window.set_size(PhysicalSize::new(width, height));
    let x = monitor_position.x + ((monitor_size.width.saturating_sub(width)) / 2) as i32;
    let y = monitor_position.y + ((monitor_size.height.saturating_sub(height)) / 2) as i32;
    let _ = window.set_position(PhysicalPosition::new(x, y));

    if let Some(path) = marker {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, b"");
    }
}

fn first_run_marker_path(app: &tauri::App) -> Option<std::path::PathBuf> {
    app.path().app_config_dir().ok().map(|dir| dir.join(".window_initialized"))
}
