use passhound_core::Vault;
use rusqlite::Connection;
use tempfile::TempDir;

/// Open a freshly-created vault and verify the recovery_feedback table exists
/// after the migration runner ran during Vault::create. This proves the new
/// migration is wired in.
#[test]
fn fresh_vault_has_recovery_feedback_table_after_phase8() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("v.db");
    let _ = Vault::create(&path, b"hunter2").unwrap();

    let conn = Connection::open(&path).unwrap();

    // Confirm schema_version is 8.
    let val: Vec<u8> = conn
        .query_row(
            "SELECT value FROM vault_meta WHERE key='schema_version'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(val.as_slice(), b"8");

    // Confirm the table exists by querying its columns.
    let mut stmt = conn.prepare("PRAGMA table_info(recovery_feedback)").unwrap();
    let cols: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    let expected: &[&str] = &[
        "id", "account_id", "provenance", "score", "rank", "worked",
        "length", "has_digit", "has_symbol", "has_upper", "has_lower", "feedback_at",
    ];
    for col in expected {
        assert!(cols.iter().any(|c| c == col), "missing column {} in recovery_feedback", col);
    }

    // Confirm the index exists.
    let idx_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_recovery_feedback_worked'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(idx_count, 1, "idx_recovery_feedback_worked missing");
}
