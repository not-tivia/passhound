use serde::Serialize;
use thiserror::Error;

/// Frontend-facing error type. Serialized over the IPC channel.
#[derive(Debug, Serialize, Error)]
#[serde(tag = "kind", content = "message")]
pub enum GuiError {
    #[error("vault not found")]
    NotFound,
    #[error("vault is locked")]
    Locked,
    #[error("wrong master password")]
    WrongPassword,
    #[error("vault already exists")]
    AlreadyExists,
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("vault has no password history to recover from")]
    EmptyVault,
    #[error("no era named '{0}'")]
    EraNotFound(String),
    #[error("no active recovery — call recover_candidates first")]
    NoActiveRecovery,
    #[error("candidate rank out of bounds")]
    RankOutOfBounds,
    #[error("no pending import — call pick_and_import_csv_dry_run first")]
    NoPendingImport,
    #[error("internal: {0}")]
    Internal(String),
}

impl From<std::io::Error> for GuiError {
    fn from(e: std::io::Error) -> Self {
        GuiError::Internal(format!("io: {e}"))
    }
}

impl From<rusqlite::Error> for GuiError {
    fn from(e: rusqlite::Error) -> Self {
        GuiError::Internal(format!("sqlite: {e}"))
    }
}

impl From<passhound_core::error::Error> for GuiError {
    fn from(e: passhound_core::error::Error) -> Self {
        use passhound_core::error::Error as E;
        match e {
            E::NotFound => GuiError::NotFound,
            E::Locked => GuiError::Locked,
            // core::Error::Aead = "encryption or decryption failed (wrong key
            // or tampered ciphertext)". For the unlock path, the only realistic
            // cause is a wrong master password — surface as WrongPassword.
            E::Aead => GuiError::WrongPassword,
            E::AlreadyExists => GuiError::AlreadyExists,
            E::InvalidInput(s) => GuiError::InvalidInput(s),
            E::EmptyVault => GuiError::EmptyVault,
            E::EraNotFound(name) => GuiError::EraNotFound(name),
            // Catch-all for: Io, Sqlite, Argon2, Import, NeedsColumnMapping.
            // None of these are user-actionable in a distinct way at the GUI
            // level — surface as a generic internal error. Add a specific arm
            // here if a future variant needs its own UI treatment.
            other => GuiError::Internal(other.to_string()),
        }
    }
}
