//! Linear weighted-sum scorer.

use crate::recovery::clean_pattern;
use crate::recovery::score::{
    W_CLEAN_PATTERN, W_FAV_BASE, W_FREQ, W_HINT, W_HISTORY_SEED, W_LEN, W_ORIG_CASING, W_SITE,
};
use crate::recovery::stats::{count_trailing_digits, trailing_year};
use crate::recovery::{Candidate, RecoverContext, RuleId};
use std::collections::HashMap;

pub fn score(
    c: &Candidate,
    ctx: &RecoverContext<'_>,
    multipliers: Option<&HashMap<RuleId, f32>>,
) -> f32 {
    score_with_breakdown(c, ctx, multipliers).0
}

pub fn score_with_breakdown(
    c: &Candidate,
    ctx: &RecoverContext<'_>,
    multipliers: Option<&HashMap<RuleId, f32>>,
) -> (f32, crate::recovery::score::ScoreBreakdown) {
    // Site signal: prefer the seed-derived site_match_strength when the candidate
    // descends from a real history seed. For generated candidates (BaseWordPool /
    // WordCombine, no seed_history_id) fall back to a post-hoc check: when the
    // user passed --site, reward candidates whose password contains any
    // configured site abbreviation (case-insensitive). Without this, synthesis
    // chains like "MoonBeam$2019Rd" get a neutral 0.5 site score and lose to
    // shorter hint-matched chains like "thunder-moon!!" which the W_SITE
    // weight should otherwise discriminate against.
    let site_seed = c.seed_history_id
        .and_then(|id| ctx.pool.seeds.iter().find(|s| s.history_id == id))
        .map(|s| s.site_match_strength);
    let site = match site_seed {
        Some(s) => s,
        None => {
            if ctx.config.site.is_some() && contains_site_abbrev(c, ctx) {
                1.0
            } else {
                0.5
            }
        }
    };
    let hint = match &ctx.config.hint {
        Some(h) => {
            if ascii_contains_ignore_case(c.password.as_str(), h.as_str()) { 1.0 } else { 0.0 }
        }
        None => 0.5,
    };
    let freq = pattern_freq_match(c, ctx);
    let fav  = if contains_any_favorite(c, ctx) { 1.0 } else { 0.0 };
    let len  = length_match(c, ctx);
    let orig = if c.provenance.contains(&RuleId::OriginalCasing) { 1.0 } else { 0.0 };
    let history_seed = if c.provenance.contains(&RuleId::HistorySeed) { 1.0 } else { 0.0 };

    // Clean-pattern bonus (Phase 3.8 + 3.9): additive bonus scaled by the
    // count of DISTINCT segment types in the decomposition. Returns 0..=4:
    //   0 -> dirty (no bonus)
    //   1..=4 -> clean with that many distinct types
    // Applied inside ranking::score (not as a ScoreModifier) so the bonus
    // influences intermediate cap truncation — the Phase 3.7 trace showed
    // `MoonBeam$2019Rd` was dropped during pass 1 SpecialSuffix's
    // hint-partition truncation, which sorts by ranking::score. Score
    // modifiers run after the pipeline, too late. Phase 3.9's distinct-type
    // scaling rewards the user-pattern shape `<F><S><D><A>` (4 types) over
    // `<F><F><D><A>` (3 types: no symbol separator) which was crowding
    // the target out via len_match.
    let diversity = clean_pattern::is_clean_pattern(c.password.as_str(), ctx);
    let clean_pattern = if diversity > 0 { diversity as f32 / 4.0 } else { 0.0 };

    let site_weighted         = W_SITE          * site;
    let hint_weighted         = W_HINT          * hint;
    let freq_weighted         = W_FREQ          * freq;
    let fav_weighted          = W_FAV_BASE      * fav;
    let len_weighted          = W_LEN           * len;
    let orig_casing_weighted  = W_ORIG_CASING   * orig;
    let clean_pattern_weighted = W_CLEAN_PATTERN * clean_pattern;
    let history_seed_weighted = W_HISTORY_SEED  * history_seed;

    let mut total = site_weighted + hint_weighted + freq_weighted + fav_weighted
        + len_weighted + orig_casing_weighted + clean_pattern_weighted
        + history_seed_weighted;

    let multiplier = match multipliers {
        None => 1.0,
        Some(_) if c.provenance.is_empty() => 1.0,
        Some(m) => {
            let sum: f32 = c.provenance.iter()
                .map(|r| m.get(r).copied().unwrap_or(1.0))
                .sum();
            sum / c.provenance.len() as f32
        }
    };
    total *= multiplier;

    let breakdown = crate::recovery::score::ScoreBreakdown {
        site, site_weighted,
        hint, hint_weighted,
        freq, freq_weighted,
        fav, fav_weighted,
        len, len_weighted,
        orig_casing: orig, orig_casing_weighted,
        clean_pattern, clean_pattern_weighted,
        history_seed, history_seed_weighted,
        multiplier,
        total,
    };
    (total, breakdown)
}

fn pattern_freq_match(c: &Candidate, ctx: &RecoverContext<'_>) -> f32 {
    let stats = &ctx.stats;
    let mut s = 0.0;
    if let Some(last) = c.password.chars().last() {
        s += stats.trailing_symbol_freq.get(&last).copied().unwrap_or(0.0);
    }
    let trailing_digits = count_trailing_digits(c.password.as_str());
    s += stats.trailing_digit_count_freq.get(&trailing_digits).copied().unwrap_or(0.0);
    if let Some(y) = trailing_year(c.password.as_str()) {
        s += stats.year_suffix_freq.get(&y).copied().unwrap_or(0.0);
    }
    s.min(1.0)
}

fn contains_any_favorite(c: &Candidate, ctx: &RecoverContext<'_>) -> bool {
    ctx.pool.favorite_base_words.iter().any(|w| {
        ascii_contains_ignore_case(c.password.as_str(), w.canonical.as_str())
    })
}

fn contains_site_abbrev(c: &Candidate, ctx: &RecoverContext<'_>) -> bool {
    ctx.pool.site_abbreviations.iter().any(|a| {
        !a.is_empty() && ascii_contains_ignore_case(c.password.as_str(), a.as_str())
    })
}

/// Case-insensitive substring search for ASCII-dominated strings.
/// Falls back to allocating `to_lowercase()` when either input is non-ASCII.
fn ascii_contains_ignore_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if haystack.is_ascii() && needle.is_ascii() {
        let h = haystack.as_bytes();
        let n = needle.as_bytes();
        if n.len() > h.len() {
            return false;
        }
        h.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n))
    } else {
        haystack.to_lowercase().contains(&needle.to_lowercase())
    }
}

fn length_match(c: &Candidate, ctx: &RecoverContext<'_>) -> f32 {
    if ctx.stats.mean_length == 0.0 { return 0.5; }
    let dist = (c.password.len() as f32 - ctx.stats.mean_length).abs();
    (1.0 - (dist / 10.0)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recovery::{DecryptedBaseWordEntry, HistoryStats, Pool, PoolSeed, RecoverConfig, RuleId};
    use crate::vault::Vault;
    use chrono::Utc;
    use std::collections::HashMap;
    use tempfile::TempDir;
    use zeroize::Zeroizing;

    fn dummy_vault() -> &'static Vault {
        // Vault contains rusqlite::Connection (not Sync), so OnceLock<Vault>
        // doesn't compile. Box::leak gives &'static; tests don't touch SQL on
        // this vault and the process is short-lived, so the leak is fine.
        let tmp = Box::leak(Box::new(TempDir::new().unwrap()));
        let path = tmp.path().join("v.db");
        let v = Vault::create(&path, b"x").unwrap();
        Box::leak(Box::new(v))
    }

    #[test]
    fn score_in_zero_one_when_no_signals() {
        let p = Pool { seeds: vec![], favorite_base_words: vec![], all_base_words: vec![], site_abbreviations: vec![], era_window: None };
        let s = HistoryStats::default();
        let c = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let cand = Candidate { password: Zeroizing::new("plain".into()), score: 0.0, provenance: vec![], seed_history_id: None, breakdown: None };
        let v = score(&cand, &rc, None);
        assert!((0.0..=1.0).contains(&v));
    }

    #[test]
    fn hint_bumps_score() {
        let p = Pool { seeds: vec![], favorite_base_words: vec![], all_base_words: vec![], site_abbreviations: vec![], era_window: None };
        let s = HistoryStats::default();
        let mut c = RecoverConfig::default();
        c.hint = Some("flu".into());
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let with_hint = Candidate { password: Zeroizing::new("Fluffy".into()), score: 0.0, provenance: vec![], seed_history_id: None, breakdown: None };
        let no_hint = Candidate { password: Zeroizing::new("Other".into()), score: 0.0, provenance: vec![], seed_history_id: None, breakdown: None };
        assert!(score(&with_hint, &rc, None) > score(&no_hint, &rc, None));
    }

    #[test]
    fn site_match_increases_score() {
        let strong_seed = PoolSeed {
            history_id: 1, plaintext: Zeroizing::new("x".into()),
            created_at: Utc::now(), site_id: Some(1), site_match_strength: 1.0,
        };
        let weak_seed = PoolSeed {
            history_id: 2, plaintext: Zeroizing::new("y".into()),
            created_at: Utc::now(), site_id: Some(2), site_match_strength: 0.5,
        };
        let p = Pool { seeds: vec![strong_seed, weak_seed], favorite_base_words: vec![], all_base_words: vec![], site_abbreviations: vec![], era_window: None };
        let s = HistoryStats::default();
        let c = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let strong = Candidate { password: Zeroizing::new("X".into()), score: 0.0, provenance: vec![], seed_history_id: Some(1), breakdown: None };
        let weak = Candidate { password: Zeroizing::new("Y".into()), score: 0.0, provenance: vec![], seed_history_id: Some(2), breakdown: None };
        assert!(score(&strong, &rc, None) > score(&weak, &rc, None));
    }

    #[test]
    fn frequency_match_contributes() {
        let p = Pool { seeds: vec![], favorite_base_words: vec![], all_base_words: vec![], site_abbreviations: vec![], era_window: None };
        let mut s = HistoryStats::default();
        s.trailing_symbol_freq = HashMap::from([('!', 0.9)]);
        let c = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let with = Candidate { password: Zeroizing::new("abc!".into()), score: 0.0, provenance: vec![], seed_history_id: None, breakdown: None };
        let without = Candidate { password: Zeroizing::new("abcz".into()), score: 0.0, provenance: vec![], seed_history_id: None, breakdown: None };
        assert!(score(&with, &rc, None) > score(&without, &rc, None));
    }

    #[test]
    fn favorite_base_word_contributes() {
        let p = Pool {
            seeds: vec![],
            favorite_base_words: vec![DecryptedBaseWordEntry {
                canonical: Zeroizing::new("fluffy".into()),
                original: Zeroizing::new("fluffy".into()),
            }],
            all_base_words: vec![DecryptedBaseWordEntry {
                canonical: Zeroizing::new("fluffy".into()),
                original: Zeroizing::new("fluffy".into()),
            }],
            site_abbreviations: vec![], era_window: None,
        };
        let s = HistoryStats::default();
        let c = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let with = Candidate { password: Zeroizing::new("Fluffy123".into()), score: 0.0, provenance: vec![], seed_history_id: None, breakdown: None };
        let without = Candidate { password: Zeroizing::new("Banana123".into()), score: 0.0, provenance: vec![], seed_history_id: None, breakdown: None };
        assert!(score(&with, &rc, None) > score(&without, &rc, None));
    }

    #[test]
    fn post_hoc_site_match_rewards_candidate_containing_abbrev() {
        let p = Pool {
            seeds: vec![],
            favorite_base_words: vec![],
            all_base_words: vec![],
            site_abbreviations: vec!["Rd".into()],
            era_window: None,
        };
        let s = HistoryStats::default();
        let mut c = RecoverConfig::default();
        c.site = Some("Reddit".into());
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let with_abbr = Candidate {
            password: Zeroizing::new("MoonBeam$2019Rd".into()),
            score: 0.0,
            provenance: vec![],
            seed_history_id: None,
            breakdown: None,
        };
        let without = Candidate {
            password: Zeroizing::new("MoonBeam$2019".into()),
            score: 0.0,
            provenance: vec![],
            seed_history_id: None,
            breakdown: None,
        };
        assert!(score(&with_abbr, &rc, None) > score(&without, &rc, None),
            "containing site abbrev should bump score when --site is set");
    }

    #[test]
    fn original_casing_provenance_boosts_score() {
        let p = Pool { seeds: vec![], favorite_base_words: vec![], all_base_words: vec![], site_abbreviations: vec![], era_window: None };
        let s = HistoryStats::default();
        let c = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let with_oc = Candidate {
            password: Zeroizing::new("MoonBeam".into()),
            score: 0.0,
            provenance: vec![RuleId::WordCombine, RuleId::OriginalCasing],
            seed_history_id: None,
            breakdown: None,
        };
        let without_oc = Candidate {
            password: Zeroizing::new("MoonBeam".into()),
            score: 0.0,
            provenance: vec![RuleId::WordCombine],
            seed_history_id: None,
            breakdown: None,
        };
        assert!(score(&with_oc, &rc, None) > score(&without_oc, &rc, None),
            "OriginalCasing in provenance must boost score");
    }

    #[test]
    fn clean_pattern_bonus_increases_score() {
        // Two candidates with identical other factors: same provenance, same
        // length, same hint match, same site abbrev, same favorite. The only
        // difference is one is "clean" (last seg = Abbrev) and the other has a
        // trailing extra symbol (last seg = SymbolRun after Abbrev → dirty).
        let p = Pool {
            seeds: vec![],
            favorite_base_words: vec![DecryptedBaseWordEntry {
                canonical: Zeroizing::new("moon".into()),
                original: Zeroizing::new("Moon".into()),
            }, DecryptedBaseWordEntry {
                canonical: Zeroizing::new("beam".into()),
                original: Zeroizing::new("Beam".into()),
            }],
            all_base_words: vec![DecryptedBaseWordEntry {
                canonical: Zeroizing::new("moon".into()),
                original: Zeroizing::new("Moon".into()),
            }, DecryptedBaseWordEntry {
                canonical: Zeroizing::new("beam".into()),
                original: Zeroizing::new("Beam".into()),
            }],
            site_abbreviations: vec!["Rd".into()],
            era_window: None,
        };
        let s = HistoryStats::default();
        let mut c = RecoverConfig::default();
        c.site = Some("Reddit".into());
        c.hint = Some("moon".into());
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let clean = Candidate {
            password: Zeroizing::new("MoonBeam$2019Rd".into()),
            score: 0.0,
            provenance: vec![RuleId::WordCombine, RuleId::OriginalCasing],
            seed_history_id: None,
            breakdown: None,
        };
        let dirty = Candidate {
            password: Zeroizing::new("MoonBeam$2019Rd!".into()),
            score: 0.0,
            provenance: vec![RuleId::WordCombine, RuleId::OriginalCasing],
            seed_history_id: None,
            breakdown: None,
        };
        let s_clean = score(&clean, &rc, None);
        let s_dirty = score(&dirty, &rc, None);
        assert!(
            s_clean > s_dirty,
            "clean pattern must outrank dirty trailing-symbol child; got clean={s_clean} dirty={s_dirty}"
        );
    }

    #[test]
    fn score_with_multiplier_applies_average() {
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("v.db");
        let v = crate::vault::Vault::create(&path, b"hunter2").unwrap();
        let config = RecoverConfig { limit: 100, ..Default::default() };
        let pool = Pool {
            seeds: vec![],
            favorite_base_words: vec![],
            all_base_words: vec![],
            site_abbreviations: vec![],
            era_window: None,
        };
        let stats = HistoryStats::default();
        let ctx = RecoverContext { vault: &v, config: &config, pool: &pool, stats: &stats };
        let c = Candidate {
            password: Zeroizing::new("MoonBeam$2019".into()),
            score: 0.0,
            provenance: vec![RuleId::BaseWordPool, RuleId::SiteAffix],
            seed_history_id: None,
            breakdown: None,
        };

        let base = score(&c, &ctx, None);

        let mut m: HashMap<RuleId, f32> = HashMap::new();
        m.insert(RuleId::BaseWordPool, 1.2);
        m.insert(RuleId::SiteAffix, 1.0);
        let with_mult = score(&c, &ctx, Some(&m));

        // Average is (1.2 + 1.0) / 2 = 1.1.
        let expected = base * 1.1;
        let tolerance = 1e-4_f32;
        assert!(
            (with_mult - expected).abs() < tolerance,
            "expected {} (= base {} * 1.1), got {}",
            expected, base, with_mult
        );
    }

    #[test]
    fn history_seed_provenance_outranks_synthesized_ceteris_paribus() {
        let p = Pool { seeds: vec![], favorite_base_words: vec![], all_base_words: vec![], site_abbreviations: vec![], era_window: None };
        let s = HistoryStats::default();
        let c = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let with_history = Candidate {
            password: Zeroizing::new("Whatever".into()),
            score: 0.0,
            provenance: vec![RuleId::HistorySeed],
            seed_history_id: Some(1),
            breakdown: None,
        };
        let without_history = Candidate {
            password: Zeroizing::new("Whatever".into()),
            score: 0.0,
            provenance: vec![RuleId::BaseWordPool],
            seed_history_id: None,
            breakdown: None,
        };
        assert!(
            score(&with_history, &rc, None) > score(&without_history, &rc, None),
            "HistorySeed in provenance must outscore non-history candidates ceteris paribus",
        );
    }

    #[test]
    fn score_with_breakdown_returns_total_equal_to_score() {
        let p = Pool { seeds: vec![], favorite_base_words: vec![], all_base_words: vec![], site_abbreviations: vec![], era_window: None };
        let s = HistoryStats::default();
        let mut c = RecoverConfig::default();
        c.hint = Some("xyz".into());
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let cand = Candidate {
            password: Zeroizing::new("xyz123".into()),
            score: 0.0,
            provenance: vec![RuleId::HistorySeed, RuleId::OriginalCasing],
            seed_history_id: Some(1),
            breakdown: None,
        };
        let total = score(&cand, &rc, None);
        let (total2, breakdown) = score_with_breakdown(&cand, &rc, None);
        assert!((total - total2).abs() < 1e-6, "score and score_with_breakdown must agree");
        assert!((breakdown.total - total).abs() < 1e-6, "breakdown.total must equal returned score");
        assert_eq!(breakdown.history_seed, 1.0, "history_seed raw factor should be 1.0 for HistorySeed provenance");
        assert!((breakdown.history_seed_weighted - 1.0).abs() < 1e-6, "history_seed_weighted = W_HISTORY_SEED (1.0) when present");
        assert_eq!(breakdown.orig_casing, 1.0, "orig_casing raw should be 1.0 for OriginalCasing in provenance");
    }
}
