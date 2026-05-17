//! Phase 4.15 hardening regression tests.

use passhound_core::Vault;
use rusqlite::Connection;
use tempfile::TempDir;

#[test]
fn sqlite_version_is_at_least_3_50_2() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("v.db");
    let _ = Vault::create(&path, b"pw").unwrap();
    let conn = Connection::open(&path).unwrap();
    let version: String = conn
        .query_row("SELECT sqlite_version()", [], |r| r.get(0))
        .unwrap();
    let parts: Vec<u32> = version.split('.').filter_map(|s| s.parse().ok()).collect();
    assert!(parts.len() >= 3, "unexpected sqlite_version format: {version}");
    let (maj, min, patch) = (parts[0], parts[1], parts[2]);
    assert!(
        (maj, min, patch) >= (3, 50, 2),
        "expected SQLite >= 3.50.2 (CVE-2025-6965 defensive), got {version}"
    );
}
