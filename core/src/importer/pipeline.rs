//! Pipeline: classify imported entries against the vault, then commit / undo.

use super::{ImportEntry, ParseResult};
use crate::error::{Error, Result};
use crate::repo::{accounts, passwords, sites};
use crate::vault::Vault;

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
    let all_sites = sites::list(vault)?;
    let mut preview = Preview::default();

    for entry in entries {
        let matched_site = all_sites.iter().find(|s| s.name == entry.site);
        let (classification, site_id, account_id) = match matched_site {
            None => (Classification::New, None, None),
            Some(site) => {
                let accs = accounts::list_for_site(vault, site.id)?;
                let target_user = entry.username.as_deref().unwrap_or("");
                let matched_account = accs.iter().find(|a| {
                    a.username.as_deref().unwrap_or("") == target_user
                });
                match matched_account {
                    None => (Classification::New, Some(site.id), None),
                    Some(acc) => {
                        let current = passwords::current_plaintext(vault, acc.id)?;
                        match current {
                            None => (Classification::New, Some(site.id), Some(acc.id)),
                            Some(pt) if pt.as_str() == entry.password => {
                                (Classification::DuplicateOfTriple, Some(site.id), Some(acc.id))
                            }
                            Some(_) => {
                                (Classification::MergeWithNewPassword, Some(site.id), Some(acc.id))
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
            password: pw.into(),
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
}
