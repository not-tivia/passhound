use passhound_core::importer::{parse_paste, pipeline};
use passhound_core::repo::sites;
use passhound_core::schema;
use passhound_core::Vault;
use rusqlite::{params, Connection};
use tempfile::TempDir;

/// Manually construct a Phase-1-shaped vault (apply_initial only, no
/// schema_version row), then re-open with the new code and run an import.
#[test]
fn phase1_vault_auto_upgrades_on_open() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("v.db");

    {
        // Build a Phase-1 vault by hand.
        let mut conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        let tx = conn.transaction().unwrap();
        schema::apply_initial(&tx).unwrap();

        // Derive a master key + verifier; write salt/verifier_ct/verifier_nonce.
        // We mirror Vault::create's writes WITHOUT calling apply_migrations.
        use passhound_core::crypto::{aead, kdf};
        let salt = kdf::generate_salt();
        let key_bytes = kdf::derive_key(b"hunter2", &salt).unwrap();
        let (vct, vnonce) = aead::encrypt(&key_bytes, b"passhound-vault-v1").unwrap();
        tx.execute(
            "INSERT INTO vault_meta (key, value) VALUES (?1, ?2)",
            params!["salt", salt.as_slice()],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO vault_meta (key, value) VALUES (?1, ?2)",
            params!["verifier_ct", vct],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO vault_meta (key, value) VALUES (?1, ?2)",
            params!["verifier_nonce", vnonce.as_slice()],
        )
        .unwrap();
        tx.commit().unwrap();
        // Confirm: schema_version row absent.
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM vault_meta WHERE key='schema_version'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(exists, 0, "Phase-1 fixture should not have schema_version");
    }

    // Now open with the Phase-2 binary (Vault::open runs apply_migrations).
    let mut v = Vault::open(&path).unwrap();
    v.unlock(b"hunter2").unwrap();

    // Confirm schema_version is now present and equals 4.
    let val: Vec<u8> = v
        .conn()
        .query_row(
            "SELECT value FROM vault_meta WHERE key='schema_version'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(val.as_slice(), b"4");

    // Confirm the new column exists.
    let mut stmt = v.conn().prepare("PRAGMA table_info(password_history)").unwrap();
    let cols: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(cols.contains(&"source_import_id".into()));

    // And the upgraded vault is fully usable: run an import end-to-end.
    let parse = parse_paste("site: Foo\npassword: pw\n");
    let preview = pipeline::preview(&v, parse.entries).unwrap();
    pipeline::commit(&v, preview, "paste", None).unwrap();
    assert_eq!(sites::list(&v).unwrap().len(), 1);
}
