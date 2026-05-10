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
    #[error("internal: {0}")]
    Internal(String),
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
            // Catch-all for: Io, Sqlite, Argon2, Import, NeedsColumnMapping,
            // EmptyVault, EraNotFound. None of these are user-actionable in a
            // distinct way at the GUI level — surface as a generic internal
            // error. Add a specific arm here if a future variant needs its
            // own UI treatment.
            other => GuiError::Internal(other.to_string()),
        }
    }
}
