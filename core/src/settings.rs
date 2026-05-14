//! User-tunable settings stored in the vault_meta key/value table.
//!
//! All settings are stored as ASCII bytes under keys prefixed with "settings.".
//! Missing keys silently fall back to defaults; malformed values do too. Values
//! written through `set_u32` / `set_bool` are clamped to per-key ranges so the
//! GUI can pass through raw user input without re-validating.

use crate::error::Result;
use crate::vault::Vault;
use rusqlite::params;

pub struct SettingsView {
    pub idle_lock_seconds: u32,
    pub clipboard_clear_seconds: u32,
    pub analyze_top_n: u32,
    pub default_reveal: bool,
}

impl Default for SettingsView {
    fn default() -> Self {
        Self {
            idle_lock_seconds: 0,
            clipboard_clear_seconds: 0,
            analyze_top_n: 10,
            default_reveal: false,
        }
    }
}

pub const KEY_IDLE_LOCK: &str = "settings.idle_lock_seconds";
pub const KEY_CLIPBOARD_CLEAR: &str = "settings.clipboard_clear_seconds";
pub const KEY_ANALYZE_TOP_N: &str = "settings.analyze_top_n";
pub const KEY_DEFAULT_REVEAL: &str = "settings.default_reveal";

const MAX_SECONDS: u32 = 86_400;
const MAX_ANALYZE_TOP_N: u32 = 100;
const MIN_ANALYZE_TOP_N: u32 = 1;

fn read_u32(vault: &Vault, key: &str, default: u32) -> Result<u32> {
    let opt: Option<Vec<u8>> = vault
        .conn()
        .query_row(
            "SELECT value FROM vault_meta WHERE key = ?1",
            params![key],
            |r| r.get(0),
        )
        .ok();
    Ok(match opt {
        Some(bytes) => std::str::from_utf8(&bytes)
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(default),
        None => default,
    })
}

fn read_bool(vault: &Vault, key: &str, default: bool) -> Result<bool> {
    let opt: Option<Vec<u8>> = vault
        .conn()
        .query_row(
            "SELECT value FROM vault_meta WHERE key = ?1",
            params![key],
            |r| r.get(0),
        )
        .ok();
    Ok(match opt {
        Some(bytes) => match bytes.as_slice() {
            b"1" => true,
            b"0" => false,
            _ => default,
        },
        None => default,
    })
}

pub fn get(vault: &Vault) -> Result<SettingsView> {
    Ok(SettingsView {
        idle_lock_seconds: read_u32(vault, KEY_IDLE_LOCK, 0)?,
        clipboard_clear_seconds: read_u32(vault, KEY_CLIPBOARD_CLEAR, 0)?,
        analyze_top_n: read_u32(vault, KEY_ANALYZE_TOP_N, 10)?,
        default_reveal: read_bool(vault, KEY_DEFAULT_REVEAL, false)?,
    })
}

pub fn set_u32(vault: &Vault, key: &str, value: u32) -> Result<()> {
    let clamped = match key {
        KEY_IDLE_LOCK | KEY_CLIPBOARD_CLEAR => value.min(MAX_SECONDS),
        KEY_ANALYZE_TOP_N => value.clamp(MIN_ANALYZE_TOP_N, MAX_ANALYZE_TOP_N),
        _ => value,
    };
    let s = clamped.to_string();
    vault.conn().execute(
        "INSERT OR REPLACE INTO vault_meta (key, value) VALUES (?1, ?2)",
        params![key, s.as_bytes()],
    )?;
    Ok(())
}

pub fn set_bool(vault: &Vault, key: &str, value: bool) -> Result<()> {
    let v: &[u8] = if value { b"1" } else { b"0" };
    vault.conn().execute(
        "INSERT OR REPLACE INTO vault_meta (key, value) VALUES (?1, ?2)",
        params![key, v],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn vault() -> (TempDir, Vault) {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("v.db");
        let v = Vault::create(&path, b"hunter2").unwrap();
        (tmp, v)
    }

    #[test]
    fn defaults_when_keys_absent() {
        let (_t, v) = vault();
        let s = get(&v).unwrap();
        assert_eq!(s.idle_lock_seconds, 0);
        assert_eq!(s.clipboard_clear_seconds, 0);
        assert_eq!(s.analyze_top_n, 10);
        assert!(!s.default_reveal);
    }

    #[test]
    fn set_u32_round_trip() {
        let (_t, v) = vault();
        set_u32(&v, KEY_IDLE_LOCK, 900).unwrap();
        let s = get(&v).unwrap();
        assert_eq!(s.idle_lock_seconds, 900);
        // Other keys untouched
        assert_eq!(s.clipboard_clear_seconds, 0);
        assert_eq!(s.analyze_top_n, 10);
    }

    #[test]
    fn set_bool_round_trip() {
        let (_t, v) = vault();
        set_bool(&v, KEY_DEFAULT_REVEAL, true).unwrap();
        assert!(get(&v).unwrap().default_reveal);
        set_bool(&v, KEY_DEFAULT_REVEAL, false).unwrap();
        assert!(!get(&v).unwrap().default_reveal);
    }

    #[test]
    fn clamping_top_n() {
        let (_t, v) = vault();
        set_u32(&v, KEY_ANALYZE_TOP_N, 500).unwrap();
        assert_eq!(get(&v).unwrap().analyze_top_n, 100);
        set_u32(&v, KEY_ANALYZE_TOP_N, 0).unwrap();
        assert_eq!(get(&v).unwrap().analyze_top_n, 1);
    }

    #[test]
    fn multiple_keys_independent() {
        let (_t, v) = vault();
        set_u32(&v, KEY_ANALYZE_TOP_N, 25).unwrap();
        set_u32(&v, KEY_IDLE_LOCK, 600).unwrap();
        set_u32(&v, KEY_CLIPBOARD_CLEAR, 30).unwrap();
        set_bool(&v, KEY_DEFAULT_REVEAL, true).unwrap();
        let s = get(&v).unwrap();
        assert_eq!(s.analyze_top_n, 25);
        assert_eq!(s.idle_lock_seconds, 600);
        assert_eq!(s.clipboard_clear_seconds, 30);
        assert!(s.default_reveal);
    }
}
