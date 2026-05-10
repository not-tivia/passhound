//! Tauri IPC commands. Each public `#[tauri::command]` function delegates to
//! an `_inner` helper that takes a plain `&VaultState`, so unit tests can
//! exercise the command logic without spinning up the Tauri runtime.

// Filled in Task 2.
