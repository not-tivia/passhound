use crate::error::{Error, Result};
use crate::repo::common;
use crate::site_name::{canonical_site_name, strip_url_noise};
use crate::vault::Vault;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const URL_CLEANUP_KEY: &str = "url_cleanup_v1";

#[derive(Debug, Clone, Default)]
pub struct CleanupReport {
    pub renamed: usize,
    pub skipped_collisions: usize,
}

/// One-shot data migration that rewrites URL-shaped values in `sites.name`
/// to their bare-brand canonical form (via `strip_url_noise`). Idempotent —
/// a `url_cleanup_v1` flag in `vault_meta` records completion so subsequent
/// vault opens are no-ops.
///
/// Conflict handling: if rewriting row A would collide with an existing row B
/// (same canonical name), row A is left untouched and counted as skipped.
/// The user can manually merge later.
///
/// Preserves the original URL on each rewritten row: if `sites.url` is NULL
/// or empty, the original `name` is copied into `url` so the URL information
/// is not lost.
///
/// Takes a `&Connection` (not `&Vault`) so it can run inside the same
/// transaction as `schema::apply_migrations` during `Vault::open`.
pub fn cleanup_url_shaped_site_names(conn: &Connection) -> Result<CleanupReport> {
    let done: Option<Vec<u8>> = conn
        .query_row(
            "SELECT value FROM vault_meta WHERE key = ?1",
            params![URL_CLEANUP_KEY],
            |r| r.get(0),
        )
        .optional()?;
    if done.is_some() {
        return Ok(CleanupReport::default());
    }

    let mut stmt = conn.prepare("SELECT id, name, url FROM sites")?;
    let rows: Vec<(i64, String, Option<String>)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .filter_map(|r| r.ok())
        .collect();
    drop(stmt);

    // canonical -> count of rows currently claiming it. Used to detect
    // collisions both pre-existing (multiple rows already canonicalize to
    // the same brand — leave them alone, let the user merge manually) and
    // newly-created (a rename would land on a slot claimed by another row).
    let mut taken: HashMap<String, usize> = HashMap::new();
    for (_, n, _) in &rows {
        *taken.entry(canonical_site_name(n)).or_insert(0) += 1;
    }

    let mut renamed = 0usize;
    let mut skipped = 0usize;
    for (id, current_name, current_url) in &rows {
        let proposed = strip_url_noise(current_name);
        if proposed == *current_name || proposed.is_empty() {
            continue;
        }
        let current_canonical = canonical_site_name(current_name);
        let proposed_canonical = canonical_site_name(&proposed);
        let collision = if proposed_canonical == current_canonical {
            // Rename is canonical-preserving. Skip only if there's already
            // ANOTHER row sharing this canonical (count > 1) — that pre-existing
            // duplicate signals the user has manual cleanup to do.
            taken.get(&current_canonical).copied().unwrap_or(0) > 1
        } else {
            // Rename moves to a different canonical. Skip if that slot is
            // already claimed by any row.
            taken.get(&proposed_canonical).copied().unwrap_or(0) > 0
        };
        if collision {
            skipped += 1;
            continue;
        }
        let new_url: Option<String> = match current_url.as_deref() {
            Some(s) if !s.trim().is_empty() => current_url.clone(),
            _ => Some(current_name.clone()),
        };
        conn.execute(
            "UPDATE sites SET name = ?1, url = ?2 WHERE id = ?3",
            params![proposed, new_url, id],
        )?;
        renamed += 1;
        if let Some(c) = taken.get_mut(&current_canonical) { *c -= 1; }
        *taken.entry(proposed_canonical).or_insert(0) += 1;
    }

    conn.execute(
        "INSERT INTO vault_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![URL_CLEANUP_KEY, b"done".to_vec()],
    )?;

    Ok(CleanupReport { renamed, skipped_collisions: skipped })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Site {
    pub id: i64,
    pub name: String,
    pub url: Option<String>,
    pub category: Option<String>,
    pub abbreviations: Vec<String>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct NewSite {
    pub name: String,
    pub url: Option<String>,
    pub category: Option<String>,
    pub abbreviations: Vec<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateSite {
    pub name: String,
    pub url: Option<String>,
    pub category: Option<String>,
    pub abbreviations: Vec<String>,
    pub notes: Option<String>,
}

pub fn create(vault: &Vault, new: NewSite) -> Result<Site> {
    if new.name.trim().is_empty() {
        return Err(Error::InvalidInput("site name required".into()));
    }
    let abbr_json = serde_json::to_string(&new.abbreviations)
        .map_err(|e| Error::InvalidInput(format!("abbreviations: {e}")))?;
    let now = Utc::now();
    vault.conn().execute(
        "INSERT INTO sites (name, url, category, abbreviations, notes, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![new.name, new.url, new.category, abbr_json, new.notes, now.to_rfc3339()],
    )?;
    let id = vault.conn().last_insert_rowid();
    Ok(Site {
        id,
        name: new.name,
        url: new.url,
        category: new.category,
        abbreviations: new.abbreviations,
        notes: new.notes,
        created_at: now,
    })
}

pub fn update(vault: &Vault, id: i64, changes: UpdateSite) -> Result<()> {
    if changes.name.trim().is_empty() {
        return Err(Error::InvalidInput("site name required".into()));
    }
    let abbr_json = serde_json::to_string(&changes.abbreviations)
        .map_err(|e| Error::InvalidInput(format!("abbreviations: {e}")))?;
    let n = vault.conn().execute(
        "UPDATE sites SET name = ?1, url = ?2, category = ?3,
         abbreviations = ?4, notes = ?5 WHERE id = ?6",
        params![changes.name, changes.url, changes.category, abbr_json, changes.notes, id],
    )?;
    common::ensure_affected(n)
}

pub fn get(vault: &Vault, id: i64) -> Result<Site> {
    vault.conn().query_row(
        "SELECT id, name, url, category, abbreviations, notes, created_at FROM sites WHERE id = ?1",
        params![id],
        row_to_site,
    ).map_err(common::not_found_or_db)
}

pub fn find_by_name(vault: &Vault, name: &str) -> Result<Option<Site>> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let res = vault.conn().query_row(
        "SELECT id, name, url, category, abbreviations, notes, created_at
         FROM sites WHERE LOWER(name) = LOWER(?1)",
        params![trimmed],
        row_to_site,
    );
    match res {
        Ok(s) => Ok(Some(s)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn list(vault: &Vault) -> Result<Vec<Site>> {
    let mut stmt = vault.conn().prepare(
        "SELECT id, name, url, category, abbreviations, notes, created_at FROM sites ORDER BY name",
    )?;
    let rows = stmt
        .query_map([], row_to_site)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn row_to_site(row: &rusqlite::Row<'_>) -> rusqlite::Result<Site> {
    let abbr_json: String = row.get(4)?;
    let abbreviations: Vec<String> = serde_json::from_str(&abbr_json).unwrap_or_default();
    let created_str: String = row.get(6)?;
    let created_at = DateTime::parse_from_rfc3339(&created_str)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    Ok(Site {
        id: row.get(0)?,
        name: row.get(1)?,
        url: row.get(2)?,
        category: row.get(3)?,
        abbreviations,
        notes: row.get(5)?,
        created_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn vault() -> (TempDir, Vault) {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("v.db");
        let v = Vault::create(&path, b"hunter2").unwrap();
        (tmp, v)
    }

    #[test]
    fn create_and_get() {
        let (_tmp, v) = vault();
        let s = create(&v, NewSite {
            name: "RuneScape".into(),
            url: Some("runescape.com".into()),
            category: Some("Gaming".into()),
            abbreviations: vec!["RS".into(), "rs07".into()],
            notes: None,
        }).unwrap();
        assert_eq!(s.name, "RuneScape");
        let fetched = get(&v, s.id).unwrap();
        assert_eq!(fetched.id, s.id);
        assert_eq!(fetched.abbreviations, vec!["RS".to_string(), "rs07".to_string()]);
    }

    #[test]
    fn create_rejects_empty_name() {
        let (_tmp, v) = vault();
        let err = create(&v, NewSite::default()).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[test]
    fn list_returns_sites_ordered_by_name() {
        let (_tmp, v) = vault();
        for name in ["Zoom", "Amazon", "GitHub"] {
            create(&v, NewSite { name: name.into(), ..Default::default() }).unwrap();
        }
        let names: Vec<String> = list(&v).unwrap().into_iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["Amazon", "GitHub", "Zoom"]);
    }

    #[test]
    fn get_returns_not_found_for_unknown_id() {
        let (_tmp, v) = vault();
        let err = get(&v, 999).unwrap_err();
        assert!(matches!(err, Error::NotFound));
    }

    #[test]
    fn find_by_name_is_case_insensitive_and_returns_none_for_missing() {
        let (_tmp, v) = vault();
        create(&v, NewSite { name: "RuneScape".into(), ..Default::default() }).unwrap();
        let found = find_by_name(&v, "runescape").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "RuneScape");
        let missing = find_by_name(&v, "nonexistent").unwrap();
        assert!(missing.is_none());
        let empty = find_by_name(&v, "   ").unwrap();
        assert!(empty.is_none());
    }

    #[test]
    fn update_changes_fields_round_trip() {
        let (_tmp, v) = vault();
        let s = create(&v, NewSite {
            name: "RuneScape".into(),
            url: Some("runescape.com".into()),
            category: Some("Gaming".into()),
            abbreviations: vec!["RS".into()],
            notes: None,
        }).unwrap();

        update(&v, s.id, UpdateSite {
            name: "Runescape Classic".into(),
            url: Some("classic.runescape.com".into()),
            category: Some("MMO".into()),
            abbreviations: vec!["RSC".into(), "rs07".into()],
            notes: Some("legacy account".into()),
        }).unwrap();

        let got = get(&v, s.id).unwrap();
        assert_eq!(got.name, "Runescape Classic");
        assert_eq!(got.url.as_deref(), Some("classic.runescape.com"));
        assert_eq!(got.category.as_deref(), Some("MMO"));
        assert_eq!(got.abbreviations, vec!["RSC".to_string(), "rs07".to_string()]);
        assert_eq!(got.notes.as_deref(), Some("legacy account"));
    }

    // Phase 4.23 ---------------------------------------------------------------

    fn raw_name(v: &Vault, id: i64) -> String {
        v.conn().query_row(
            "SELECT name FROM sites WHERE id = ?1",
            params![id],
            |r| r.get::<_, String>(0),
        ).unwrap()
    }

    fn raw_url(v: &Vault, id: i64) -> Option<String> {
        v.conn().query_row(
            "SELECT url FROM sites WHERE id = ?1",
            params![id],
            |r| r.get::<_, Option<String>>(0),
        ).unwrap()
    }

    fn clear_cleanup_flag(v: &Vault) {
        v.conn().execute(
            "DELETE FROM vault_meta WHERE key = ?1",
            params![URL_CLEANUP_KEY],
        ).unwrap();
    }

    #[test]
    fn cleanup_rewrites_url_shaped_names_and_preserves_original_in_url() {
        let (_tmp, v) = vault();
        // Fresh vault already ran cleanup at create — clear the flag so the
        // test exercises a non-trivial pass.
        clear_cleanup_flag(&v);
        let s = create(&v, NewSite {
            name: "https://www.github.com".into(),
            url: None,
            ..Default::default()
        }).unwrap();
        let report = cleanup_url_shaped_site_names(v.conn()).unwrap();
        assert_eq!(report.renamed, 1);
        assert_eq!(report.skipped_collisions, 0);
        assert_eq!(raw_name(&v, s.id), "github");
        // Original URL form preserved in the url column.
        assert_eq!(raw_url(&v, s.id).as_deref(), Some("https://www.github.com"));
    }

    #[test]
    fn cleanup_handles_android_packages() {
        let (_tmp, v) = vault();
        clear_cleanup_flag(&v);
        let s1 = create(&v, NewSite {
            name: "android://abc==@com.tumblr/".into(), ..Default::default()
        }).unwrap();
        let s2 = create(&v, NewSite {
            name: "android://xyz==@com.jagex.oldscape.android/".into(), ..Default::default()
        }).unwrap();
        let report = cleanup_url_shaped_site_names(v.conn()).unwrap();
        assert_eq!(report.renamed, 2);
        assert_eq!(raw_name(&v, s1.id), "tumblr");
        assert_eq!(raw_name(&v, s2.id), "jagex");
    }

    #[test]
    fn cleanup_skips_collisions() {
        // Both "https://www.github.com" and a separate "github" exist. Rewriting
        // the URL row would collide with the bare row — skip it.
        let (_tmp, v) = vault();
        clear_cleanup_flag(&v);
        let url_row = create(&v, NewSite {
            name: "https://www.github.com".into(), ..Default::default()
        }).unwrap();
        let bare_row = create(&v, NewSite {
            name: "GitHub".into(), ..Default::default()
        }).unwrap();
        let report = cleanup_url_shaped_site_names(v.conn()).unwrap();
        assert_eq!(report.renamed, 0);
        assert_eq!(report.skipped_collisions, 1);
        // URL row left unchanged.
        assert_eq!(raw_name(&v, url_row.id), "https://www.github.com");
        // Bare row also untouched.
        assert_eq!(raw_name(&v, bare_row.id), "GitHub");
    }

    #[test]
    fn cleanup_is_idempotent_via_flag() {
        let (_tmp, v) = vault();
        clear_cleanup_flag(&v);
        let s = create(&v, NewSite {
            name: "https://www.github.com".into(), ..Default::default()
        }).unwrap();
        let first = cleanup_url_shaped_site_names(v.conn()).unwrap();
        assert_eq!(first.renamed, 1);
        // Second call sees the flag set, returns zeros without re-scanning.
        let second = cleanup_url_shaped_site_names(v.conn()).unwrap();
        assert_eq!(second.renamed, 0);
        assert_eq!(second.skipped_collisions, 0);
        // And the row stayed canonical.
        assert_eq!(raw_name(&v, s.id), "github");
    }

    #[test]
    fn cleanup_leaves_already_canonical_names_untouched() {
        let (_tmp, v) = vault();
        clear_cleanup_flag(&v);
        let s = create(&v, NewSite {
            name: "Tumblr".into(),
            url: Some("https://tumblr.com".into()),
            ..Default::default()
        }).unwrap();
        let report = cleanup_url_shaped_site_names(v.conn()).unwrap();
        assert_eq!(report.renamed, 0);
        assert_eq!(report.skipped_collisions, 0);
        assert_eq!(raw_name(&v, s.id), "Tumblr");
        assert_eq!(raw_url(&v, s.id).as_deref(), Some("https://tumblr.com"));
    }

    #[test]
    fn cleanup_does_not_clobber_existing_url() {
        // If url column already has a value, the rewrite must NOT overwrite it.
        let (_tmp, v) = vault();
        clear_cleanup_flag(&v);
        let s = create(&v, NewSite {
            name: "https://www.github.com".into(),
            url: Some("https://github.com/explicit-url".into()),
            ..Default::default()
        }).unwrap();
        let report = cleanup_url_shaped_site_names(v.conn()).unwrap();
        assert_eq!(report.renamed, 1);
        assert_eq!(raw_name(&v, s.id), "github");
        // Existing url preserved, not replaced with the original name.
        assert_eq!(raw_url(&v, s.id).as_deref(), Some("https://github.com/explicit-url"));
    }

    #[test]
    fn cleanup_runs_automatically_during_vault_open() {
        // Vault::create stamps the flag; reopening a vault with no URL-shaped
        // rows must remain a no-op (flag is set, no scan).
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("v.db");
        {
            let v = Vault::create(&path, b"pw").unwrap();
            create(&v, NewSite { name: "Tumblr".into(), ..Default::default() }).unwrap();
        }
        let v2 = Vault::open(&path).unwrap();
        // Flag should still be set.
        let val: Vec<u8> = v2.conn().query_row(
            "SELECT value FROM vault_meta WHERE key = ?1",
            params![URL_CLEANUP_KEY],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(val, b"done");
    }

    #[test]
    fn update_returns_not_found_for_missing_id() {
        let (_tmp, v) = vault();
        let err = update(&v, 999, UpdateSite {
            name: "x".into(), ..Default::default()
        }).unwrap_err();
        assert!(matches!(err, Error::NotFound));
    }
}
