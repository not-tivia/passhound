use crate::crypto::{kdf, key::MasterKey};
use crate::error::{Error, Result};
use crate::schema;
use rusqlite::{params, Connection};
use std::fmt;
use std::path::{Path, PathBuf};

const KDF_VERIFIER_PLAINTEXT: &[u8] = b"passhound-vault-v1";

const META_SALT: &str = "salt";
const META_VERIFIER_CT: &str = "verifier_ct";
const META_VERIFIER_NONCE: &str = "verifier_nonce";

/// An open vault. May be locked (no key) or unlocked (key in memory).
pub struct Vault {
    conn: Connection,
    path: PathBuf,
    key: Option<MasterKey>,
}

impl fmt::Debug for Vault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Vault")
            .field("path", &self.path)
            .field("unlocked", &self.key.is_some())
            .finish()
    }
}

impl Vault {
    /// Create a new vault file at `path`. Fails if the file exists.
    pub fn create(path: impl AsRef<Path>, password: &[u8]) -> Result<Self> {
        let path = path.as_ref();
        if path.exists() {
            return Err(Error::AlreadyExists);
        }
        let mut conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;

        let salt = kdf::generate_salt();
        let key_bytes = kdf::derive_key(password, &salt)?;
        let key = MasterKey::new(key_bytes);

        // Store the salt and a verifier blob (encrypted known plaintext) so we can
        // detect wrong passwords on later opens without leaking anything useful.
        let (verifier_ct, verifier_nonce) =
            crate::crypto::aead::encrypt(key.as_bytes(), KDF_VERIFIER_PLAINTEXT)?;

        // Apply schema and persist meta atomically — a crash mid-create must not
        // leave a half-built vault that Vault::open can never read.
        let tx = conn.transaction()?;
        schema::apply_initial(&tx)?;
        tx.execute(
            "INSERT INTO vault_meta (key, value) VALUES (?1, ?2)",
            params![META_SALT, salt.as_slice()],
        )?;
        tx.execute(
            "INSERT INTO vault_meta (key, value) VALUES (?1, ?2)",
            params![META_VERIFIER_CT, verifier_ct],
        )?;
        tx.execute(
            "INSERT INTO vault_meta (key, value) VALUES (?1, ?2)",
            params![META_VERIFIER_NONCE, verifier_nonce.as_slice()],
        )?;
        tx.commit()?;

        Ok(Self { conn, path: path.into(), key: Some(key) })
    }

    /// Open an existing vault file. Returns a *locked* vault — call `unlock` next.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(Error::NotFound);
        }
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(Self { conn, path: path.into(), key: None })
    }

    /// Unlock with the master password. Verifies via the stored verifier blob.
    pub fn unlock(&mut self, password: &[u8]) -> Result<()> {
        let salt: Vec<u8> = self.conn.query_row(
            "SELECT value FROM vault_meta WHERE key = ?1",
            params![META_SALT],
            |r| r.get(0),
        )?;
        let verifier_ct: Vec<u8> = self.conn.query_row(
            "SELECT value FROM vault_meta WHERE key = ?1",
            params![META_VERIFIER_CT],
            |r| r.get(0),
        )?;
        let verifier_nonce_vec: Vec<u8> = self.conn.query_row(
            "SELECT value FROM vault_meta WHERE key = ?1",
            params![META_VERIFIER_NONCE],
            |r| r.get(0),
        )?;
        let mut verifier_nonce = [0u8; crate::crypto::aead::NONCE_LEN];
        if verifier_nonce_vec.len() != verifier_nonce.len() {
            return Err(Error::InvalidInput("malformed verifier nonce".into()));
        }
        verifier_nonce.copy_from_slice(&verifier_nonce_vec);

        let key_bytes = kdf::derive_key(password, &salt)?;
        let candidate = MasterKey::new(key_bytes);

        // If the password is wrong, decrypt fails -> Error::Aead.
        let pt = crate::crypto::aead::decrypt(candidate.as_bytes(), &verifier_ct, &verifier_nonce)?;
        if pt != KDF_VERIFIER_PLAINTEXT {
            return Err(Error::Aead);
        }
        self.key = Some(candidate);
        Ok(())
    }

    /// Drop the master key from memory.
    pub fn lock(&mut self) {
        self.key = None; // MasterKey is ZeroizeOnDrop.
    }

    pub fn path(&self) -> &Path { &self.path }
    pub fn is_unlocked(&self) -> bool { self.key.is_some() }

    /// Internal helper: returns the key or `Error::Locked`.
    pub(crate) fn require_key(&self) -> Result<&MasterKey> {
        self.key.as_ref().ok_or(Error::Locked)
    }

    pub(crate) fn conn(&self) -> &Connection { &self.conn }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn create_writes_file_and_unlocked_state() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("vault.db");
        let vault = Vault::create(&path, b"hunter2").unwrap();
        assert!(path.exists());
        assert!(vault.is_unlocked());
    }

    #[test]
    fn create_fails_if_path_exists() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("vault.db");
        Vault::create(&path, b"hunter2").unwrap();
        let err = Vault::create(&path, b"hunter2").unwrap_err();
        assert!(matches!(err, Error::AlreadyExists));
    }

    #[test]
    fn create_persists_salt_and_verifier() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("vault.db");
        Vault::create(&path, b"hunter2").unwrap();
        let conn = Connection::open(&path).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM vault_meta WHERE key IN ('salt','verifier_ct','verifier_nonce')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 3);
    }

    #[test]
    fn open_starts_locked() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("vault.db");
        Vault::create(&path, b"hunter2").unwrap();
        let v = Vault::open(&path).unwrap();
        assert!(!v.is_unlocked());
    }

    #[test]
    fn unlock_with_correct_password_succeeds() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("vault.db");
        Vault::create(&path, b"hunter2").unwrap();
        let mut v = Vault::open(&path).unwrap();
        v.unlock(b"hunter2").unwrap();
        assert!(v.is_unlocked());
    }

    #[test]
    fn unlock_with_wrong_password_fails() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("vault.db");
        Vault::create(&path, b"hunter2").unwrap();
        let mut v = Vault::open(&path).unwrap();
        let err = v.unlock(b"WRONG").unwrap_err();
        assert!(matches!(err, Error::Aead));
        assert!(!v.is_unlocked());
    }

    #[test]
    fn lock_clears_key() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("vault.db");
        let mut v = Vault::create(&path, b"hunter2").unwrap();
        assert!(v.is_unlocked());
        v.lock();
        assert!(!v.is_unlocked());
    }
}
