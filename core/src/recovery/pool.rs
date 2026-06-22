use crate::crypto::aead::{self, NONCE_LEN};
use crate::error::{Error, Result};
use crate::recovery::{Pool, PoolSeed, RecoverConfig};
use crate::repo::{base_words, eras};
use crate::vault::Vault;
use chrono::{DateTime, Utc};
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
    // Canonicalize so URL-shaped stored names ("https://www.github.com") match a
    // user-typed bare target ("github").
    let target_site_canonical = cfg.site.as_ref().map(|s| canonical_site_name(s));
    let target_account_lower = cfg.account.as_ref().map(|s| s.to_lowercase());

    // First pass: identify the primary matched-site category, if any. Scan the
    // sites table and compare canonical names in Rust — SQL's LOWER() alone
    // can't strip URL noise, and a name-matched site with no password rows
    // still needs to contribute its category to the same-category fallback.
    let primary_category: Option<String> = if let Some(target_canonical) = &target_site_canonical {
        let mut stmt = vault.conn().prepare(
            "SELECT name, category FROM sites",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?;
        let mut found: Option<String> = None;
        for row in rows {
            let (name, category) = row?;
            if canonical_site_name(&name) == *target_canonical {
                found = category;
                break;
            }
        }
        found
    } else {
        None
    };
    let mut seeds: Vec<PoolSeed> = Vec::new();
    let mut site_abbrev_set: Vec<String> = Vec::new();

    // P3: pull abbreviations from any sites row whose canonical name matches
    // cfg.site, independent of whether any password rows from that site survive
    // the era + decrypt pipeline. This catches the case where the answer site
    // exists in `sites` but its passwords are excluded (era-filtered or
    // otherwise) — its declared abbreviations are the exact ones the user uses
    // for that site.
    if let Some(target_canonical) = &target_site_canonical {
        let mut stmt = vault.conn().prepare(
            "SELECT name, abbreviations FROM sites",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for row in rows.flatten() {
            let (name, abbr_json) = row;
            if canonical_site_name(&name) != *target_canonical { continue; }
            let abbreviations: Vec<String> = serde_json::from_str(&abbr_json).unwrap_or_default();
            for a in abbreviations {
                if !site_abbrev_set.iter().any(|x| x.eq_ignore_ascii_case(&a)) {
                    site_abbrev_set.push(a);
                }
            }
        }
    }

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
        let site_match_strength: f32 = match &target_site_canonical {
            None => 0.5,
            Some(target_canonical) => {
                let name_match = canonical_site_name(&site_name) == *target_canonical;
                let abbrev_match = abbreviations.iter().any(|a| a.to_lowercase() == *target_canonical);
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

    // Pull base words. Each entry exposes both lowercase canonical (for hint
    // matching, dedup, etc.) and the reconstructed original casing (for
    // BaseWordPool / WordCombine to emit as a privileged seed variant).
    let bw = base_words::fetch_decrypted(vault)?;
    let mut favorite_base_words: Vec<crate::recovery::DecryptedBaseWordEntry> = Vec::new();
    let mut all_base_words: Vec<crate::recovery::DecryptedBaseWordEntry> = Vec::new();
    for w in bw {
        let canonical = w.word.as_str().to_owned();
        let original = base_words::apply_casing_mask(&canonical, w.casing_mask);
        let entry = crate::recovery::DecryptedBaseWordEntry {
            canonical: Zeroizing::new(canonical),
            original: Zeroizing::new(original),
        };
        if w.is_favorite {
            favorite_base_words.push(entry.clone());
        }
        all_base_words.push(entry);
    }

    Ok(Pool {
        seeds,
        favorite_base_words,
        all_base_words,
        site_abbreviations: site_abbrev_set,
        era_window,
    })
}

pub use crate::site_name::{canonical_site_name, strip_url_noise};

/// Derive plausible abbreviations from a site name. Conservative — never panics.
/// URL noise is stripped first so e.g. "https://www.github.com" produces
/// ["G", "Git"] rather than ["Htt"]. Case is preserved through the strip so
/// camel-case boundary acronym detection still works on names like "GitHub".
/// "RuneScape"               -> ["RS", "Run"]
/// "Amazon"                  -> ["AM", "Ama"]
/// "GitHub"                  -> ["GH", "Git"]
/// "google"                  -> ["G", "goo"]
/// "https://www.github.com"  -> ["G", "Git"]
/// "https://www.GitHub.com"  -> ["GH", "Git"]
pub fn derive_abbreviations(name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let stripped = strip_url_noise(name);
    let trimmed = stripped.trim();
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

    // Phase 4.22 ---------------------------------------------------------------

    #[test]
    fn strip_url_noise_full_url() {
        assert_eq!(strip_url_noise("https://www.github.com"), "github");
        assert_eq!(strip_url_noise("http://github.com"), "github");
        assert_eq!(strip_url_noise("https://www.github.com/login?next=/"), "github");
        assert_eq!(strip_url_noise("github.com:8080"), "github");
    }

    #[test]
    fn strip_url_noise_preserves_case() {
        assert_eq!(strip_url_noise("https://www.GitHub.com"), "GitHub");
        assert_eq!(strip_url_noise("  GitHub  "), "GitHub");
    }

    #[test]
    fn strip_url_noise_idempotent_on_bare_names() {
        assert_eq!(strip_url_noise("RuneScape"), "RuneScape");
        assert_eq!(strip_url_noise("Amazon"), "Amazon");
    }

    #[test]
    fn canonical_site_name_lowercases() {
        assert_eq!(canonical_site_name("https://www.GitHub.com"), "github");
        assert_eq!(canonical_site_name("GitHub"), "github");
        assert_eq!(canonical_site_name("Tumblr"), "tumblr");
    }

    #[test]
    fn derive_abbreviations_full_url_no_protocol_leak() {
        // The user-reported bug: "https://www.github.com" produced ["Htt"]
        // because the first 3 alnum chars of the raw URL were "htt". After
        // URL noise stripping it should produce GitHub-flavored abbreviations.
        let abbrs = derive_abbreviations("https://www.github.com");
        assert!(!abbrs.iter().any(|a| a.eq_ignore_ascii_case("Htt")),
            "abbreviations must not leak the URL protocol prefix; got {:?}", abbrs);
        assert!(abbrs.iter().any(|a| a.eq_ignore_ascii_case("Git")),
            "expected Git-prefix abbreviation from github.com; got {:?}", abbrs);
    }

    #[test]
    fn derive_abbreviations_url_preserves_camel_acronym() {
        // "GitHub.com" (or with URL prefix) should still produce "GH" via the
        // camel-case boundary path, because strip_url_noise preserves case.
        let abbrs = derive_abbreviations("https://www.GitHub.com");
        assert!(abbrs.iter().any(|a| a == "GH"),
            "expected GH camel-acronym from GitHub URL; got {:?}", abbrs);
    }

    #[test]
    fn strip_url_noise_android_package_brand() {
        // Google Password Manager Android entries — extract the brand segment
        // from the reverse-dns package name, discarding the cert hash entirely.
        assert_eq!(
            strip_url_noise("android://abc123==@com.tumblr/"),
            "tumblr",
        );
        assert_eq!(
            strip_url_noise("android://xyz==@com.jagex.oldscape.android/"),
            "jagex",
        );
        assert_eq!(
            strip_url_noise("android://hash==@com.snapchat.android/"),
            "snapchat",
        );
        assert_eq!(
            strip_url_noise("android://hash==@org.mozilla.firefox/"),
            "mozilla",
        );
    }

    #[test]
    fn derive_abbreviations_android_url() {
        // The user-reported bug v2: android:// URLs produced abbreviations
        // derived from "and" (first 3 chars of "android://...") — same class
        // of leak as Htt from https://. After strip_url_noise these should
        // produce brand-name abbreviations.
        let abbrs = derive_abbreviations("android://abc==@com.tumblr/");
        assert!(!abbrs.iter().any(|a| a.eq_ignore_ascii_case("And")),
            "abbreviations must not leak the android:// prefix; got {:?}", abbrs);
        assert!(abbrs.iter().any(|a| a.eq_ignore_ascii_case("Tum")),
            "expected Tum-prefix abbreviation from com.tumblr; got {:?}", abbrs);
    }

    #[test]
    fn canonical_site_name_android_matches_brand_query() {
        // A user-typed query "tumblr" should match a stored
        // "android://hash==@com.tumblr/" via canonical_site_name equality.
        assert_eq!(
            canonical_site_name("android://abc==@com.tumblr/"),
            canonical_site_name("Tumblr"),
        );
        assert_eq!(
            canonical_site_name("android://abc==@com.snapchat.android/"),
            canonical_site_name("Snapchat"),
        );
    }

    // Phase 4.25 — canonicalizer redesign

    #[test]
    fn brand_is_rightmost_after_tld() {
        assert_eq!(strip_url_noise("auth.riotgames.com"), "riotgames");
        assert_eq!(strip_url_noise("na.account.amazon.com"), "amazon");
        assert_eq!(strip_url_noise("oldschool.runescape.com"), "runescape");
        assert_eq!(strip_url_noise("login.aol.com"), "aol");
    }

    #[test]
    fn no_tld_keeps_brand() {
        // No recognized TLD -> the rightmost segment IS the brand; don't eat it.
        assert_eq!(strip_url_noise("us.battle"), "battle");
        assert_eq!(strip_url_noise("eu.battle"), "battle");
        assert_eq!(strip_url_noise("account.battleon"), "battleon");
    }

    #[test]
    fn compound_tld_dropped() {
        assert_eq!(strip_url_noise("amazon.co.uk"), "amazon");
        assert_eq!(strip_url_noise("www.shop.example.com.au"), "example");
    }

    #[test]
    fn canonical_unifies_spacing_and_subdomains() {
        // The straggler the merge missed: spaced name vs subdomained URL.
        assert_eq!(canonical_site_name("Riot games"), "riotgames");
        assert_eq!(canonical_site_name("auth.riotgames.com"), "riotgames");
        assert_eq!(canonical_site_name("Riot games"), canonical_site_name("auth.riotgames.com"));
        // Battle family collapses too.
        assert_eq!(canonical_site_name("us.battle"), canonical_site_name("eu.battle"));
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

    #[test]
    fn build_collects_abbrevs_from_name_matched_site_even_when_no_seeds_survive() {
        use crate::repo::accounts::{self, NewAccount};
        use crate::repo::eras;
        use crate::repo::sites::{self, NewSite};
        use chrono::{NaiveDate, TimeZone, Utc};
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("v.db");
        let v = Vault::create(&path, b"hunter2").unwrap();

        // Reddit site row exists with abbreviation "Rd". One password is in 2010
        // (way outside the College era window we're going to filter on), and we
        // do NOT add a row inside the College window — so when era filter fires,
        // no Reddit seed survives. We expect the abbreviation "Rd" to still
        // appear in pool.site_abbreviations because pool::build does a
        // pre-query that pulls abbreviations from name-matched sites
        // independent of seed survival.
        let reddit = sites::create(&v, NewSite {
            name: "Reddit".into(),
            url: Some("reddit.com".into()),
            category: Some("Social".into()),
            abbreviations: vec!["Rd".into()],
            notes: None,
        }).unwrap();
        let acct = accounts::create(&v, NewAccount { site_id: reddit.id, ..Default::default() }).unwrap();
        crate::repo::passwords::insert(&v, crate::repo::passwords::NewPassword {
            account_id: acct.id,
            plaintext: "out-of-era-pw",
            source: "manual".into(),
            confidence: crate::repo::passwords::Confidence::Certain,
            notes: None,
            created_at: Some(Utc.with_ymd_and_hms(2010, 1, 1, 0, 0, 0).unwrap()),
        }).unwrap();

        eras::add(&v, "College",
                  Some(NaiveDate::from_ymd_opt(2016, 1, 1).unwrap()),
                  Some(NaiveDate::from_ymd_opt(2019, 12, 31).unwrap()),
                  None).unwrap();

        let cfg = RecoverConfig {
            site: Some("Reddit".into()),
            era_name: Some("College".into()),
            ..Default::default()
        };
        let pool = build(&v, &cfg).unwrap();

        // No Reddit seeds should survive the era filter:
        assert!(pool.seeds.iter().all(|s| s.site_id != Some(reddit.id)),
            "no Reddit seeds should survive era filter; got {:?}",
            pool.seeds.iter().map(|s| s.site_id).collect::<Vec<_>>());
        // But the abbreviation "Rd" should appear in site_abbreviations from the pre-query:
        assert!(pool.site_abbreviations.iter().any(|a| a == "Rd"),
            "expected 'Rd' in site_abbreviations; got {:?}", pool.site_abbreviations);
    }
}
