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
            pick_and_import_csv_dry_run,
            import_csv_dry_run_with_pending,
            import_csv_commit_pending,
            cancel_pending_import,
            list_attachments,
            attach_file,
            read_attachment,
            delete_attachment,
            delete_account,
            delete_password,
            list_tags,
            create_tag,
            rename_tag,
            delete_tag,
            list_account_tags,
            assign_tag,
            unassign_tag,
            bulk_assign_tag,
            bulk_unassign_tag,
            bulk_delete_accounts,
            list_sites,
            find_or_create_site,
            add_account,
            update_account,
            add_password,
            promote_password,
            recover_candidates,
            reveal_candidate,
            copy_candidate,
            add_era,
            update_era,
            delete_era,
            list_eras,
            list_base_words,
            promote_base_word,
            demote_base_word,
            analyze_base_words,
            get_settings,
            set_setting,
            change_master_password,
            record_recovery_feedback,
            clear_recovery_feedback,
            update_site,
            list_site_merge_groups,
            merge_sites,
            generate_password,
            add_base_word,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
