use crate::crypto::aead::{self, NONCE_LEN};
use crate::error::{Error, Result};
use crate::recovery::{Pool, PoolSeed, RecoverConfig};
use crate::repo::{base_words, eras};
use crate::vault::Vault;
use chrono::{DateTime, Utc};
use rusqlite::params;
use serde_json;
use zeroize::Zeroizing;

pub fn build(vault: &Vault, cfg: &RecoverConfig) -> Result<Pool> {
    // Resolve era window first, since site filter and era filter compose.
    let era_window = match &cfg.era_name {
        Some(name) => {
            let era = eras::find_by_name(vault, name)?
                .ok_or_else(|| Error::EraNotFound(name.clone()))?;
            match (era.start_date, era.end_date) {
                (Some(s), Some(e)) => Some((s, e)),
                _ => None,
            }
        }
        None => None,
    };

    // Decrypt + classify history rows.
    let key = vault.require_key()?;
    let mut stmt = vault.conn().prepare(
        "SELECT ph.id, ph.password_encrypted, ph.password_nonce, ph.created_at,
                a.id, a.username,
                s.id, s.name, s.category, s.abbreviations
         FROM password_history ph
         JOIN accounts a ON a.id = ph.account_id
         JOIN sites s ON s.id = a.site_id
         WHERE ph.retired_at IS NULL",
    )?;

    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,                  // history_id
            r.get::<_, Vec<u8>>(1)?,              // ct
            r.get::<_, Vec<u8>>(2)?,              // nonce
            r.get::<_, String>(3)?,               // created_at rfc3339
            r.get::<_, i64>(4)?,                  // account_id
            r.get::<_, Option<String>>(5)?,       // username
            r.get::<_, i64>(6)?,                  // site_id
            r.get::<_, String>(7)?,               // site_name
            r.get::<_, Option<String>>(8)?,       // category
            r.get::<_, String>(9)?,               // abbreviations json
        ))
    })?.collect::<std::result::Result<Vec<_>, _>>()?;

    // Determine the matched site set (for category fallback) when --site is given.
    let target_site_lower = cfg.site.as_ref().map(|s| s.to_lowercase());
    let target_account_lower = cfg.account.as_ref().map(|s| s.to_lowercase());

    // First pass: identify the primary matched-site category, if any.
    let primary_category: Option<String> = if let Some(target) = &target_site_lower {
        rows.iter()
            .find(|(_, _, _, _, _, _, _, name, _, _)| name.to_lowercase() == *target)
            .and_then(|(_, _, _, _, _, _, _, _, cat, _)| cat.clone())
    } else {
        None
    };

    let mut seeds: Vec<PoolSeed> = Vec::new();
    let mut site_abbrev_set: Vec<String> = Vec::new();

    for (history_id, ct, nonce_vec, created_at_str, _account_id, username, site_id, site_name, category, abbr_json) in rows {
        // Account filter (substring on username).
        if let Some(needle) = &target_account_lower {
            let hay = username.as_deref().unwrap_or("").to_lowercase();
            if !hay.contains(needle) { continue; }
        }

        // Era filter on created_at.
        let created_at = DateTime::parse_from_rfc3339(&created_at_str)
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        if let Some((start, end)) = era_window {
            let day = created_at.date_naive();
            if day < start || day > end { continue; }
        }

        // Site classification.
        let abbreviations: Vec<String> = serde_json::from_str(&abbr_json).unwrap_or_default();
        let site_match_strength: f32 = match &target_site_lower {
            None => 0.5,
            Some(target) => {
                let name_match = site_name.to_lowercase() == *target;
                let abbrev_match = abbreviations.iter().any(|a| a.to_lowercase() == *target);
                if name_match || abbrev_match {
                    1.0
                } else if let (Some(c), Some(p)) = (category.as_deref(), primary_category.as_deref()) {
                    if c.eq_ignore_ascii_case(p) { 0.5 } else { continue; }
                } else {
                    continue;
                }
            }
        };

        // Decrypt.
        if nonce_vec.len() != NONCE_LEN {
            return Err(Error::InvalidInput("malformed pool nonce".into()));
        }
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&nonce_vec);
        let pt = Zeroizing::new(aead::decrypt(key.as_bytes(), &ct, &nonce)?);
        let s = std::str::from_utf8(&pt).map_err(|_| Error::InvalidInput("non-utf8 pool plaintext".into()))?;

        // Collect abbreviations for matched-site rows only (so we don't pollute with
        // unrelated category neighbors' abbreviations).
        if (site_match_strength - 1.0).abs() < f32::EPSILON {
            for a in abbreviations {
                if !site_abbrev_set.iter().any(|x| x.eq_ignore_ascii_case(&a)) {
                    site_abbrev_set.push(a);
                }
            }
        }

        seeds.push(PoolSeed {
            history_id,
            plaintext: Zeroizing::new(s.to_owned()),
            created_at,
            site_id: Some(site_id),
            site_match_strength,
        });
    }

    // If --site was passed but produced no abbreviations from matched sites, derive a
    // fallback from the site name itself (first 2-3 chars uppercased + first-letters-of-words).
    if let Some(s) = &cfg.site {
        if site_abbrev_set.is_empty() {
            site_abbrev_set.extend(derive_abbreviations(s));
        }
    }

    // Pull base words.
    let bw = base_words::fetch_decrypted(vault)?;
    let mut favorite_base_words: Vec<Zeroizing<String>> = Vec::new();
    let mut all_base_words: Vec<Zeroizing<String>> = Vec::new();
    for w in bw {
        if w.is_favorite {
            favorite_base_words.push(Zeroizing::new(w.word.as_str().to_owned()));
        }
        all_base_words.push(Zeroizing::new(w.word.as_str().to_owned()));
    }

    Ok(Pool {
        seeds,
        favorite_base_words,
        all_base_words,
        site_abbreviations: site_abbrev_set,
        era_window,
    })
}

/// Derive plausible abbreviations from a site name. Conservative — never panics.
/// "RuneScape"   -> ["RS", "Run"]
/// "Amazon"      -> ["AM", "Ama"]
/// "GitHub"      -> ["GH", "Git"]
/// "google"      -> ["GO", "goo"]
pub fn derive_abbreviations(name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let trimmed = name.trim();
    if trimmed.is_empty() { return out; }

    // First letters of each "word" segment (split on whitespace, dashes, underscores,
    // OR transition from lowercase->uppercase).
    let mut acronym = String::new();
    let chars: Vec<char> = trimmed.chars().collect();
    let mut at_word_start = true;
    for (i, c) in chars.iter().enumerate() {
        let separator = !c.is_alphanumeric();
        if separator { at_word_start = true; continue; }
        let camel_boundary = i > 0 && c.is_ascii_uppercase()
            && chars[i - 1].is_ascii_lowercase();
        if at_word_start || camel_boundary {
            acronym.push(c.to_ascii_uppercase());
        }
        at_word_start = false;
    }
    if !acronym.is_empty() && acronym.chars().count() <= 3 {
        out.push(acronym);
    }

    // First 3 chars (alphanumeric only, lowercase preserved).
    let prefix: String = trimmed.chars().filter(|c| c.is_alphanumeric()).take(3).collect();
    if !prefix.is_empty() {
        let cap = capitalize(&prefix);
        if !out.iter().any(|x| x.eq_ignore_ascii_case(&cap)) { out.push(cap); }
    }

    out
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_abbreviations_runescape() {
        let abbrs = derive_abbreviations("RuneScape");
        assert!(abbrs.iter().any(|a| a == "RS"));
    }

    #[test]
    fn derive_abbreviations_amazon() {
        let abbrs = derive_abbreviations("Amazon");
        assert!(abbrs.iter().any(|a| a == "A"));
        assert!(abbrs.iter().any(|a| a.eq_ignore_ascii_case("Ama")));
    }

    #[test]
    fn derive_abbreviations_two_words() {
        let abbrs = derive_abbreviations("Cloud Bank");
        assert!(abbrs.iter().any(|a| a == "CB"));
    }

    #[test]
    fn derive_abbreviations_empty_input() {
        assert!(derive_abbreviations("   ").is_empty());
    }

    #[test]
    fn build_with_no_filters_returns_all_seeds() {
        use crate::repo::accounts::{self, NewAccount};
        use crate::repo::sites::{self, NewSite};
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("v.db");
        let v = Vault::create(&path, b"hunter2").unwrap();
        let s = sites::create(&v, NewSite { name: "S".into(), ..Default::default() }).unwrap();
        let a = accounts::create(&v, NewAccount { site_id: s.id, ..Default::default() }).unwrap();
        crate::repo::passwords::insert(&v, crate::repo::passwords::NewPassword {
            account_id: a.id,
            plaintext: "p1",
            source: "manual".into(),
            confidence: crate::repo::passwords::Confidence::Certain,
            notes: None,
            created_at: None,
        }).unwrap();
        let pool = build(&v, &RecoverConfig::default()).unwrap();
        assert_eq!(pool.seeds.len(), 1);
        assert!((pool.seeds[0].site_match_strength - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn build_excludes_unmatched_sites_when_site_filter_present() {
        use crate::repo::accounts::{self, NewAccount};
        use crate::repo::sites::{self, NewSite};
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("v.db");
        let v = Vault::create(&path, b"hunter2").unwrap();
        let s1 = sites::create(&v, NewSite { name: "Match".into(), category: Some("X".into()), ..Default::default() }).unwrap();
        let s2 = sites::create(&v, NewSite { name: "Other".into(), category: Some("Y".into()), ..Default::default() }).unwrap();
        for sid in [s1.id, s2.id] {
            let a = accounts::create(&v, NewAccount { site_id: sid, ..Default::default() }).unwrap();
            crate::repo::passwords::insert(&v, crate::repo::passwords::NewPassword {
                account_id: a.id, plaintext: "p", source: "m".into(),
                confidence: crate::repo::passwords::Confidence::Certain, notes: None, created_at: None,
            }).unwrap();
        }
        let cfg = RecoverConfig { site: Some("Match".into()), ..Default::default() };
        let pool = build(&v, &cfg).unwrap();
        assert_eq!(pool.seeds.len(), 1);
        assert!((pool.seeds[0].site_match_strength - 1.0).abs() < f32::EPSILON);
    }
}
