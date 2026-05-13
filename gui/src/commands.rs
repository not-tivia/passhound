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
) -> Result<Vec<AccountSummary>, GuiError> {
    list_accounts_inner(&state, filter.as_deref(), tag_ids)
}

pub fn list_accounts_inner(
    state: &VaultState,
    filter: Option<&str>,
    tag_ids: Option<Vec<i64>>,
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

#[derive(Serialize)]
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
    pub diagnostics: Vec<String>,
}

#[derive(Serialize)]
pub struct CommitResult {
    pub import_id: i64,
    pub counts: PreviewCounts,
}

const SAMPLE_ROW_LIMIT: usize = 5;

#[tauri::command]
pub fn import_csv_dry_run(
    state: State<'_, VaultState>,
    path: String,
    site_override: Option<String>,
    mapping: Option<MappingArgs>,
) -> Result<PreviewResult, GuiError> {
    import_csv_dry_run_inner(&state, &std::path::PathBuf::from(path), site_override, mapping)
}

pub fn import_csv_dry_run_inner(
    state: &VaultState,
    path: &std::path::Path,
    site_override: Option<String>,
    mapping: Option<MappingArgs>,
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

    let diagnostics: Vec<String> = parse
        .diagnostics
        .iter()
        .map(|d| format!("row {}: {}", d.row, d.reason))
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

#[tauri::command]
pub fn import_csv_commit(
    state: State<'_, VaultState>,
    path: String,
    site_override: Option<String>,
    mapping: Option<MappingArgs>,
) -> Result<CommitResult, GuiError> {
    import_csv_commit_inner(&state, &std::path::PathBuf::from(path), site_override, mapping)
}

pub fn import_csv_commit_inner(
    state: &VaultState,
    path: &std::path::Path,
    site_override: Option<String>,
    mapping: Option<MappingArgs>,
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

#[tauri::command]
pub fn add_password(
    state: State<'_, VaultState>,
    account_id: i64,
    plaintext: String,
) -> Result<i64, GuiError> {
    add_password_inner(&state, account_id, &plaintext)
}

pub fn add_password_inner(
    state: &VaultState,
    account_id: i64,
    plaintext: &str,
) -> Result<i64, GuiError> {
    let guard = state.vault.lock().map_err(poisoned)?;
    let v = guard.as_ref().ok_or(GuiError::Locked)?;
    let record = passhound_core::repo::passwords::set_current(v, account_id, plaintext, "manual")?;
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

#[derive(serde::Serialize, Debug, Clone)]
pub struct CandidateView {
    pub rank: usize,
    pub score: f32,
    pub password: String,
    pub provenance: Vec<RuleTag>,
}

#[derive(serde::Serialize, Debug, Clone)]
pub struct EraSummary {
    pub id: i64,
    pub name: String,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
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
    Ok(candidates
        .into_iter()
        .enumerate()
        .map(|(i, c)| CandidateView {
            rank: i + 1,
            score: c.score,
            password: c.password.to_string(),
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
        })
        .collect())
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
        let list = list_accounts_inner(&state, None, None).unwrap();
        assert!(list.is_empty());
        // Lock
        vault_lock_inner(&state).unwrap();
        assert!(matches!(list_accounts_inner(&state, None, None), Err(GuiError::Locked)));
        // Re-unlock
        vault_unlock_inner(&state, &path, b"hunter2").unwrap();
        let list = list_accounts_inner(&state, None, None).unwrap();
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
        let list = list_accounts_inner(&state, None, None).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].site_name, "Reddit");
        assert_eq!(list[0].username.as_deref(), Some("chris"));
        assert_eq!(list[0].category.as_deref(), Some("Social"));
        assert!(list[0].last_changed.is_some());
        assert_eq!(list[0].display_name.as_deref(), Some("MaxedNoob"));
        // Filter
        let filtered = list_accounts_inner(&state, Some("redd"), None).unwrap();
        assert_eq!(filtered.len(), 1);
        let unfiltered = list_accounts_inner(&state, Some("zzz_no_match_zzz"), None).unwrap();
        assert_eq!(unfiltered.len(), 0);
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

        let preview = import_csv_dry_run_inner(&state, &csv_path, None, None).unwrap();
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

        let r = import_csv_commit_inner(&state, &csv_path, None, None).unwrap();
        assert_eq!(r.counts.new, 1);
        assert!(r.import_id > 0);

        // Verify the row is in the vault now.
        let list = list_accounts_inner(&state, None, None).unwrap();
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
        let before = list_accounts_inner(&state, None, None).unwrap();
        assert_eq!(before.len(), 1);

        delete_account_inner(&state, account_id).unwrap();

        // Account is gone.
        let after = list_accounts_inner(&state, None, None).unwrap();
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
        let filtered = list_accounts_inner(&state, None, Some(vec![tag.id])).unwrap();
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
        let remaining = list_accounts_inner(&state, None, None).unwrap();
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
        add_password_inner(&state, aid, "second").unwrap();

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
            assert!(!c.password.is_empty(), "password non-empty");
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
}
