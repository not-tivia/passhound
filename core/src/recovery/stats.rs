use crate::crypto::aead::{self, NONCE_LEN};
use crate::error::{Error, Result};
use crate::recovery::HistoryStats;
use crate::vault::Vault;
use std::collections::HashMap;
use zeroize::Zeroizing;

impl HistoryStats {
    /// Compute pattern statistics from EVERY password_history row (current + retired).
    /// Vault must be unlocked.
    pub fn compute(vault: &Vault) -> Result<Self> {
        let key = vault.require_key()?;
        let mut stmt = vault.conn().prepare(
            "SELECT password_encrypted, password_nonce FROM password_history",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?))
        })?.collect::<std::result::Result<Vec<_>, _>>()?;

        if rows.is_empty() {
            return Ok(Self::default());
        }

        let mut trailing_symbol_counts: HashMap<char, usize> = HashMap::new();
        let mut trailing_digit_count_counts: HashMap<u8, usize> = HashMap::new();
        let mut total_len: usize = 0;
        let mut year_counts: HashMap<u16, usize> = HashMap::new();
        let total_rows = rows.len();

        for (ct, nonce_vec) in rows {
            if nonce_vec.len() != NONCE_LEN {
                return Err(Error::InvalidInput("malformed stats nonce".into()));
            }
            let mut nonce = [0u8; NONCE_LEN];
            nonce.copy_from_slice(&nonce_vec);
            let pt = Zeroizing::new(aead::decrypt(key.as_bytes(), &ct, &nonce)?);
            let s = match std::str::from_utf8(&pt) {
                Ok(s) => s,
                Err(_) => continue, // skip non-utf8 (shouldn't happen, defensive)
            };
            total_len += s.chars().count();

            // Trailing non-alphanumeric symbol.
            if let Some(last) = s.chars().last() {
                if !last.is_alphanumeric() {
                    *trailing_symbol_counts.entry(last).or_insert(0) += 1;
                }
            }

            // Trailing digit count.
            let td = count_trailing_digits(s);
            *trailing_digit_count_counts.entry(td).or_insert(0) += 1;

            // Trailing year (4 consecutive digits at end).
            if let Some(year) = trailing_year(s) {
                *year_counts.entry(year).or_insert(0) += 1;
            }
        }

        let total = total_rows as f32;
        let trailing_symbol_freq: HashMap<char, f32> = trailing_symbol_counts.into_iter()
            .map(|(c, n)| (c, n as f32 / total))
            .collect();
        let trailing_digit_count_freq: HashMap<u8, f32> = trailing_digit_count_counts.into_iter()
            .map(|(c, n)| (c, n as f32 / total))
            .collect();
        let year_suffix_freq: HashMap<u16, f32> = year_counts.into_iter()
            .map(|(y, n)| (y, n as f32 / total))
            .collect();
        let mean_length = total_len as f32 / total;

        Ok(Self {
            trailing_symbol_freq,
            trailing_digit_count_freq,
            mean_length,
            year_suffix_freq,
        })
    }
}

pub fn count_trailing_digits(s: &str) -> u8 {
    let mut n: u8 = 0;
    for &b in s.as_bytes().iter().rev() {
        if b.is_ascii_digit() {
            n = n.saturating_add(1);
        } else {
            break;
        }
    }
    n
}

pub fn trailing_year(s: &str) -> Option<u16> {
    // Zero-allocation: work on bytes (years are ASCII digits).
    let b = s.as_bytes();
    let mut end = b.len();
    // Find start of trailing digit run.
    let mut start = end;
    while start > 0 && b[start - 1].is_ascii_digit() {
        start -= 1;
    }
    let digit_len = end - start;
    if digit_len < 4 { return None; }
    // Take last 4 digits of the run.
    let d = &b[end - 4..end];
    let year: u16 = (d[0] - b'0') as u16 * 1000
        + (d[1] - b'0') as u16 * 100
        + (d[2] - b'0') as u16 * 10
        + (d[3] - b'0') as u16;
    if (1990..=2099).contains(&year) { Some(year) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::accounts::{self, NewAccount};
    use crate::repo::sites::{self, NewSite};
    use tempfile::TempDir;

    fn make_vault_with_passwords(passwords: &[&str]) -> (TempDir, Vault) {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("v.db");
        let v = Vault::create(&path, b"hunter2").unwrap();
        let s = sites::create(&v, NewSite { name: "S".into(), ..Default::default() }).unwrap();
        let a = accounts::create(&v, NewAccount { site_id: s.id, ..Default::default() }).unwrap();
        for pw in passwords {
            crate::repo::passwords::insert(&v, crate::repo::passwords::NewPassword {
                account_id: a.id, plaintext: pw, source: "m".into(),
                confidence: crate::repo::passwords::Confidence::Certain, notes: None, created_at: None,
            }).unwrap();
        }
        (tmp, v)
    }

    #[test]
    fn count_trailing_digits_basics() {
        assert_eq!(count_trailing_digits("abc"), 0);
        assert_eq!(count_trailing_digits("abc1"), 1);
        assert_eq!(count_trailing_digits("abc1234"), 4);
        assert_eq!(count_trailing_digits("1abc"), 0);
    }

    #[test]
    fn trailing_year_recognizes_plausible_years() {
        assert_eq!(trailing_year("Fluffy!2014"), Some(2014));
        assert_eq!(trailing_year("p2099"), Some(2099));
        assert_eq!(trailing_year("p1989"), None, "outside [1990, 2099]");
        assert_eq!(trailing_year("p999"), None, "fewer than 4 trailing digits");
    }

    #[test]
    fn compute_empty_history_returns_default() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("v.db");
        let v = Vault::create(&path, b"hunter2").unwrap();
        let stats = HistoryStats::compute(&v).unwrap();
        assert!(stats.trailing_symbol_freq.is_empty());
        assert_eq!(stats.mean_length, 0.0);
    }

    #[test]
    fn compute_summarizes_basic_pool() {
        let (_t, v) = make_vault_with_passwords(&[
            "Fluffy!2014", "MoonBeam$2018", "thunder!2020",
        ]);
        let stats = HistoryStats::compute(&v).unwrap();
        assert!(stats.trailing_symbol_freq.is_empty(), "all end in digits");
        let mean_expected = (11 + 13 + 12) as f32 / 3.0;
        assert!((stats.mean_length - mean_expected).abs() < 0.01);
        assert!(stats.year_suffix_freq.contains_key(&2014));
        assert!(stats.year_suffix_freq.contains_key(&2018));
        assert!(stats.year_suffix_freq.contains_key(&2020));
    }

    #[test]
    fn compute_records_trailing_symbols() {
        let (_t, v) = make_vault_with_passwords(&["abcd!", "efgh!", "ijkl?"]);
        let stats = HistoryStats::compute(&v).unwrap();
        assert!((stats.trailing_symbol_freq.get(&'!').copied().unwrap_or(0.0) - 2.0/3.0).abs() < 0.01);
        assert!((stats.trailing_symbol_freq.get(&'?').copied().unwrap_or(0.0) - 1.0/3.0).abs() < 0.01);
    }
}
