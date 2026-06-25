use crate::error::{Error, Result};
use rusqlite::{params, Connection, OptionalExtension};

const INITIAL: &str = include_str!("001_initial.sql");

pub const LATEST_VERSION: i32 = 9;
const SCHEMA_VERSION_KEY: &str = "schema_version";
const MIGRATION_002: &str = include_str!("002_source_provenance.sql");
const MIGRATION_003: &str = include_str!("003_base_word_manual_override.sql");
const MIGRATION_004: &str = include_str!("004_base_word_casing.sql");
const MIGRATION_005: &str = include_str!("005_account_display_name.sql");
const MIGRATION_006: &str = include_str!("006_attachments.sql");
const MIGRATION_007: &str = include_str!("007_tags.sql");
const MIGRATION_008: &str = include_str!("008_recovery_feedback.sql");
const MIGRATION_009: &str = include_str!("009_site_aliases.sql");

/// Apply the initial schema to a fresh DB. NOT idempotent — calling on an
/// already-initialized DB fails with a SQLite "table already exists" error,
/// which is the caller's signal to use `Vault::open` rather than `Vault::create`.
pub fn apply_initial(conn: &Connection) -> Result<()> {
    conn.execute_batch(INITIAL)?;
    Ok(())
}

/// Apply any migrations newer than the DB's current schema_version. Idempotent.
/// Convention: a vault with no schema_version row is treated as version 1
/// (because apply_initial covers schema 001). On a fresh DB, callers should
/// invoke apply_initial first, then apply_migrations.
pub fn apply_migrations(conn: &Connection) -> Result<()> {
    let current: i32 = conn
        .query_row(
            "SELECT value FROM vault_meta WHERE key = ?1",
            params![SCHEMA_VERSION_KEY],
            |r| {
                let v: Vec<u8> = r.get(0)?;
                Ok(std::str::from_utf8(&v)
                    .ok()
                    .and_then(|s| s.parse::<i32>().ok())
                    .unwrap_or(1))
            },
        )
        .optional()?
        .unwrap_or(1);

    if current < 1 || current > LATEST_VERSION {
        return Err(Error::InvalidInput(format!(
            "unsupported schema_version {current}"
        )));
    }
    if current >= LATEST_VERSION {
        return Ok(());
    }
    // The caller is responsible for atomicity: invoke this either inside an
    // already-open transaction (Vault::create) or wrap it in one (Vault::open).
    // Opening a nested tx here would fail because SQLite doesn't support them.
    if current < 2 {
        conn.execute_batch(MIGRATION_002)?;
    }
    if current < 3 {
        conn.execute_batch(MIGRATION_003)?;
    }
    if current < 4 {
        conn.execute_batch(MIGRATION_004)?;
    }
    if current < 5 {
        conn.execute_batch(MIGRATION_005)?;
    }
    if current < 6 {
        conn.execute_batch(MIGRATION_006)?;
    }
    if current < 7 {
        conn.execute_batch(MIGRATION_007)?;
    }
    if current < 8 {
        conn.execute_batch(MIGRATION_008)?;
    }
    if current < 9 {
        conn.execute_batch(MIGRATION_009)?;
    }
    conn.execute(
        "INSERT INTO vault_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![SCHEMA_VERSION_KEY, LATEST_VERSION.to_string().as_bytes()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_conn_with_initial() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        apply_initial(&conn).unwrap();
        conn
    }

    #[test]
    fn applies_to_fresh_db() {
        let conn = Connection::open_in_memory().unwrap();
        apply_initial(&conn).unwrap();
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap();
        let names: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        // sqlite_master lists internal tables too (e.g. sqlite_sequence from
        // AUTOINCREMENT) — we only check that our 9 user tables are present.
        assert!(names.contains(&"sites".into()));
        assert!(names.contains(&"accounts".into()));
        assert!(names.contains(&"password_history".into()));
        assert!(names.contains(&"base_words".into()));
        assert!(names.contains(&"eras".into()));
        assert!(names.contains(&"tags".into()));
        assert!(names.contains(&"account_tags".into()));
        assert!(names.contains(&"imports".into()));
        assert!(names.contains(&"vault_meta".into()));
    }

    #[test]
    fn apply_migrations_on_fresh_db_sets_version_to_latest() {
        let conn = fresh_conn_with_initial();
        apply_migrations(&conn).unwrap();
        let v: Vec<u8> = conn
            .query_row(
                "SELECT value FROM vault_meta WHERE key = ?1",
                params![SCHEMA_VERSION_KEY],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(v.as_slice(), b"9");
    }

    #[test]
    fn apply_migrations_is_idempotent() {
        let conn = fresh_conn_with_initial();
        apply_migrations(&conn).unwrap();
        apply_migrations(&conn).unwrap();
        let v: Vec<u8> = conn
            .query_row(
                "SELECT value FROM vault_meta WHERE key = ?1",
                params![SCHEMA_VERSION_KEY],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(v.as_slice(), b"9");
    }

    #[test]
    fn apply_migrations_upgrades_phase_1_vault() {
        // Simulate a Phase-1 vault: apply_initial only, no schema_version row.
        let conn = fresh_conn_with_initial();
        // Assert columns not yet present.
        let mut stmt = conn.prepare("PRAGMA table_info(password_history)").unwrap();
        let cols_before: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(!cols_before.contains(&"source_import_id".into()));

        let mut stmt = conn.prepare("PRAGMA table_info(base_words)").unwrap();
        let bw_before: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(!bw_before.contains(&"manual_override".into()));
        assert!(!bw_before.contains(&"casing_mask".into()));

        apply_migrations(&conn).unwrap();

        let mut stmt = conn.prepare("PRAGMA table_info(attachments)").unwrap();
        let attachment_cols: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(attachment_cols.contains(&"blob_encrypted".into()),
            "attachments table with blob_encrypted column should exist; got {attachment_cols:?}");

        let mut stmt = conn.prepare("PRAGMA table_info(password_history)").unwrap();
        let cols_after: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(cols_after.contains(&"source_import_id".into()));

        let mut stmt = conn.prepare("PRAGMA table_info(base_words)").unwrap();
        let bw_after: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(bw_after.contains(&"manual_override".into()));
        assert!(bw_after.contains(&"casing_mask".into()));

        let mut stmt = conn.prepare("PRAGMA table_info(accounts)").unwrap();
        let acct_cols: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(acct_cols.contains(&"display_name".into()),
            "accounts.display_name column should exist post-migration; got {acct_cols:?}");
    }

    #[test]
    fn apply_migrations_rejects_unknown_version() {
        let conn = fresh_conn_with_initial();
        // Manually write a bogus future version.
        conn.execute(
            "INSERT INTO vault_meta (key, value) VALUES (?1, ?2)",
            params![SCHEMA_VERSION_KEY, b"99"],
        )
        .unwrap();
        let err = apply_migrations(&conn).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }
}
