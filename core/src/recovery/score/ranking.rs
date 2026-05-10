//! Linear weighted-sum scorer.

use crate::recovery::clean_pattern;
use crate::recovery::score::{
    W_CLEAN_PATTERN, W_FAV_BASE, W_FREQ, W_HINT, W_LEN, W_ORIG_CASING, W_SITE,
};
use crate::recovery::stats::{count_trailing_digits, trailing_year};
use crate::recovery::{Candidate, RecoverContext, RuleId};

pub fn score(c: &Candidate, ctx: &RecoverContext<'_>) -> f32 {
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
        Some(h) if c.password.to_lowercase().contains(&h.to_lowercase()) => 1.0,
        Some(_) => 0.0,
        None => 0.5,
    };
    let freq = pattern_freq_match(c, ctx);
    let fav  = if contains_any_favorite(c, ctx) { 1.0 } else { 0.0 };
    let len  = length_match(c, ctx);
    let orig = if c.provenance.contains(&RuleId::OriginalCasing) { 1.0 } else { 0.0 };
    let mut total = W_SITE * site
        + W_HINT * hint
        + W_FREQ * freq
        + W_FAV_BASE * fav
        + W_LEN * len
        + W_ORIG_CASING * orig;
    // Clean-pattern bonus (Phase 3.8): additive +W_CLEAN_PATTERN when the
    // password fully decomposes into recognized segments and ends in a
    // natural terminator. Applied inside ranking::score (not as a
    // ScoreModifier) so the bonus influences intermediate cap truncation
    // — the Phase 3.7 trace showed `MoonBeam$2019Rd` was dropped during
    // pass 1 SpecialSuffix's hint-partition truncation, which sorts by
    // ranking::score. Score modifiers run after the pipeline, too late.
    let is_clean_pattern = clean_pattern::decompose(c.password.as_str(), ctx)
        .map(|segs| clean_pattern::is_clean(&segs))
        .unwrap_or(false);
    if is_clean_pattern {
        total += W_CLEAN_PATTERN;
    }
    total
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
    let lower = c.password.to_lowercase();
    ctx.pool.favorite_base_words.iter().any(|w| lower.contains(w.canonical.as_str()))
}

fn contains_site_abbrev(c: &Candidate, ctx: &RecoverContext<'_>) -> bool {
    let lower = c.password.to_lowercase();
    ctx.pool.site_abbreviations.iter().any(|a| !a.is_empty() && lower.contains(&a.to_lowercase()))
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
        let cand = Candidate { password: Zeroizing::new("plain".into()), score: 0.0, provenance: vec![], seed_history_id: None };
        let v = score(&cand, &rc);
        assert!((0.0..=1.0).contains(&v));
    }

    #[test]
    fn hint_bumps_score() {
        let p = Pool { seeds: vec![], favorite_base_words: vec![], all_base_words: vec![], site_abbreviations: vec![], era_window: None };
        let s = HistoryStats::default();
        let mut c = RecoverConfig::default();
        c.hint = Some("flu".into());
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let with_hint = Candidate { password: Zeroizing::new("Fluffy".into()), score: 0.0, provenance: vec![], seed_history_id: None };
        let no_hint = Candidate { password: Zeroizing::new("Other".into()), score: 0.0, provenance: vec![], seed_history_id: None };
        assert!(score(&with_hint, &rc) > score(&no_hint, &rc));
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
        let strong = Candidate { password: Zeroizing::new("X".into()), score: 0.0, provenance: vec![], seed_history_id: Some(1) };
        let weak = Candidate { password: Zeroizing::new("Y".into()), score: 0.0, provenance: vec![], seed_history_id: Some(2) };
        assert!(score(&strong, &rc) > score(&weak, &rc));
    }

    #[test]
    fn frequency_match_contributes() {
        let p = Pool { seeds: vec![], favorite_base_words: vec![], all_base_words: vec![], site_abbreviations: vec![], era_window: None };
        let mut s = HistoryStats::default();
        s.trailing_symbol_freq = HashMap::from([('!', 0.9)]);
        let c = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let with = Candidate { password: Zeroizing::new("abc!".into()), score: 0.0, provenance: vec![], seed_history_id: None };
        let without = Candidate { password: Zeroizing::new("abcz".into()), score: 0.0, provenance: vec![], seed_history_id: None };
        assert!(score(&with, &rc) > score(&without, &rc));
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
        let with = Candidate { password: Zeroizing::new("Fluffy123".into()), score: 0.0, provenance: vec![], seed_history_id: None };
        let without = Candidate { password: Zeroizing::new("Banana123".into()), score: 0.0, provenance: vec![], seed_history_id: None };
        assert!(score(&with, &rc) > score(&without, &rc));
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
        };
        let without = Candidate {
            password: Zeroizing::new("MoonBeam$2019".into()),
            score: 0.0,
            provenance: vec![],
            seed_history_id: None,
        };
        assert!(score(&with_abbr, &rc) > score(&without, &rc),
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
        };
        let without_oc = Candidate {
            password: Zeroizing::new("MoonBeam".into()),
            score: 0.0,
            provenance: vec![RuleId::WordCombine],
            seed_history_id: None,
        };
        assert!(score(&with_oc, &rc) > score(&without_oc, &rc),
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
        };
        let dirty = Candidate {
            password: Zeroizing::new("MoonBeam$2019Rd!".into()),
            score: 0.0,
            provenance: vec![RuleId::WordCombine, RuleId::OriginalCasing],
            seed_history_id: None,
        };
        let s_clean = score(&clean, &rc);
        let s_dirty = score(&dirty, &rc);
        assert!(
            s_clean > s_dirty,
            "clean pattern must outrank dirty trailing-symbol child; got clean={s_clean} dirty={s_dirty}"
        );
    }
}
