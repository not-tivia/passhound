use std::sync::Mutex;
use zeroize::Zeroizing;

/// Tauri-managed state. Holds the unlocked vault plus in-memory caches
/// that must clear on lock.
pub struct VaultState {
    pub vault: Mutex<Option<passhound_core::Vault>>,
    /// Recovery candidates kept in Rust memory so plaintext never crosses
    /// the IPC boundary in bulk. Indexed by `rank - 1`. Cleared on every
    /// new `recover_candidates` call and on lock.
    pub candidate_cache: Mutex<Vec<Zeroizing<String>>>,
    /// Path of the most recent import-dialog selection. Set by
    /// `pick_and_import_csv_dry_run`, taken by `import_csv_commit_pending`,
    /// cleared by `cancel_pending_import` and on lock.
    pub pending_import_path: Mutex<Option<std::path::PathBuf>>,
}

impl VaultState {
    pub fn new() -> Self {
        Self {
            vault: Mutex::new(None),
            candidate_cache: Mutex::new(Vec::new()),
            pending_import_path: Mutex::new(None),
        }
    }
}

impl Default for VaultState {
    fn default() -> Self { Self::new() }
}
