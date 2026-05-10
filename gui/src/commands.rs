//! Tauri IPC commands. Each public `#[tauri::command]` function delegates to
//! an `_inner` helper that takes a plain `&VaultState`, so unit tests can
//! exercise the command logic without spinning up the Tauri runtime.

use crate::error::GuiError;
use crate::state::VaultState;
use passhound_core::{repo, Vault};
use serde::Serialize;
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

#[derive(Serialize)]
pub struct AccountSummary {
    pub id: i64,
    pub site_name: String,
    pub username: Option<String>,
    pub last_changed: Option<String>,
    pub category: Option<String>,
}

#[tauri::command]
pub fn list_accounts(
    state: State<'_, VaultState>,
    filter: Option<String>,
) -> Result<Vec<AccountSummary>, GuiError> {
    list_accounts_inner(&state, filter.as_deref())
}

pub fn list_accounts_inner(
    state: &VaultState,
    filter: Option<&str>,
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
    let sql = "
        SELECT a.id, s.name, a.username, s.category,
               (SELECT MAX(ph.created_at) FROM password_history ph WHERE ph.account_id = a.id) AS last_changed
        FROM accounts a
        JOIN sites s ON s.id = a.site_id
        ORDER BY last_changed DESC NULLS LAST, s.name ASC
    ";
    let mut stmt = v.conn().prepare(sql)?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<String>>(4)?,
        ))
    })?;
    let mut out: Vec<AccountSummary> = Vec::new();
    for row in rows {
        let (id, site_name, username, category, last_changed) = row?;
        if let Some(needle) = &needle {
            let hay = format!(
                "{} {} {}",
                site_name.to_lowercase(),
                username.as_deref().unwrap_or("").to_lowercase(),
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
            last_changed,
            category,
        });
    }
    Ok(out)
}

#[derive(Serialize)]
pub struct AccountDetail {
    pub id: i64,
    pub site_name: String,
    pub site_url: Option<String>,
    pub site_category: Option<String>,
    pub site_abbreviations: Vec<String>,
    pub username: Option<String>,
    pub history: Vec<HistoryEntry>,
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
    Ok(AccountDetail {
        id: acct.id,
        site_name: site.name,
        site_url: site.url,
        site_category: site.category,
        site_abbreviations: site.abbreviations,
        username: acct.username,
        history,
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

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use passhound_core::repo::{accounts, passwords, sites};
    use tempfile::TempDir;

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
        let list = list_accounts_inner(&state, None).unwrap();
        assert!(list.is_empty());
        // Lock
        vault_lock_inner(&state).unwrap();
        assert!(matches!(list_accounts_inner(&state, None), Err(GuiError::Locked)));
        // Re-unlock
        vault_unlock_inner(&state, &path, b"hunter2").unwrap();
        let list = list_accounts_inner(&state, None).unwrap();
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
        let list = list_accounts_inner(&state, None).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].site_name, "Reddit");
        assert_eq!(list[0].username.as_deref(), Some("chris"));
        assert_eq!(list[0].category.as_deref(), Some("Social"));
        assert!(list[0].last_changed.is_some());
        // Filter
        let filtered = list_accounts_inner(&state, Some("redd")).unwrap();
        assert_eq!(filtered.len(), 1);
        let unfiltered = list_accounts_inner(&state, Some("zzz_no_match_zzz")).unwrap();
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
}
