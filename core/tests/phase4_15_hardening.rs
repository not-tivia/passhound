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

