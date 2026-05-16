//! Phase 4.13 hardening regression tests:
//! - Vault::open returns Error::NotFound for a missing path
//! - Vault file is 0600 on Unix after create
//! - PRAGMA secure_delete is ON after re-open
//! - EXPLAIN QUERY PLAN on the partial idx_pw_current shows it's used

use passhound_core::{Error, Vault};
use rusqlite::Connection;
use std::path::PathBuf;
use tempfile::TempDir;

#[test]
fn vault_open_on_missing_path_returns_not_found() {
    let nonexistent = PathBuf::from("/tmp/passhound_does_not_exist_phase_4_13_xyz.db");
    let result = Vault::open(&nonexistent);
    assert!(
        matches!(result, Err(Error::NotFound)),
        "expected Err(Error::NotFound), got {result:?}"
    );
}

#[cfg(unix)]
#[test]
fn vault_file_has_0600_perms_on_unix() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("v.db");
    let _ = Vault::create(&path, b"hunter2").unwrap();

    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    let owner_only = mode & 0o777;
    assert_eq!(
        owner_only, 0o600,
        "expected 0600 perms, got 0{:o}",
        owner_only
    );
}

#[test]
fn pragma_secure_delete_is_on_after_open() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("v.db");
    {
        let _ = Vault::create(&path, b"hunter2").unwrap();
        // Drop the vault so Vault::open below uses a fresh connection.
    }
    let v = Vault::open(&path).unwrap();

    // secure_delete is a per-connection PRAGMA that Vault::open sets on its
    // own connection. Query it through the Vault's exposed conn() accessor.
    let val: i64 = v
        .conn()
        .query_row("PRAGMA secure_delete", [], |r| r.get(0))
        .unwrap();
    assert_eq!(val, 1, "expected PRAGMA secure_delete = 1, got {val}");
}

#[test]
fn explain_query_plan_uses_idx_pw_current() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("v.db");
    let _ = Vault::create(&path, b"hunter2").unwrap();

    let conn = Connection::open(&path).unwrap();
    // EXPLAIN QUERY PLAN column layout in rusqlite: id | parent | notused | detail
    let plan: String = conn
        .query_row(
            "EXPLAIN QUERY PLAN SELECT id FROM password_history \
             WHERE account_id = 1 AND retired_at IS NULL",
            [],
            |r| r.get::<_, String>(3),
        )
        .unwrap_or_default();

    assert!(
        plan.contains("idx_pw_current"),
        "expected query plan to use idx_pw_current, got: {plan}"
    );
}
