//! Pipeline: classify imported entries against the vault, then commit / undo.

use super::{ImportEntry, ParseResult, RowPatch};
use crate::error::{Error, Result};
use crate::repo::accounts::{self, NewAccount};
use crate::repo::passwords;
use crate::repo::sites::{self, NewSite};
use crate::vault::Vault;
use chrono::Utc;
use rusqlite::params;
use std::path::Path;

/// A unique id for an import batch (matches the `imports` table's primary key).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportId(pub i64);

/// Classification of a single entry against current vault state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    /// No matching site or no matching account → fully new.
    New,
    /// Matching account whose CURRENT password equals the entry's plaintext.
    DuplicateOfTriple,
    /// Matching account, but current password differs.
    MergeWithNewPassword,
}

/// One classified entry with the resolved site/account ids when present.
#[derive(Debug, Clone)]
pub struct ClassifiedEntry {
    pub entry: ImportEntry,
    pub classification: Classification,
    pub matched_site_id: Option<i64>,
    pub matched_account_id: Option<i64>,
}

/// Aggregate dry-run preview.
#[derive(Debug, Clone, Default)]
pub struct Preview {
    pub items: Vec<ClassifiedEntry>,
    pub new: usize,
    pub duplicates: usize,
    pub merges: usize,
}

/// Classify a batch of import entries against the current vault state.
///
/// Vault must be unlocked (we decrypt matched passwords to detect duplicates).
pub fn preview(vault: &Vault, entries: Vec<ImportEntry>) -> Result<Preview> {
    let _ = vault.require_key()?; // bail early if locked
    let mut preview = Preview::default();

    for entry in entries {
        let matched_site_id = sites::resolve_for_import(vault, &entry.site)?;
        let (classification, site_id, account_id) = match matched_site_id {
            None => (Classification::New, None, None),
            Some(sid) => {
                let accs = accounts::list_for_site(vault, sid, &[])?;
                let target_user = entry.username.as_deref().unwrap_or("");
                let matched_account = accs.iter().find(|a| {
                    a.username.as_deref().unwrap_or("") == target_user
                });
                match matched_account {
                    None => (Classification::New, Some(sid), None),
                    Some(acc) => {
                        let current = passwords::current_plaintext(vault, acc.id)?;
                        match current {
                            None => (Classification::New, Some(sid), Some(acc.id)),
                            Some(pt) if pt.as_str() == entry.password.as_str() => {
                                (Classification::DuplicateOfTriple, Some(sid), Some(acc.id))
                            }
                            Some(_) => {
                                (Classification::MergeWithNewPassword, Some(sid), Some(acc.id))
                            }
                        }
                    }
                }
            }
        };

        match classification {
            Classification::New => preview.new += 1,
            Classification::DuplicateOfTriple => preview.duplicates += 1,
            Classification::MergeWithNewPassword => preview.merges += 1,
        }
        preview.items.push(ClassifiedEntry {
            entry,
            classification,
            matched_site_id: site_id,
            matched_account_id: account_id,
        });
    }

    Ok(preview)
}

/// Build a Preview directly from a [`ParseResult`]'s entries (convenience).
pub fn preview_parse_result(vault: &Vault, parse: ParseResult) -> Result<Preview> {
    preview(vault, parse.entries)
}

/// Write the import. Single transaction. Returns the new `ImportId`.
///
/// Source label appears in the `imports.source` column (e.g., "csv", "paste").
/// Source path is recorded in `imports.file_path` if provided.
pub fn commit(
    vault: &Vault,
    preview: Preview,
    source_label: &str,
    source_path: Option<&Path>,
) -> Result<ImportId> {
    let _ = vault.require_key()?;
    let now = Utc::now();
    let tx = vault.conn().unchecked_transaction()?;

    tx.execute(
        "INSERT INTO imports (source, file_path, imported_at, entries_added, notes)
         VALUES (?1, ?2, ?3, 0, NULL)",
        params![
            source_label,
            source_path.map(|p| p.display().to_string()),
            now.to_rfc3339(),
        ],
    )?;
    let import_id = ImportId(tx.last_insert_rowid());

    let mut entries_added: i64 = 0;
    for item in preview.items {
        match item.classification {
            Classification::DuplicateOfTriple => continue,
            Classification::New => {
                // Re-resolve the site (name -> alias -> canonical) inside the
                // transaction. Preview's matched_site_id was a snapshot taken
                // before the batch ran, so earlier entries in this same CSV may
                // have just created the same site. Without this check we'd
                // create duplicate sites for CSVs with multiple rows per site.
                let site_id = match item.matched_site_id {
                    Some(id) => id,
                    None => match sites::resolve_for_import(vault, &item.entry.site)? {
                        Some(id) => id,
                        None => {
                            let s = sites::create(
                                vault,
                                NewSite {
                                    name: item.entry.site.clone(),
                                    url: item.entry.url.clone(),
                                    category: None,
                                    abbreviations: vec![],
                                    notes: None,
                                },
                            )?;
                            s.id
                        }
                    },
                };
                // Re-check account by (site_id, username) for the same reason.
                // If a prior entry in this batch already created the account,
                // we treat this row as a merge: retire current, insert new.
                let mut treat_as_merge = false;
                let account_id = match item.matched_account_id {
                    Some(id) => id,
                    None => {
                        let accs = accounts::list_for_site(vault, site_id, &[])?;
                        let target_user = item.entry.username.as_deref().unwrap_or("");
                        let existing = accs.iter().find(|a| {
                            a.username.as_deref().unwrap_or("") == target_user
                        });
                        match existing {
                            Some(a) => {
                                treat_as_merge = true;
                                a.id
                            }
                            None => {
                                let a = accounts::create(
                                    vault,
                                    NewAccount {
                                        site_id,
                                        username: item.entry.username.clone(),
                                        display_name: item.entry.display_name.clone(),
                                        alias: None,
                                        notes: None,
                                    },
                                )?;
                                a.id
                            }
                        }
                    }
                };
                if treat_as_merge {
                    // Earlier in this batch we created this account. Retire
                    // its current password before inserting the new one — same
                    // semantics as the Merge classification path below.
                    tx.execute(
                        "UPDATE password_history SET retired_at = ?1
                         WHERE account_id = ?2 AND retired_at IS NULL",
                        params![now.to_rfc3339(), account_id],
                    )?;
                }
                insert_password_with_provenance(
                    &tx,
                    vault,
                    account_id,
                    &item.entry,
                    import_id,
                )?;
                entries_added += 1;
            }
            Classification::MergeWithNewPassword => {
                let account_id = item
                    .matched_account_id
                    .expect("Merge classification implies a matched account");
                tx.execute(
                    "UPDATE password_history SET retired_at = ?1
                     WHERE account_id = ?2 AND retired_at IS NULL",
                    params![now.to_rfc3339(), account_id],
                )?;
                insert_password_with_provenance(
                    &tx,
                    vault,
                    account_id,
                    &item.entry,
                    import_id,
                )?;
                entries_added += 1;
            }
        }
    }

    tx.execute(
        "UPDATE imports SET entries_added = ?1 WHERE id = ?2",
        params![entries_added, import_id.0],
    )?;
    tx.commit()?;
    Ok(import_id)
}

fn insert_password_with_provenance(
    tx: &rusqlite::Transaction<'_>,
    vault: &Vault,
    account_id: i64,
    entry: &ImportEntry,
    import_id: ImportId,
) -> Result<()> {
    let key = vault.require_key()?;
    let (ct, nonce) = crate::crypto::aead::encrypt(key.as_bytes(), entry.password.as_bytes())?;
    let created_at = entry.created_at.unwrap_or_else(Utc::now);
    let auto_note = match entry.notes.as_deref() {
        Some(n) if !n.is_empty() => n.to_string(),
        _ if entry.created_at.is_none() => {
            "imported; original timestamp unknown".to_string()
        }
        _ => String::new(),
    };
    let notes_to_store: Option<String> = if auto_note.is_empty() {
        None
    } else {
        Some(auto_note)
    };

    tx.execute(
        "INSERT INTO password_history
            (account_id, password_encrypted, password_nonce, created_at, source, confidence, notes, source_import_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            account_id,
            ct,
            nonce.as_slice(),
            created_at.to_rfc3339(),
            "import",
            "uncertain",
            notes_to_store,
            import_id.0,
        ],
    )?;
    Ok(())
}

/// Counts of rows deleted by `undo`.
#[derive(Debug, Clone, Copy)]
pub struct UndoCounts {
    pub passwords: i64,
    pub accounts: i64,
    pub sites: i64,
}

/// Reverse a previous import. Deletes the import's password rows, then any
/// orphan accounts (no remaining password_history rows), then any orphan sites
/// (no remaining accounts), then the imports row itself. All in one transaction.
///
/// Errors with `Error::NotFound` if the import id doesn't exist.
pub fn undo(vault: &Vault, import_id: ImportId) -> Result<UndoCounts> {
    let tx = vault.conn().unchecked_transaction()?;

    let pw_deleted = tx.execute(
        "DELETE FROM password_history WHERE source_import_id = ?1",
        params![import_id.0],
    )? as i64;

    let acc_deleted = tx.execute(
        "DELETE FROM accounts WHERE id NOT IN (SELECT DISTINCT account_id FROM password_history)",
        [],
    )? as i64;

    let site_deleted = tx.execute(
        "DELETE FROM sites WHERE id NOT IN (SELECT DISTINCT site_id FROM accounts)",
        [],
    )? as i64;

    let imp_deleted = tx.execute(
        "DELETE FROM imports WHERE id = ?1",
        params![import_id.0],
    )?;
    if imp_deleted == 0 {
        return Err(Error::NotFound);
    }

    tx.commit()?;
    Ok(UndoCounts {
        passwords: pw_deleted,
        accounts: acc_deleted,
        sites: site_deleted,
    })
}

/// Stitch user-supplied patches into a ParseResult: for each diagnostic
/// whose row appears in the patches and where the patch supplies all
/// still-missing required fields (site + password), the diagnostic is
/// promoted to a full ImportEntry. Whitespace-only patches and orphan
/// patches (referencing rows not in current diagnostics) are silently
/// dropped.
pub fn apply_patches(
    mut result: ParseResult,
    patches: &[RowPatch],
) -> ParseResult {
    use std::collections::HashMap;
    let patch_by_row: HashMap<usize, &RowPatch> =
        patches.iter().map(|p| (p.row, p)).collect();

    let mut kept_diagnostics = Vec::new();
    for d in result.diagnostics.drain(..) {
        let Some(p) = patch_by_row.get(&d.row) else {
            kept_diagnostics.push(d);
            continue;
        };

        // Resolve required fields: patch first (trimmed, non-empty),
        // falling back to whatever the parser already extracted.
        let site = p
            .site
            .as_deref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| d.parsed.site.clone());
        let password = p
            .password
            .as_deref()
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .map(zeroize::Zeroizing::new)
            .or_else(|| d.parsed.password.clone());

        let (Some(site), Some(password)) = (site, password) else {
            kept_diagnostics.push(d);
            continue;
        };

        result.entries.push(ImportEntry {
            site,
            url: d.parsed.url.clone(),
            username: d.parsed.username.clone(),
            display_name: d.parsed.display_name.clone(),
            password,
            notes: d.parsed.notes.clone(),
            created_at: d.parsed.created_at,
            source_row: d.parsed.source_row.clone(),
        });
    }
    result.diagnostics = kept_diagnostics;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::{accounts::NewAccount, passwords::{Confidence, NewPassword}, sites::NewSite};
    use tempfile::TempDir;

    fn vault() -> (TempDir, Vault) {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("v.db");
        let v = Vault::create(&path, b"hunter2").unwrap();
        (tmp, v)
    }

    fn seed_site_account_password(v: &Vault, site: &str, user: &str, pw: &str) -> i64 {
        let s = sites::create(v, NewSite { name: site.into(), ..Default::default() }).unwrap();
        let a = accounts::create(v, NewAccount { site_id: s.id, username: Some(user.into()), ..Default::default() }).unwrap();
        passwords::insert(v, NewPassword {
            account_id: a.id,
            plaintext: pw,
            source: "seed".into(),
            confidence: Confidence::Certain,
            notes: None,
            created_at: None,
        }).unwrap();
        a.id
    }

    fn entry(site: &str, user: Option<&str>, pw: &str) -> ImportEntry {
        ImportEntry {
            site: site.into(),
            url: None,
            username: user.map(|u| u.to_string()),
            display_name: None,
            password: zeroize::Zeroizing::new(pw.to_string()),
            created_at: None,
            notes: None,
            source_row: None,
        }
    }

    #[test]
    fn preview_classifies_new_duplicate_merge() {
        let (_t, v) = vault();
        let _aid = seed_site_account_password(&v, "RuneScape", "chris", "Fluffy!2014");

        let p = preview(&v, vec![
            entry("RuneScape", Some("chris"), "Fluffy!2014"),  // duplicate of triple
            entry("RuneScape", Some("chris"), "NewerPass"),    // merge
            entry("Amazon", Some("chris"), "AmzPass"),         // new (new site)
        ]).unwrap();

        assert_eq!(p.items.len(), 3);
        assert_eq!(p.duplicates, 1);
        assert_eq!(p.merges, 1);
        assert_eq!(p.new, 1);
        assert!(matches!(p.items[0].classification, Classification::DuplicateOfTriple));
        assert!(matches!(p.items[1].classification, Classification::MergeWithNewPassword));
        assert!(matches!(p.items[2].classification, Classification::New));
    }

    #[test]
    fn preview_requires_unlocked_vault() {
        let (_t, mut v) = vault();
        v.lock();
        let err = preview(&v, vec![entry("Foo", None, "pw")]).unwrap_err();
        assert!(matches!(err, Error::Locked));
    }

    #[test]
    fn preview_treats_missing_username_as_empty_string() {
        let (_t, v) = vault();
        // Seed an account with NO username (None); attempt match with None username.
        let s = sites::create(&v, NewSite { name: "Foo".into(), ..Default::default() }).unwrap();
        let a = accounts::create(&v, NewAccount { site_id: s.id, username: None, ..Default::default() }).unwrap();
        passwords::insert(&v, NewPassword {
            account_id: a.id,
            plaintext: "pw",
            source: "seed".into(),
            confidence: Confidence::Certain,
            notes: None,
            created_at: None,
        }).unwrap();

        let p = preview(&v, vec![entry("Foo", None, "pw")]).unwrap();
        assert_eq!(p.duplicates, 1);
    }

    #[test]
    fn commit_writes_imports_row_and_increments_entries_added() {
        let (_t, v) = vault();
        let p = preview(&v, vec![entry("Foo", Some("u"), "pw")]).unwrap();
        let id = commit(&v, p, "test", None).unwrap();
        let (source, entries_added): (String, i64) = v.conn().query_row(
            "SELECT source, entries_added FROM imports WHERE id = ?1",
            params![id.0],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
        ).unwrap();
        assert_eq!(source, "test");
        assert_eq!(entries_added, 1);
    }

    #[test]
    fn commit_assigns_source_import_id_on_inserted_passwords() {
        let (_t, v) = vault();
        let p = preview(&v, vec![entry("Foo", Some("u"), "pw")]).unwrap();
        let id = commit(&v, p, "test", None).unwrap();
        let count: i64 = v.conn().query_row(
            "SELECT COUNT(*) FROM password_history WHERE source_import_id = ?1",
            params![id.0],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn commit_skips_duplicate_classifications() {
        let (_t, v) = vault();
        seed_site_account_password(&v, "Foo", "u", "pw");
        let p = preview(&v, vec![entry("Foo", Some("u"), "pw")]).unwrap();
        assert_eq!(p.duplicates, 1);
        let id = commit(&v, p, "test", None).unwrap();
        let count_for_import: i64 = v.conn().query_row(
            "SELECT COUNT(*) FROM password_history WHERE source_import_id = ?1",
            params![id.0],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(count_for_import, 0);
        let entries_added: i64 = v.conn().query_row(
            "SELECT entries_added FROM imports WHERE id = ?1",
            params![id.0],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(entries_added, 0);
    }

    #[test]
    fn commit_retires_previous_current_for_merges() {
        let (_t, v) = vault();
        seed_site_account_password(&v, "Foo", "u", "old");
        let p = preview(&v, vec![entry("Foo", Some("u"), "new")]).unwrap();
        assert_eq!(p.merges, 1);
        commit(&v, p, "test", None).unwrap();

        let current_count: i64 = v.conn().query_row(
            "SELECT COUNT(*) FROM password_history
             WHERE account_id = (SELECT id FROM accounts WHERE site_id = (SELECT id FROM sites WHERE name='Foo'))
             AND retired_at IS NULL",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(current_count, 1);
        // Verify the new current decrypts to "new".
        let aid: i64 = v.conn().query_row(
            "SELECT id FROM accounts WHERE site_id = (SELECT id FROM sites WHERE name='Foo')",
            [],
            |r| r.get(0),
        ).unwrap();
        let pt = passwords::current_plaintext(&v, aid).unwrap().unwrap();
        assert_eq!(pt.as_str(), "new");
    }

    #[test]
    fn commit_inserts_inferred_timestamp_note_when_created_at_absent() {
        let (_t, v) = vault();
        let mut e = entry("Foo", Some("u"), "pw");
        e.created_at = None;
        let p = preview(&v, vec![e]).unwrap();
        let id = commit(&v, p, "test", None).unwrap();
        let notes: Option<String> = v.conn().query_row(
            "SELECT notes FROM password_history WHERE source_import_id = ?1",
            params![id.0],
            |r| r.get(0),
        ).unwrap();
        assert!(notes.unwrap().contains("original timestamp unknown"));
    }

    #[test]
    fn undo_deletes_only_that_imports_data() {
        let (_t, v) = vault();
        // Pre-existing data we want to preserve.
        seed_site_account_password(&v, "Keepers", "k", "kpw");

        // Import we'll undo.
        let p = preview(&v, vec![
            entry("Foo", Some("u"), "fpw"),
            entry("Bar", Some("u"), "bpw"),
        ]).unwrap();
        let id = commit(&v, p, "test", None).unwrap();

        let counts = undo(&v, id).unwrap();
        assert_eq!(counts.passwords, 2);
        assert_eq!(counts.accounts, 2);
        assert_eq!(counts.sites, 2);

        // Pre-existing data still there.
        let keepers_count: i64 = v.conn().query_row(
            "SELECT COUNT(*) FROM sites WHERE name='Keepers'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(keepers_count, 1);
        // Imported data gone.
        let foo_count: i64 = v.conn().query_row(
            "SELECT COUNT(*) FROM sites WHERE name IN ('Foo','Bar')",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(foo_count, 0);
        // imports row gone.
        let import_count: i64 = v.conn().query_row(
            "SELECT COUNT(*) FROM imports WHERE id = ?1",
            params![id.0],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(import_count, 0);
    }

    #[test]
    fn undo_unknown_id_returns_not_found() {
        let (_t, v) = vault();
        let err = undo(&v, ImportId(9999)).unwrap_err();
        assert!(matches!(err, Error::NotFound));
    }

    #[test]
    fn undo_keeps_account_when_other_passwords_remain() {
        let (_t, v) = vault();
        // Seed a site/account/password (NOT via import — no source_import_id).
        let aid = seed_site_account_password(&v, "Foo", "u", "first");

        // Import a merge — appends a NEW current row with source_import_id.
        let p = preview(&v, vec![entry("Foo", Some("u"), "second")]).unwrap();
        assert_eq!(p.merges, 1);
        let id = commit(&v, p, "test", None).unwrap();

        // Undo the import. The "first" row had no source_import_id so it stays;
        // the account therefore stays; the site stays.
        undo(&v, id).unwrap();

        let acc_count: i64 = v.conn().query_row(
            "SELECT COUNT(*) FROM accounts WHERE id = ?1",
            params![aid],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(acc_count, 1);
        let site_count: i64 = v.conn().query_row(
            "SELECT COUNT(*) FROM sites WHERE name='Foo'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(site_count, 1);
        // The retired-then-undone row's "second" password should be gone; "first" was retired by the merge but still present.
        let pw_count: i64 = v.conn().query_row(
            "SELECT COUNT(*) FROM password_history WHERE account_id = ?1",
            params![aid],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(pw_count, 1);
    }

    // --- apply_patches tests ---

    use super::super::{ParseDiagnostic, PartialEntry, RowPatch};

    fn diag_missing_site(row: usize) -> ParseDiagnostic {
        ParseDiagnostic {
            row,
            raw: format!("row {row} raw"),
            reason: "missing site".to_string(),
            parsed: PartialEntry {
                site: None,
                url: Some("https://example.com".to_string()),
                username: Some("alice".to_string()),
                display_name: None,
                password: Some(zeroize::Zeroizing::new("hunter2".to_string())),
                notes: Some("from import".to_string()),
                created_at: None,
                source_row: Some(format!("row {row} raw")),
            },
        }
    }

    fn diag_missing_password(row: usize) -> ParseDiagnostic {
        ParseDiagnostic {
            row,
            raw: format!("row {row} raw"),
            reason: "missing password".to_string(),
            parsed: PartialEntry {
                site: Some("Reddit".to_string()),
                url: None,
                username: Some("bob".to_string()),
                display_name: None,
                password: None,
                notes: None,
                created_at: None,
                source_row: Some(format!("row {row} raw")),
            },
        }
    }

    #[test]
    fn apply_patches_promotes_missing_site_to_entry_when_site_supplied() {
        let result = ParseResult {
            entries: Vec::new(),
            diagnostics: vec![diag_missing_site(5)],
        };
        let patches = vec![RowPatch {
            row: 5,
            site: Some("Reddit".to_string()),
            password: None,
        }];
        let out = apply_patches(result, &patches);
        assert_eq!(out.entries.len(), 1);
        assert_eq!(out.entries[0].site, "Reddit");
        assert_eq!(out.entries[0].username.as_deref(), Some("alice"));
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn apply_patches_promotes_missing_password_to_entry_when_password_supplied() {
        let result = ParseResult {
            entries: Vec::new(),
            diagnostics: vec![diag_missing_password(7)],
        };
        let patches = vec![RowPatch {
            row: 7,
            site: None,
            password: Some("S3cretPw!".to_string()),
        }];
        let out = apply_patches(result, &patches);
        assert_eq!(out.entries.len(), 1);
        assert_eq!(out.entries[0].site, "Reddit");
        assert_eq!(out.entries[0].password.as_str(), "S3cretPw!");
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn apply_patches_leaves_diagnostic_when_patch_field_empty_whitespace() {
        let result = ParseResult {
            entries: Vec::new(),
            diagnostics: vec![diag_missing_site(5)],
        };
        let patches = vec![RowPatch {
            row: 5,
            site: Some("   ".to_string()),
            password: None,
        }];
        let out = apply_patches(result, &patches);
        assert!(out.entries.is_empty());
        assert_eq!(out.diagnostics.len(), 1);
        assert_eq!(out.diagnostics[0].row, 5);
    }

    #[test]
    fn apply_patches_drops_orphan_patches_referencing_unknown_rows() {
        let result = ParseResult {
            entries: Vec::new(),
            diagnostics: vec![diag_missing_site(5)],
        };
        let patches = vec![
            RowPatch { row: 999, site: Some("X".to_string()), password: None },
        ];
        let out = apply_patches(result, &patches);
        assert!(out.entries.is_empty());
        assert_eq!(out.diagnostics.len(), 1);
        assert_eq!(out.diagnostics[0].row, 5);
    }

    #[test]
    fn import_routes_aliased_site_to_survivor() {
        use crate::repo::{site_aliases, sites::{self, NewSite}};
        let (_t, v) = vault();
        let rs = sites::create(&v, NewSite { name: "RuneScape".into(), ..Default::default() }).unwrap();
        site_aliases::record(&v, "jagex", rs.id, "Jagex").unwrap();
        // Import an entry whose site is "Jagex" -> should land under RuneScape, not a new site.
        let entries = vec![crate::importer::ImportEntry {
            site: "Jagex".into(),
            url: None,
            username: Some("me".into()),
            display_name: None,
            password: zeroize::Zeroizing::new("pw1".into()),
            notes: None,
            created_at: None,
            source_row: None,
        }];
        let prev = preview(&v, entries).unwrap();
        commit(&v, prev, "test", None).unwrap();
        // Exactly one site (RuneScape), and the account is under it.
        assert_eq!(sites::list(&v).unwrap().len(), 1);
        assert_eq!(accounts::list_for_site(&v, rs.id, &[]).unwrap().len(), 1);
    }

    #[test]
    fn apply_patches_no_op_when_patches_empty() {
        let result = ParseResult {
            entries: Vec::new(),
            diagnostics: vec![diag_missing_site(5), diag_missing_password(7)],
        };
        let out = apply_patches(result, &[]);
        assert!(out.entries.is_empty());
        assert_eq!(out.diagnostics.len(), 2);
    }

    #[test]
    fn apply_patches_combines_parsed_partial_with_patch_for_full_entry() {
        let result = ParseResult {
            entries: Vec::new(),
            diagnostics: vec![diag_missing_site(5)],
        };
        let patches = vec![RowPatch {
            row: 5,
            site: Some("ExampleSite".to_string()),
            password: None,
        }];
        let out = apply_patches(result, &patches);
        let e = &out.entries[0];
        assert_eq!(e.site, "ExampleSite");
        assert_eq!(e.url.as_deref(), Some("https://example.com"));
        assert_eq!(e.username.as_deref(), Some("alice"));
        assert_eq!(e.password.as_str(), "hunter2");
        assert_eq!(e.notes.as_deref(), Some("from import"));
    }
}
