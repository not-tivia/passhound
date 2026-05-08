//! CSV importer: parses a `.csv` file into [`ImportEntry`] records.
//!
//! Mapping resolution order:
//! 1. Explicit `Some(mapping)` argument.
//! 2. (Task 7) Saved mapping in `vault_meta` keyed on header fingerprint.
//! 3. Auto-detect from synonym table.
//! 4. `Err(Error::NeedsColumnMapping)` if auto-detect fails.

use super::{ImportEntry, ParseDiagnostic, ParseResult};
use crate::error::{Error, Result};
use crate::vault::Vault;
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Column-index mapping from logical fields to CSV column indices.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mapping {
    pub site: usize,
    pub url: Option<usize>,
    pub username: Option<usize>,
    pub password: usize,
    pub notes: Option<usize>,
    pub created_at: Option<usize>,
}

const SITE_SYNONYMS: &[&str] = &["name", "site", "site_name", "website", "title"];
const URL_SYNONYMS: &[&str] = &["url", "login_url", "web_address"];
const USERNAME_SYNONYMS: &[&str] = &["username", "login", "user", "email"];
const PASSWORD_SYNONYMS: &[&str] = &["password", "pass"];
const NOTES_SYNONYMS: &[&str] = &["note", "notes", "comment", "comments"];
const CREATED_AT_SYNONYMS: &[&str] = &["created_at", "created", "date", "timestamp"];

fn find_index(headers: &[String], synonyms: &[&str]) -> Option<usize> {
    for (i, h) in headers.iter().enumerate() {
        let h_norm = h.trim().to_ascii_lowercase();
        if synonyms.iter().any(|s| *s == h_norm.as_str()) {
            return Some(i);
        }
    }
    None
}

/// Try to auto-detect a Mapping from the headers. Returns `None` if site or
/// password can't be located.
pub fn auto_detect(headers: &[String]) -> Option<Mapping> {
    let site = find_index(headers, SITE_SYNONYMS)?;
    let password = find_index(headers, PASSWORD_SYNONYMS)?;
    Some(Mapping {
        site,
        url: find_index(headers, URL_SYNONYMS),
        username: find_index(headers, USERNAME_SYNONYMS),
        password,
        notes: find_index(headers, NOTES_SYNONYMS),
        created_at: find_index(headers, CREATED_AT_SYNONYMS),
    })
}

/// Compute a stable fingerprint for a header tuple. Sorts headers
/// (lowercased, trimmed) and joins with NUL, then hex-encodes.
pub fn header_fingerprint(headers: &[String]) -> String {
    let mut norm: Vec<String> = headers
        .iter()
        .map(|h| h.trim().to_ascii_lowercase())
        .collect();
    norm.sort();
    let joined = norm.join("\u{0}");
    let mut hex = String::with_capacity(joined.len() * 2);
    for b in joined.as_bytes() {
        use std::fmt::Write as _;
        let _ = write!(hex, "{:02x}", b);
    }
    hex
}

fn mapping_meta_key(fingerprint: &str) -> String {
    format!("csv_mapping_{fingerprint}")
}

/// Load a previously-saved mapping for this header shape, if any.
pub fn load_saved_mapping(vault: &Vault, headers: &[String]) -> Result<Option<Mapping>> {
    let key = mapping_meta_key(&header_fingerprint(headers));
    let row: Option<Vec<u8>> = vault
        .conn()
        .query_row(
            "SELECT value FROM vault_meta WHERE key = ?1",
            params![key],
            |r| r.get(0),
        )
        .optional()?;
    let bytes = match row {
        Some(b) => b,
        None => return Ok(None),
    };
    let s = std::str::from_utf8(&bytes)
        .map_err(|_| Error::Import("saved mapping is not utf-8".into()))?;
    let m: Mapping = serde_json::from_str(s)
        .map_err(|e| Error::Import(format!("saved mapping json: {e}")))?;
    Ok(Some(m))
}

/// Persist a mapping for this header shape.
pub fn save_mapping(vault: &Vault, headers: &[String], mapping: &Mapping) -> Result<()> {
    let key = mapping_meta_key(&header_fingerprint(headers));
    let json = serde_json::to_string(mapping)
        .map_err(|e| Error::Import(format!("serialize mapping: {e}")))?;
    vault.conn().execute(
        "INSERT INTO vault_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, json.as_bytes()],
    )?;
    Ok(())
}

/// Parse a CSV file into a [`ParseResult`].
///
/// Mapping resolution order:
/// 1. Explicit `Some(mapping)` argument.
/// 2. Saved mapping in `vault_meta` for this header shape.
/// 3. Auto-detect from synonyms.
/// 4. `Err(Error::NeedsColumnMapping { headers })`.
pub fn parse_file(vault: &Vault, path: &Path, mapping: Option<Mapping>) -> Result<ParseResult> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_path(path)
        .map_err(|e| Error::Import(format!("open csv: {e}")))?;
    let headers: Vec<String> = rdr
        .headers()
        .map_err(|e| Error::Import(format!("read headers: {e}")))?
        .iter()
        .map(|s| s.to_string())
        .collect();

    let map = match mapping {
        Some(m) => m,
        None => match load_saved_mapping(vault, &headers)? {
            Some(m) => m,
            None => match auto_detect(&headers) {
                Some(m) => m,
                None => return Err(Error::NeedsColumnMapping { headers }),
            },
        },
    };

    let mut result = ParseResult::default();
    for (row_idx, rec) in rdr.records().enumerate() {
        let row_num = row_idx + 2;
        let rec = match rec {
            Ok(r) => r,
            Err(e) => {
                result.diagnostics.push(ParseDiagnostic {
                    row: row_num,
                    raw: String::new(),
                    reason: format!("csv parse: {e}"),
                });
                continue;
            }
        };
        let raw = rec.iter().collect::<Vec<_>>().join(",");

        let site = match rec.get(map.site).map(|s| s.trim()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => {
                result.diagnostics.push(ParseDiagnostic {
                    row: row_num,
                    raw,
                    reason: "missing site".to_string(),
                });
                continue;
            }
        };
        let password = match rec.get(map.password).map(|s| s.trim()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => {
                result.diagnostics.push(ParseDiagnostic {
                    row: row_num,
                    raw,
                    reason: "missing password".to_string(),
                });
                continue;
            }
        };
        let url = map
            .url
            .and_then(|i| rec.get(i).map(|s| s.trim()))
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let username = map
            .username
            .and_then(|i| rec.get(i).map(|s| s.trim()))
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let notes = map
            .notes
            .and_then(|i| rec.get(i).map(|s| s.trim()))
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let created_at = map
            .created_at
            .and_then(|i| rec.get(i).map(|s| s.trim()))
            .filter(|s| !s.is_empty())
            .and_then(|s| {
                DateTime::parse_from_rfc3339(s)
                    .ok()
                    .map(|d| d.with_timezone(&Utc))
            });

        result.entries.push(ImportEntry {
            site,
            url,
            username,
            password,
            created_at,
            notes,
            source_row: Some(raw),
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

    fn write_csv(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    fn vault() -> (TempDir, Vault) {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("v.db");
        let v = Vault::create(&path, b"hunter2").unwrap();
        (tmp, v)
    }

    #[test]
    fn auto_detect_google_pm_format() {
        let (_t, v) = vault();
        let content = "name,url,username,password,note\n\
RuneScape,runescape.com,chris,Fluffy!2014,note1\n\
Amazon,amazon.com,chris@example.com,Bezos$1,\n";
        let f = write_csv(content);
        let r = parse_file(&v, f.path(), None).unwrap();
        assert_eq!(r.entries.len(), 2);
        assert_eq!(r.entries[0].site, "RuneScape");
        assert_eq!(r.entries[0].password, "Fluffy!2014");
        assert_eq!(r.entries[1].notes, None);
    }

    #[test]
    fn auto_detect_generic_format() {
        let (_t, v) = vault();
        let content = "name,login,password\nFoo,user1,pw1\nBar,user2,pw2\n";
        let f = write_csv(content);
        let r = parse_file(&v, f.path(), None).unwrap();
        assert_eq!(r.entries.len(), 2);
        assert_eq!(r.entries[0].username.as_deref(), Some("user1"));
    }

    #[test]
    fn missing_required_column_returns_needs_mapping() {
        let (_t, v) = vault();
        let content = "username,password\nu,p\n";
        let f = write_csv(content);
        let err = parse_file(&v, f.path(), None).unwrap_err();
        match err {
            Error::NeedsColumnMapping { headers } => {
                assert_eq!(headers, vec!["username".to_string(), "password".to_string()]);
            }
            other => panic!("expected NeedsColumnMapping, got {other:?}"),
        }
    }

    #[test]
    fn explicit_mapping_overrides_auto_detect() {
        let (_t, v) = vault();
        let content = "label,word\nMySite,MyPass\n";
        let f = write_csv(content);
        let m = Mapping {
            site: 0,
            url: None,
            username: None,
            password: 1,
            notes: None,
            created_at: None,
        };
        let r = parse_file(&v, f.path(), Some(m)).unwrap();
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].site, "MySite");
    }

    #[test]
    fn empty_password_row_becomes_diagnostic() {
        let (_t, v) = vault();
        let content = "name,password\nFoo,\nBar,baz\n";
        let f = write_csv(content);
        let r = parse_file(&v, f.path(), None).unwrap();
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.diagnostics.len(), 1);
    }

    #[test]
    fn header_fingerprint_is_deterministic() {
        let h1 = vec!["Name".to_string(), "Password".to_string()];
        let h2 = vec!["password".to_string(), "name".to_string()];
        assert_eq!(header_fingerprint(&h1), header_fingerprint(&h2));
    }

    #[test]
    fn save_then_load_mapping() {
        let (_t, v) = vault();
        let headers = vec!["label".to_string(), "word".to_string()];
        let m = Mapping {
            site: 0,
            url: None,
            username: None,
            password: 1,
            notes: None,
            created_at: None,
        };
        save_mapping(&v, &headers, &m).unwrap();
        let loaded = load_saved_mapping(&v, &headers).unwrap().unwrap();
        assert_eq!(loaded.site, 0);
        assert_eq!(loaded.password, 1);
    }

    #[test]
    fn saved_mapping_is_used_by_parse_file() {
        let (_t, v) = vault();
        let headers = vec!["label".to_string(), "word".to_string()];
        let m = Mapping {
            site: 0,
            url: None,
            username: None,
            password: 1,
            notes: None,
            created_at: None,
        };
        save_mapping(&v, &headers, &m).unwrap();

        let content = "label,word\nFooSite,FooPass\n";
        let f = write_csv(content);
        let r = parse_file(&v, f.path(), None).unwrap();
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].site, "FooSite");
        assert_eq!(r.entries[0].password, "FooPass");
    }
}
