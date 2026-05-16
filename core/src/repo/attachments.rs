//! Per-account file attachments. Bytes are encrypted with the vault's master
//! key (XChaCha20-Poly1305, fresh per-blob nonce). Metadata (filename,
//! mime_type, size, created_at) is plaintext for queryability.
//!
//! 10 MB cap per file. Cascade-deletes with the parent account.

use crate::crypto::aead::{self, NONCE_LEN};
use crate::error::{Error, Result};
use crate::repo::common;
use crate::vault::Vault;
use chrono::{DateTime, Utc};
use rusqlite::params;
use serde::Serialize;
use zeroize::Zeroizing;

const MAX_ATTACHMENT_SIZE: usize = 10 * 1024 * 1024; // 10 MB

#[derive(Debug, Clone, Serialize)]
pub struct AttachmentSummary {
    pub id: i64,
    pub account_id: i64,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewAttachment<'a> {
    pub account_id: i64,
    pub filename: &'a str,
    pub mime_type: &'a str,
    pub bytes: &'a [u8],
}

pub fn insert(vault: &Vault, new: NewAttachment<'_>) -> Result<AttachmentSummary> {
    let key = vault.require_key()?;
    if new.bytes.len() > MAX_ATTACHMENT_SIZE {
        return Err(Error::InvalidInput(
            "attachment too large (max 10 MB; compress first)".into(),
        ));
    }
    let filename = new.filename.trim();
    if filename.is_empty() {
        return Err(Error::InvalidInput("filename required".into()));
    }
    let (ct, nonce) = aead::encrypt(key.as_bytes(), new.bytes)?;
    let created_at = Utc::now();
    let size_bytes = new.bytes.len() as i64;
    vault.conn().execute(
        "INSERT INTO attachments
            (account_id, filename, mime_type, size_bytes, blob_encrypted, blob_nonce, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            new.account_id,
            filename,
            new.mime_type,
            size_bytes,
            ct,
            nonce.as_slice(),
            created_at.to_rfc3339(),
        ],
    )?;
    let id = vault.conn().last_insert_rowid();
    Ok(AttachmentSummary {
        id,
        account_id: new.account_id,
        filename: filename.to_string(),
        mime_type: new.mime_type.to_string(),
        size_bytes,
        created_at,
    })
}

pub fn list_for_account(vault: &Vault, account_id: i64) -> Result<Vec<AttachmentSummary>> {
    let mut stmt = vault.conn().prepare(
        "SELECT id, account_id, filename, mime_type, size_bytes, created_at
         FROM attachments
         WHERE account_id = ?1
         ORDER BY created_at ASC, id ASC",
    )?;
    let rows = stmt
        .query_map(params![account_id], |r| {
            let created_str: String = r.get(5)?;
            let created_at = DateTime::parse_from_rfc3339(&created_str)
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());
            Ok(AttachmentSummary {
                id: r.get(0)?,
                account_id: r.get(1)?,
                filename: r.get(2)?,
                mime_type: r.get(3)?,
                size_bytes: r.get(4)?,
                created_at,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn decrypt(vault: &Vault, id: i64) -> Result<(AttachmentSummary, Zeroizing<Vec<u8>>)> {
    let key = vault.require_key()?;
    let (account_id, filename, mime_type, size_bytes, ct, nonce_vec, created_str):
        (i64, String, String, i64, Vec<u8>, Vec<u8>, String) = vault.conn().query_row(
        "SELECT account_id, filename, mime_type, size_bytes, blob_encrypted, blob_nonce, created_at
         FROM attachments WHERE id = ?1",
        params![id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
    ).map_err(common::not_found_or_db)?;
    if nonce_vec.len() != NONCE_LEN {
        return Err(Error::InvalidInput("malformed attachment nonce".into()));
    }
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&nonce_vec);
    let plaintext = Zeroizing::new(aead::decrypt(key.as_bytes(), &ct, &nonce)?);
    let created_at = DateTime::parse_from_rfc3339(&created_str)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let summary = AttachmentSummary {
        id,
        account_id,
        filename,
        mime_type,
        size_bytes,
        created_at,
    };
    Ok((summary, plaintext))
}

pub fn delete(vault: &Vault, id: i64) -> Result<()> {
    let n = vault.conn().execute(
        "DELETE FROM attachments WHERE id = ?1",
        params![id],
    )?;
    common::ensure_affected(n)
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
        let s = sites::create(&v, NewSite { name: "S".into(), ..Default::default() }).unwrap();
        let a = accounts::create(&v, NewAccount { site_id: s.id, ..Default::default() }).unwrap();
        (tmp, v, a.id)
    }

    #[test]
    fn insert_and_list_round_trip() {
        let (_t, v, acct_id) = setup();
        let bytes = b"hello world";
        let summary = insert(&v, NewAttachment {
            account_id: acct_id,
            filename: "hello.txt",
            mime_type: "text/plain",
            bytes,
        }).unwrap();
        assert_eq!(summary.account_id, acct_id);
        assert_eq!(summary.filename, "hello.txt");
        assert_eq!(summary.mime_type, "text/plain");
        assert_eq!(summary.size_bytes, bytes.len() as i64);

        let list = list_for_account(&v, acct_id).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, summary.id);
        assert_eq!(list[0].filename, "hello.txt");
    }

    #[test]
    fn decrypt_returns_plaintext_bytes() {
        let (_t, v, acct_id) = setup();
        let bytes: Vec<u8> = (0..1024).map(|i| (i % 256) as u8).collect();
        let summary = insert(&v, NewAttachment {
            account_id: acct_id,
            filename: "bin.dat",
            mime_type: "application/octet-stream",
            bytes: &bytes,
        }).unwrap();

        let (got_summary, plaintext) = decrypt(&v, summary.id).unwrap();
        assert_eq!(got_summary.filename, "bin.dat");
        assert_eq!(plaintext.as_slice(), bytes.as_slice());
    }

    #[test]
    fn insert_rejects_oversized() {
        let (_t, v, acct_id) = setup();
        // 10 MB + 1 byte.
        let oversize = vec![0u8; MAX_ATTACHMENT_SIZE + 1];
        let err = insert(&v, NewAttachment {
            account_id: acct_id,
            filename: "big.bin",
            mime_type: "application/octet-stream",
            bytes: &oversize,
        }).unwrap_err();
        match err {
            Error::InvalidInput(msg) => assert!(msg.contains("10 MB"), "got {msg}"),
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    #[test]
    fn insert_rejects_empty_filename() {
        let (_t, v, acct_id) = setup();
        let err = insert(&v, NewAttachment {
            account_id: acct_id,
            filename: "   ",
            mime_type: "text/plain",
            bytes: b"x",
        }).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[test]
    fn cascade_delete_with_account() {
        let (_t, v, acct_id) = setup();
        insert(&v, NewAttachment {
            account_id: acct_id,
            filename: "a.txt",
            mime_type: "text/plain",
            bytes: b"a",
        }).unwrap();
        // Delete the parent account directly via SQL (no public account-delete API yet).
        v.conn().execute("DELETE FROM accounts WHERE id = ?1", params![acct_id]).unwrap();
        let list = list_for_account(&v, acct_id).unwrap();
        assert!(list.is_empty(), "attachments should cascade-delete with account; got {list:?}");
    }
}
