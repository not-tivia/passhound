use crate::crypto::aead::{self, NONCE_LEN};
use crate::error::{Error, Result};
use crate::repo::common;
use crate::vault::Vault;
use chrono::{DateTime, Utc};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Confidence {
    Certain,
    Uncertain,
}

impl Confidence {
    fn as_str(self) -> &'static str {
        match self {
            Self::Certain => "certain",
            Self::Uncertain => "uncertain",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "uncertain" => Self::Uncertain,
            _ => Self::Certain,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PasswordRecord {
    pub id: i64,
    pub account_id: i64,
    pub created_at: DateTime<Utc>,
    pub retired_at: Option<DateTime<Utc>>,
    pub source: String,
    pub confidence: Confidence,
    pub notes: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewPassword<'a> {
    pub account_id: i64,
    pub plaintext: &'a str,
    pub source: String,
    pub confidence: Confidence,
    pub notes: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
}

/// Insert a new password row. Encrypts plaintext under the vault's master key.
pub fn insert(vault: &Vault, new: NewPassword<'_>) -> Result<PasswordRecord> {
    let key = vault.require_key()?;
    if new.plaintext.is_empty() {
        return Err(Error::InvalidInput("password must not be empty".into()));
    }
    let (ct, nonce) = aead::encrypt(key.as_bytes(), new.plaintext.as_bytes())?;
    let created_at = new.created_at.unwrap_or_else(Utc::now);
    vault.conn().execute(
        "INSERT INTO password_history
            (account_id, password_encrypted, password_nonce, created_at, source, confidence, notes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            new.account_id,
            ct,
            nonce.as_slice(),
            created_at.to_rfc3339(),
            new.source,
            new.confidence.as_str(),
            new.notes,
        ],
    )?;
    let id = vault.conn().last_insert_rowid();
    Ok(PasswordRecord {
        id,
        account_id: new.account_id,
        created_at,
        retired_at: None,
        source: new.source,
        confidence: new.confidence,
        notes: new.notes,
    })
}

/// Mark a password as retired (no longer current).
pub fn retire(vault: &Vault, id: i64, when: DateTime<Utc>) -> Result<()> {
    let n = vault.conn().execute(
        "UPDATE password_history SET retired_at = ?1 WHERE id = ?2 AND retired_at IS NULL",
        params![when.to_rfc3339(), id],
    )?;
    common::ensure_affected(n)
}

/// Promote a history row to current. Retires any existing current for that account
/// in the same transaction so the schema invariant (at most one row with
/// retired_at IS NULL per account) is preserved.
pub fn promote(vault: &Vault, history_id: i64) -> Result<()> {
    let tx = vault.conn().unchecked_transaction()?;
    let account_id: i64 = tx.query_row(
        "SELECT account_id FROM password_history WHERE id = ?1",
        params![history_id],
        |row| row.get(0),
    ).map_err(common::not_found_or_db)?;

    tx.execute(
        "UPDATE password_history SET retired_at = ?1
         WHERE account_id = ?2 AND retired_at IS NULL AND id != ?3",
        params![Utc::now().to_rfc3339(), account_id, history_id],
    )?;

    tx.execute(
        "UPDATE password_history SET retired_at = NULL WHERE id = ?1",
        params![history_id],
    )?;

    tx.commit()?;
    Ok(())
}

/// Hard-delete a single password history row by id.
/// Returns `Error::NotFound` if no row with that id exists.
pub fn delete(vault: &Vault, id: i64) -> Result<()> {
    let n = vault.conn().execute(
        "DELETE FROM password_history WHERE id = ?1",
        params![id],
    )?;
    common::ensure_affected(n)
}

/// Return the current (non-retired) password's plaintext for an account, if any.
pub fn current_plaintext(vault: &Vault, account_id: i64) -> Result<Option<Zeroizing<String>>> {
    let key = vault.require_key()?;
    let row = match vault.conn().query_row(
        "SELECT password_encrypted, password_nonce FROM password_history
         WHERE account_id = ?1 AND retired_at IS NULL
         ORDER BY created_at DESC LIMIT 1",
        params![account_id],
        |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?)),
    ) {
        Ok(t) => Some(t),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(other) => return Err(Error::from(other)),
    };
    let Some((ct, nonce_vec)) = row else { return Ok(None); };
    if nonce_vec.len() != NONCE_LEN {
        return Err(Error::InvalidInput("malformed nonce".into()));
    }
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&nonce_vec);
    let pt = Zeroizing::new(aead::decrypt(key.as_bytes(), &ct, &nonce)?);
    let s = std::str::from_utf8(&pt).map_err(|_| Error::InvalidInput("non-utf8 password".into()))?;
    Ok(Some(Zeroizing::new(s.to_owned())))
}

/// Set a new current password for an account, retiring any previous current.
/// Returns the new record. The retire+insert pair is atomic: a partial
/// failure cannot leave the account with no current password.
pub fn set_current(vault: &Vault, account_id: i64, plaintext: &str, source: &str) -> Result<PasswordRecord> {
    let now = Utc::now();
    let tx = vault.conn().unchecked_transaction()?;
    // Retire whatever's current (silent if none).
    tx.execute(
        "UPDATE password_history SET retired_at = ?1
         WHERE account_id = ?2 AND retired_at IS NULL",
        params![now.to_rfc3339(), account_id],
    )?;
    let record = insert(vault, NewPassword {
        account_id,
        plaintext,
        source: source.into(),
        confidence: Confidence::Certain,
        notes: None,
        created_at: Some(now),
    })?;
    tx.commit()?;
    Ok(record)
}

/// List all password records for an account, newest `created_at` first.
/// Does NOT decrypt; returns metadata only. Use `current_plaintext` or `decrypt_record`
/// for the actual passwords.
pub fn list_history(vault: &Vault, account_id: i64) -> Result<Vec<PasswordRecord>> {
    let mut stmt = vault.conn().prepare(
        "SELECT id, account_id, created_at, retired_at, source, confidence, notes
         FROM password_history WHERE account_id = ?1 ORDER BY created_at DESC",
    )?;
    let rows = stmt
        .query_map(params![account_id], |row| {
            let created_str: String = row.get(2)?;
            let retired_str: Option<String> = row.get(3)?;
            let confidence_str: String = row.get(5)?;
            Ok(PasswordRecord {
                id: row.get(0)?,
                account_id: row.get(1)?,
                created_at: DateTime::parse_from_rfc3339(&created_str)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                retired_at: retired_str.and_then(|s| {
                    DateTime::parse_from_rfc3339(&s).ok().map(|d| d.with_timezone(&Utc))
                }),
                source: row.get(4)?,
                confidence: Confidence::parse(&confidence_str),
                notes: row.get(6)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Decrypt a specific record by id. Vault must be unlocked.
pub fn decrypt_record(vault: &Vault, id: i64) -> Result<Zeroizing<String>> {
    let key = vault.require_key()?;
    let (ct, nonce_vec): (Vec<u8>, Vec<u8>) = vault.conn().query_row(
        "SELECT password_encrypted, password_nonce FROM password_history WHERE id = ?1",
        params![id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    ).map_err(common::not_found_or_db)?;
    if nonce_vec.len() != NONCE_LEN {
        return Err(Error::InvalidInput("malformed nonce".into()));
    }
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&nonce_vec);
    let pt = Zeroizing::new(aead::decrypt(key.as_bytes(), &ct, &nonce)?);
    let s = std::str::from_utf8(&pt).map_err(|_| Error::InvalidInput("non-utf8 password".into()))?;
    Ok(Zeroizing::new(s.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::accounts::{self, NewAccount};
    use crate::repo::sites::{self, NewSite};
    use tempfile::TempDir;

    fn setup() -> (TempDir, Vault, i64) {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("v.db");
        let v = Vault::create(&path, b"hunter2").unwrap();
        let s = sites::create(&v, NewSite { name: "RS".into(), ..Default::default() }).unwrap();
        let a = accounts::create(&v, NewAccount { site_id: s.id, ..Default::default() }).unwrap();
        (tmp, v, a.id)
    }

    #[test]
    fn insert_then_decrypt_round_trip() {
        let (_t, v, aid) = setup();
        insert(&v, NewPassword {
            account_id: aid,
            plaintext: "Fluffy!2014",
            source: "manual".into(),
            confidence: Confidence::Certain,
            notes: None,
            created_at: None,
        }).unwrap();
        let pt = current_plaintext(&v, aid).unwrap().unwrap();
        assert_eq!(pt.as_str(), "Fluffy!2014");
    }

    #[test]
    fn set_current_retires_previous() {
        let (_t, v, aid) = setup();
        set_current(&v, aid, "old", "manual").unwrap();
        set_current(&v, aid, "new", "manual").unwrap();
        let pt = current_plaintext(&v, aid).unwrap().unwrap();
        assert_eq!(pt.as_str(), "new");
        let count: i64 = v.conn().query_row(
            "SELECT COUNT(*) FROM password_history WHERE account_id = ?1 AND retired_at IS NULL",
            params![aid],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(count, 1);
        let total: i64 = v.conn().query_row(
            "SELECT COUNT(*) FROM password_history WHERE account_id = ?1",
            params![aid],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(total, 2);
    }

    #[test]
    fn current_plaintext_returns_none_for_no_passwords() {
        let (_t, v, aid) = setup();
        let pt = current_plaintext(&v, aid).unwrap();
        assert!(pt.is_none());
    }

    #[test]
    fn insert_requires_unlocked_vault() {
        let (_t, mut v, aid) = setup();
        v.lock();
        let err = insert(&v, NewPassword {
            account_id: aid,
            plaintext: "x",
            source: "manual".into(),
            confidence: Confidence::Certain,
            notes: None,
            created_at: None,
        }).unwrap_err();
        assert!(matches!(err, Error::Locked));
    }

    #[test]
    fn insert_rejects_empty_plaintext() {
        let (_t, v, aid) = setup();
        let err = insert(&v, NewPassword {
            account_id: aid,
            plaintext: "",
            source: "manual".into(),
            confidence: Confidence::Certain,
            notes: None,
            created_at: None,
        }).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[test]
    fn list_history_returns_all_records_newest_first() {
        let (_t, v, aid) = setup();
        set_current(&v, aid, "p1", "manual").unwrap();
        set_current(&v, aid, "p2", "manual").unwrap();
        set_current(&v, aid, "p3", "manual").unwrap();
        let records = list_history(&v, aid).unwrap();
        assert_eq!(records.len(), 3);
        // Newest first: p3 (current, retired_at=None) is record[0].
        assert!(records[0].retired_at.is_none());
        assert!(records[1].retired_at.is_some());
        assert!(records[2].retired_at.is_some());
    }

    #[test]
    fn promote_retires_previous_current_and_clears_chosen() {
        let (_t, v, aid) = setup();
        let p1 = insert(&v, NewPassword {
            account_id: aid,
            plaintext: "old",
            source: "manual".into(),
            confidence: Confidence::Certain,
            notes: None,
            created_at: None,
        }).unwrap();
        set_current(&v, aid, "new", "manual").unwrap();

        promote(&v, p1.id).unwrap();

        let hist = list_history(&v, aid).unwrap();
        let p1_row = hist.iter().find(|h| h.id == p1.id).unwrap();
        assert!(p1_row.retired_at.is_none(), "promoted row should have retired_at cleared");
        let other = hist.iter().find(|h| h.id != p1.id).unwrap();
        assert!(other.retired_at.is_some(), "previous current should now be retired");
    }

    #[test]
    fn delete_removes_one_history_row() {
        let (_t, v, aid) = setup();
        let p1 = insert(&v, NewPassword {
            account_id: aid,
            plaintext: "alpha",
            source: "manual".into(),
            confidence: Confidence::Certain,
            notes: None,
            created_at: None,
        }).unwrap();
        let _p2 = insert(&v, NewPassword {
            account_id: aid,
            plaintext: "beta",
            source: "manual".into(),
            confidence: Confidence::Certain,
            notes: None,
            created_at: None,
        }).unwrap();

        delete(&v, p1.id).unwrap();

        let remaining = list_history(&v, aid).unwrap();
        assert_eq!(remaining.len(), 1, "only one history row should remain");
        assert!(matches!(delete(&v, p1.id), Err(crate::error::Error::NotFound)));
    }
}
