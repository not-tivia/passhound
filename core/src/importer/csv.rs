//! CSV importer: parses a `.csv` file into [`ImportEntry`] records.
//!
//! Mapping resolution order:
//! 1. Explicit `Some(mapping)` argument.
//! 2. (Task 7) Saved mapping in `vault_meta` keyed on header fingerprint.
//! 3. Auto-detect from synonym table.
//! 4. `Err(Error::NeedsColumnMapping)` if auto-detect fails.

use super::{ImportEntry, ParseDiagnostic, ParseResult};
use crate::error::{Error, Result};
use chrono::{DateTime, Utc};
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

/// Parse a CSV file into a [`ParseResult`].
///
/// If `mapping` is `Some`, uses it directly. Otherwise tries auto-detect from
/// headers; on failure, returns `Err(Error::NeedsColumnMapping { headers })`.
///
/// Task 7 will extend this to also consult saved mappings in `vault_meta`.
pub fn parse_file(path: &Path, mapping: Option<Mapping>) -> Result<ParseResult> {
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
        None => match auto_detect(&headers) {
            Some(m) => m,
            None => return Err(Error::NeedsColumnMapping { headers }),
        },
    };

    let mut result = ParseResult::default();
    for (row_idx, rec) in rdr.records().enumerate() {
        let row_num = row_idx + 2; // +1 for 1-based, +1 for header row
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
    use tempfile::NamedTempFile;

    fn write_csv(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    #[test]
    fn auto_detect_google_pm_format() {
        let content = "name,url,username,password,note\n\
RuneScape,runescape.com,chris,Fluffy!2014,note1\n\
Amazon,amazon.com,chris@example.com,Bezos$1,\n";
        let f = write_csv(content);
        let r = parse_file(f.path(), None).unwrap();
        assert_eq!(r.entries.len(), 2);
        assert_eq!(r.entries[0].site, "RuneScape");
        assert_eq!(r.entries[0].password, "Fluffy!2014");
        assert_eq!(r.entries[0].username.as_deref(), Some("chris"));
        assert_eq!(r.entries[1].notes, None); // empty cell -> None
    }

    #[test]
    fn auto_detect_generic_format() {
        let content = "name,login,password\n\
Foo,user1,pw1\n\
Bar,user2,pw2\n";
        let f = write_csv(content);
        let r = parse_file(f.path(), None).unwrap();
        assert_eq!(r.entries.len(), 2);
        assert_eq!(r.entries[0].username.as_deref(), Some("user1"));
    }

    #[test]
    fn missing_required_column_returns_needs_mapping() {
        let content = "username,password\nu,p\n";
        let f = write_csv(content);
        let err = parse_file(f.path(), None).unwrap_err();
        match err {
            Error::NeedsColumnMapping { headers } => {
                assert_eq!(headers, vec!["username".to_string(), "password".to_string()]);
            }
            other => panic!("expected NeedsColumnMapping, got {other:?}"),
        }
    }

    #[test]
    fn explicit_mapping_overrides_auto_detect() {
        // Headers contain no site synonym, but explicit mapping points to col 0.
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
        let r = parse_file(f.path(), Some(m)).unwrap();
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].site, "MySite");
        assert_eq!(r.entries[0].password, "MyPass");
    }

    #[test]
    fn empty_password_row_becomes_diagnostic() {
        let content = "name,password\nFoo,\nBar,baz\n";
        let f = write_csv(content);
        let r = parse_file(f.path(), None).unwrap();
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.diagnostics.len(), 1);
        assert!(r.diagnostics[0].reason.contains("password"));
    }
}
