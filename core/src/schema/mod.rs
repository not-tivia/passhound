use crate::error::Result;
use rusqlite::Connection;

const INITIAL: &str = include_str!("001_initial.sql");

/// Apply the initial schema to a fresh DB. NOT idempotent — calling on an
/// already-initialized DB fails with a SQLite "table already exists" error,
/// which is the caller's signal to use `Vault::open` rather than `Vault::create`.
pub fn apply_initial(conn: &Connection) -> Result<()> {
    conn.execute_batch(INITIAL)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
