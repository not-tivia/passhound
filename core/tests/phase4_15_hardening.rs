//! Phase 4.15 hardening regression tests.

/// SQLite version floor: 3.50.2 (post CVE-2025-6965).
/// Encoded as `MMMNNNPPP` to match `rusqlite::version_number()`'s return shape.
const SQLITE_MIN_VERSION: i32 = 3_050_002;

#[test]
fn sqlite_version_is_at_least_3_50_2() {
    let got = rusqlite::version_number();
    assert!(
        got >= SQLITE_MIN_VERSION,
        "expected SQLite >= 3.50.2 (CVE-2025-6965 defensive), got {}",
        rusqlite::version()
    );
}

/// Verify that `chmod_journal_if_present` sets 0600 on a pre-existing journal sidecar.
///
/// We use the staged-journal approach: pre-create a `.db-journal` file with 0644
/// perms at the path that `Vault::create` will inspect post-commit, then call
/// `Vault::create`. The helper fires after `tx.commit()` and, if the staged file
/// is still there (SQLite hasn't overwritten it with its own journal), it should
/// be chmoded to 0600.
///
/// Because we are testing the chmod helper in isolation (the real SQLite journal
/// is ephemeral), we call `chmod_journal_if_present` directly with a known-good
/// journal file. This is the most reliable approach.
#[cfg(unix)]
#[test]
fn journal_sidecar_chmod_helper_sets_0600() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::TempDir::new().unwrap();
    let db_path = tmp.path().join("v.db");
    let journal_path = tmp.path().join("v.db-journal");

    // Touch a fake journal with permissive default perms (e.g., 0644).
    std::fs::write(&journal_path, b"fake journal contents").unwrap();
    std::fs::set_permissions(&journal_path, std::fs::Permissions::from_mode(0o644)).unwrap();

    // Confirm it starts at 0644.
    let before = std::fs::metadata(&journal_path).unwrap().permissions().mode() & 0o777;
    assert_eq!(before, 0o644, "pre-condition: journal starts at 0644");

    // Drive the helper directly (it is pub(crate), accessible from integration
    // tests via the re-export below). db_path is ".../v.db" and
    // db_path.with_extension("db-journal") == ".../v.db-journal", which matches
    // journal_path, so the helper will find and chmod it.
    passhound_core::vault_chmod_journal_if_present(&db_path);

    let after = std::fs::metadata(&journal_path).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        after, 0o600,
        "expected 0600 on staged journal sidecar after chmod helper, got 0{:o}",
        after
    );
}
