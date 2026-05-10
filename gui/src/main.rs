// Hide the console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(passhound_gui::state::VaultState::new())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
