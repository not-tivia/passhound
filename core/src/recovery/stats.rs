use crate::crypto::aead::{self, NONCE_LEN};
use crate::error::{Error, Result};
use crate::recovery::HistoryStats;
use crate::vault::Vault;
use std::collections::HashMap;
use zeroize::Zeroizing;

/// `s` starts or ends (case-insensitive) with one of the site abbreviations
/// (each length >= 2). Boundary-only: an abbreviation appearing in the interior
/// does not count.
pub fn has_site_affix(s: &str, abbrevs: &[String]) -> bool {
    if !s.is_ascii() {
        return false;
    }
    let low = s.to_ascii_lowercase();
    abbrevs.iter().any(|a| {
        let a = a.to_ascii_lowercase();
        a.len() >= 2 && low.len() > a.len() && (low.starts_with(&a) || low.ends_with(&a))
    })
}

/// `s`'s last character is a non-alphanumeric symbol.
pub fn ends_with_symbol(s: &str) -> bool {
    s.chars().last().map(|c| !c.is_alphanumeric()).unwrap_or(false)
}

/// `s` ends in at least one digit.
pub fn ends_with_digit(s: &str) -> bool {
    count_trailing_digits(s) > 0
}

/// `s` contains an interior digit flanked by ASCII letters (in-word leetspeak,
/// e.g. `p4ss`). A trailing or leading digit run does NOT count.
pub fn has_interior_leet(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() < 3 {
        return false;
    }
    for i in 1..b.len() - 1 {
        if b[i].is_ascii_digit() && (b[i - 1].is_ascii_alphabetic() && b[i + 1].is_ascii_alphabetic()) {
            return true;
        }
    }
    false
}

/// `s` contains at least one uppercase ASCII letter.
pub fn has_uppercase(s: &str) -> bool {
    s.bytes().any(|b| b.is_ascii_uppercase())
}

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

        // Site abbreviations (declared + brand) for the SiteAffix detector.
        let mut abbrevs: Vec<String> = Vec::new();
        {
            let mut astmt = vault.conn().prepare("SELECT name, abbreviations FROM sites")?;
            let arows = astmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in arows.flatten() {
                let (name, abbr_json) = row;
                for a in serde_json::from_str::<Vec<String>>(&abbr_json).unwrap_or_default() {
                    if a.len() >= 2 { abbrevs.push(a); }
                }
                let brand = crate::site_name::strip_url_noise(&name);
                if brand.len() >= 2 { abbrevs.push(brand); }
            }
        }

        let mut trailing_symbol_counts: HashMap<char, usize> = HashMap::new();
        let mut trailing_digit_count_counts: HashMap<u8, usize> = HashMap::new();
        let mut total_len: usize = 0;
        let mut year_counts: HashMap<u16, usize> = HashMap::new();
        let total_rows = rows.len();
        let mut applic: HashMap<crate::recovery::RuleId, usize> = HashMap::new();

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

            // Per-rule applicability detectors.
            use crate::recovery::RuleId;
            if has_site_affix(s, &abbrevs)   { *applic.entry(RuleId::SiteAffix).or_insert(0) += 1; }
            if ends_with_symbol(s)           { *applic.entry(RuleId::SpecialSuffix).or_insert(0) += 1; }
            if ends_with_digit(s)            { *applic.entry(RuleId::NumberIncrement).or_insert(0) += 1; }
            if has_interior_leet(s)          { *applic.entry(RuleId::LeetSwap).or_insert(0) += 1; }
            if has_uppercase(s)              { *applic.entry(RuleId::CaseVariations).or_insert(0) += 1; }
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
        let rule_applicability: HashMap<crate::recovery::RuleId, f32> = applic.into_iter()
            .map(|(r, n)| (r, n as f32 / total))
            .collect();
        let rule_fit = crate::recovery::score::rule_fit::compute(&rule_applicability, total_rows);

        Ok(Self {
            trailing_symbol_freq,
            trailing_digit_count_freq,
            mean_length,
            year_suffix_freq,
            rule_applicability,
            corpus_size: total_rows,
            rule_fit,
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
    let end = b.len();
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

    #[test]
    fn site_affix_detector() {
        let abbr = vec!["rg".to_string(), "rio".to_string()];
        assert!(has_site_affix("RGmoonbeam", &abbr));   // prefix
        assert!(has_site_affix("moonbeamRG", &abbr));   // suffix
        assert!(has_site_affix("RIOpassword", &abbr));  // case-insensitive
        assert!(!has_site_affix("moonbeam", &abbr));    // no affix
        assert!(!has_site_affix("regretful", &abbr));   // "rg" not a boundary affix (interior)
    }

    #[test]
    fn structural_detectors() {
        assert!(ends_with_symbol("moonbeam!"));
        assert!(!ends_with_symbol("moonbeam1"));
        assert!(ends_with_digit("moonbeam2019"));
        assert!(!ends_with_digit("moonbeam!"));
        assert!(has_interior_leet("p4ssword"));     // digit between letters
        assert!(!has_interior_leet("password1"));   // trailing digit is NOT interior leet
        assert!(!has_interior_leet("2019moon"));    // leading digit, letter only on one side at idx0 -> not interior
        assert!(has_uppercase("MoonBeam"));
        assert!(!has_uppercase("moonbeam"));
    }

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

    #[test]
    fn compute_measures_rule_applicability() {
        use crate::repo::accounts::{self, NewAccount};
        use crate::repo::sites::{self, NewSite};
        use crate::repo::passwords::{self, Confidence, NewPassword};
        use crate::vault::Vault;
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        let v = Vault::create(&tmp.path().join("v.db"), b"x").unwrap();
        let s = sites::create(&v, NewSite { name: "RuneScape".into(), abbreviations: vec!["RS".into()], ..Default::default() }).unwrap();
        for pw in ["moonbeam2019", "thunder2020", "fluffy!", "plainword"] {
            let a = accounts::create(&v, NewAccount { site_id: s.id, ..Default::default() }).unwrap();
            passwords::insert(&v, NewPassword { account_id: a.id, plaintext: pw, source: "m".into(), confidence: Confidence::Certain, notes: None, created_at: None }).unwrap();
        }
        let st = HistoryStats::compute(&v).unwrap();
        assert_eq!(st.corpus_size, 4);
        // 2 of 4 end in digits, 1 of 4 ends in a symbol, 0 have a site affix.
        assert!((st.rule_applicability.get(&crate::recovery::RuleId::NumberIncrement).copied().unwrap_or(0.0) - 0.5).abs() < 1e-6);
        assert!((st.rule_applicability.get(&crate::recovery::RuleId::SpecialSuffix).copied().unwrap_or(0.0) - 0.25).abs() < 1e-6);
        assert_eq!(st.rule_applicability.get(&crate::recovery::RuleId::SiteAffix).copied().unwrap_or(0.0), 0.0);
    }
}
