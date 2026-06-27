use crate::error::{Error, Result};
use crate::repo::common;
use crate::repo::passwords;
use crate::repo::site_aliases;
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

/// One site row inside a merge group.
#[derive(Debug, Clone, Serialize)]
pub struct MergeMember {
    pub site_id: i64,
    pub name: String,
    pub account_count: usize,
}

/// A canonical-brand group backed by more than one `sites` row.
#[derive(Debug, Clone, Serialize)]
pub struct MergeGroup {
    pub canonical: String,
    pub clean_name: String,
    pub survivor_id: i64,
    pub members: Vec<MergeMember>,
    pub total_accounts: usize,
}

/// List every canonical-name group that has more than one site row, with
/// per-row account counts and a proposed survivor. Read-only; needs no master
/// key (site names are plaintext metadata). Single-row brands are omitted.
///
/// Survivor selection: prefer a row whose stored name already equals its
/// `strip_url_noise` form (i.e. already clean); otherwise the lowest id.
/// Groups are returned biggest-first (by total account count, then canonical).
pub fn find_merge_groups(vault: &Vault) -> Result<Vec<MergeGroup>> {
    let conn = vault.conn();

    // account counts per site
    let mut counts: HashMap<i64, usize> = HashMap::new();
    {
        let mut stmt = conn.prepare("SELECT site_id, COUNT(*) FROM accounts GROUP BY site_id")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
        for r in rows {
            let (sid, c) = r?;
            counts.insert(sid, c as usize);
        }
    }

    // all sites, in id order so grouping is deterministic
    let sites: Vec<(i64, String)> = {
        let mut stmt = conn.prepare("SELECT id, name FROM sites ORDER BY id")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    };

    // group by canonical, preserving first-seen order
    let mut groups: HashMap<String, Vec<(i64, String)>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for (id, name) in sites {
        let canon = canonical_site_name(&name);
        if canon.is_empty() {
            continue;
        }
        if !groups.contains_key(&canon) {
            order.push(canon.clone());
        }
        groups.entry(canon).or_default().push((id, name));
    }

    let mut out: Vec<MergeGroup> = Vec::new();
    for canon in order {
        let rows = &groups[&canon];
        if rows.len() < 2 {
            continue;
        }
        // survivor: first already-clean row (id order), else lowest id
        let survivor_id = rows
            .iter()
            .find(|(_, n)| strip_url_noise(n) == *n)
            .or_else(|| rows.iter().min_by_key(|(id, _)| *id))
            .map(|(id, _)| *id)
            .unwrap();
        let survivor_name = rows.iter().find(|(id, _)| *id == survivor_id).map(|(_, n)| n).unwrap();
        let clean_name = strip_url_noise(survivor_name);

        let mut members: Vec<MergeMember> = rows
            .iter()
            .map(|(id, n)| MergeMember {
                site_id: *id,
                name: n.clone(),
                account_count: counts.get(id).copied().unwrap_or(0),
            })
            .collect();
        // survivor first, then losers by id
        members.sort_by_key(|m| (m.site_id != survivor_id, m.site_id));
        let total_accounts = members.iter().map(|m| m.account_count).sum();

        out.push(MergeGroup { canonical: canon, clean_name, survivor_id, members, total_accounts });
    }

    out.sort_by(|a, b| b.total_accounts.cmp(&a.total_accounts).then(a.canonical.cmp(&b.canonical)));
    Ok(out)
}

/// Outcome of one or more site merges.
#[derive(Debug, Clone, Default, Serialize)]
pub struct MergeResult {
    pub groups_merged: usize,
    pub rows_removed: usize,
    pub accounts_repointed: usize,
}

/// Merge `loser_ids` into `survivor_id`: re-point every account from the losers
/// onto the survivor, rename the survivor to its clean brand, backfill any empty
/// survivor field (url/category/notes) from the first loser that has it, then
/// delete the loser rows. Runs in a single transaction.
///
/// Validation (before any write): `loser_ids` non-empty, must not contain
/// `survivor_id`, every id must exist, and all losers must share the survivor's
/// `canonical_site_name`. Returns `Error::InvalidInput` / `Error::NotFound`.
pub fn merge_sites(vault: &Vault, survivor_id: i64, loser_ids: &[i64]) -> Result<MergeResult> {
    if loser_ids.is_empty() {
        return Err(Error::InvalidInput("no sites to merge".into()));
    }
    if loser_ids.contains(&survivor_id) {
        return Err(Error::InvalidInput("survivor cannot also be a loser".into()));
    }

    let survivor = get(vault, survivor_id)?; // NotFound propagates
    let survivor_canon = canonical_site_name(&survivor.name);

    let mut losers: Vec<Site> = Vec::with_capacity(loser_ids.len());
    for &lid in loser_ids {
        let s = get(vault, lid)?; // NotFound propagates
        let canon = canonical_site_name(&s.name);
        if canon != survivor_canon {
            return Err(Error::InvalidInput(format!(
                "site {lid} canonical '{canon}' does not match survivor '{survivor_canon}'"
            )));
        }
        losers.push(s);
    }

    let conn = vault.conn();
    let tx = conn.unchecked_transaction()?;

    // 1. Re-point accounts (per-loser keeps the SQL trivial; loser sets are small)
    let mut accounts_repointed = 0usize;
    for &lid in loser_ids {
        accounts_repointed += conn.execute(
            "UPDATE accounts SET site_id = ?1 WHERE site_id = ?2",
            params![survivor_id, lid],
        )?;
    }

    // 2. Rename survivor + backfill empty fields from first loser that has them
    let clean = strip_url_noise(&survivor.name);
    let new_url = first_non_empty(survivor.url.clone(), losers.iter().map(|l| l.url.clone()));
    let new_category = first_non_empty(survivor.category.clone(), losers.iter().map(|l| l.category.clone()));
    let new_notes = first_non_empty(survivor.notes.clone(), losers.iter().map(|l| l.notes.clone()));
    conn.execute(
        "UPDATE sites SET name = ?1, url = ?2, category = ?3, notes = ?4 WHERE id = ?5",
        params![clean, new_url, new_category, new_notes, survivor_id],
    )?;

    // 3. Delete losers
    for &lid in loser_ids {
        conn.execute("DELETE FROM sites WHERE id = ?1", params![lid])?;
    }

    tx.commit()?;
    Ok(MergeResult {
        groups_merged: 1,
        rows_removed: loser_ids.len(),
        accounts_repointed,
    })
}

/// Merge `loser_ids` into `survivor_id` REGARDLESS of name (user-chosen). The
/// survivor keeps its name. For each loser: re-point its accounts, record an
/// alias (loser canonical -> survivor) unless the canonical already equals the
/// survivor's, re-point the loser's own aliases onto the survivor, then delete
/// the loser row. One transaction.
pub fn merge_named_sites(vault: &Vault, survivor_id: i64, loser_ids: &[i64]) -> Result<MergeResult> {
    if loser_ids.is_empty() {
        return Err(Error::InvalidInput("no sites to merge".into()));
    }
    if loser_ids.contains(&survivor_id) {
        return Err(Error::InvalidInput("survivor cannot also be a loser".into()));
    }
    let survivor = get(vault, survivor_id)?;
    let survivor_canon = canonical_site_name(&survivor.name);

    let mut losers: Vec<Site> = Vec::with_capacity(loser_ids.len());
    for &lid in loser_ids {
        losers.push(get(vault, lid)?); // NotFound propagates; NO canonical check
    }

    let conn = vault.conn();
    let tx = conn.unchecked_transaction()?;

    let mut accounts_repointed = 0usize;
    for loser in &losers {
        accounts_repointed += conn.execute(
            "UPDATE accounts SET site_id = ?1 WHERE site_id = ?2",
            params![survivor_id, loser.id],
        )?;
        let loser_canon = canonical_site_name(&loser.name);
        if loser_canon != survivor_canon && !loser_canon.is_empty() {
            site_aliases::record(vault, &loser_canon, survivor_id, &loser.name)?;
        }
        // Carry the loser's own aliases onto the new survivor.
        site_aliases::repoint(vault, loser.id, survivor_id)?;
    }

    // Backfill empty survivor fields from the first loser that has them (no rename).
    let new_url = first_non_empty(survivor.url.clone(), losers.iter().map(|l| l.url.clone()));
    let new_category = first_non_empty(survivor.category.clone(), losers.iter().map(|l| l.category.clone()));
    let new_notes = first_non_empty(survivor.notes.clone(), losers.iter().map(|l| l.notes.clone()));
    conn.execute(
        "UPDATE sites SET url = ?1, category = ?2, notes = ?3 WHERE id = ?4",
        params![new_url, new_category, new_notes, survivor_id],
    )?;

    for loser in &losers {
        conn.execute("DELETE FROM sites WHERE id = ?1", params![loser.id])?;
    }

    tx.commit()?;
    Ok(MergeResult { groups_merged: 1, rows_removed: loser_ids.len(), accounts_repointed })
}

/// Return `primary` if it is Some and non-blank; otherwise the first non-blank
/// value from `fallbacks`; otherwise `primary` unchanged (preserves None/empty).
fn first_non_empty(
    primary: Option<String>,
    fallbacks: impl Iterator<Item = Option<String>>,
) -> Option<String> {
    if primary.as_deref().map(|s| !s.trim().is_empty()).unwrap_or(false) {
        return primary;
    }
    for f in fallbacks {
        if f.as_deref().map(|s| !s.trim().is_empty()).unwrap_or(false) {
            return f;
        }
    }
    primary
}

/// True iff some account under `bare_site` and some account under `domain_site`
/// share BOTH the same username (case-insensitive, trimmed) AND the same current
/// (non-retired) password plaintext. Decrypts current passwords via the vault key.
fn credentials_corroborate(vault: &Vault, bare_site: i64, domain_site: i64) -> Result<bool> {
    // (normalized_username, current_password) for every account on a site that
    // has BOTH a non-blank username and a current password.
    fn logins(vault: &Vault, site_id: i64) -> Result<Vec<(String, String)>> {
        let ids: Vec<(i64, Option<String>)> = {
            let conn = vault.conn();
            let mut stmt = conn.prepare("SELECT id, username FROM accounts WHERE site_id = ?1")?;
            let rows = stmt.query_map(params![site_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?)))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        let mut out = Vec::new();
        for (aid, username) in ids {
            let user = match username.as_deref().map(|s| s.trim().to_lowercase()) {
                Some(u) if !u.is_empty() => u,
                _ => continue,
            };
            if let Some(pw) = passwords::current_plaintext(vault, aid)? {
                out.push((user, pw.to_string()));
            }
        }
        Ok(out)
    }

    let bare = logins(vault, bare_site)?;
    if bare.is_empty() {
        return Ok(false);
    }
    let domain = logins(vault, domain_site)?;
    Ok(bare.iter().any(|(bu, bp)| domain.iter().any(|(du, dp)| bu == du && bp == dp)))
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

/// Resolve an incoming import site name to an existing site id:
/// 1. exact/case-insensitive name match (find_by_name)
/// 2. explicit alias match (canonical -> survivor)
/// 3. canonical match against an existing site (auto-dedup: "zotacstore.com"
///    finds existing "zotacstore", so a URL-shaped re-import does not re-add it)
/// None -> caller creates a new site.
pub fn resolve_for_import(vault: &Vault, name: &str) -> Result<Option<i64>> {
    if let Some(s) = find_by_name(vault, name)? {
        return Ok(Some(s.id));
    }
    let canon = canonical_site_name(name);
    if canon.is_empty() {
        return Ok(None);
    }
    if let Some(id) = crate::repo::site_aliases::resolve(vault, &canon)? {
        return Ok(Some(id));
    }
    for s in list(vault)? {
        if canonical_site_name(&s.name) == canon {
            return Ok(Some(s.id));
        }
    }
    Ok(None)
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
    use crate::repo::accounts::{self, NewAccount};
    use crate::repo::passwords::{self, NewPassword, Confidence};
    use crate::repo::site_aliases;
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
        assert_eq!(raw_name(&v, s.id), "github.com");
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
        // "https://www.github.com" wants to rename to "github.com", but a
        // separate "github.com" row already occupies that canonical slot —
        // both canonicalize to "github.com", so the URL row is skipped.
        let (_tmp, v) = vault();
        clear_cleanup_flag(&v);
        let url_row = create(&v, NewSite {
            name: "https://www.github.com".into(), ..Default::default()
        }).unwrap();
        let bare_row = create(&v, NewSite {
            name: "github.com".into(), ..Default::default()
        }).unwrap();
        let report = cleanup_url_shaped_site_names(v.conn()).unwrap();
        assert_eq!(report.renamed, 0);
        assert_eq!(report.skipped_collisions, 1);
        // URL row left unchanged.
        assert_eq!(raw_name(&v, url_row.id), "https://www.github.com");
        // Already-clean row also untouched.
        assert_eq!(raw_name(&v, bare_row.id), "github.com");
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
        assert_eq!(raw_name(&v, s.id), "github.com");
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
        assert_eq!(raw_name(&v, s.id), "github.com");
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

    // Phase 4.24 ---------------------------------------------------------------

    #[test]
    fn find_merge_groups_lists_only_multi_row_canonicals() {
        let (_tmp, v) = vault();
        // Two rows that canonicalize to "github.com"
        let url = create(&v, NewSite { name: "https://www.github.com".into(), ..Default::default() }).unwrap();
        create(&v, NewSite { name: "github.com".into(), ..Default::default() }).unwrap();
        // A lone site -> not a group
        create(&v, NewSite { name: "reddit.com".into(), ..Default::default() }).unwrap();
        // Give the URL row one account so counts are exercised
        accounts::create(&v, NewAccount { site_id: url.id, username: Some("me".into()), ..Default::default() }).unwrap();

        let groups = find_merge_groups(&v).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].canonical, "github.com");
        assert_eq!(groups[0].members.len(), 2);
        assert_eq!(groups[0].total_accounts, 1);
    }

    #[test]
    fn find_merge_groups_picks_already_clean_survivor() {
        let (_tmp, v) = vault();
        create(&v, NewSite { name: "https://www.github.com".into(), ..Default::default() }).unwrap();
        let clean = create(&v, NewSite { name: "github.com".into(), ..Default::default() }).unwrap();
        let groups = find_merge_groups(&v).unwrap();
        assert_eq!(groups[0].survivor_id, clean.id, "the already-clean 'github.com' row wins");
        assert_eq!(groups[0].clean_name, "github.com");
        assert_eq!(groups[0].members[0].site_id, clean.id, "survivor listed first");
    }

    #[test]
    fn merge_sites_repoints_accounts_and_deletes_losers() {
        let (_tmp, v) = vault();
        let url = create(&v, NewSite { name: "https://www.github.com".into(), ..Default::default() }).unwrap();
        let clean = create(&v, NewSite { name: "github.com".into(), ..Default::default() }).unwrap();
        let a = accounts::create(&v, NewAccount { site_id: url.id, username: Some("me".into()), ..Default::default() }).unwrap();

        let res = merge_sites(&v, clean.id, &[url.id]).unwrap();
        assert_eq!(res.groups_merged, 1);
        assert_eq!(res.rows_removed, 1);
        assert_eq!(res.accounts_repointed, 1);

        assert!(matches!(get(&v, url.id), Err(Error::NotFound)), "loser row deleted");
        assert_eq!(get(&v, clean.id).unwrap().name, "github.com", "survivor renamed to registrable domain");
        let under = accounts::list_for_site(&v, clean.id, &[]).unwrap();
        assert_eq!(under.len(), 1);
        assert_eq!(under[0].id, a.id, "account now under survivor");
    }

    #[test]
    fn merge_sites_backfills_empty_survivor_fields() {
        let (_tmp, v) = vault();
        let survivor = create(&v, NewSite { name: "github.com".into(), category: None, ..Default::default() }).unwrap();
        let loser = create(&v, NewSite {
            name: "https://github.com".into(),
            category: Some("Dev".into()),
            notes: Some("from import".into()),
            ..Default::default()
        }).unwrap();
        merge_sites(&v, survivor.id, &[loser.id]).unwrap();
        let s = get(&v, survivor.id).unwrap();
        assert_eq!(s.category.as_deref(), Some("Dev"));
        assert_eq!(s.notes.as_deref(), Some("from import"));
    }

    #[test]
    fn merge_sites_preserves_password_history() {
        let (_tmp, v) = vault();
        let loser = create(&v, NewSite { name: "https://www.github.com".into(), ..Default::default() }).unwrap();
        let survivor = create(&v, NewSite { name: "github.com".into(), ..Default::default() }).unwrap();
        let a = accounts::create(&v, NewAccount { site_id: loser.id, username: Some("me".into()), ..Default::default() }).unwrap();
        passwords::insert(&v, NewPassword {
            account_id: a.id,
            plaintext: "pw1",
            source: "manual".into(),
            confidence: Confidence::Certain,
            notes: None,
            created_at: None,
        }).unwrap();

        merge_sites(&v, survivor.id, &[loser.id]).unwrap();

        let n: i64 = v.conn().query_row(
            "SELECT COUNT(*) FROM password_history WHERE account_id = ?1",
            params![a.id],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(n, 1, "password history rides along with the account");
    }

    #[test]
    fn merge_sites_rejects_cross_canonical_or_unknown_ids() {
        let (_tmp, v) = vault();
        let gh = create(&v, NewSite { name: "github.com".into(), ..Default::default() }).unwrap();
        let rd = create(&v, NewSite { name: "reddit.com".into(), ..Default::default() }).unwrap();
        assert!(matches!(merge_sites(&v, gh.id, &[rd.id]), Err(Error::InvalidInput(_))), "cross-canonical rejected");
        assert!(matches!(merge_sites(&v, gh.id, &[9999]), Err(Error::NotFound)), "unknown loser rejected");
        assert!(matches!(merge_sites(&v, gh.id, &[gh.id]), Err(Error::InvalidInput(_))), "survivor-as-loser rejected");
    }

    // Phase 4.29 ---------------------------------------------------------------

    #[test]
    fn merge_named_sites_records_alias_and_repoints() {
        let (_tmp, v) = vault();
        let runescape = create(&v, NewSite { name: "RuneScape".into(), ..Default::default() }).unwrap();
        let jagex = create(&v, NewSite { name: "Jagex".into(), ..Default::default() }).unwrap();
        let a = accounts::create(&v, NewAccount { site_id: jagex.id, username: Some("me".into()), ..Default::default() }).unwrap();

        let res = merge_named_sites(&v, runescape.id, &[jagex.id]).unwrap();
        assert_eq!(res.rows_removed, 1);
        assert_eq!(res.accounts_repointed, 1);

        // Jagex row gone; account now under RuneScape.
        assert!(matches!(get(&v, jagex.id), Err(Error::NotFound)));
        assert_eq!(accounts::list_for_site(&v, runescape.id, &[]).unwrap()[0].id, a.id);
        // Survivor keeps its name (NOT renamed to a clean brand).
        assert_eq!(get(&v, runescape.id).unwrap().name, "RuneScape");
        // Alias jagex -> RuneScape recorded.
        assert_eq!(site_aliases::resolve(&v, "jagex").unwrap(), Some(runescape.id));
    }

    #[test]
    fn merge_named_sites_repoints_existing_loser_aliases() {
        let (_tmp, v) = vault();
        let survivor = create(&v, NewSite { name: "Survivor".into(), ..Default::default() }).unwrap();
        let loser = create(&v, NewSite { name: "Loser".into(), ..Default::default() }).unwrap();
        // Loser already has an alias of its own.
        site_aliases::record(&v, "oldname", loser.id, "OldName").unwrap();
        merge_named_sites(&v, survivor.id, &[loser.id]).unwrap();
        // The loser's alias now points at the survivor.
        assert_eq!(site_aliases::resolve(&v, "oldname").unwrap(), Some(survivor.id));
        assert_eq!(site_aliases::resolve(&v, "loser").unwrap(), Some(survivor.id));
    }

    #[test]
    fn merge_named_sites_rejects_bad_ids() {
        let (_tmp, v) = vault();
        let a = create(&v, NewSite { name: "A".into(), ..Default::default() }).unwrap();
        assert!(matches!(merge_named_sites(&v, a.id, &[a.id]), Err(Error::InvalidInput(_))));
        assert!(matches!(merge_named_sites(&v, a.id, &[9999]), Err(Error::NotFound)));
        assert!(matches!(merge_named_sites(&v, a.id, &[]), Err(Error::InvalidInput(_))));
    }

    // Phase 4.31 -------------------------------------------------------------

    fn add_login(v: &Vault, site_id: i64, user: &str, pw: &str) -> i64 {
        let a = accounts::create(v, NewAccount { site_id, username: Some(user.into()), ..Default::default() }).unwrap();
        passwords::insert(v, NewPassword {
            account_id: a.id, plaintext: pw, source: "manual".into(),
            confidence: Confidence::Certain, notes: None, created_at: None,
        }).unwrap();
        a.id
    }

    #[test]
    fn credentials_corroborate_requires_user_and_password() {
        let (_tmp, v) = vault();
        let bare = create(&v, NewSite { name: "reddit".into(), ..Default::default() }).unwrap();
        let dom = create(&v, NewSite { name: "reddit.com".into(), ..Default::default() }).unwrap();
        add_login(&v, bare.id, "chris@example.com", "hunter2");
        add_login(&v, dom.id, "chris@example.com", "hunter2");
        assert!(credentials_corroborate(&v, bare.id, dom.id).unwrap(), "same user + same pw matches");
    }

    #[test]
    fn credentials_corroborate_username_match_password_differ_is_false() {
        let (_tmp, v) = vault();
        let bare = create(&v, NewSite { name: "reddit".into(), ..Default::default() }).unwrap();
        let dom = create(&v, NewSite { name: "reddit.com".into(), ..Default::default() }).unwrap();
        add_login(&v, bare.id, "chris@example.com", "hunter2");
        add_login(&v, dom.id, "chris@example.com", "DIFFERENT");
        assert!(!credentials_corroborate(&v, bare.id, dom.id).unwrap(), "password must also match");
    }

    #[test]
    fn credentials_corroborate_password_match_username_differ_is_false() {
        let (_tmp, v) = vault();
        let bare = create(&v, NewSite { name: "reddit".into(), ..Default::default() }).unwrap();
        let dom = create(&v, NewSite { name: "reddit.com".into(), ..Default::default() }).unwrap();
        add_login(&v, bare.id, "alt@example.com", "hunter2");
        add_login(&v, dom.id, "chris@example.com", "hunter2");
        assert!(!credentials_corroborate(&v, bare.id, dom.id).unwrap(), "username must also match");
    }

    #[test]
    fn credentials_corroborate_username_case_insensitive() {
        let (_tmp, v) = vault();
        let bare = create(&v, NewSite { name: "reddit".into(), ..Default::default() }).unwrap();
        let dom = create(&v, NewSite { name: "reddit.com".into(), ..Default::default() }).unwrap();
        add_login(&v, bare.id, "Chris@Example.com ", "hunter2");
        add_login(&v, dom.id, "chris@example.com", "hunter2");
        assert!(credentials_corroborate(&v, bare.id, dom.id).unwrap(), "username compared lower/trimmed");
    }

    #[test]
    fn credentials_corroborate_no_accounts_is_false() {
        let (_tmp, v) = vault();
        let bare = create(&v, NewSite { name: "reddit".into(), ..Default::default() }).unwrap();
        let dom = create(&v, NewSite { name: "reddit.com".into(), ..Default::default() }).unwrap();
        add_login(&v, dom.id, "chris@example.com", "hunter2"); // bare has no accounts
        assert!(!credentials_corroborate(&v, bare.id, dom.id).unwrap(), "no bare login -> no corroboration");
    }

    // Phase 4.29 Task 3 -----------------------------------------------------------

    #[test]
    fn resolve_for_import_name_then_alias() {
        let (_tmp, v) = vault();
        let rs = create(&v, NewSite { name: "RuneScape".into(), ..Default::default() }).unwrap();
        // Exact/case-insensitive name match.
        assert_eq!(resolve_for_import(&v, "runescape").unwrap(), Some(rs.id));
        // Alias match when no name matches.
        site_aliases::record(&v, "jagex", rs.id, "Jagex").unwrap();
        assert_eq!(resolve_for_import(&v, "Jagex").unwrap(), Some(rs.id));
        // Neither -> None.
        assert_eq!(resolve_for_import(&v, "TotallyNewSite").unwrap(), None);
    }

    #[test]
    fn resolve_for_import_dedups_by_canonical() {
        let (_tmp, v) = vault();
        let z = create(&v, NewSite { name: "zotacstore.com".into(), ..Default::default() }).unwrap();
        // A URL-shaped re-import of the same registrable domain finds the existing
        // site by canonical name (the Chrome re-import "everything is new" fix).
        assert_eq!(resolve_for_import(&v, "www.zotacstore.com").unwrap(), Some(z.id));
        assert_eq!(resolve_for_import(&v, "https://www.zotacstore.com/login").unwrap(), Some(z.id));
        // A genuinely new brand still returns None.
        assert_eq!(resolve_for_import(&v, "brandnewsite.com").unwrap(), None);
    }
}
