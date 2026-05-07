// Prevent extra console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    ai_cli_installer_lib::run()
}
