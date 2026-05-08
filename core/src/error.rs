use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("argon2: {0}")]
    Argon2(String),

    #[error("aead: encryption or decryption failed (wrong key or tampered ciphertext)")]
    Aead,

    #[error("vault is locked")]
    Locked,

    #[error("vault already exists at this path")]
    AlreadyExists,

    #[error("not found")]
    NotFound,

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("import: {0}")]
    Import(String),

    #[error("import: column mapping required for headers {headers:?}")]
    NeedsColumnMapping { headers: Vec<String> },
}
