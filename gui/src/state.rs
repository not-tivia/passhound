use std::sync::Mutex;

/// Tauri-managed state. Holds the unlocked vault between commands.
pub struct VaultState {
    pub vault: Mutex<Option<passhound_core::Vault>>,
}

impl VaultState {
    pub fn new() -> Self {
        Self { vault: Mutex::new(None) }
    }
}

impl Default for VaultState {
    fn default() -> Self { Self::new() }
}
