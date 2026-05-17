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
use zeroize::Zeroizing;
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Column-index mapping from logical fields to CSV column indices.
/// `site` is optional — when None, callers must supply a `site_override`
/// to `parse_file` (e.g. via the CLI's `--site NAME` flag) so per-site CSVs
/// that lack a site column can still import.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mapping {
    pub site: Option<usize>,
    pub url: Option<usize>,
    pub username: Option<usize>,
    /// Column index for the public-facing display name (e.g. game handle).
    /// `#[serde(default)]` keeps backward-compat with mappings serialized
    /// before Phase 4.2 — older shapes deserialize with None.
    #[serde(default)]
    pub display_name: Option<usize>,
    pub password: usize,
    pub notes: Option<usize>,
    pub created_at: Option<usize>,
    /// Additional column indices to merge into each row's notes field as
    /// labeled lines `"<header>: <value>"`. Empty values skipped per-row.
    /// `#[serde(default)]` keeps backward-compat with mappings serialized
    /// before this phase (older shapes deserialize with an empty Vec).
    #[serde(default)]
    pub extras_into_notes: Vec<usize>,
}

const SITE_SYNONYMS: &[&str] = &[
    "name", "site", "site_name", "website", "title", "service",
];
const URL_SYNONYMS: &[&str] = &["url", "login_url", "web_address", "link"];
const USERNAME_SYNONYMS: &[&str] = &[
    "username", "login", "user", "email", "account login",
    "account_login", "account email", "account_email", "user name",
    "user_name", "account name", "account_name",
];
const PASSWORD_SYNONYMS: &[&str] = &[
    "password", "pass", "pwd", "passwd", "account password",
    "account_password", "master password", "master_password",
];
const NOTES_SYNONYMS: &[&str] = &[
    "note", "notes", "comment", "comments", "description", "remarks",
];
const CREATED_AT_SYNONYMS: &[&str] = &[
    "created_at", "created", "date", "timestamp", "date_created",
];
const DISPLAY_NAME_SYNONYMS: &[&str] = &[
    "display name", "display_name", "displayname",
    "account display name", "account_display_name",
    "screen name", "screen_name", "screenname",
    "handle", "nick", "nickname",
];

fn find_index(headers: &[String], synonyms: &[&str]) -> Option<usize> {
    for (i, h) in headers.iter().enumerate() {
        let h_norm = h.trim().to_ascii_lowercase();
        if synonyms.iter().any(|s| *s == h_norm.as_str()) {
            return Some(i);
        }
    }
    None
}

/// Try to auto-detect a Mapping from the headers. Returns `None` if
/// `password` can't be located (the only strictly required column). The site
/// column is optional in the mapping — `parse_file` enforces that either
/// `Mapping.site` is set OR a `site_override` argument is supplied.
///
/// Every header that does NOT match a standard synonym (site/url/username/
/// password/notes/created_at) goes into `extras_into_notes` so its per-row
/// value is preserved as a labeled line in the notes field. Nothing is
/// silently dropped.
pub fn auto_detect(headers: &[String]) -> Option<Mapping> {
    let password = find_index(headers, PASSWORD_SYNONYMS)?;
    let site = find_index(headers, SITE_SYNONYMS);
    let url = find_index(headers, URL_SYNONYMS);
    let username = find_index(headers, USERNAME_SYNONYMS);
    let display_name = find_index(headers, DISPLAY_NAME_SYNONYMS);
    let notes = find_index(headers, NOTES_SYNONYMS);
    let created_at = find_index(headers, CREATED_AT_SYNONYMS);

    let mapped: std::collections::HashSet<usize> =
        [site, url, username, display_name, Some(password), notes, created_at]
            .into_iter()
            .flatten()
            .collect();
    let extras_into_notes: Vec<usize> =
        (0..headers.len()).filter(|i| !mapped.contains(i)).collect();

    Some(Mapping {
        site,
        url,
        username,
        display_name,
        password,
        notes,
        created_at,
        extras_into_notes,
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
///
/// `site_override` — when `Some(name)`, every imported row uses this site
/// name and `Mapping.site` is ignored. Lets per-site CSVs (no site column)
/// import via CLI `--site NAME`. When `None`, `Mapping.site` must be set
/// or the parse rejects with a per-row "missing site" diagnostic.
pub fn parse_file(
    vault: &Vault,
    path: &Path,
    mapping: Option<Mapping>,
    site_override: Option<String>,
) -> Result<ParseResult> {
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

    // If neither the mapping nor the override supplies a site, surface a
    // single clear error rather than emitting "missing site" for every row.
    if map.site.is_none() && site_override.is_none() {
        return Err(Error::Import(
            "no site column found in CSV; pass --site NAME or add a site column".into(),
        ));
    }

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
        let raw = rec.iter().enumerate()
            .map(|(i, field)| if i == map.password { "<redacted>" } else { field })
            .collect::<Vec<_>>()
            .join(",");

        let site = if let Some(name) = &site_override {
            name.clone()
        } else {
            // map.site is guaranteed Some at this point — the up-front check
            // at the top of parse_file rejects the file otherwise.
            let idx = map.site.expect("map.site must be set when no site_override");
            match rec.get(idx).map(|s| s.trim()) {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => {
                    result.diagnostics.push(ParseDiagnostic {
                        row: row_num,
                        raw,
                        reason: "missing site".to_string(),
                    });
                    continue;
                }
            }
        };
        let password = match rec.get(map.password).map(|s| s.trim()) {
            Some(s) if !s.is_empty() => Zeroizing::new(s.to_string()),
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
        let display_name = map
            .display_name
            .and_then(|i| rec.get(i).map(|s| s.trim()))
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        // Notes assembly: start with the mapped notes column (if any), then
        // append labeled lines for each extras index whose row value is
        // non-empty. Multiple parts joined by '\n'.
        let mut notes_parts: Vec<String> = Vec::new();
        if let Some(idx) = map.notes {
            if let Some(s) = rec.get(idx).map(|s| s.trim()).filter(|s| !s.is_empty()) {
                notes_parts.push(s.to_string());
            }
        }
        for idx in &map.extras_into_notes {
            if let Some(value) = rec.get(*idx).map(|s| s.trim()).filter(|s| !s.is_empty()) {
                let header = headers.get(*idx).map(|s| s.as_str()).unwrap_or("extra");
                notes_parts.push(format!("{header}: {value}"));
            }
        }
        let notes = if notes_parts.is_empty() {
            None
        } else {
            Some(notes_parts.join("\n"))
        };
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
            display_name,
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
        let r = parse_file(&v, f.path(), None, None).unwrap();
        assert_eq!(r.entries.len(), 2);
        assert_eq!(r.entries[0].site, "RuneScape");
        assert_eq!(r.entries[0].password.as_str(), "Fluffy!2014");
        assert_eq!(r.entries[1].notes, None);
    }

    #[test]
    fn auto_detect_generic_format() {
        let (_t, v) = vault();
        let content = "name,login,password\nFoo,user1,pw1\nBar,user2,pw2\n";
        let f = write_csv(content);
        let r = parse_file(&v, f.path(), None, None).unwrap();
        assert_eq!(r.entries.len(), 2);
        assert_eq!(r.entries[0].username.as_deref(), Some("user1"));
    }

    #[test]
    fn missing_password_column_returns_needs_mapping() {
        let (_t, v) = vault();
        // Phase 4.1+: only `password` is strictly required by auto_detect.
        // No site column is fine when `--site NAME` is passed (or an explicit
        // mapping supplies one). Test the password-missing case.
        let content = "username,note\nu,n\n";
        let f = write_csv(content);
        let err = parse_file(&v, f.path(), None, None).unwrap_err();
        match err {
            Error::NeedsColumnMapping { headers } => {
                assert_eq!(headers, vec!["username".to_string(), "note".to_string()]);
            }
            other => panic!("expected NeedsColumnMapping, got {other:?}"),
        }
    }

    #[test]
    fn site_override_imports_without_site_column() {
        let (_t, v) = vault();
        // CSV has no site column — just user/password/notes. With
        // `site_override = Some("RuneScape")`, every row gets that site.
        let content = "login,password,notes\nchris,Fluffy!2014,first acct\nchris2,Bezos$1,second acct\n";
        let f = write_csv(content);
        let r = parse_file(&v, f.path(), None, Some("RuneScape".into())).unwrap();
        assert_eq!(r.entries.len(), 2);
        assert_eq!(r.entries[0].site, "RuneScape");
        assert_eq!(r.entries[0].username.as_deref(), Some("chris"));
        assert_eq!(r.entries[0].password.as_str(), "Fluffy!2014");
        assert_eq!(r.entries[1].site, "RuneScape");
    }

    #[test]
    fn no_site_and_no_override_rejects_with_clear_error() {
        let (_t, v) = vault();
        let content = "login,password\nu,p\n";
        let f = write_csv(content);
        let err = parse_file(&v, f.path(), None, None).unwrap_err();
        match err {
            Error::Import(msg) => assert!(msg.contains("--site"), "expected --site hint in: {msg}"),
            other => panic!("expected Import error with --site hint, got {other:?}"),
        }
    }

    #[test]
    fn explicit_mapping_overrides_auto_detect() {
        let (_t, v) = vault();
        let content = "label,word\nMySite,MyPass\n";
        let f = write_csv(content);
        let m = Mapping {
            site: Some(0),
            url: None,
            username: None,
            display_name: None,
            password: 1,
            notes: None,
            created_at: None,
            extras_into_notes: vec![],
        };
        let r = parse_file(&v, f.path(), Some(m), None).unwrap();
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].site, "MySite");
    }

    #[test]
    fn empty_password_row_becomes_diagnostic() {
        let (_t, v) = vault();
        let content = "name,password\nFoo,\nBar,baz\n";
        let f = write_csv(content);
        let r = parse_file(&v, f.path(), None, None).unwrap();
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
            site: Some(0),
            url: None,
            username: None,
            display_name: None,
            password: 1,
            notes: None,
            created_at: None,
            extras_into_notes: vec![],
        };
        save_mapping(&v, &headers, &m).unwrap();
        let loaded = load_saved_mapping(&v, &headers).unwrap().unwrap();
        assert_eq!(loaded.site, Some(0));
        assert_eq!(loaded.password, 1);
    }

    #[test]
    fn saved_mapping_is_used_by_parse_file() {
        let (_t, v) = vault();
        let headers = vec!["label".to_string(), "word".to_string()];
        let m = Mapping {
            site: Some(0),
            url: None,
            username: None,
            display_name: None,
            password: 1,
            notes: None,
            created_at: None,
            extras_into_notes: vec![],
        };
        save_mapping(&v, &headers, &m).unwrap();

        let content = "label,word\nFooSite,FooPass\n";
        let f = write_csv(content);
        let r = parse_file(&v, f.path(), None, None).unwrap();
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].site, "FooSite");
        assert_eq!(r.entries[0].password.as_str(), "FooPass");
    }

    #[test]
    fn extras_into_notes_appends_labeled_lines() {
        let (_t, v) = vault();
        let content = "name,login,password,clan,total level\n\
            RuneScape,chris,Fluffy!2014,BobClan,99\n";
        let f = write_csv(content);
        let r = parse_file(&v, f.path(), None, None).unwrap();
        assert_eq!(r.entries.len(), 1);
        assert_eq!(
            r.entries[0].notes.as_deref(),
            Some("clan: BobClan\ntotal level: 99")
        );
    }

    #[test]
    fn extras_into_notes_skips_empty_values_per_row() {
        let (_t, v) = vault();
        let content = "name,login,password,clan,total level\n\
            RuneScape,chris,Fluffy!2014,BobClan,99\n\
            RuneScape,chris2,Fluffy!2015,,\n";
        let f = write_csv(content);
        let r = parse_file(&v, f.path(), None, None).unwrap();
        assert_eq!(r.entries.len(), 2);
        assert_eq!(
            r.entries[0].notes.as_deref(),
            Some("clan: BobClan\ntotal level: 99")
        );
        // Row 2 has empty extras → no notes generated.
        assert_eq!(r.entries[1].notes, None);
    }

    #[test]
    fn extras_into_notes_combines_with_explicit_notes_column() {
        let (_t, v) = vault();
        // The "notes" column maps to the notes field; "clan" and
        // "total level" are extras. Notes ordered: existing notes column
        // first, then labeled extras.
        let content = "name,login,password,notes,clan,total level\n\
            RuneScape,chris,Fluffy!2014,maxed combat,BobClan,99\n";
        let f = write_csv(content);
        let r = parse_file(&v, f.path(), None, None).unwrap();
        assert_eq!(r.entries.len(), 1);
        assert_eq!(
            r.entries[0].notes.as_deref(),
            Some("maxed combat\nclan: BobClan\ntotal level: 99")
        );
    }

    #[test]
    fn auto_detect_populates_extras_with_unmapped_columns() {
        let headers: Vec<String> = vec![
            "name".into(),
            "login".into(),
            "password".into(),
            "clan".into(),
            "total level".into(),
        ];
        let m = auto_detect(&headers).expect("password is present");
        assert_eq!(m.site, Some(0));
        assert_eq!(m.username, Some(1));
        assert_eq!(m.password, 2);
        assert_eq!(m.extras_into_notes, vec![3, 4]);
    }

    #[test]
    fn auto_detect_maps_account_display_name() {
        let headers: Vec<String> = vec![
            "name".into(),
            "login".into(),
            "password".into(),
            "Account Display Name".into(),
        ];
        let m = auto_detect(&headers).expect("password is present");
        assert_eq!(m.display_name, Some(3));
        // Display name column must NOT be in extras_into_notes (it's a first-class field).
        assert!(!m.extras_into_notes.contains(&3),
            "display_name column should be excluded from extras; got {:?}", m.extras_into_notes);
    }

    #[test]
    fn parse_file_extracts_display_name() {
        let (_t, v) = vault();
        let content = "name,login,password,display name\nReddit,chris,Pw1,MaxedNoob\n";
        let f = write_csv(content);
        let r = parse_file(&v, f.path(), None, None).unwrap();
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].display_name.as_deref(), Some("MaxedNoob"));
    }

    /// Verify that ParseDiagnostic.raw never leaks the password plaintext.
    ///
    /// We craft a CSV where the first row has an empty site (triggers a
    /// "missing site" diagnostic) but a real password value (`S3cr3tPw!`).
    /// The resulting diagnostic's `raw` must contain the literal string
    /// `<redacted>` in the password column position and must NOT contain the
    /// original password text.
    #[test]
    fn parse_diagnostic_raw_redacts_password_column() {
        let (_t, v) = vault();
        // name(col 0), password(col 1) — site_override is None so the name
        // column is used. An empty site triggers the "missing site" diagnostic
        // and we can inspect its `raw` field.
        let secret = "S3cr3tPw!";
        let content = format!("name,password\n,{secret}\nFoo,OtherPw\n");
        let f = write_csv(&content);
        let r = parse_file(&v, f.path(), None, None).unwrap();

        // The empty-site row should produce exactly one diagnostic.
        assert_eq!(r.diagnostics.len(), 1, "expected one missing-site diagnostic");
        let raw = &r.diagnostics[0].raw;

        // Password plaintext must NOT appear in the raw diagnostic field.
        assert!(
            !raw.contains(secret),
            "diagnostic raw must not contain password plaintext; got: {raw:?}"
        );
        // The redacted sentinel must appear instead.
        assert!(
            raw.contains("<redacted>"),
            "diagnostic raw must contain '<redacted>'; got: {raw:?}"
        );
    }
}
