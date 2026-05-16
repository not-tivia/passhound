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
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(path, perms)?;
        }
        conn.execute_batch(
            "PRAGMA foreign_keys = ON; \
             PRAGMA synchronous = FULL; \
             PRAGMA secure_delete = ON;"
        )?;

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
        schema::apply_migrations(&tx)?;
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
        conn.execute_batch(
            "PRAGMA foreign_keys = ON; \
             PRAGMA synchronous = FULL; \
             PRAGMA secure_delete = ON;"
        )?;
        let tx = conn.unchecked_transaction()?;
        schema::apply_migrations(&tx)?;
        tx.commit()?;
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
    pub fn require_key(&self) -> Result<&MasterKey> {
        self.key.as_ref().ok_or(Error::Locked)
    }

    pub fn conn(&self) -> &Connection { &self.conn }

    /// Re-derive the vault key under `new_pw` and re-encrypt every encrypted
    /// blob in the vault, atomically. On success, the in-memory MasterKey is
    /// swapped to the new key and vault_meta is updated with a fresh salt +
    /// verifier_ct + verifier_nonce.
    ///
    /// `current_pw` is verified against the existing verifier BEFORE any
    /// writes; mismatch returns `Error::Aead` (which `From<core::Error> for
    /// GuiError` maps to `WrongPassword`).
    ///
    /// Any failure during re-encryption rolls back the transaction; the vault
    /// stays on `current_pw` and the in-memory key is unchanged.
    pub fn change_master_password(
        &mut self,
        current_pw: &[u8],
        new_pw: &[u8],
    ) -> Result<()> {
        // Step 1: Verify the current password.
        let old_salt: Vec<u8> = self.conn.query_row(
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

        let old_key_bytes = kdf::derive_key(current_pw, &old_salt)?;
        let old_key = MasterKey::new(old_key_bytes);
        let pt = crate::crypto::aead::decrypt(old_key.as_bytes(), &verifier_ct, &verifier_nonce)?;
        if pt != KDF_VERIFIER_PLAINTEXT {
            return Err(Error::Aead);
        }

        // Step 2: Derive the new key under a fresh salt.
        let new_salt = kdf::generate_salt();
        let new_key_bytes = kdf::derive_key(new_pw, &new_salt)?;
        let new_key = MasterKey::new(new_key_bytes);

        // Step 3-7: Re-encrypt every blob in a single transaction.
        let tx = self.conn.unchecked_transaction()?;

        // Step 4: password_history
        let rows: Vec<(i64, Vec<u8>, Vec<u8>)> = tx
            .prepare("SELECT id, password_encrypted, password_nonce FROM password_history")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for (id, ct, nonce_vec) in rows {
            if nonce_vec.len() != crate::crypto::aead::NONCE_LEN {
                return Err(Error::InvalidInput("malformed password nonce".into()));
            }
            let mut nonce = [0u8; crate::crypto::aead::NONCE_LEN];
            nonce.copy_from_slice(&nonce_vec);
            let plaintext = crate::crypto::aead::decrypt(old_key.as_bytes(), &ct, &nonce)?;
            let (new_ct, new_nonce) = crate::crypto::aead::encrypt(new_key.as_bytes(), &plaintext)?;
            tx.execute(
                "UPDATE password_history SET password_encrypted = ?1, password_nonce = ?2 WHERE id = ?3",
                params![new_ct, new_nonce.as_slice(), id],
            )?;
        }

        // Step 5: base_words
        let rows: Vec<(i64, Vec<u8>, Vec<u8>)> = tx
            .prepare("SELECT id, word_encrypted, word_nonce FROM base_words")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for (id, ct, nonce_vec) in rows {
            if nonce_vec.len() != crate::crypto::aead::NONCE_LEN {
                return Err(Error::InvalidInput("malformed base_word nonce".into()));
            }
            let mut nonce = [0u8; crate::crypto::aead::NONCE_LEN];
            nonce.copy_from_slice(&nonce_vec);
            let plaintext = crate::crypto::aead::decrypt(old_key.as_bytes(), &ct, &nonce)?;
            let (new_ct, new_nonce) = crate::crypto::aead::encrypt(new_key.as_bytes(), &plaintext)?;
            tx.execute(
                "UPDATE base_words SET word_encrypted = ?1, word_nonce = ?2 WHERE id = ?3",
                params![new_ct, new_nonce.as_slice(), id],
            )?;
        }

        // Step 6: attachments
        let rows: Vec<(i64, Vec<u8>, Vec<u8>)> = tx
            .prepare("SELECT id, blob_encrypted, blob_nonce FROM attachments")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for (id, ct, nonce_vec) in rows {
            if nonce_vec.len() != crate::crypto::aead::NONCE_LEN {
                return Err(Error::InvalidInput("malformed attachment nonce".into()));
            }
            let mut nonce = [0u8; crate::crypto::aead::NONCE_LEN];
            nonce.copy_from_slice(&nonce_vec);
            let plaintext = crate::crypto::aead::decrypt(old_key.as_bytes(), &ct, &nonce)?;
            let (new_ct, new_nonce) = crate::crypto::aead::encrypt(new_key.as_bytes(), &plaintext)?;
            tx.execute(
                "UPDATE attachments SET blob_encrypted = ?1, blob_nonce = ?2 WHERE id = ?3",
                params![new_ct, new_nonce.as_slice(), id],
            )?;
        }

        // Step 7: re-encrypt verifier + rewrite meta.
        let (new_verifier_ct, new_verifier_nonce) =
            crate::crypto::aead::encrypt(new_key.as_bytes(), KDF_VERIFIER_PLAINTEXT)?;
        tx.execute(
            "UPDATE vault_meta SET value = ?1 WHERE key = ?2",
            params![new_salt.as_slice(), META_SALT],
        )?;
        tx.execute(
            "UPDATE vault_meta SET value = ?1 WHERE key = ?2",
            params![new_verifier_ct, META_VERIFIER_CT],
        )?;
        tx.execute(
            "UPDATE vault_meta SET value = ?1 WHERE key = ?2",
            params![new_verifier_nonce.as_slice(), META_VERIFIER_NONCE],
        )?;

        // Step 8: commit.
        tx.commit()?;

        // Step 9: swap in-memory key. (The old MasterKey drops here.)
        self.key = Some(new_key);

        Ok(())
    }
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

    #[test]
    fn change_master_password_round_trip() {
        use crate::repo::{accounts, base_words, passwords, sites};

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("vault.db");

        // Seed a vault with content: a site, an account with one password,
        // and a base_word.
        {
            let v = Vault::create(&path, b"old").unwrap();
            let s = sites::create(&v, sites::NewSite {
                name: "Reddit".into(),
                ..Default::default()
            }).unwrap();
            let a = accounts::create(&v, accounts::NewAccount {
                site_id: s.id,
                ..Default::default()
            }).unwrap();
            passwords::set_current(&v, a.id, "MoonBeam$2019", "manual").unwrap();
            base_words::upsert_aggregated(&v, base_words::AggregatedToken {
                word: "moonbeam",
                usage_count: 1,
                first_seen_at: chrono::Utc::now(),
                last_seen_at: chrono::Utc::now(),
                casing_mask: 0,
            }).unwrap();
        }

        // Change the password.
        {
            let mut v = Vault::open(&path).unwrap();
            v.unlock(b"old").unwrap();
            v.change_master_password(b"old", b"new").unwrap();
        }

        // Reopen with the NEW password; assert content intact.
        let mut v = Vault::open(&path).unwrap();
        assert!(v.unlock(b"old").is_err(), "old password must no longer unlock");
        let mut v = Vault::open(&path).unwrap();
        v.unlock(b"new").unwrap();

        let sites_list = sites::list(&v).unwrap();
        assert_eq!(sites_list.len(), 1);
        assert_eq!(sites_list[0].name, "Reddit");

        // Verify password decrypts. Use the existing repo helper.
        let acct = accounts::list_all(&v, &[]).unwrap()[0].clone();
        let current = passwords::current_plaintext(&v, acct.id).unwrap().unwrap();
        assert_eq!(current.as_str(), "MoonBeam$2019");

        // Verify base_word decrypts.
        let words = base_words::fetch_decrypted(&v).unwrap();
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].word.as_str(), "moonbeam");
    }

    #[test]
    fn change_master_password_rejects_wrong_current() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("vault.db");
        {
            let _ = Vault::create(&path, b"actual").unwrap();
        }

        let mut v = Vault::open(&path).unwrap();
        v.unlock(b"actual").unwrap();
        let err = v.change_master_password(b"wrong", b"new").unwrap_err();
        assert!(matches!(err, Error::Aead), "expected Error::Aead, got {:?}", err);

        // Vault is still on the original password.
        let mut v = Vault::open(&path).unwrap();
        v.unlock(b"actual").unwrap();
        assert!(v.is_unlocked());
        assert!(Vault::open(&path).unwrap().unlock(b"new").is_err());
    }

    #[test]
    fn change_master_password_atomic_under_failure() {
        use crate::repo::{accounts, passwords, sites};

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("vault.db");

        // Seed a vault with one valid password row.
        {
            let v = Vault::create(&path, b"old").unwrap();
            let s = sites::create(&v, sites::NewSite {
                name: "Site".into(),
                ..Default::default()
            }).unwrap();
            let a = accounts::create(&v, accounts::NewAccount {
                site_id: s.id,
                ..Default::default()
            }).unwrap();
            passwords::set_current(&v, a.id, "valid_password", "manual").unwrap();
        }

        // Inject a CORRUPT password_history row directly via SQL. It has valid
        // schema-shape bytes but the ciphertext will not decrypt under the
        // current key. This forces the re-encryption loop to fail at step 4.
        {
            let conn = Connection::open(&path).unwrap();
            // Insert a second password_history row with garbage ciphertext but
            // a correctly-sized nonce. account_id reuses the row above.
            let aid: i64 = conn.query_row("SELECT id FROM accounts LIMIT 1", [], |r| r.get(0)).unwrap();
            conn.execute(
                "INSERT INTO password_history
                   (account_id, password_encrypted, password_nonce, source, confidence, created_at, retired_at, notes)
                 VALUES (?1, ?2, ?3, 'manual', 'certain', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', NULL)",
                params![aid, vec![0u8; 32], vec![0u8; crate::crypto::aead::NONCE_LEN]],
            ).unwrap();
        }

        // Try to change. The corrupt row should cause decrypt to fail; the
        // transaction must roll back leaving the vault on the OLD password.
        let mut v = Vault::open(&path).unwrap();
        v.unlock(b"old").unwrap();
        let err = v.change_master_password(b"old", b"new").unwrap_err();
        assert!(matches!(err, Error::Aead), "expected Error::Aead, got {:?}", err);

        // Reopen with the OLD password -- should still work.
        let mut v = Vault::open(&path).unwrap();
        v.unlock(b"old").unwrap();

        // The valid row's plaintext is still decryptable.
        let acct = accounts::list_all(&v, &[]).unwrap()[0].clone();
        let current = passwords::current_plaintext(&v, acct.id).unwrap().unwrap();
        assert_eq!(current.as_str(), "valid_password");

        // The NEW password does NOT unlock.
        assert!(Vault::open(&path).unwrap().unlock(b"new").is_err());
    }
}
