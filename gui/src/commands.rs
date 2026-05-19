//! Tauri IPC commands. Each public `#[tauri::command]` function delegates to
//! an `_inner` helper that takes a plain `&VaultState`, so unit tests can
//! exercise the command logic without spinning up the Tauri runtime.

use crate::error::GuiError;
use crate::state::VaultState;
use passhound_core::{repo, Vault};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use tauri::State;
use zeroize::Zeroizing;

// rusqlite is a direct dep (same version + features as core) so that the
// From<rusqlite::Error> impl and the conn().prepare() join queries compile.

/// Default vault path: `~/.local/share/passhound/vault.db` on Linux,
/// matching the CLI's default.
fn default_vault_path() -> Result<PathBuf, GuiError> {
    let dir = dirs::data_local_dir()
        .ok_or_else(|| GuiError::Internal("could not resolve data_local_dir".into()))?;
    Ok(dir.join("passhound").join("vault.db"))
}

// ============================================================================
// Vault lifecycle
// ============================================================================

#[tauri::command]
pub fn vault_exists(_state: State<'_, VaultState>) -> Result<bool, GuiError> {
    vault_exists_inner(&default_vault_path()?)
}

pub fn vault_exists_inner(path: &std::path::Path) -> Result<bool, GuiError> {
    Ok(path.is_file())
}

#[tauri::command]
pub fn vault_create(
    state: State<'_, VaultState>,
    master_pw: String,
) -> Result<(), GuiError> {
    let pw = Zeroizing::new(master_pw);
    let path = default_vault_path()?;
    vault_create_inner(&state, &path, pw.as_bytes())
}

pub fn vault_create_inner(
    state: &VaultState,
    path: &std::path::Path,
    master_pw: &[u8],
) -> Result<(), GuiError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let v = Vault::create(path, master_pw)?;
    let mut guard = state.vault.lock().map_err(poisoned)?;
    *guard = Some(v);
    Ok(())
}

#[tauri::command]
pub fn vault_unlock(
    state: State<'_, VaultState>,
    master_pw: String,
) -> Result<(), GuiError> {
    let pw = Zeroizing::new(master_pw);
    let path = default_vault_path()?;
    vault_unlock_inner(&state, &path, pw.as_bytes())
}

pub fn vault_unlock_inner(
    state: &VaultState,
    path: &std::path::Path,
    master_pw: &[u8],
) -> Result<(), GuiError> {
    if !path.is_file() {
        return Err(GuiError::NotFound);
    }
    let mut v = Vault::open(path)?;
    v.unlock(master_pw)?;
    let mut guard = state.vault.lock().map_err(poisoned)?;
    *guard = Some(v);
    Ok(())
}

#[tauri::command]
pub fn vault_lock(state: State<'_, VaultState>) -> Result<(), GuiError> {
    vault_lock_inner(&state)
}

pub fn vault_lock_inner(state: &VaultState) -> Result<(), GuiError> {
    let mut guard = state.vault.lock().map_err(poisoned)?;
    *guard = None;
    // Clear both in-memory caches — they outlive the vault otherwise.
    let mut candidates = state.candidate_cache.lock().map_err(poisoned)?;
    candidates.clear();
    let mut pending = state.pending_import_path.lock().map_err(poisoned)?;
    *pending = None;
    Ok(())
}

// ============================================================================
// Read commands
// ============================================================================

#[derive(Serialize, Debug, Clone)]
pub struct TagSummary {
    pub id: i64,
    pub name: String,
}

#[derive(Serialize, Debug, Clone)]
pub struct TagWithCount {
    pub id: i64,
    pub name: String,
    pub account_count: i64,
}

#[derive(Serialize)]
pub struct AccountSummary {
    pub id: i64,
    pub site_name: String,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub last_changed: Option<String>,
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Vec<TagSummary>,
}

#[tauri::command]
pub fn list_accounts(
    state: State<'_, VaultState>,
    filter: Option<String>,
    tag_ids: Option<Vec<i64>>,
    era_id: Option<i64>,
) -> Result<Vec<AccountSummary>, GuiError> {
    list_accounts_inner(&state, filter.as_deref(), tag_ids, era_id)
}

pub fn list_accounts_inner(
    state: &VaultState,
    filter: Option<&str>,
    tag_ids: Option<Vec<i64>>,
    era_id: Option<i64>,
) -> Result<Vec<AccountSummary>, GuiError> {
    // TODO(perf): the guard is held across the SQL / decrypt call below —
    // acceptable for the MVP single-user case but a `vault_lock` IPC would
    // stall waiting on a slow query. Revisit if list latency becomes an issue.
    // `Some(vault)` always implies unlocked here because `vault_create_inner`
    // and `vault_unlock_inner` are the only writers and they store post-unlock
    // vaults; downstream repo calls invoke `require_key()` which will surface
    // `core::Error::Locked → GuiError::Locked` defensively if invariant breaks.
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    // Resolve era filter to a date window. An unknown era id returns
    // empty list silently (stale frontend filter state shouldn't crash).
    let era_window: Option<(Option<String>, Option<String>)> = match era_id {
        None => None,
        Some(id) => {
            let row: Option<(Option<String>, Option<String>)> = v
                .conn()
                .query_row(
                    "SELECT start_date, end_date FROM eras WHERE id = ?1",
                    rusqlite::params![id],
                    |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?)),
                )
                .ok();
            match row {
                None => return Ok(Vec::new()), // unknown era → empty list silently
                Some((None, None)) => None,    // era has no dates → no filter
                Some(window) => Some(window),
            }
        }
    };
    // Joined query: accounts × sites + max(password_history.created_at) for the
    // most-recent password (current or retired). Ordered by last_changed desc
    // with NULLs (accounts with no password history) last.
    let needle = filter
        .map(|s| s.to_lowercase())
        .filter(|s| !s.is_empty());
    let tag_filter = tag_ids.as_deref().unwrap_or(&[]);
    // Build the base SQL with optional tag join/filter.
    let sql = if tag_filter.is_empty() {
        "SELECT a.id, s.name, a.username, a.display_name, s.category,
                (SELECT MAX(ph.created_at) FROM password_history ph WHERE ph.account_id = a.id) AS last_changed
         FROM accounts a
         JOIN sites s ON s.id = a.site_id
         ORDER BY last_changed DESC NULLS LAST, s.name ASC".to_string()
    } else {
        let placeholders = tag_filter
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "SELECT a.id, s.name, a.username, a.display_name, s.category,
                    (SELECT MAX(ph.created_at) FROM password_history ph WHERE ph.account_id = a.id) AS last_changed
             FROM accounts a
             JOIN sites s ON s.id = a.site_id
             JOIN account_tags at ON at.account_id = a.id
             WHERE at.tag_id IN ({placeholders})
             GROUP BY a.id
             HAVING COUNT(DISTINCT at.tag_id) = ?{count_param}
             ORDER BY last_changed DESC NULLS LAST, s.name ASC",
            placeholders = placeholders,
            count_param = tag_filter.len() + 1,
        )
    };
    let mut stmt = v.conn().prepare(&sql)?;
    let rows: Vec<(i64, String, Option<String>, Option<String>, Option<String>, Option<String>)> =
        if tag_filter.is_empty() {
            stmt.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                ))
            })?.collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            let mut params_vec: Vec<&dyn rusqlite::ToSql> =
                tag_filter.iter().map(|x| x as &dyn rusqlite::ToSql).collect();
            let count: i64 = tag_filter.len() as i64;
            params_vec.push(&count);
            stmt.query_map(params_vec.as_slice(), |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                ))
            })?.collect::<rusqlite::Result<Vec<_>>>()?
        };
    let mut out: Vec<AccountSummary> = Vec::new();
    for (id, site_name, username, display_name, category, last_changed) in rows {
        if let Some(needle) = &needle {
            let hay = format!(
                "{} {} {} {}",
                site_name.to_lowercase(),
                username.as_deref().unwrap_or("").to_lowercase(),
                display_name.as_deref().unwrap_or("").to_lowercase(),
                category.as_deref().unwrap_or("").to_lowercase()
            );
            if !hay.contains(needle) {
                continue;
            }
        }
        out.push(AccountSummary {
            id,
            site_name,
            username,
            display_name,
            last_changed,
            category,
            tags: Vec::new(),
        });
    }
    // Era filter post-pass. An era with both dates NULL is a no-op
    // (we returned None at resolve time). Each retained account has
    // at least one password_history row in the (start, end) window.
    if let Some((era_start, era_end)) = era_window {
        let mut filter_stmt = v.conn().prepare(
            "SELECT 1 FROM password_history
             WHERE account_id = ?1
               AND retired_at IS NULL
               AND (?2 IS NULL OR created_at >= ?2)
               AND (?3 IS NULL OR created_at <= ?3)
             LIMIT 1",
        )?;
        out.retain(|a| {
            filter_stmt
                .exists(rusqlite::params![a.id, era_start.as_deref(), era_end.as_deref()])
                .unwrap_or(false)
        });
    }
    // Merge tags via a single join query over all collected account ids.
    let account_ids: Vec<i64> = out.iter().map(|a| a.id).collect();
    let mut tags_map = fetch_tags_by_account(v, &account_ids)?;
    for summary in &mut out {
        if let Some(tags) = tags_map.remove(&summary.id) {
            summary.tags = tags;
        }
    }
    Ok(out)
}

#[derive(Serialize)]
pub struct AccountDetail {
    pub id: i64,
    pub site_id: i64,
    pub site_name: String,
    pub site_url: Option<String>,
    pub site_category: Option<String>,
    pub site_abbreviations: Vec<String>,
    pub site_notes: Option<String>,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub alias: Option<String>,
    pub notes: Option<String>,
    pub history: Vec<HistoryEntry>,
    #[serde(default)]
    pub tags: Vec<TagSummary>,
}

#[derive(Serialize)]
pub struct HistoryEntry {
    pub id: i64,
    pub created_at: String,
    pub source: String,
    pub is_current: bool,
    pub notes: Option<String>,
}

#[tauri::command]
pub fn get_account(
    state: State<'_, VaultState>,
    id: i64,
) -> Result<AccountDetail, GuiError> {
    get_account_inner(&state, id)
}

pub fn get_account_inner(state: &VaultState, id: i64) -> Result<AccountDetail, GuiError> {
    // TODO(perf): the guard is held across the SQL / decrypt call below —
    // acceptable for the MVP single-user case but a `vault_lock` IPC would
    // stall waiting on a slow query. Revisit if list latency becomes an issue.
    // `Some(vault)` always implies unlocked here because `vault_create_inner`
    // and `vault_unlock_inner` are the only writers and they store post-unlock
    // vaults; downstream repo calls invoke `require_key()` which will surface
    // `core::Error::Locked → GuiError::Locked` defensively if invariant breaks.
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    let acct = repo::accounts::get(v, id)?;
    let site = repo::sites::get(v, acct.site_id)?;
    let history_records = repo::passwords::list_history(v, id)?;
    let history: Vec<HistoryEntry> = history_records
        .into_iter()
        .map(|r| HistoryEntry {
            id: r.id,
            created_at: r.created_at.to_rfc3339(),
            source: r.source,
            is_current: r.retired_at.is_none(),
            notes: r.notes,
        })
        .collect();
    let raw_tags = repo::account_tags::list_for_account(v, id)?;
    let tags: Vec<TagSummary> = raw_tags
        .into_iter()
        .map(|t| TagSummary { id: t.id, name: t.name })
        .collect();
    Ok(AccountDetail {
        id: acct.id,
        site_id: acct.site_id,
        site_name: site.name,
        site_url: site.url,
        site_category: site.category,
        site_abbreviations: site.abbreviations,
        site_notes: site.notes.clone(),
        username: acct.username,
        display_name: acct.display_name,
        alias: acct.alias,
        notes: acct.notes,
        history,
        tags,
    })
}

#[tauri::command]
pub fn reveal_password(
    state: State<'_, VaultState>,
    history_id: i64,
) -> Result<String, GuiError> {
    reveal_password_inner(&state, history_id)
}

pub fn reveal_password_inner(
    state: &VaultState,
    history_id: i64,
) -> Result<String, GuiError> {
    // TODO(perf): the guard is held across the SQL / decrypt call below —
    // acceptable for the MVP single-user case but a `vault_lock` IPC would
    // stall waiting on a slow query. Revisit if list latency becomes an issue.
    // `Some(vault)` always implies unlocked here because `vault_create_inner`
    // and `vault_unlock_inner` are the only writers and they store post-unlock
    // vaults; downstream repo calls invoke `require_key()` which will surface
    // `core::Error::Locked → GuiError::Locked` defensively if invariant breaks.
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    let plaintext = repo::passwords::decrypt_record(v, history_id)?;
    // The Zeroizing<String> goes out of scope after this clone, so the
    // intermediate buffer zeros. The String returned to Tauri is on the JS
    // side after IPC serialization — that copy is unprotected (documented in
    // spec; out of scope to harden in 4.1).
    Ok(plaintext.as_str().to_owned())
}

#[tauri::command]
pub fn copy_to_clipboard(
    app: tauri::AppHandle,
    text: String,
) -> Result<(), GuiError> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    // No Zeroizing wrap here: the OS clipboard owns the plaintext after this
    // call returns. Zeroing the local String doesn't affect the clipboard
    // buffer. Clipboard auto-clear is Settings work (Phase 4.6).
    app.clipboard()
        .write_text(text)
        .map_err(|e| GuiError::Internal(format!("clipboard: {e}")))
}

// ============================================================================
// Helpers
// ============================================================================

fn poisoned<T>(_: std::sync::PoisonError<T>) -> GuiError {
    GuiError::Internal("vault state mutex poisoned".into())
}

/// Single join query that fetches all tags for a set of account ids, returning
/// a map from account_id to its tag list (ordered by LOWER(tag.name)).
fn fetch_tags_by_account(v: &Vault, account_ids: &[i64]) -> Result<HashMap<i64, Vec<TagSummary>>, GuiError> {
    if account_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = (1..=account_ids.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT at.account_id, t.id, t.name
         FROM account_tags at
         JOIN tags t ON t.id = at.tag_id
         WHERE at.account_id IN ({placeholders})
         ORDER BY at.account_id, LOWER(t.name)"
    );
    let mut stmt = v.conn().prepare(&sql)?;
    let params: Vec<&dyn rusqlite::ToSql> =
        account_ids.iter().map(|x| x as &dyn rusqlite::ToSql).collect();
    let mut map: HashMap<i64, Vec<TagSummary>> = HashMap::new();
    let rows = stmt.query_map(params.as_slice(), |row| {
        Ok((
            row.get::<_, i64>(0)?,
            TagSummary {
                id: row.get(1)?,
                name: row.get(2)?,
            },
        ))
    })?;
    for r in rows {
        let (aid, ts) = r?;
        map.entry(aid).or_default().push(ts);
    }
    Ok(map)
}

// ============================================================================
// CSV import (Phase 4.2)
// ============================================================================

/// TS-friendly mirror of `passhound_core::importer::csv::Mapping`. Lives in the
/// gui crate so the IPC contract doesn't expose core's internal type directly.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MappingArgs {
    pub site: Option<usize>,
    pub url: Option<usize>,
    pub username: Option<usize>,
    #[serde(default)]
    pub display_name: Option<usize>,
    pub password: usize,
    pub notes: Option<usize>,
    pub created_at: Option<usize>,
    #[serde(default)]
    pub extras_into_notes: Vec<usize>,
}

impl From<MappingArgs> for passhound_core::importer::csv::Mapping {
    fn from(m: MappingArgs) -> Self {
        passhound_core::importer::csv::Mapping {
            site: m.site,
            url: m.url,
            username: m.username,
            display_name: m.display_name,
            password: m.password,
            notes: m.notes,
            created_at: m.created_at,
            extras_into_notes: m.extras_into_notes,
        }
    }
}

impl From<passhound_core::importer::csv::Mapping> for MappingArgs {
    fn from(m: passhound_core::importer::csv::Mapping) -> Self {
        MappingArgs {
            site: m.site,
            url: m.url,
            username: m.username,
            display_name: m.display_name,
            password: m.password,
            notes: m.notes,
            created_at: m.created_at,
            extras_into_notes: m.extras_into_notes,
        }
    }
}

/// IPC-side mirror of `passhound_core::importer::RowPatch`. Frontend
/// sends one per skipped row the user wants to patch.
#[derive(serde::Deserialize, Debug, Clone)]
pub struct RowPatchArgs {
    pub row: usize,
    pub site: Option<String>,
    pub password: Option<String>,
}

impl From<RowPatchArgs> for passhound_core::importer::RowPatch {
    fn from(a: RowPatchArgs) -> Self {
        passhound_core::importer::RowPatch {
            row: a.row,
            site: a.site,
            password: a.password,
        }
    }
}

/// Serializable subset of `passhound_core::importer::PartialEntry`.
/// Omits the plaintext password (the frontend doesn't need to SEE it
/// for patching — only knowing whether it exists) per Phase 4.16 M-2
/// minimization principle.
#[derive(serde::Serialize, Debug, Clone)]
pub struct PreviewPartial {
    pub site: Option<String>,
    pub url: Option<String>,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub has_password: bool,
    pub notes: Option<String>,
}

impl From<&passhound_core::importer::PartialEntry> for PreviewPartial {
    fn from(p: &passhound_core::importer::PartialEntry) -> Self {
        Self {
            site: p.site.clone(),
            url: p.url.clone(),
            username: p.username.clone(),
            display_name: p.display_name.clone(),
            has_password: p.password.is_some(),
            notes: p.notes.clone(),
        }
    }
}

/// Structured diagnostic for the frontend's SkippedRowsPanel.
#[derive(serde::Serialize, Debug, Clone)]
pub struct PreviewDiagnostic {
    pub row: usize,
    pub raw: String,
    pub reason: String,
    pub parsed: PreviewPartial,
}

#[derive(Debug, Serialize)]
pub struct PreviewCounts {
    pub new: usize,
    pub duplicates: usize,
    pub merges: usize,
    pub errors: usize,
}

#[derive(Serialize)]
pub struct SampleRow {
    pub site: String,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub password_length: usize,
    pub notes: Option<String>,
}

#[derive(Serialize)]
pub struct PreviewResult {
    pub headers: Vec<String>,
    pub detected_mapping: MappingArgs,
    pub effective_mapping: MappingArgs,
    pub counts: PreviewCounts,
    pub sample_rows: Vec<SampleRow>,
    pub diagnostics: Vec<PreviewDiagnostic>,
}

#[derive(Debug, Serialize)]
pub struct CommitResult {
    pub import_id: i64,
    pub counts: PreviewCounts,
}

const SAMPLE_ROW_LIMIT: usize = 5;

pub fn import_csv_dry_run_inner(
    state: &VaultState,
    path: &std::path::Path,
    site_override: Option<String>,
    mapping: Option<MappingArgs>,
    patches: Vec<passhound_core::importer::RowPatch>,
) -> Result<PreviewResult, GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;

    // Read headers ourselves so we can return them to the frontend even if
    // auto_detect or parse_file would have failed.
    let headers = read_csv_headers(path)?;
    let detected = passhound_core::importer::csv::auto_detect(&headers)
        .map(MappingArgs::from)
        .ok_or_else(|| GuiError::InvalidInput(
            "CSV has no recognizable password column".into(),
        ))?;
    let effective: passhound_core::importer::csv::Mapping = match &mapping {
        Some(m) => m.clone().into(),
        None => detected.clone().into(),
    };

    let parse = passhound_core::importer::csv::parse_file(
        v,
        path,
        Some(effective.clone()),
        site_override,
    )?;
    let parse = passhound_core::importer::pipeline::apply_patches(parse, &patches);

    let preview = passhound_core::importer::pipeline::preview(v, parse.entries.clone())?;

    let counts = PreviewCounts {
        new: preview.new,
        duplicates: preview.duplicates,
        merges: preview.merges,
        errors: parse.diagnostics.len(),
    };

    // Always-redacted sample rows.
    let sample_rows: Vec<SampleRow> = parse
        .entries
        .iter()
        .take(SAMPLE_ROW_LIMIT)
        .map(|e| SampleRow {
            site: e.site.clone(),
            username: e.username.clone(),
            display_name: e.display_name.clone(),
            password_length: e.password.chars().count(),
            notes: e.notes.clone(),
        })
        .collect();

    let diagnostics: Vec<PreviewDiagnostic> = parse
        .diagnostics
        .iter()
        .map(|d| PreviewDiagnostic {
            row: d.row,
            raw: d.raw.clone(),
            reason: d.reason.clone(),
            parsed: (&d.parsed).into(),
        })
        .collect();

    Ok(PreviewResult {
        headers,
        detected_mapping: detected,
        effective_mapping: effective.into(),
        counts,
        sample_rows,
        diagnostics,
    })
}

pub fn import_csv_commit_inner(
    state: &VaultState,
    path: &std::path::Path,
    site_override: Option<String>,
    mapping: Option<MappingArgs>,
    patches: Vec<passhound_core::importer::RowPatch>,
) -> Result<CommitResult, GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;

    let effective: passhound_core::importer::csv::Mapping = match mapping {
        Some(m) => m.into(),
        None => {
            let headers = read_csv_headers(path)?;
            passhound_core::importer::csv::auto_detect(&headers).ok_or_else(|| {
                GuiError::InvalidInput("CSV has no recognizable password column".into())
            })?
        }
    };

    let parse = passhound_core::importer::csv::parse_file(
        v,
        path,
        Some(effective),
        site_override,
    )?;
    let parse = passhound_core::importer::pipeline::apply_patches(parse, &patches);
    let preview = passhound_core::importer::pipeline::preview(v, parse.entries.clone())?;

    let counts = PreviewCounts {
        new: preview.new,
        duplicates: preview.duplicates,
        merges: preview.merges,
        errors: parse.diagnostics.len(),
    };

    let import_id = passhound_core::importer::pipeline::commit(
        v,
        preview,
        "csv",
        Some(path),
    )?;

    Ok(CommitResult {
        import_id: import_id.0,
        counts,
    })
}

#[tauri::command]
pub async fn pick_and_import_csv_dry_run(
    app: tauri::AppHandle,
    state: State<'_, VaultState>,
    site_override: Option<String>,
    mapping: Option<MappingArgs>,
    patches: Vec<RowPatchArgs>,
) -> Result<Option<PreviewResult>, GuiError> {
    use tauri_plugin_dialog::DialogExt;

    let picked = app
        .dialog()
        .file()
        .add_filter("CSV", &["csv"])
        .blocking_pick_file();
    let Some(file_path) = picked else { return Ok(None); };

    let pb: std::path::PathBuf = file_path
        .into_path()
        .map_err(|e| GuiError::Internal(format!("dialog path: {e}")))?;

    // Stash the path for the eventual commit.
    {
        let mut slot = state.pending_import_path.lock().map_err(poisoned)?;
        *slot = Some(pb.clone());
    }

    let core_patches: Vec<passhound_core::importer::RowPatch> =
        patches.into_iter().map(Into::into).collect();
    let preview = import_csv_dry_run_inner(&state, &pb, site_override, mapping, core_patches)?;
    Ok(Some(preview))
}

#[tauri::command]
pub fn import_csv_commit_pending(
    state: State<'_, VaultState>,
    site_override: Option<String>,
    mapping: Option<MappingArgs>,
    patches: Vec<RowPatchArgs>,
) -> Result<CommitResult, GuiError> {
    import_csv_commit_pending_inner(&state, site_override, mapping, patches.into_iter().map(Into::into).collect())
}

pub fn import_csv_commit_pending_inner(
    state: &VaultState,
    site_override: Option<String>,
    mapping: Option<MappingArgs>,
    patches: Vec<passhound_core::importer::RowPatch>,
) -> Result<CommitResult, GuiError> {
    let path = {
        let mut slot = state.pending_import_path.lock().map_err(poisoned)?;
        slot.take().ok_or(GuiError::NoPendingImport)?
    };
    import_csv_commit_inner(state, &path, site_override, mapping, patches)
}

#[tauri::command]
pub fn cancel_pending_import(
    state: State<'_, VaultState>,
) -> Result<(), GuiError> {
    cancel_pending_import_inner(&state)
}

pub fn cancel_pending_import_inner(
    state: &VaultState,
) -> Result<(), GuiError> {
    let mut slot = state.pending_import_path.lock().map_err(poisoned)?;
    *slot = None;
    Ok(())
}

#[tauri::command]
pub fn import_csv_dry_run_with_pending(
    state: State<'_, VaultState>,
    site_override: Option<String>,
    mapping: Option<MappingArgs>,
    patches: Vec<RowPatchArgs>,
) -> Result<PreviewResult, GuiError> {
    import_csv_dry_run_with_pending_inner(&state, site_override, mapping, patches.into_iter().map(Into::into).collect())
}

pub fn import_csv_dry_run_with_pending_inner(
    state: &VaultState,
    site_override: Option<String>,
    mapping: Option<MappingArgs>,
    patches: Vec<passhound_core::importer::RowPatch>,
) -> Result<PreviewResult, GuiError> {
    let path = {
        let slot = state.pending_import_path.lock().map_err(poisoned)?;
        slot.clone().ok_or(GuiError::NoPendingImport)?
    };
    import_csv_dry_run_inner(state, &path, site_override, mapping, patches)
}

fn read_csv_headers(path: &std::path::Path) -> Result<Vec<String>, GuiError> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_path(path)
        .map_err(|e| GuiError::InvalidInput(format!("open csv: {e}")))?;
    let headers = rdr
        .headers()
        .map_err(|e| GuiError::InvalidInput(format!("read headers: {e}")))?
        .iter()
        .map(|s| s.to_string())
        .collect();
    Ok(headers)
}

// ============================================================================
// Attachments (Phase 4.4)
// ============================================================================

#[derive(Debug, Serialize)]
pub struct AttachmentSummaryArgs {
    pub id: i64,
    pub account_id: i64,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub created_at: String,
}

impl From<passhound_core::repo::attachments::AttachmentSummary> for AttachmentSummaryArgs {
    fn from(s: passhound_core::repo::attachments::AttachmentSummary) -> Self {
        AttachmentSummaryArgs {
            id: s.id,
            account_id: s.account_id,
            filename: s.filename,
            mime_type: s.mime_type,
            size_bytes: s.size_bytes,
            created_at: s.created_at.to_rfc3339(),
        }
    }
}

#[derive(Serialize)]
pub struct AttachmentReadResult {
    pub id: i64,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: i64,
    /// Base64-encoded plaintext bytes.
    pub bytes_base64: String,
}

#[tauri::command]
pub fn list_attachments(
    state: State<'_, VaultState>,
    account_id: i64,
) -> Result<Vec<AttachmentSummaryArgs>, GuiError> {
    list_attachments_inner(&state, account_id)
}

pub fn list_attachments_inner(
    state: &VaultState,
    account_id: i64,
) -> Result<Vec<AttachmentSummaryArgs>, GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    let list = passhound_core::repo::attachments::list_for_account(v, account_id)?;
    Ok(list.into_iter().map(AttachmentSummaryArgs::from).collect())
}

#[tauri::command]
pub fn attach_file(
    state: State<'_, VaultState>,
    account_id: i64,
    filename: String,
    mime_type: String,
    bytes_base64: String,
) -> Result<AttachmentSummaryArgs, GuiError> {
    attach_file_inner(&state, account_id, &filename, &mime_type, &bytes_base64)
}

pub fn attach_file_inner(
    state: &VaultState,
    account_id: i64,
    filename: &str,
    mime_type: &str,
    bytes_base64: &str,
) -> Result<AttachmentSummaryArgs, GuiError> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let bytes = STANDARD
        .decode(bytes_base64)
        .map_err(|e| GuiError::InvalidInput(format!("invalid base64: {e}")))?;
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    let summary = passhound_core::repo::attachments::insert(
        v,
        passhound_core::repo::attachments::NewAttachment {
            account_id,
            filename,
            mime_type,
            bytes: &bytes,
        },
    )?;
    Ok(AttachmentSummaryArgs::from(summary))
}

#[tauri::command]
pub fn read_attachment(
    state: State<'_, VaultState>,
    attachment_id: i64,
) -> Result<AttachmentReadResult, GuiError> {
    read_attachment_inner(&state, attachment_id)
}

pub fn read_attachment_inner(
    state: &VaultState,
    attachment_id: i64,
) -> Result<AttachmentReadResult, GuiError> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    let (summary, plaintext) = passhound_core::repo::attachments::decrypt(v, attachment_id)?;
    let bytes_base64 = STANDARD.encode(plaintext.as_slice());
    Ok(AttachmentReadResult {
        id: summary.id,
        filename: summary.filename,
        mime_type: summary.mime_type,
        size_bytes: summary.size_bytes,
        bytes_base64,
    })
}

#[tauri::command]
pub fn delete_attachment(
    state: State<'_, VaultState>,
    attachment_id: i64,
) -> Result<(), GuiError> {
    delete_attachment_inner(&state, attachment_id)
}

pub fn delete_attachment_inner(
    state: &VaultState,
    attachment_id: i64,
) -> Result<(), GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    passhound_core::repo::attachments::delete(v, attachment_id)?;
    Ok(())
}

#[tauri::command]
pub fn delete_account(state: State<'_, VaultState>, account_id: i64) -> Result<(), GuiError> {
    delete_account_inner(&state, account_id)
}

pub fn delete_account_inner(state: &VaultState, account_id: i64) -> Result<(), GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    passhound_core::repo::accounts::delete(v, account_id)?;
    Ok(())
}

#[tauri::command]
pub fn delete_password(state: State<'_, VaultState>, history_id: i64) -> Result<(), GuiError> {
    delete_password_inner(&state, history_id)
}

pub fn delete_password_inner(state: &VaultState, history_id: i64) -> Result<(), GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    passhound_core::repo::passwords::delete(v, history_id)?;
    Ok(())
}

// ============================================================================
// Account mutation (Phase 4.7)
// ============================================================================

/// Fields for creating a new account, optionally seeding its first password.
#[derive(serde::Deserialize, Debug)]
pub struct AddAccountFields {
    pub site_id: i64,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub alias: Option<String>,
    pub notes: Option<String>,
    pub initial_password: Option<String>,
}

/// Fields to overwrite on an existing account.
#[derive(serde::Deserialize, Debug)]
pub struct UpdateAccountFields {
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub alias: Option<String>,
    pub notes: Option<String>,
}

/// Slim site descriptor returned by `find_or_create_site`.
#[derive(serde::Serialize, Debug, Clone)]
pub struct SiteSummary {
    pub id: i64,
    pub name: String,
}

#[tauri::command]
pub fn list_sites(state: State<'_, VaultState>) -> Result<Vec<SiteSummary>, GuiError> {
    list_sites_inner(&state)
}

pub fn list_sites_inner(state: &VaultState) -> Result<Vec<SiteSummary>, GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    let sites = passhound_core::repo::sites::list(v)?;
    Ok(sites.into_iter().map(|s| SiteSummary { id: s.id, name: s.name }).collect())
}

#[tauri::command]
pub fn find_or_create_site(
    state: State<'_, VaultState>,
    name: String,
) -> Result<SiteSummary, GuiError> {
    find_or_create_site_inner(&state, &name)
}

pub fn find_or_create_site_inner(
    state: &VaultState,
    name: &str,
) -> Result<SiteSummary, GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    let trimmed = name.trim();
    if let Some(s) = passhound_core::repo::sites::find_by_name(v, trimmed)? {
        return Ok(SiteSummary { id: s.id, name: s.name });
    }
    let s = passhound_core::repo::sites::create(
        v,
        passhound_core::repo::sites::NewSite {
            name: trimmed.to_string(),
            ..Default::default()
        },
    )?;
    Ok(SiteSummary { id: s.id, name: s.name })
}

#[tauri::command]
pub fn add_account(
    state: State<'_, VaultState>,
    fields: AddAccountFields,
) -> Result<i64, GuiError> {
    add_account_inner(&state, &fields)
}

pub fn add_account_inner(
    state: &VaultState,
    fields: &AddAccountFields,
) -> Result<i64, GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    let acct = passhound_core::repo::accounts::create(
        v,
        passhound_core::repo::accounts::NewAccount {
            site_id: fields.site_id,
            username: fields.username.clone(),
            display_name: fields.display_name.clone(),
            alias: fields.alias.clone(),
            notes: fields.notes.clone(),
            ..Default::default()
        },
    )?;
    if let Some(pw) = fields.initial_password.as_ref() {
        if !pw.is_empty() {
            passhound_core::repo::passwords::set_current(v, acct.id, pw, "manual")?;
        }
    }
    Ok(acct.id)
}

#[tauri::command]
pub fn update_account(
    state: State<'_, VaultState>,
    account_id: i64,
    fields: UpdateAccountFields,
) -> Result<(), GuiError> {
    update_account_inner(&state, account_id, &fields)
}

pub fn update_account_inner(
    state: &VaultState,
    account_id: i64,
    fields: &UpdateAccountFields,
) -> Result<(), GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    passhound_core::repo::accounts::update(
        v,
        account_id,
        passhound_core::repo::accounts::UpdateAccount {
            username: fields.username.clone(),
            display_name: fields.display_name.clone(),
            alias: fields.alias.clone(),
            notes: fields.notes.clone(),
        },
    )?;
    Ok(())
}

#[derive(serde::Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AddPasswordPayload {
    pub account_id: i64,
    pub plaintext: String,
    #[serde(default)]
    pub source: Option<String>,
}

#[tauri::command]
pub fn add_password(
    state: State<'_, VaultState>,
    payload: AddPasswordPayload,
) -> Result<i64, GuiError> {
    add_password_inner(&state, payload)
}

pub fn add_password_inner(
    state: &VaultState,
    payload: AddPasswordPayload,
) -> Result<i64, GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    let source = payload.source.as_deref().unwrap_or("manual");
    let record = passhound_core::repo::passwords::set_current(v, payload.account_id, &payload.plaintext, source)?;
    Ok(record.id)
}

#[tauri::command]
pub fn promote_password(
    state: State<'_, VaultState>,
    history_id: i64,
) -> Result<(), GuiError> {
    promote_password_inner(&state, history_id)
}

pub fn promote_password_inner(
    state: &VaultState,
    history_id: i64,
) -> Result<(), GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    passhound_core::repo::passwords::promote(v, history_id)?;
    Ok(())
}

// ============================================================================
// Tags (Phase 4.6)
// ============================================================================

#[tauri::command]
pub fn list_tags(state: State<'_, VaultState>) -> Result<Vec<TagWithCount>, GuiError> {
    list_tags_inner(&state)
}

pub fn list_tags_inner(state: &VaultState) -> Result<Vec<TagWithCount>, GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    let rows = passhound_core::repo::tags::list_with_counts(v)?;
    Ok(rows
        .into_iter()
        .map(|r| TagWithCount {
            id: r.id,
            name: r.name,
            account_count: r.account_count,
        })
        .collect())
}

#[tauri::command]
pub fn create_tag(state: State<'_, VaultState>, name: String) -> Result<TagSummary, GuiError> {
    create_tag_inner(&state, &name)
}

pub fn create_tag_inner(state: &VaultState, name: &str) -> Result<TagSummary, GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    let trimmed = name.trim();
    if let Some(existing) = passhound_core::repo::tags::find_by_name(v, trimmed)? {
        return Ok(TagSummary { id: existing.id, name: existing.name });
    }
    let created = passhound_core::repo::tags::create(
        v,
        passhound_core::repo::tags::NewTag {
            name: trimmed,
            created_at: None,
        },
    )?;
    Ok(TagSummary { id: created.id, name: created.name })
}

#[tauri::command]
pub fn rename_tag(
    state: State<'_, VaultState>,
    tag_id: i64,
    new_name: String,
) -> Result<(), GuiError> {
    rename_tag_inner(&state, tag_id, &new_name)
}

pub fn rename_tag_inner(state: &VaultState, tag_id: i64, new_name: &str) -> Result<(), GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    passhound_core::repo::tags::rename(v, tag_id, new_name)?;
    Ok(())
}

#[tauri::command]
pub fn delete_tag(state: State<'_, VaultState>, tag_id: i64) -> Result<(), GuiError> {
    delete_tag_inner(&state, tag_id)
}

pub fn delete_tag_inner(state: &VaultState, tag_id: i64) -> Result<(), GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    passhound_core::repo::tags::delete(v, tag_id)?;
    Ok(())
}

#[tauri::command]
pub fn list_account_tags(
    state: State<'_, VaultState>,
    account_id: i64,
) -> Result<Vec<TagSummary>, GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    let tags = passhound_core::repo::account_tags::list_for_account(v, account_id)?;
    Ok(tags
        .into_iter()
        .map(|t| TagSummary { id: t.id, name: t.name })
        .collect())
}

#[tauri::command]
pub fn assign_tag(
    state: State<'_, VaultState>,
    account_id: i64,
    tag_id: i64,
) -> Result<(), GuiError> {
    assign_tag_inner(&state, account_id, tag_id)
}

pub fn assign_tag_inner(
    state: &VaultState,
    account_id: i64,
    tag_id: i64,
) -> Result<(), GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    passhound_core::repo::account_tags::assign(v, account_id, tag_id)?;
    Ok(())
}

#[tauri::command]
pub fn unassign_tag(
    state: State<'_, VaultState>,
    account_id: i64,
    tag_id: i64,
) -> Result<(), GuiError> {
    unassign_tag_inner(&state, account_id, tag_id)
}

pub fn unassign_tag_inner(
    state: &VaultState,
    account_id: i64,
    tag_id: i64,
) -> Result<(), GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    passhound_core::repo::account_tags::unassign(v, account_id, tag_id)?;
    Ok(())
}

#[tauri::command]
pub fn bulk_assign_tag(
    state: State<'_, VaultState>,
    account_ids: Vec<i64>,
    tag_id: i64,
) -> Result<usize, GuiError> {
    bulk_assign_tag_inner(&state, &account_ids, tag_id)
}

pub fn bulk_assign_tag_inner(
    state: &VaultState,
    account_ids: &[i64],
    tag_id: i64,
) -> Result<usize, GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    let n = passhound_core::repo::account_tags::bulk_assign(v, account_ids, tag_id)?;
    Ok(n)
}

#[tauri::command]
pub fn bulk_unassign_tag(
    state: State<'_, VaultState>,
    account_ids: Vec<i64>,
    tag_id: i64,
) -> Result<usize, GuiError> {
    bulk_unassign_tag_inner(&state, &account_ids, tag_id)
}

pub fn bulk_unassign_tag_inner(
    state: &VaultState,
    account_ids: &[i64],
    tag_id: i64,
) -> Result<usize, GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    let n = passhound_core::repo::account_tags::bulk_unassign(v, account_ids, tag_id)?;
    Ok(n)
}

#[tauri::command]
pub fn bulk_delete_accounts(
    state: State<'_, VaultState>,
    account_ids: Vec<i64>,
) -> Result<usize, GuiError> {
    bulk_delete_accounts_inner(&state, &account_ids)
}

pub fn bulk_delete_accounts_inner(
    state: &VaultState,
    account_ids: &[i64],
) -> Result<usize, GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    let mut count = 0usize;
    let tx = v.conn().unchecked_transaction()?;
    for &id in account_ids {
        match passhound_core::repo::accounts::delete(v, id) {
            Ok(()) => count += 1,
            Err(passhound_core::error::Error::NotFound) => {}
            Err(e) => return Err(e.into()),
        }
    }
    tx.commit()?;
    Ok(count)
}

// =================================================================
// Phase 4.8 — Recovery
// =================================================================

#[derive(serde::Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RecoverFilters {
    pub site: Option<String>,
    pub account: Option<String>,
    pub era: Option<String>,
    pub hint: Option<String>,
    pub limit: usize,
    pub min_length: Option<usize>,
    pub require_symbol: bool,
    pub require_digit: bool,
}

#[derive(serde::Serialize, Debug, Clone)]
pub struct RuleTag {
    pub tag: String,
    pub name: String,
}

/// Metadata for a single recovery candidate. The plaintext lives in
/// `VaultState.candidate_cache` and is fetched via `reveal_candidate`
/// or written to the clipboard via `copy_candidate`. Indexed by
/// `rank - 1`.
#[derive(serde::Serialize, Debug, Clone)]
pub struct CandidateView {
    pub rank: usize,
    pub score: f32,
    pub provenance: Vec<RuleTag>,
}

#[derive(serde::Serialize, Debug, Clone)]
pub struct EraSummary {
    pub id: i64,
    pub name: String,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub notes: Option<String>,
}

#[tauri::command]
pub fn recover_candidates(
    state: State<'_, VaultState>,
    filters: RecoverFilters,
) -> Result<Vec<CandidateView>, GuiError> {
    recover_candidates_inner(&state, &filters)
}

pub fn recover_candidates_inner(
    state: &VaultState,
    filters: &RecoverFilters,
) -> Result<Vec<CandidateView>, GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;

    let cfg = passhound_core::RecoverConfig {
        site: filters.site.clone(),
        account: filters.account.clone(),
        era_name: filters.era.clone(),
        hint: filters.hint.clone(),
        limit: filters.limit,
        min_length: filters.min_length,
        require_symbol: filters.require_symbol,
        require_digit: filters.require_digit,
    };

    let candidates = passhound_core::recover(v, cfg)?;

    // Replace the cache atomically with the new candidates' plaintexts.
    {
        let mut cache = state.candidate_cache.lock().map_err(poisoned)?;
        cache.clear();
        cache.extend(candidates.iter().map(|c| c.password.clone()));
    }

    Ok(candidates
        .into_iter()
        .enumerate()
        .map(|(i, c)| CandidateView {
            rank: i + 1,
            score: c.score,
            provenance: c
                .provenance
                .iter()
                .map(|r| RuleTag {
                    tag: r.tag().to_string(),
                    name: r.name().to_string(),
                })
                .collect(),
        })
        .collect())
}

#[tauri::command]
pub fn reveal_candidate(
    state: State<'_, VaultState>,
    rank: usize,
) -> Result<String, GuiError> {
    reveal_candidate_inner(&state, rank)
}

pub fn reveal_candidate_inner(
    state: &VaultState,
    rank: usize,
) -> Result<String, GuiError> {
    // Verify vault is unlocked; we don't need the vault itself but we
    // do not want to leak cache contents post-lock.
    {
        let guard = state.vault.lock().map_err(poisoned)?;
        if guard.is_none() {
            return Err(GuiError::Locked);
        }
    }
    let cache = state.candidate_cache.lock().map_err(poisoned)?;
    if cache.is_empty() {
        return Err(GuiError::NoActiveRecovery);
    }
    let idx = rank.checked_sub(1).ok_or(GuiError::RankOutOfBounds)?;
    let pt = cache.get(idx).ok_or(GuiError::RankOutOfBounds)?;
    Ok(pt.to_string())
}

#[tauri::command]
pub fn copy_candidate(
    app: tauri::AppHandle,
    state: State<'_, VaultState>,
    rank: usize,
) -> Result<(), GuiError> {
    copy_candidate_inner(&app, &state, rank)
}

pub fn copy_candidate_inner(
    app: &tauri::AppHandle,
    state: &VaultState,
    rank: usize,
) -> Result<(), GuiError> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let pt = reveal_candidate_inner(state, rank)?;
    app.clipboard()
        .write_text(&pt)
        .map_err(|e| GuiError::Internal(format!("clipboard: {e}")))?;
    Ok(())
}

#[derive(serde::Deserialize, Debug)]
pub struct EraFormArgs {
    pub name: String,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub notes: Option<String>,
}

fn parse_era_form(args: EraFormArgs) -> Result<
    (String, Option<chrono::NaiveDate>, Option<chrono::NaiveDate>, Option<String>),
    GuiError,
> {
    let start = args.start_date
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d"))
        .transpose()
        .map_err(|e| GuiError::InvalidInput(format!("start_date: {e}")))?;
    let end = args.end_date
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d"))
        .transpose()
        .map_err(|e| GuiError::InvalidInput(format!("end_date: {e}")))?;
    if let (Some(s), Some(e)) = (start, end) {
        if e < s {
            return Err(GuiError::InvalidInput("end_date must be on or after start_date".into()));
        }
    }
    Ok((args.name, start, end, args.notes))
}

#[tauri::command]
pub fn add_era(state: State<'_, VaultState>, args: EraFormArgs) -> Result<i64, GuiError> {
    add_era_inner(&state, args)
}

pub fn add_era_inner(state: &VaultState, args: EraFormArgs) -> Result<i64, GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    let (name, start, end, notes) = parse_era_form(args)?;
    let id = passhound_core::repo::eras::add(v, &name, start, end, notes.as_deref())?;
    Ok(id)
}

#[tauri::command]
pub fn update_era(state: State<'_, VaultState>, id: i64, args: EraFormArgs) -> Result<(), GuiError> {
    update_era_inner(&state, id, args)
}

pub fn update_era_inner(state: &VaultState, id: i64, args: EraFormArgs) -> Result<(), GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    let (name, start, end, notes) = parse_era_form(args)?;
    passhound_core::repo::eras::update(v, id, &name, start, end, notes.as_deref())?;
    Ok(())
}

#[tauri::command]
pub fn delete_era(state: State<'_, VaultState>, id: i64) -> Result<(), GuiError> {
    delete_era_inner(&state, id)
}

pub fn delete_era_inner(state: &VaultState, id: i64) -> Result<(), GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    passhound_core::repo::eras::delete(v, id)?;
    Ok(())
}

#[tauri::command]
pub fn list_eras(state: State<'_, VaultState>) -> Result<Vec<EraSummary>, GuiError> {
    list_eras_inner(&state)
}

pub fn list_eras_inner(state: &VaultState) -> Result<Vec<EraSummary>, GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    let eras = passhound_core::repo::eras::list(v)?;
    Ok(eras
        .into_iter()
        .map(|e| EraSummary {
            id: e.id,
            name: e.name,
            start_date: e.start_date.map(|d| d.format("%Y-%m-%d").to_string()),
            end_date: e.end_date.map(|d| d.format("%Y-%m-%d").to_string()),
            notes: e.notes,
        })
        .collect())
}

// =================================================================
// Phase 4.9 — Base Words
// =================================================================

#[derive(serde::Serialize, Debug, Clone)]
pub struct BaseWordView {
    pub id: i64,
    pub word: String,
    pub is_favorite: bool,
    pub manual_override: bool,
    pub usage_count: i64,
    pub first_seen_at: Option<String>,
    pub last_seen_at: Option<String>,
}

#[derive(serde::Serialize, Debug, Clone)]
pub struct AnalyzeReportView {
    pub tokens_seen: usize,
    pub base_words_written: usize,
    pub favorites_set: usize,
}

#[tauri::command]
pub fn list_base_words(
    state: State<'_, VaultState>,
) -> Result<Vec<BaseWordView>, GuiError> {
    list_base_words_inner(&state)
}

pub fn list_base_words_inner(state: &VaultState) -> Result<Vec<BaseWordView>, GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    let meta = passhound_core::repo::base_words::list(v)?;
    let decrypted = passhound_core::repo::base_words::fetch_decrypted(v)?;
    // Build id -> plaintext map. Plaintext crosses the IPC boundary as JSON,
    // so the Zeroizing wrapper drops here. Same trade-off as reveal_password.
    let mut by_id: HashMap<i64, String> = HashMap::with_capacity(decrypted.len());
    for dw in decrypted {
        by_id.insert(dw.id, dw.word.to_string());
    }
    let mut out = Vec::with_capacity(meta.len());
    for m in meta {
        let word = by_id.remove(&m.id).unwrap_or_default();
        out.push(BaseWordView {
            id: m.id,
            word,
            is_favorite: m.is_favorite,
            manual_override: m.manual_override,
            usage_count: m.usage_count,
            first_seen_at: m.first_seen_at.map(|d| d.to_rfc3339()),
            last_seen_at: m.last_seen_at.map(|d| d.to_rfc3339()),
        });
    }
    Ok(out)
}

#[tauri::command]
pub fn promote_base_word(
    state: State<'_, VaultState>,
    id: i64,
) -> Result<(), GuiError> {
    promote_base_word_inner(&state, id)
}

pub fn promote_base_word_inner(state: &VaultState, id: i64) -> Result<(), GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    passhound_core::repo::base_words::promote(v, id)?;
    Ok(())
}

#[tauri::command]
pub fn demote_base_word(
    state: State<'_, VaultState>,
    id: i64,
) -> Result<(), GuiError> {
    demote_base_word_inner(&state, id)
}

pub fn demote_base_word_inner(state: &VaultState, id: i64) -> Result<(), GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    passhound_core::repo::base_words::demote(v, id)?;
    Ok(())
}

#[tauri::command]
pub fn analyze_base_words(
    state: State<'_, VaultState>,
) -> Result<AnalyzeReportView, GuiError> {
    analyze_base_words_inner(&state)
}

pub fn analyze_base_words_inner(state: &VaultState) -> Result<AnalyzeReportView, GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    let s = passhound_core::settings::get(v)?;
    let report = passhound_core::recovery::analyze::extract_base_words_from_history(v, s.analyze_top_n as usize)?;
    Ok(AnalyzeReportView {
        tokens_seen: report.tokens_seen,
        base_words_written: report.base_words_written,
        favorites_set: report.favorites_set,
    })
}

// =================================================================
// Phase 4.10 — Settings
// =================================================================

#[derive(serde::Serialize, Debug, Clone)]
pub struct SettingsView {
    pub idle_lock_seconds: u32,
    pub clipboard_clear_seconds: u32,
    pub analyze_top_n: u32,
    pub default_reveal: bool,
    pub reveal_clear_seconds: u32,
}

#[derive(serde::Deserialize, Debug)]
#[serde(tag = "key", content = "value", rename_all = "snake_case")]
pub enum SettingChange {
    IdleLockSeconds(u32),
    ClipboardClearSeconds(u32),
    AnalyzeTopN(u32),
    DefaultReveal(bool),
    RevealClearSeconds(u32),
}

#[tauri::command]
pub fn get_settings(state: State<'_, VaultState>) -> Result<SettingsView, GuiError> {
    get_settings_inner(&state)
}

pub fn get_settings_inner(state: &VaultState) -> Result<SettingsView, GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    let s = passhound_core::settings::get(v)?;
    Ok(SettingsView {
        idle_lock_seconds: s.idle_lock_seconds,
        clipboard_clear_seconds: s.clipboard_clear_seconds,
        analyze_top_n: s.analyze_top_n,
        default_reveal: s.default_reveal,
        reveal_clear_seconds: s.reveal_clear_seconds,
    })
}

#[tauri::command]
pub fn set_setting(
    state: State<'_, VaultState>,
    change: SettingChange,
) -> Result<(), GuiError> {
    set_setting_inner(&state, change)
}

pub fn set_setting_inner(state: &VaultState, change: SettingChange) -> Result<(), GuiError> {
    use passhound_core::settings;
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    match change {
        SettingChange::IdleLockSeconds(n) => settings::set_u32(v, settings::KEY_IDLE_LOCK, n)?,
        SettingChange::ClipboardClearSeconds(n) => settings::set_u32(v, settings::KEY_CLIPBOARD_CLEAR, n)?,
        SettingChange::AnalyzeTopN(n) => settings::set_u32(v, settings::KEY_ANALYZE_TOP_N, n)?,
        SettingChange::DefaultReveal(b) => settings::set_bool(v, settings::KEY_DEFAULT_REVEAL, b)?,
        SettingChange::RevealClearSeconds(n) => settings::set_u32(v, settings::KEY_REVEAL_CLEAR_SECONDS, n)?,
    }
    Ok(())
}

// =================================================================
// Phase 4.11 — Master password change
// =================================================================

#[tauri::command]
pub fn change_master_password(
    state: State<'_, VaultState>,
    current_pw: String,
    new_pw: String,
) -> Result<(), GuiError> {
    change_master_password_inner(&state, current_pw.as_bytes(), new_pw.as_bytes())
}

pub fn change_master_password_inner(
    state: &VaultState,
    current_pw: &[u8],
    new_pw: &[u8],
) -> Result<(), GuiError> {
    let mut guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_mut().ok_or(GuiError::Locked)?;
    v.change_master_password(current_pw, new_pw)?;
    Ok(())
}

// =================================================================
// Phase 4.12 — Recovery feedback
// =================================================================

#[derive(serde::Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RecordFeedbackPayload {
    pub account_id: Option<i64>,
    pub provenance: Vec<String>,
    pub score: f32,
    pub rank: i64,
    pub worked: bool,
    pub length: i64,
    pub has_digit: bool,
    pub has_symbol: bool,
    pub has_upper: bool,
    pub has_lower: bool,
}

#[tauri::command]
pub fn record_recovery_feedback(
    state: State<'_, VaultState>,
    payload: RecordFeedbackPayload,
) -> Result<(), GuiError> {
    record_recovery_feedback_inner(&state, payload)
}

pub fn record_recovery_feedback_inner(
    state: &VaultState,
    payload: RecordFeedbackPayload,
) -> Result<(), GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    let provenance: Vec<passhound_core::recovery::RuleId> = payload.provenance
        .iter()
        .filter_map(|s| passhound_core::recovery::RuleId::from_tag(s))
        .collect();
    let event = passhound_core::recovery::feedback::FeedbackEvent {
        account_id: payload.account_id,
        provenance,
        score: payload.score,
        rank: payload.rank,
        worked: payload.worked,
        length: payload.length,
        has_digit: payload.has_digit,
        has_symbol: payload.has_symbol,
        has_upper: payload.has_upper,
        has_lower: payload.has_lower,
    };
    passhound_core::recovery::feedback::record(v, event)?;
    Ok(())
}

#[tauri::command]
pub fn clear_recovery_feedback(
    state: State<'_, VaultState>,
) -> Result<usize, GuiError> {
    clear_recovery_feedback_inner(&state)
}

pub fn clear_recovery_feedback_inner(
    state: &VaultState,
) -> Result<usize, GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    let n = passhound_core::recovery::feedback::clear(v)?;
    Ok(n)
}

// =================================================================
// Phase 4.14 — Small items bundle
// =================================================================

#[derive(serde::Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSitePayload {
    pub name: String,
    pub url: Option<String>,
    pub category: Option<String>,
    pub abbreviations: Vec<String>,
    pub notes: Option<String>,
}

#[tauri::command]
pub fn update_site(
    state: State<'_, VaultState>,
    site_id: i64,
    payload: UpdateSitePayload,
) -> Result<(), GuiError> {
    update_site_inner(&state, site_id, payload)
}

pub fn update_site_inner(
    state: &VaultState,
    site_id: i64,
    payload: UpdateSitePayload,
) -> Result<(), GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    passhound_core::repo::sites::update(v, site_id, passhound_core::repo::sites::UpdateSite {
        name: payload.name,
        url: payload.url,
        category: payload.category,
        abbreviations: payload.abbreviations,
        notes: payload.notes,
    })?;
    Ok(())
}

#[derive(serde::Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GeneratorOptionsPayload {
    pub length: u8,
    pub lowercase: bool,
    pub uppercase: bool,
    pub digits: bool,
    pub symbols: bool,
    pub avoid_ambiguous: bool,
}

#[tauri::command]
pub fn generate_password(
    payload: GeneratorOptionsPayload,
) -> Result<String, GuiError> {
    generate_password_inner(payload)
}

pub fn generate_password_inner(payload: GeneratorOptionsPayload) -> Result<String, GuiError> {
    let opts = passhound_core::generator::GeneratorOptions {
        length: payload.length,
        lowercase: payload.lowercase,
        uppercase: payload.uppercase,
        digits: payload.digits,
        symbols: payload.symbols,
        avoid_ambiguous: payload.avoid_ambiguous,
    };
    let pw = passhound_core::generator::generate(opts)?;
    // Drop Zeroizing here — the plaintext crosses the IPC as a String.
    // Same privacy trade-off as reveal_password (4.1) and Recovery (4.8).
    Ok(pw.to_string())
}

#[tauri::command]
pub fn add_base_word(
    state: State<'_, VaultState>,
    text: String,
) -> Result<BaseWordView, GuiError> {
    add_base_word_inner(&state, text)
}

pub fn add_base_word_inner(
    state: &VaultState,
    text: String,
) -> Result<BaseWordView, GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    let new = passhound_core::repo::base_words::manual_insert(v, &text)?;
    // Fetch the decrypted view to get the plaintext word for the BaseWordView.
    let decrypted = passhound_core::repo::base_words::fetch_decrypted(v)?;
    let found = decrypted.into_iter().find(|d| d.id == new.id)
        .ok_or_else(|| GuiError::Internal("inserted base word not found in fetch_decrypted".into()))?;
    Ok(BaseWordView {
        id: new.id,
        word: found.word.to_string(),
        is_favorite: new.is_favorite,
        manual_override: new.manual_override,
        usage_count: new.usage_count,
        first_seen_at: new.first_seen_at.map(|d| d.to_rfc3339()),
        last_seen_at: new.last_seen_at.map(|d| d.to_rfc3339()),
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use passhound_core::repo::{accounts, passwords, sites};
    use tempfile::TempDir;
    use passhound_core::repo::eras;
    use chrono::NaiveDate;

    fn temp_vault() -> (TempDir, std::path::PathBuf) {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("vault.db");
        (tmp, path)
    }

    #[test]
    fn vault_create_then_unlock_round_trip() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        // Create
        vault_create_inner(&state, &path, b"hunter2").unwrap();
        assert!(vault_exists_inner(&path).unwrap());
        // Empty vault — listing returns no accounts
        let list = list_accounts_inner(&state, None, None, None).unwrap();
        assert!(list.is_empty());
        // Lock
        vault_lock_inner(&state).unwrap();
        assert!(matches!(list_accounts_inner(&state, None, None, None), Err(GuiError::Locked)));
        // Re-unlock
        vault_unlock_inner(&state, &path, b"hunter2").unwrap();
        let list = list_accounts_inner(&state, None, None, None).unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn list_accounts_returns_inserted_rows() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"hunter2").unwrap();
        // Insert site/account/password directly through core
        {
            let guard = state.vault.lock().unwrap();
            let v = guard.as_ref().unwrap();
            let s = sites::create(v, sites::NewSite {
                name: "Reddit".into(),
                url: Some("reddit.com".into()),
                category: Some("Social".into()),
                abbreviations: vec!["Rd".into()],
                notes: None,
            }).unwrap();
            let a = accounts::create(v, accounts::NewAccount {
                site_id: s.id,
                username: Some("chris".into()),
                display_name: Some("MaxedNoob".into()),
                ..Default::default()
            }).unwrap();
            passwords::insert(v, passwords::NewPassword {
                account_id: a.id,
                plaintext: "MoonBeam$2019Rd",
                source: "manual".into(),
                confidence: passwords::Confidence::Certain,
                notes: None,
                created_at: None,
            }).unwrap();
        }
        let list = list_accounts_inner(&state, None, None, None).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].site_name, "Reddit");
        assert_eq!(list[0].username.as_deref(), Some("chris"));
        assert_eq!(list[0].category.as_deref(), Some("Social"));
        assert!(list[0].last_changed.is_some());
        assert_eq!(list[0].display_name.as_deref(), Some("MaxedNoob"));
        // Filter
        let filtered = list_accounts_inner(&state, Some("redd"), None, None).unwrap();
        assert_eq!(filtered.len(), 1);
        let unfiltered = list_accounts_inner(&state, Some("zzz_no_match_zzz"), None, None).unwrap();
        assert_eq!(unfiltered.len(), 0);
    }

    #[test]
    fn list_accounts_filters_by_era_overlap() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();

        let acct_in: i64;
        let acct_out: i64;
        let era_id: i64;
        {
            let guard = state.vault.lock().unwrap();
            let v = guard.as_ref().unwrap();

            let s_in = sites::create(v, sites::NewSite {
                name: "SiteIn".into(),
                ..Default::default()
            }).unwrap();
            let a_in = accounts::create(v, accounts::NewAccount {
                site_id: s_in.id,
                ..Default::default()
            }).unwrap();
            acct_in = a_in.id;

            let s_out = sites::create(v, sites::NewSite {
                name: "SiteOut".into(),
                ..Default::default()
            }).unwrap();
            let a_out = accounts::create(v, accounts::NewAccount {
                site_id: s_out.id,
                ..Default::default()
            }).unwrap();
            acct_out = a_out.id;

            // acct_in: password history row inside the 2010-2015 window.
            v.conn().execute(
                "INSERT INTO password_history (account_id, password_encrypted, password_nonce, source, confidence, created_at, retired_at)
                 VALUES (?1, X'00', X'00', 'manual', 'certain', '2012-06-15T00:00:00Z', NULL)",
                rusqlite::params![acct_in],
            ).unwrap();
            // acct_out: password history row outside the window (2018).
            v.conn().execute(
                "INSERT INTO password_history (account_id, password_encrypted, password_nonce, source, confidence, created_at, retired_at)
                 VALUES (?1, X'00', X'00', 'manual', 'certain', '2018-06-15T00:00:00Z', NULL)",
                rusqlite::params![acct_out],
            ).unwrap();

            era_id = eras::add(
                v,
                "Window 2010-2015",
                Some(NaiveDate::from_ymd_opt(2010, 1, 1).unwrap()),
                Some(NaiveDate::from_ymd_opt(2015, 12, 31).unwrap()),
                None,
            ).unwrap();
        }

        // Without era filter: both accounts visible.
        let all = list_accounts_inner(&state, None, None, None).unwrap();
        assert_eq!(all.len(), 2, "no era filter should show both accounts");

        // With era filter: only acct_in.
        let filtered = list_accounts_inner(&state, None, None, Some(era_id)).unwrap();
        assert_eq!(filtered.len(), 1, "era filter should match only the 2012 account");
        assert_eq!(filtered[0].id, acct_in);

        // Unknown era id: empty list silently.
        let unknown = list_accounts_inner(&state, None, None, Some(9999)).unwrap();
        assert!(unknown.is_empty(), "unknown era id should return empty list");
    }

    #[test]
    fn get_account_returns_detail_with_history() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"hunter2").unwrap();
        let account_id;
        {
            let guard = state.vault.lock().unwrap();
            let v = guard.as_ref().unwrap();
            let s = sites::create(v, sites::NewSite {
                name: "Reddit".into(),
                url: Some("reddit.com".into()),
                category: Some("Social".into()),
                abbreviations: vec!["Rd".into()],
                notes: None,
            }).unwrap();
            let a = accounts::create(v, accounts::NewAccount {
                site_id: s.id,
                username: Some("chris".into()),
                ..Default::default()
            }).unwrap();
            account_id = a.id;
            passwords::insert(v, passwords::NewPassword {
                account_id: a.id,
                plaintext: "old-password",
                source: "manual".into(),
                confidence: passwords::Confidence::Certain,
                notes: None,
                created_at: None,
            }).unwrap();
            // set_current retires the previous entry and inserts a new current.
            passwords::set_current(v, a.id, "current-password", "manual").unwrap();
        }
        let detail = get_account_inner(&state, account_id).unwrap();
        assert_eq!(detail.site_name, "Reddit");
        assert_eq!(detail.site_url.as_deref(), Some("reddit.com"));
        assert_eq!(detail.site_category.as_deref(), Some("Social"));
        assert_eq!(detail.site_abbreviations, vec!["Rd".to_string()]);
        assert_eq!(detail.username.as_deref(), Some("chris"));
        assert_eq!(detail.history.len(), 2);
        // Exactly one is_current entry.
        let current_count = detail.history.iter().filter(|h| h.is_current).count();
        assert_eq!(current_count, 1);
    }

    #[test]
    fn reveal_password_returns_plaintext() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"hunter2").unwrap();
        let history_id;
        {
            let guard = state.vault.lock().unwrap();
            let v = guard.as_ref().unwrap();
            let s = sites::create(v, sites::NewSite {
                name: "Site".into(), ..Default::default()
            }).unwrap();
            let a = accounts::create(v, accounts::NewAccount {
                site_id: s.id, ..Default::default()
            }).unwrap();
            let pw = passwords::insert(v, passwords::NewPassword {
                account_id: a.id,
                plaintext: "secret-123",
                source: "manual".into(),
                confidence: passwords::Confidence::Certain,
                notes: None,
                created_at: None,
            }).unwrap();
            history_id = pw.id;
        }
        let revealed = reveal_password_inner(&state, history_id).unwrap();
        assert_eq!(revealed, "secret-123");
    }

    #[test]
    fn reveal_password_locked_returns_locked_error() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"hunter2").unwrap();
        let history_id;
        {
            let guard = state.vault.lock().unwrap();
            let v = guard.as_ref().unwrap();
            let s = sites::create(v, sites::NewSite {
                name: "Site".into(), ..Default::default()
            }).unwrap();
            let a = accounts::create(v, accounts::NewAccount {
                site_id: s.id, ..Default::default()
            }).unwrap();
            let pw = passwords::insert(v, passwords::NewPassword {
                account_id: a.id,
                plaintext: "secret",
                source: "manual".into(),
                confidence: passwords::Confidence::Certain,
                notes: None,
                created_at: None,
            }).unwrap();
            history_id = pw.id;
        }
        vault_lock_inner(&state).unwrap();
        let err = reveal_password_inner(&state, history_id).unwrap_err();
        assert!(matches!(err, GuiError::Locked), "expected Locked, got {err:?}");
    }

    #[test]
    fn import_csv_dry_run_returns_redacted_preview() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"hunter2").unwrap();

        // Write a CSV file in a separate tempdir.
        let csv_tmp = TempDir::new().unwrap();
        let csv_path = csv_tmp.path().join("input.csv");
        std::fs::write(
            &csv_path,
            "name,login,password,note\nReddit,chris,SecretPw,first row\n",
        )
        .unwrap();

        let preview = import_csv_dry_run_inner(&state, &csv_path, None, None, vec![]).unwrap();
        assert_eq!(preview.headers, vec!["name", "login", "password", "note"]);
        assert_eq!(preview.counts.new, 1);
        assert_eq!(preview.counts.duplicates, 0);
        assert_eq!(preview.sample_rows.len(), 1);
        assert_eq!(preview.sample_rows[0].site, "Reddit");
        assert_eq!(preview.sample_rows[0].password_length, "SecretPw".len());
        // Password value MUST NOT be in any serialized field.
        let serialized = serde_json::to_string(&preview).unwrap();
        assert!(
            !serialized.contains("SecretPw"),
            "password leaked in preview JSON: {serialized}"
        );
    }

    #[test]
    fn import_csv_commit_writes_entries() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"hunter2").unwrap();

        let csv_tmp = TempDir::new().unwrap();
        let csv_path = csv_tmp.path().join("input.csv");
        std::fs::write(
            &csv_path,
            "name,login,password,displayname,total level\n\
             RuneScape,chris,Fluffy!2014,Bob,99\n",
        )
        .unwrap();

        let r = import_csv_commit_inner(&state, &csv_path, None, None, vec![]).unwrap();
        assert_eq!(r.counts.new, 1);
        assert!(r.import_id > 0);

        // Verify the row is in the vault now.
        let list = list_accounts_inner(&state, None, None, None).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].site_name, "RuneScape");
    }

    #[test]
    fn attach_file_round_trips_bytes() {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"hunter2").unwrap();
        let account_id = {
            let guard = state.vault.lock().unwrap();
            let v = guard.as_ref().unwrap();
            let s = sites::create(v, sites::NewSite {
                name: "S".into(), ..Default::default()
            }).unwrap();
            let a = accounts::create(v, accounts::NewAccount {
                site_id: s.id, ..Default::default()
            }).unwrap();
            a.id
        };

        let original = b"hello world binary \x00\xff\x01\xfe";
        let encoded = STANDARD.encode(original);
        let summary = attach_file_inner(
            &state,
            account_id,
            "hello.bin",
            "application/octet-stream",
            &encoded,
        ).unwrap();
        assert_eq!(summary.filename, "hello.bin");
        assert_eq!(summary.size_bytes, original.len() as i64);

        let read = read_attachment_inner(&state, summary.id).unwrap();
        let decoded = STANDARD.decode(&read.bytes_base64).unwrap();
        assert_eq!(decoded.as_slice(), original);
    }

    #[test]
    fn list_attachments_returns_inserted_files() {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"hunter2").unwrap();
        let account_id = {
            let guard = state.vault.lock().unwrap();
            let v = guard.as_ref().unwrap();
            let s = sites::create(v, sites::NewSite {
                name: "S".into(), ..Default::default()
            }).unwrap();
            let a = accounts::create(v, accounts::NewAccount {
                site_id: s.id, ..Default::default()
            }).unwrap();
            a.id
        };

        attach_file_inner(&state, account_id, "a.png", "image/png", &STANDARD.encode(b"pngdata")).unwrap();
        attach_file_inner(&state, account_id, "b.pdf", "application/pdf", &STANDARD.encode(b"pdfdata")).unwrap();

        let list = list_attachments_inner(&state, account_id).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].filename, "a.png");
        assert_eq!(list[0].mime_type, "image/png");
        assert_eq!(list[1].filename, "b.pdf");
    }

    #[test]
    fn delete_attachment_removes_file() {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"hunter2").unwrap();
        let account_id = {
            let guard = state.vault.lock().unwrap();
            let v = guard.as_ref().unwrap();
            let s = sites::create(v, sites::NewSite {
                name: "S".into(), ..Default::default()
            }).unwrap();
            let a = accounts::create(v, accounts::NewAccount {
                site_id: s.id, ..Default::default()
            }).unwrap();
            a.id
        };

        let summary = attach_file_inner(
            &state, account_id, "a.txt", "text/plain",
            &STANDARD.encode(b"hello"),
        ).unwrap();

        delete_attachment_inner(&state, summary.id).unwrap();

        let list = list_attachments_inner(&state, account_id).unwrap();
        assert!(list.is_empty(), "expected empty list after delete; got {list:?}");
    }

    #[test]
    fn delete_account_removes_row_and_cascades() {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"hunter2").unwrap();
        let account_id = {
            let guard = state.vault.lock().unwrap();
            let v = guard.as_ref().unwrap();
            let s = sites::create(v, sites::NewSite {
                name: "DeleteMe".into(), ..Default::default()
            }).unwrap();
            let a = accounts::create(v, accounts::NewAccount {
                site_id: s.id,
                username: Some("to_delete".into()),
                ..Default::default()
            }).unwrap();
            passwords::insert(v, passwords::NewPassword {
                account_id: a.id,
                plaintext: "password123",
                source: "manual".into(),
                confidence: passwords::Confidence::Certain,
                notes: None,
                created_at: None,
            }).unwrap();
            a.id
        };

        // Attach a file so we can verify cascade deletes it too.
        attach_file_inner(
            &state, account_id, "doc.txt", "text/plain",
            &STANDARD.encode(b"contents"),
        ).unwrap();

        // Sanity: account is present before delete.
        let before = list_accounts_inner(&state, None, None, None).unwrap();
        assert_eq!(before.len(), 1);

        delete_account_inner(&state, account_id).unwrap();

        // Account is gone.
        let after = list_accounts_inner(&state, None, None, None).unwrap();
        assert!(after.is_empty(), "expected no accounts after delete; got {} rows", after.len());

        // Attachment cascaded away.
        let attachments = list_attachments_inner(&state, account_id).unwrap();
        assert!(attachments.is_empty(), "expected attachments cascade-deleted; got {attachments:?}");
    }

    #[test]
    fn delete_password_removes_one_history_row() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"hunter2").unwrap();
        let (account_id, first_id) = {
            let guard = state.vault.lock().unwrap();
            let v = guard.as_ref().unwrap();
            let s = sites::create(v, sites::NewSite {
                name: "Site".into(), ..Default::default()
            }).unwrap();
            let a = accounts::create(v, accounts::NewAccount {
                site_id: s.id, ..Default::default()
            }).unwrap();
            let pw1 = passwords::insert(v, passwords::NewPassword {
                account_id: a.id,
                plaintext: "first-password",
                source: "manual".into(),
                confidence: passwords::Confidence::Certain,
                notes: None,
                created_at: None,
            }).unwrap();
            passwords::set_current(v, a.id, "second-password", "manual").unwrap();
            (a.id, pw1.id)
        };

        // Two history rows before delete.
        let before = get_account_inner(&state, account_id).unwrap();
        assert_eq!(before.history.len(), 2);

        delete_password_inner(&state, first_id).unwrap();

        // One history row remains.
        let after = get_account_inner(&state, account_id).unwrap();
        assert_eq!(after.history.len(), 1, "expected 1 history row after deleting one; got {:?}", after.history.len());
    }

    // -----------------------------------------------------------------------
    // Phase 4.6 tag tests
    // -----------------------------------------------------------------------

    use std::sync::atomic::{AtomicI64, Ordering};
    static COUNTER: AtomicI64 = AtomicI64::new(0);

    /// Helper: insert a site + account and return the account id.
    fn seed_account(state: &VaultState) -> i64 {
        let guard = state.vault.lock().unwrap();
        let v = guard.as_ref().unwrap();
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let s = sites::create(v, sites::NewSite {
            name: format!("SeedSite{n}"),
            ..Default::default()
        }).unwrap();
        accounts::create(v, accounts::NewAccount {
            site_id: s.id,
            ..Default::default()
        }).unwrap().id
    }

    #[test]
    fn create_tag_and_list_tags_round_trip() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        let t = create_tag_inner(&state, "throwaway").unwrap();
        let list = list_tags_inner(&state).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, t.id);
        assert_eq!(list[0].account_count, 0);
    }

    #[test]
    fn assign_tag_then_get_account_includes_tag() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        let account_id = seed_account(&state);
        let tag = create_tag_inner(&state, "main").unwrap();
        assign_tag_inner(&state, account_id, tag.id).unwrap();
        let detail = get_account_inner(&state, account_id).unwrap();
        assert_eq!(detail.tags.len(), 1);
        assert_eq!(detail.tags[0].name, "main");
    }

    #[test]
    fn list_accounts_filters_by_tag_ids() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        let a1 = seed_account(&state);
        let _a2 = seed_account(&state);
        let tag = create_tag_inner(&state, "foo").unwrap();
        assign_tag_inner(&state, a1, tag.id).unwrap();
        let filtered = list_accounts_inner(&state, None, Some(vec![tag.id]), None).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, a1);
    }

    #[test]
    fn bulk_assign_tag_inserts_for_all() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        let ids = vec![seed_account(&state), seed_account(&state), seed_account(&state)];
        let tag = create_tag_inner(&state, "bulk").unwrap();
        let n = bulk_assign_tag_inner(&state, &ids, tag.id).unwrap();
        assert_eq!(n, 3);
    }

    #[test]
    fn bulk_delete_accounts_cascades_everything() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        let ids = vec![seed_account(&state), seed_account(&state)];
        let tag = create_tag_inner(&state, "t").unwrap();
        bulk_assign_tag_inner(&state, &ids, tag.id).unwrap();
        let n = bulk_delete_accounts_inner(&state, &ids).unwrap();
        assert_eq!(n, 2);
        let remaining = list_accounts_inner(&state, None, None, None).unwrap();
        assert!(remaining.is_empty());
    }

    // -----------------------------------------------------------------------
    // Phase 4.7 account mutation tests
    // -----------------------------------------------------------------------

    #[test]
    fn list_sites_empty_vault_returns_empty() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        let sites = list_sites_inner(&state).unwrap();
        assert!(sites.is_empty());
    }

    #[test]
    fn add_account_with_initial_password_creates_both() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        let site = find_or_create_site_inner(&state, "Reddit").unwrap();
        let aid = add_account_inner(&state, &AddAccountFields {
            site_id: site.id,
            username: Some("alice".into()),
            display_name: None,
            alias: None,
            notes: None,
            initial_password: Some("hunter2".into()),
        }).unwrap();
        let detail = get_account_inner(&state, aid).unwrap();
        assert_eq!(detail.history.len(), 1);
    }

    #[test]
    fn add_account_without_password_creates_account_only() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        let site = find_or_create_site_inner(&state, "Twitter").unwrap();
        let aid = add_account_inner(&state, &AddAccountFields {
            site_id: site.id,
            username: Some("bob".into()),
            display_name: None,
            alias: None,
            notes: None,
            initial_password: None,
        }).unwrap();
        let detail = get_account_inner(&state, aid).unwrap();
        assert!(detail.history.is_empty());
    }

    #[test]
    fn update_account_changes_fields() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        let site = find_or_create_site_inner(&state, "Gmail").unwrap();
        let aid = add_account_inner(&state, &AddAccountFields {
            site_id: site.id,
            username: Some("alice".into()),
            display_name: None,
            alias: None,
            notes: None,
            initial_password: None,
        }).unwrap();
        update_account_inner(&state, aid, &UpdateAccountFields {
            username: Some("bob".into()),
            display_name: None,
            alias: None,
            notes: Some("updated note".into()),
        }).unwrap();
        // AccountDetail doesn't expose notes, so verify username via IPC and notes via repo.
        let detail = get_account_inner(&state, aid).unwrap();
        assert_eq!(detail.username.as_deref(), Some("bob"));
        let acct = {
            let guard = state.vault.lock().unwrap();
            let v = guard.as_ref().unwrap();
            accounts::get(v, aid).unwrap()
        };
        assert_eq!(acct.notes.as_deref(), Some("updated note"));
    }

    #[test]
    fn promote_password_makes_chosen_row_current_via_ipc() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        let site = find_or_create_site_inner(&state, "Site").unwrap();
        let aid = add_account_inner(&state, &AddAccountFields {
            site_id: site.id,
            username: Some("u".into()),
            display_name: None,
            alias: None,
            notes: None,
            initial_password: Some("first".into()),
        }).unwrap();
        add_password_inner(&state, AddPasswordPayload {
            account_id: aid,
            plaintext: "second".into(),
            source: None,
        }).unwrap();

        let detail = get_account_inner(&state, aid).unwrap();
        // Find the oldest row (the one that's currently retired).
        let first_id = detail.history.iter()
            .min_by_key(|h| h.id)
            .unwrap().id;

        promote_password_inner(&state, first_id).unwrap();

        let detail = get_account_inner(&state, aid).unwrap();
        let promoted = detail.history.iter().find(|h| h.id == first_id).unwrap();
        assert!(promoted.is_current, "promoted row should be current (is_current=true)");
    }

    #[test]
    fn recover_candidates_empty_vault_returns_error() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        let result = recover_candidates_inner(&state, &RecoverFilters {
            site: None, account: None, era: None, hint: None,
            limit: 100, min_length: None,
            require_symbol: false, require_digit: false,
        });
        assert!(matches!(result, Err(GuiError::EmptyVault)),
                "expected EmptyVault, got {:?}", result.err());
    }

    #[test]
    fn recover_candidates_with_seeded_history_returns_results() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        {
            let guard = state.vault.lock().unwrap();
            let v = guard.as_ref().unwrap();
            let s = sites::create(v, sites::NewSite {
                name: "Reddit".into(),
                url: Some("reddit.com".into()),
                category: Some("Social".into()),
                abbreviations: vec!["Rd".into()],
                notes: None,
            }).unwrap();
            let a = accounts::create(v, accounts::NewAccount {
                site_id: s.id,
                username: Some("chris".into()),
                ..Default::default()
            }).unwrap();
            passwords::set_current(v, a.id, "MoonBeam$2019Rd", "manual").unwrap();
            passwords::set_current(v, a.id, "MoonBeam$2020Rd", "manual").unwrap();
            passwords::set_current(v, a.id, "MoonBeam$2021Rd", "manual").unwrap();
        }

        let result = recover_candidates_inner(&state, &RecoverFilters {
            site: Some("Reddit".into()),
            account: None, era: None,
            hint: Some("moon".into()),
            limit: 100, min_length: None,
            require_symbol: false, require_digit: false,
        }).unwrap();

        assert!(!result.is_empty(), "recovery should produce candidates");
        for (i, c) in result.iter().enumerate() {
            assert_eq!(c.rank, i + 1, "rank should be 1-indexed and sequential");
            assert!(c.score >= 0.0 && c.score <= 1.6, "score in expected range: {}", c.score);
        }
        {
            let cache = state.candidate_cache.lock().unwrap();
            for pw in cache.iter() {
                assert!(!pw.as_str().is_empty(), "cached password non-empty");
            }
        }

        // At least one candidate should carry provenance — verifies that the
        // RuleId -> RuleTag mapping in recover_candidates_inner actually fires
        // and produces non-empty tag/name strings.
        let with_prov = result.iter().find(|c| !c.provenance.is_empty()).expect("at least one candidate should have provenance");
        for r in &with_prov.provenance {
            assert!(!r.tag.is_empty(), "RuleTag.tag should be non-empty");
            assert!(!r.name.is_empty(), "RuleTag.name should be non-empty");
        }
    }

    #[test]
    fn recover_candidates_unknown_era_returns_era_not_found() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        // Seed minimal history so the EmptyVault error doesn't fire first.
        {
            let guard = state.vault.lock().unwrap();
            let v = guard.as_ref().unwrap();
            let s = sites::create(v, sites::NewSite {
                name: "X".into(), ..Default::default()
            }).unwrap();
            let a = accounts::create(v, accounts::NewAccount {
                site_id: s.id, ..Default::default()
            }).unwrap();
            passwords::set_current(v, a.id, "anything", "manual").unwrap();
        }

        let result = recover_candidates_inner(&state, &RecoverFilters {
            site: None, account: None,
            era: Some("nonexistent".into()),
            hint: None,
            limit: 100, min_length: None,
            require_symbol: false, require_digit: false,
        });
        match result {
            Err(GuiError::EraNotFound(name)) => assert_eq!(name, "nonexistent"),
            other => panic!("expected EraNotFound, got {:?}", other),
        }
    }

    #[test]
    fn recover_candidates_populates_cache() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        {
            let guard = state.vault.lock().unwrap();
            let v = guard.as_ref().unwrap();
            let s = sites::create(v, sites::NewSite {
                name: "Reddit".into(),
                url: Some("reddit.com".into()),
                category: Some("Social".into()),
                abbreviations: vec!["Rd".into()],
                notes: None,
            }).unwrap();
            let a = accounts::create(v, accounts::NewAccount {
                site_id: s.id,
                username: Some("chris".into()),
                ..Default::default()
            }).unwrap();
            passwords::set_current(v, a.id, "MoonBeam$2019Rd", "manual").unwrap();
            passwords::set_current(v, a.id, "MoonBeam$2020Rd", "manual").unwrap();
            passwords::set_current(v, a.id, "MoonBeam$2021Rd", "manual").unwrap();
        }

        let view = recover_candidates_inner(&state, &RecoverFilters {
            site: None,
            account: None,
            era: None,
            hint: None,
            limit: 10,
            min_length: None,
            require_symbol: false,
            require_digit: false,
        }).unwrap();

        assert!(!view.is_empty(), "expected at least one candidate");
        let cache = state.candidate_cache.lock().unwrap();
        assert_eq!(
            cache.len(),
            view.len(),
            "cache length must match returned metadata count"
        );
    }

    #[test]
    fn list_eras_returns_inserted_eras() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        {
            let guard = state.vault.lock().unwrap();
            let v = guard.as_ref().unwrap();
            eras::add(
                v,
                "RuneScape years",
                Some(NaiveDate::from_ymd_opt(2010, 1, 1).unwrap()),
                Some(NaiveDate::from_ymd_opt(2015, 12, 31).unwrap()),
                None,
            ).unwrap();
        }
        let eras = list_eras_inner(&state).unwrap();
        assert_eq!(eras.len(), 1);
        assert_eq!(eras[0].name, "RuneScape years");
        assert_eq!(eras[0].start_date.as_deref(), Some("2010-01-01"));
        assert_eq!(eras[0].end_date.as_deref(), Some("2015-12-31"));
    }

    #[test]
    fn add_era_round_trip_via_ipc() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();

        let id = add_era_inner(
            &state,
            EraFormArgs {
                name: "College".into(),
                start_date: Some("2016-01-01".into()),
                end_date: Some("2020-05-15".into()),
                notes: Some("ucla".into()),
            },
        ).unwrap();

        let eras = list_eras_inner(&state).unwrap();
        assert_eq!(eras.len(), 1);
        assert_eq!(eras[0].id, id);
        assert_eq!(eras[0].name, "College");
        assert_eq!(eras[0].start_date.as_deref(), Some("2016-01-01"));
        assert_eq!(eras[0].end_date.as_deref(), Some("2020-05-15"));
        assert_eq!(eras[0].notes.as_deref(), Some("ucla"));
    }

    #[test]
    fn update_era_round_trip() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        let id = add_era_inner(
            &state,
            EraFormArgs {
                name: "Old".into(),
                start_date: None,
                end_date: None,
                notes: None,
            },
        ).unwrap();
        update_era_inner(
            &state,
            id,
            EraFormArgs {
                name: "New".into(),
                start_date: Some("2024-01-01".into()),
                end_date: Some("2024-12-31".into()),
                notes: Some("note".into()),
            },
        ).unwrap();
        let eras = list_eras_inner(&state).unwrap();
        assert_eq!(eras.len(), 1);
        assert_eq!(eras[0].name, "New");
        assert_eq!(eras[0].start_date.as_deref(), Some("2024-01-01"));
        assert_eq!(eras[0].notes.as_deref(), Some("note"));
    }

    #[test]
    fn update_era_errors_on_invalid_date_range() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        let id = add_era_inner(
            &state,
            EraFormArgs { name: "X".into(), start_date: None, end_date: None, notes: None },
        ).unwrap();
        let err = update_era_inner(
            &state,
            id,
            EraFormArgs {
                name: "X".into(),
                start_date: Some("2024-12-01".into()),
                end_date: Some("2024-01-01".into()),
                notes: None,
            },
        ).unwrap_err();
        assert!(matches!(err, GuiError::InvalidInput(_)));
    }

    #[test]
    fn delete_era_returns_not_found_after_first_delete() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        let id = add_era_inner(
            &state,
            EraFormArgs { name: "Tmp".into(), start_date: None, end_date: None, notes: None },
        ).unwrap();
        delete_era_inner(&state, id).unwrap();
        // Second delete should surface a NotFound-style error from the repo layer.
        let err = delete_era_inner(&state, id).unwrap_err();
        // Loose check: error string contains "not" and "found".
        let dbg = format!("{err:?}").to_lowercase();
        assert!(
            dbg.contains("not") && dbg.contains("found"),
            "expected not-found-style error, got {err:?}"
        );
    }

    #[test]
    fn list_base_words_returns_decrypted_pool() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        {
            let guard = state.vault.lock().unwrap();
            let v = guard.as_ref().unwrap();
            let s = sites::create(v, sites::NewSite {
                name: "Reddit".into(),
                ..Default::default()
            }).unwrap();
            let a = accounts::create(v, accounts::NewAccount {
                site_id: s.id,
                username: Some("chris".into()),
                ..Default::default()
            }).unwrap();
            passwords::set_current(v, a.id, "MoonBeam$2019", "manual").unwrap();
            passwords::set_current(v, a.id, "MoonBeam$2020", "manual").unwrap();
            passwords::set_current(v, a.id, "MoonBeam$2021", "manual").unwrap();
        }

        // Analyze populates base_words. With three "MoonBeam$YYYY" passwords, at minimum
        // "moonbeam" should be extracted multiple times -> usage_count >= 1.
        analyze_base_words_inner(&state).unwrap();

        let result = list_base_words_inner(&state).unwrap();
        assert!(!result.is_empty(), "expected at least one base word");
        for w in &result {
            assert!(!w.word.is_empty(), "decrypted word should be non-empty");
            assert!(w.usage_count >= 1, "usage_count should be at least 1");
        }
        // The tokenizer splits MoonBeam at the camelCase boundary, producing "moon" and "beam".
        let has_moon = result.iter().any(|w| w.word.eq_ignore_ascii_case("moon"));
        assert!(has_moon, "expected 'moon' in pool, got {:?}", result.iter().map(|w| &w.word).collect::<Vec<_>>());
    }

    #[test]
    fn promote_then_list_reflects_favorite_flag() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        {
            let guard = state.vault.lock().unwrap();
            let v = guard.as_ref().unwrap();
            let s = sites::create(v, sites::NewSite {
                name: "S".into(), ..Default::default()
            }).unwrap();
            let a = accounts::create(v, accounts::NewAccount {
                site_id: s.id, ..Default::default()
            }).unwrap();
            // Need enough tokens that NOT every word gets auto-favorited (default top-N = 10).
            passwords::set_current(v, a.id, "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima", "manual").unwrap();
        }
        analyze_base_words_inner(&state).unwrap();
        let words = list_base_words_inner(&state).unwrap();
        // Pick a row that is NOT currently a favorite (one of the lower-ranked tokens).
        let target = words.iter().find(|w| !w.is_favorite)
            .expect("at least one non-favorite expected with 12 tokens and top-10 cutoff");
        let target_id = target.id;

        promote_base_word_inner(&state, target_id).unwrap();

        let after = list_base_words_inner(&state).unwrap();
        let row = after.iter().find(|w| w.id == target_id).unwrap();
        assert!(row.is_favorite, "row should be favorite after promote");
        assert!(row.manual_override, "row should have manual_override=true");
    }

    #[test]
    fn demote_then_list_reflects_favorite_flag() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        {
            let guard = state.vault.lock().unwrap();
            let v = guard.as_ref().unwrap();
            let s = sites::create(v, sites::NewSite {
                name: "S".into(), ..Default::default()
            }).unwrap();
            let a = accounts::create(v, accounts::NewAccount {
                site_id: s.id, ..Default::default()
            }).unwrap();
            // Three repetitions of the same word -> the canonical "moonbeam" will be
            // a high-frequency token and (with default top-N = 10) likely a favorite.
            passwords::set_current(v, a.id, "MoonBeam", "manual").unwrap();
            passwords::set_current(v, a.id, "MoonBeam$2019", "manual").unwrap();
            passwords::set_current(v, a.id, "MoonBeam$2020", "manual").unwrap();
        }
        analyze_base_words_inner(&state).unwrap();
        let words = list_base_words_inner(&state).unwrap();
        let target = words.iter().find(|w| w.is_favorite)
            .expect("at least one favorite expected after analyze on 3 MoonBeam passwords");
        let target_id = target.id;

        demote_base_word_inner(&state, target_id).unwrap();

        let after = list_base_words_inner(&state).unwrap();
        let row = after.iter().find(|w| w.id == target_id).unwrap();
        assert!(!row.is_favorite, "row should not be favorite after demote");
        assert!(row.manual_override, "row should have manual_override=true");
    }

    #[test]
    fn analyze_with_empty_history_returns_zero_report() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        // No accounts, no passwords. analyze should return a zero-value report,
        // NOT an error. extract_base_words_from_history bails early with default()
        // when password_history is empty.
        let report = analyze_base_words_inner(&state).unwrap();
        assert_eq!(report.tokens_seen, 0);
        assert_eq!(report.base_words_written, 0);
        assert_eq!(report.favorites_set, 0);
    }

    #[test]
    fn get_settings_returns_defaults_on_fresh_vault() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        let s = get_settings_inner(&state).unwrap();
        assert_eq!(s.idle_lock_seconds, 0);
        assert_eq!(s.clipboard_clear_seconds, 0);
        assert_eq!(s.reveal_clear_seconds, 0);
        assert_eq!(s.analyze_top_n, 10);
        assert!(!s.default_reveal);
    }

    #[test]
    fn set_then_get_round_trip() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        set_setting_inner(&state, SettingChange::IdleLockSeconds(900)).unwrap();
        set_setting_inner(&state, SettingChange::DefaultReveal(true)).unwrap();
        let s = get_settings_inner(&state).unwrap();
        assert_eq!(s.idle_lock_seconds, 900);
        assert!(s.default_reveal);
        // Untouched keys retain defaults.
        assert_eq!(s.clipboard_clear_seconds, 0);
        assert_eq!(s.analyze_top_n, 10);
    }

    #[test]
    fn set_setting_reveal_clear_seconds_round_trip() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();

        set_setting_inner(&state, SettingChange::RevealClearSeconds(45)).unwrap();
        let view = get_settings_inner(&state).unwrap();
        assert_eq!(view.reveal_clear_seconds, 45);
    }

    #[test]
    fn analyze_base_words_honors_top_n_from_settings() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        {
            let guard = state.vault.lock().unwrap();
            let v = guard.as_ref().unwrap();
            let s = sites::create(v, sites::NewSite {
                name: "S".into(), ..Default::default()
            }).unwrap();
            let a = accounts::create(v, accounts::NewAccount {
                site_id: s.id, ..Default::default()
            }).unwrap();
            // 5 distinct frequent tokens: alpha bravo charlie delta echo, each
            // appearing in multiple passwords so all rank above the cutoff.
            passwords::set_current(v, a.id, "alpha bravo charlie delta echo", "manual").unwrap();
            passwords::set_current(v, a.id, "alpha bravo charlie delta echo", "manual").unwrap();
            passwords::set_current(v, a.id, "alpha bravo charlie delta echo", "manual").unwrap();
        }
        // Set top-N to 3 BEFORE analyze.
        set_setting_inner(&state, SettingChange::AnalyzeTopN(3)).unwrap();

        analyze_base_words_inner(&state).unwrap();

        let words = list_base_words_inner(&state).unwrap();
        let favs = words.iter().filter(|w| w.is_favorite).count();
        assert_eq!(favs, 3, "expected exactly 3 favorites; got {}: {:?}",
            favs, words.iter().filter(|w| w.is_favorite).map(|w| &w.word).collect::<Vec<_>>());
    }

    #[test]
    fn change_master_password_round_trip_via_ipc() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"old").unwrap();
        {
            let guard = state.vault.lock().unwrap();
            let v = guard.as_ref().unwrap();
            let s = sites::create(v, sites::NewSite {
                name: "Reddit".into(),
                ..Default::default()
            }).unwrap();
            let a = accounts::create(v, accounts::NewAccount {
                site_id: s.id,
                username: Some("chris".into()),
                ..Default::default()
            }).unwrap();
            passwords::set_current(v, a.id, "MoonBeam$2019", "manual").unwrap();
        }

        change_master_password_inner(&state, b"old", b"new").unwrap();
        vault_lock_inner(&state).unwrap();
        vault_unlock_inner(&state, &path, b"new").unwrap();

        let list = list_accounts_inner(&state, None, None, None).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].site_name, "Reddit");
        assert_eq!(list[0].username.as_deref(), Some("chris"));
    }

    #[test]
    fn change_master_password_wrong_current_returns_error() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"actual").unwrap();
        let result = change_master_password_inner(&state, b"wrong", b"new");
        assert!(matches!(result, Err(GuiError::WrongPassword)),
                "expected WrongPassword, got {:?}", result.err());
        // Vault is still on the original password.
        vault_lock_inner(&state).unwrap();
        vault_unlock_inner(&state, &path, b"actual").unwrap();
    }

    #[test]
    fn record_recovery_feedback_inserts_row() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();

        record_recovery_feedback_inner(&state, RecordFeedbackPayload {
            account_id: None,
            provenance: vec!["G".into(), "E".into()],
            score: 0.75,
            rank: 3,
            worked: true,
            length: 14,
            has_digit: true,
            has_symbol: true,
            has_upper: true,
            has_lower: true,
        }).unwrap();

        let guard = state.vault.lock().unwrap();
        let v = guard.as_ref().unwrap();
        let count: i64 = v.conn()
            .query_row("SELECT COUNT(*) FROM recovery_feedback", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn clear_recovery_feedback_removes_all() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        for _ in 0..3 {
            record_recovery_feedback_inner(&state, RecordFeedbackPayload {
                account_id: None,
                provenance: vec!["G".into()],
                score: 0.5,
                rank: 1,
                worked: true,
                length: 10,
                has_digit: false,
                has_symbol: false,
                has_upper: false,
                has_lower: true,
            }).unwrap();
        }
        let n = clear_recovery_feedback_inner(&state).unwrap();
        assert_eq!(n, 3);

        let guard = state.vault.lock().unwrap();
        let v = guard.as_ref().unwrap();
        let count: i64 = v.conn()
            .query_row("SELECT COUNT(*) FROM recovery_feedback", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn recover_candidates_affected_by_feedback() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();

        // Seed enough history that recover produces candidates with BaseWordPool
        // provenance (the "G" rule). Several MoonBeam$YYYY passwords work.
        {
            let guard = state.vault.lock().unwrap();
            let v = guard.as_ref().unwrap();
            let s = sites::create(v, sites::NewSite {
                name: "Reddit".into(), ..Default::default()
            }).unwrap();
            let a = accounts::create(v, accounts::NewAccount {
                site_id: s.id, ..Default::default()
            }).unwrap();
            passwords::set_current(v, a.id, "MoonBeam$2019", "manual").unwrap();
            passwords::set_current(v, a.id, "MoonBeam$2020", "manual").unwrap();
            passwords::set_current(v, a.id, "MoonBeam$2021", "manual").unwrap();
        }

        let filters = RecoverFilters {
            site: Some("Reddit".into()),
            account: None, era: None,
            hint: Some("moon".into()),
            limit: 100, min_length: None,
            require_symbol: false, require_digit: false,
        };

        // Baseline: no feedback.
        let baseline = recover_candidates_inner(&state, &filters).unwrap();
        let baseline_top = baseline.iter().take(20)
            .filter(|c| c.provenance.iter().any(|t| t.tag == "G"))
            .count();

        // Inject heavy BaseWordPool boost via feedback (5 worked rows).
        for _ in 0..5 {
            record_recovery_feedback_inner(&state, RecordFeedbackPayload {
                account_id: None,
                provenance: vec!["G".into()],
                score: 0.5,
                rank: 1,
                worked: true,
                length: 10,
                has_digit: false,
                has_symbol: false,
                has_upper: false,
                has_lower: true,
            }).unwrap();
        }

        // After feedback: BaseWordPool-containing candidates should rank
        // at least as well as before.
        let after = recover_candidates_inner(&state, &filters).unwrap();
        let after_top = after.iter().take(20)
            .filter(|c| c.provenance.iter().any(|t| t.tag == "G"))
            .count();

        // Feedback should not REDUCE the count of BaseWordPool candidates in
        // the top-20. (A strict assertion that count *strictly increases* would
        // be brittle — depends on the specific candidate fan; the conservative
        // assertion is non-decrease.)
        assert!(
            after_top >= baseline_top,
            "expected BaseWordPool top-20 count to not decrease after positive feedback: baseline={}, after={}",
            baseline_top, after_top
        );
    }

    #[test]
    fn update_site_round_trip_via_ipc() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();

        let site = {
            let guard = state.vault.lock().unwrap();
            let v = guard.as_ref().unwrap();
            sites::create(v, sites::NewSite {
                name: "Reddit".into(),
                ..Default::default()
            }).unwrap()
        };

        update_site_inner(&state, site.id, UpdateSitePayload {
            name: "Old Reddit".into(),
            url: Some("old.reddit.com".into()),
            category: Some("Forum".into()),
            abbreviations: vec!["OR".into()],
            notes: Some("legacy".into()),
        }).unwrap();

        let guard = state.vault.lock().unwrap();
        let v = guard.as_ref().unwrap();
        let got = sites::get(v, site.id).unwrap();
        assert_eq!(got.name, "Old Reddit");
        assert_eq!(got.category.as_deref(), Some("Forum"));
    }

    #[test]
    fn generate_password_round_trip_via_ipc() {
        let pw = generate_password_inner(GeneratorOptionsPayload {
            length: 16,
            lowercase: true,
            uppercase: true,
            digits: true,
            symbols: true,
            avoid_ambiguous: false,
        }).unwrap();
        assert_eq!(pw.len(), 16);
    }

    #[test]
    fn add_base_word_round_trip_via_ipc() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();

        let view = add_base_word_inner(&state, "MoonBeam".into()).unwrap();
        assert!(view.is_favorite);
        assert!(view.manual_override);
        assert_eq!(view.usage_count, 0);

        // Verify duplicate rejection.
        let err = add_base_word_inner(&state, "moonbeam".into()).unwrap_err();
        let _ = err;  // Some GuiError variant; the exact mapping depends on existing From impls.
    }

    fn seed_vault_with_history(state: &VaultState) {
        let guard = state.vault.lock().unwrap();
        let v = guard.as_ref().unwrap();
        let s = sites::create(v, sites::NewSite {
            name: "Reddit".into(),
            url: Some("reddit.com".into()),
            category: Some("Social".into()),
            abbreviations: vec!["Rd".into()],
            notes: None,
        }).unwrap();
        let a = accounts::create(v, accounts::NewAccount {
            site_id: s.id,
            username: Some("chris".into()),
            ..Default::default()
        }).unwrap();
        passwords::set_current(v, a.id, "MoonBeam$2019Rd", "manual").unwrap();
        passwords::set_current(v, a.id, "MoonBeam$2020Rd", "manual").unwrap();
        passwords::set_current(v, a.id, "MoonBeam$2021Rd", "manual").unwrap();
    }

    #[test]
    fn reveal_candidate_returns_plaintext_for_valid_rank() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        seed_vault_with_history(&state);

        recover_candidates_inner(&state, &RecoverFilters {
            site: None, account: None, era: None, hint: None,
            limit: 5, min_length: None, require_symbol: false, require_digit: false,
        }).unwrap();

        let plaintext = reveal_candidate_inner(&state, 1).unwrap();
        assert!(!plaintext.is_empty(), "expected non-empty plaintext at rank 1");
    }

    #[test]
    fn reveal_candidate_errors_on_empty_cache() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        // No recovery run.
        let err = reveal_candidate_inner(&state, 1).unwrap_err();
        assert!(matches!(err, GuiError::NoActiveRecovery),
            "expected NoActiveRecovery, got {err:?}");
    }

    #[test]
    fn reveal_candidate_errors_on_rank_out_of_bounds() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        seed_vault_with_history(&state);
        recover_candidates_inner(&state, &RecoverFilters {
            site: None, account: None, era: None, hint: None,
            limit: 3, min_length: None, require_symbol: false, require_digit: false,
        }).unwrap();
        let err = reveal_candidate_inner(&state, 999).unwrap_err();
        assert!(matches!(err, GuiError::RankOutOfBounds));
    }

    #[test]
    fn reveal_candidate_errors_when_locked() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        seed_vault_with_history(&state);
        recover_candidates_inner(&state, &RecoverFilters {
            site: None, account: None, era: None, hint: None,
            limit: 3, min_length: None, require_symbol: false, require_digit: false,
        }).unwrap();
        vault_lock_inner(&state).unwrap();
        let err = reveal_candidate_inner(&state, 1).unwrap_err();
        // After Task 1's lock-clears-cache, the cache is empty AND the vault is locked.
        // The vault-locked check fires first, returning Locked. But if the implementer
        // changes that order, NoActiveRecovery is also acceptable.
        assert!(
            matches!(err, GuiError::Locked | GuiError::NoActiveRecovery),
            "expected Locked or NoActiveRecovery, got {err:?}"
        );
    }

    #[test]
    fn candidate_cache_cleared_on_lock() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        seed_vault_with_history(&state);
        recover_candidates_inner(&state, &RecoverFilters {
            site: None, account: None, era: None, hint: None,
            limit: 5, min_length: None, require_symbol: false, require_digit: false,
        }).unwrap();
        assert!(!state.candidate_cache.lock().unwrap().is_empty());
        vault_lock_inner(&state).unwrap();
        assert!(state.candidate_cache.lock().unwrap().is_empty(),
            "cache must be empty after lock");
    }

    #[test]
    fn candidate_view_has_no_password_field() {
        // Compile-time check: this literal would fail to build if a `password` field were re-added.
        let _: CandidateView = CandidateView {
            rank: 1,
            score: 0.5,
            provenance: vec![],
        };
    }

    #[test]
    fn import_csv_commit_pending_errors_with_no_pending_path() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        let err = import_csv_commit_pending_inner(&state, None, None, vec![]).unwrap_err();
        assert!(matches!(err, GuiError::NoPendingImport));
    }

    #[test]
    fn import_csv_dry_run_with_pending_errors_with_no_pending() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        match import_csv_dry_run_with_pending_inner(&state, None, None, vec![]) {
            Err(GuiError::NoPendingImport) => {}
            other => panic!("expected NoPendingImport, got {:?}", other.err()),
        }
    }

    #[test]
    fn cancel_pending_import_clears_slot() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        {
            let mut slot = state.pending_import_path.lock().unwrap();
            *slot = Some(std::path::PathBuf::from("/tmp/fake.csv"));
        }
        cancel_pending_import_inner(&state).unwrap();
        assert!(state.pending_import_path.lock().unwrap().is_none());
    }

    #[test]
    fn pending_import_path_cleared_on_lock() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();
        {
            let mut slot = state.pending_import_path.lock().unwrap();
            *slot = Some(std::path::PathBuf::from("/tmp/fake.csv"));
        }
        vault_lock_inner(&state).unwrap();
        assert!(state.pending_import_path.lock().unwrap().is_none());
    }

    #[test]
    fn import_csv_dry_run_with_patches_promotes_skipped_rows_via_ipc() {
        let (_tmp, path) = temp_vault();
        let state = VaultState::new();
        vault_create_inner(&state, &path, b"pw").unwrap();

        // CSV with one good row and one missing-site row.
        let csv_text = "name,user,password\nReddit,alice,hunter2\n,bob,S3cret!\n";
        let tmp_dir = tempfile::TempDir::new().unwrap();
        let csv_path = tmp_dir.path().join("p.csv");
        std::fs::write(&csv_path, csv_text).unwrap();

        // No patches: row 3 (empty-site) is a diagnostic.
        let preview = import_csv_dry_run_inner(&state, &csv_path, None, None, vec![]).unwrap();
        assert_eq!(preview.counts.new, 1, "expected 1 successful + 1 diagnostic without patch");
        assert_eq!(preview.diagnostics.len(), 1);
        assert_eq!(preview.diagnostics[0].row, 3);
        assert_eq!(preview.diagnostics[0].reason, "missing site");
        assert!(preview.diagnostics[0].parsed.has_password);
        assert_eq!(preview.diagnostics[0].parsed.username.as_deref(), Some("bob"));

        // With a patch supplying site for row 3, both rows are entries.
        let patches = vec![passhound_core::importer::RowPatch {
            row: 3,
            site: Some("Reddit".to_string()),
            password: None,
        }];
        let preview2 = import_csv_dry_run_inner(&state, &csv_path, None, None, patches).unwrap();
        assert_eq!(preview2.counts.new, 2, "patched row should be promoted to an entry");
        assert!(preview2.diagnostics.is_empty(), "no diagnostics after patching");
    }
}
