// Hide the console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use passhound_gui::commands::*;
use passhound_gui::state::VaultState;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(VaultState::new())
        .invoke_handler(tauri::generate_handler![
            vault_exists,
            vault_create,
            vault_unlock,
            vault_lock,
            list_accounts,
            get_account,
            reveal_password,
            copy_to_clipboard,
            import_csv_dry_run,
            import_csv_commit,
            list_attachments,
            attach_file,
            read_attachment,
            delete_attachment,
            delete_account,
            delete_password,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
