//! Linear weighted-sum scorer.

use crate::recovery::clean_pattern;
use crate::recovery::score::{
    W_CLEAN_PATTERN, W_FAV_BASE, W_FREQ, W_HINT, W_HISTORY_DESCENDANT, W_HISTORY_SEED, W_LEN,
    W_ORIG_CASING, W_SITE, HISTORY_SITE_MISMATCH_FACTOR,
};
use crate::recovery::stats::{count_trailing_digits, trailing_year};
use crate::recovery::{Candidate, RecoverContext, RuleId};
use std::collections::HashMap;

/// Rules that mutate a candidate's string away from its seed. OriginalCasing,
/// BaseWordPool, WordCombine, EraBoost, and the History* markers are NOT
/// divergence transforms.
const DIVERGENCE_RULES: [RuleId; 5] = [
    RuleId::CaseVariations,
    RuleId::SpecialSuffix,
    RuleId::SiteAffix,
    RuleId::NumberIncrement,
    RuleId::LeetSwap,
];

/// Count of divergence transforms stacked on a candidate.
pub fn transform_depth(c: &Candidate) -> usize {
    c.provenance
        .iter()
        .filter(|r| DIVERGENCE_RULES.iter().any(|d| d == *r))
        .count()
}

/// Ranking tier (lower = more likely a real password). The final sort orders by
/// (tier asc, score desc): a lower-priority tier never outranks a higher one.
///   0: verbatim history, exact-site match
///   1: verbatim history, other site
///   2: one-transform variant of a real password
///   3: deeper descendants + pure synthesis
pub fn tier(c: &Candidate, ctx: &RecoverContext<'_>) -> u8 {
    if c.provenance.contains(&RuleId::HistorySeed) {
        let strength = c
            .seed_history_id
            .and_then(|id| ctx.pool.seeds.iter().find(|s| s.history_id == id))
            .map(|s| s.site_match_strength)
            .unwrap_or(0.5);
        if (strength - 1.0).abs() < f32::EPSILON { 0 } else { 1 }
    } else if c.provenance.contains(&RuleId::HistoryDescendant) && transform_depth(c) == 1 {
        2
    } else {
        3
    }
}

/// Sort a fan real-passwords-first: tier ascending, then cached score descending.
/// Computes each tier once (it does a seed lookup) rather than inside the
/// comparator. Moves candidates — no cloning.
pub fn sorted_by_tier(fan: Vec<Candidate>, ctx: &RecoverContext<'_>) -> Vec<Candidate> {
    let mut keyed: Vec<(u8, Candidate)> = fan.into_iter().map(|c| (tier(&c, ctx), c)).collect();
    keyed.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(b.1.score.partial_cmp(&a.1.score).unwrap_or(std::cmp::Ordering::Equal))
    });
    keyed.into_iter().map(|(_, c)| c).collect()
}

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

    // Phase 4.21: history bonuses are scaled by the seed's site_match_strength
    // (1.0 exact-site match, 0.5 same-category fallback, 0.5 default when no
    // pool seed is found). Without this scaling, a wrong-site historical
    // password admitted via same-category fallback would get a flat +1.0
    // bonus that dwarfs the W_SITE = 0.30 discriminator, causing unrelated
    // history to crowd out right-site synthesis. HistorySeed and
    // HistoryDescendant are mutually exclusive: dedup_exact can union a
    // raw seed's provenance with a transformer child that produced an
    // identical plaintext, leaving both rules on one candidate; only the
    // stronger HistorySeed counts.
    let raw_strength = site_seed.unwrap_or(0.5);
    // Phase 4.25 B1: site-first. When a site was queried, a seed that isn't an
    // exact-site match (strength < 1.0) is demoted hard so verbatim wrong-site
    // history can't crowd out site-relevant candidates. Freeform recovery
    // (no site) is unchanged.
    let history_strength = if ctx.config.site.is_some() && raw_strength < 1.0 {
        raw_strength * HISTORY_SITE_MISMATCH_FACTOR
    } else {
        raw_strength
    };
    let has_seed = c.provenance.contains(&RuleId::HistorySeed);
    let has_descendant = c.provenance.contains(&RuleId::HistoryDescendant);
    let history_seed = if has_seed { history_strength } else { 0.0 };
    let history_descendant = if has_descendant && !has_seed { history_strength } else { 0.0 };

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
    let history_descendant_weighted = W_HISTORY_DESCENDANT * history_descendant;

    let mut total = site_weighted + hint_weighted + freq_weighted + fav_weighted
        + len_weighted + orig_casing_weighted + clean_pattern_weighted
        + history_seed_weighted + history_descendant_weighted;

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
        history_descendant, history_descendant_weighted,
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
        let p = pool_with_seed_strength(1.0);
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

    #[test]
    fn history_descendant_scores_between_synthesized_and_history_seed() {
        let p = Pool { seeds: vec![], favorite_base_words: vec![], all_base_words: vec![], site_abbreviations: vec![], era_window: None };
        let s = HistoryStats::default();
        let c = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let seed = Candidate {
            password: Zeroizing::new("PW".into()),
            score: 0.0,
            provenance: vec![RuleId::HistorySeed],
            seed_history_id: Some(1),
            breakdown: None,
        };
        let descendant = Candidate {
            password: Zeroizing::new("PW".into()),
            score: 0.0,
            provenance: vec![RuleId::HistoryDescendant, RuleId::NumberIncrement],
            seed_history_id: Some(1),
            breakdown: None,
        };
        let synthesized = Candidate {
            password: Zeroizing::new("PW".into()),
            score: 0.0,
            provenance: vec![RuleId::BaseWordPool],
            seed_history_id: None,
            breakdown: None,
        };
        let s_seed = score(&seed, &rc, None);
        let s_desc = score(&descendant, &rc, None);
        let s_synth = score(&synthesized, &rc, None);
        assert!(s_seed > s_desc, "history seed must outscore descendant: seed={s_seed} desc={s_desc}");
        assert!(s_desc > s_synth, "descendant must outscore synthesized: desc={s_desc} synth={s_synth}");
    }

    #[test]
    fn score_with_breakdown_history_descendant_field_populated() {
        // Pool seed at full site_match_strength = 1.0 so raw factor delivers full credit.
        let p = pool_with_seed_strength(1.0);
        let s = HistoryStats::default();
        let c = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let cand = Candidate {
            password: Zeroizing::new("x".into()),
            score: 0.0,
            provenance: vec![RuleId::HistoryDescendant, RuleId::NumberIncrement],
            seed_history_id: Some(1),
            breakdown: None,
        };
        let (_, breakdown) = score_with_breakdown(&cand, &rc, None);
        assert_eq!(breakdown.history_descendant, 1.0);
        assert!((breakdown.history_descendant_weighted - 0.5).abs() < 1e-6,
            "history_descendant_weighted = W_HISTORY_DESCENDANT (0.5) when present");
        assert_eq!(breakdown.history_seed, 0.0,
            "candidate with HistoryDescendant but not HistorySeed should report history_seed = 0");
    }

    #[test]
    fn score_with_breakdown_no_history_lineage_zero_descendant() {
        let p = Pool { seeds: vec![], favorite_base_words: vec![], all_base_words: vec![], site_abbreviations: vec![], era_window: None };
        let s = HistoryStats::default();
        let c = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let cand = Candidate {
            password: Zeroizing::new("x".into()),
            score: 0.0,
            provenance: vec![RuleId::BaseWordPool],
            seed_history_id: None,
            breakdown: None,
        };
        let (_, breakdown) = score_with_breakdown(&cand, &rc, None);
        assert_eq!(breakdown.history_descendant, 0.0);
        assert_eq!(breakdown.history_descendant_weighted, 0.0);
    }

    // Phase 4.21 ---------------------------------------------------------------

    fn pool_with_seed_strength(strength: f32) -> Pool {
        Pool {
            seeds: vec![PoolSeed {
                history_id: 1,
                plaintext: Zeroizing::new("x".into()),
                created_at: Utc::now(),
                site_id: Some(1),
                site_match_strength: strength,
            }],
            favorite_base_words: vec![],
            all_base_words: vec![],
            site_abbreviations: vec![],
            era_window: None,
        }
    }

    #[test]
    fn wrong_site_history_seed_gets_half_bonus() {
        // Same-category fallback in the pool: seed has site_match_strength=0.5.
        // The history bonus should scale with it, not deliver full +1.0.
        let p = pool_with_seed_strength(0.5);
        let s = HistoryStats::default();
        let c = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let cand = Candidate {
            password: Zeroizing::new("PW".into()),
            score: 0.0,
            provenance: vec![RuleId::HistorySeed],
            seed_history_id: Some(1),
            breakdown: None,
        };
        let (_, bd) = score_with_breakdown(&cand, &rc, None);
        assert!((bd.history_seed - 0.5).abs() < 1e-6,
            "raw history_seed should equal site_match_strength (0.5), got {}", bd.history_seed);
        assert!((bd.history_seed_weighted - 0.5).abs() < 1e-6,
            "history_seed_weighted should be W_HISTORY_SEED * 0.5 = 0.5, got {}", bd.history_seed_weighted);
    }

    #[test]
    fn right_site_history_seed_gets_full_bonus() {
        let p = pool_with_seed_strength(1.0);
        let s = HistoryStats::default();
        let c = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let cand = Candidate {
            password: Zeroizing::new("PW".into()),
            score: 0.0,
            provenance: vec![RuleId::HistorySeed],
            seed_history_id: Some(1),
            breakdown: None,
        };
        let (_, bd) = score_with_breakdown(&cand, &rc, None);
        assert!((bd.history_seed - 1.0).abs() < 1e-6);
        assert!((bd.history_seed_weighted - 1.0).abs() < 1e-6);
    }

    #[test]
    fn wrong_site_history_descendant_gets_quarter_bonus() {
        // site_match_strength=0.5 × W_HISTORY_DESCENDANT (0.5) = 0.25
        let p = pool_with_seed_strength(0.5);
        let s = HistoryStats::default();
        let c = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let cand = Candidate {
            password: Zeroizing::new("PW".into()),
            score: 0.0,
            provenance: vec![RuleId::HistoryDescendant, RuleId::NumberIncrement],
            seed_history_id: Some(1),
            breakdown: None,
        };
        let (_, bd) = score_with_breakdown(&cand, &rc, None);
        assert!((bd.history_descendant - 0.5).abs() < 1e-6,
            "raw history_descendant should equal site_match_strength (0.5), got {}", bd.history_descendant);
        assert!((bd.history_descendant_weighted - 0.25).abs() < 1e-6,
            "history_descendant_weighted should be 0.5 * 0.5 = 0.25, got {}", bd.history_descendant_weighted);
    }

    #[test]
    fn dedup_collision_does_not_double_count_history() {
        // dedup_exact can union provenances when a transformer child equals a
        // verbatim seed, producing a single candidate carrying BOTH HistorySeed
        // and HistoryDescendant. The scorer must treat them as mutually
        // exclusive: only HistorySeed counts.
        let p = pool_with_seed_strength(1.0);
        let s = HistoryStats::default();
        let c = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let cand = Candidate {
            password: Zeroizing::new("PW".into()),
            score: 0.0,
            provenance: vec![RuleId::HistorySeed, RuleId::HistoryDescendant, RuleId::CaseVariations],
            seed_history_id: Some(1),
            breakdown: None,
        };
        let (_, bd) = score_with_breakdown(&cand, &rc, None);
        assert!((bd.history_seed - 1.0).abs() < 1e-6, "HistorySeed should count fully");
        assert_eq!(bd.history_descendant, 0.0,
            "HistoryDescendant must be suppressed when HistorySeed is also present");
        assert_eq!(bd.history_descendant_weighted, 0.0);
    }

    #[test]
    fn wrong_site_seed_does_not_outrank_right_site_synthesis() {
        // The user-reported bug: querying Tumblr, RuneScape historical passwords
        // (admitted to the pool via same-category fallback at strength=0.5) get
        // a flat +1.0 history bonus and crowd out actual Tumblr-relevant
        // synthesis. After Phase 4.21 site-scaling, a synthesized candidate
        // that genuinely matches the queried site + favorite + hint should
        // beat a wrong-site historical seed.
        let p = Pool {
            seeds: vec![PoolSeed {
                history_id: 1,
                plaintext: Zeroizing::new("OldGame123".into()),
                created_at: Utc::now(),
                site_id: Some(99),
                site_match_strength: 0.5, // same-category fallback
            }],
            favorite_base_words: vec![DecryptedBaseWordEntry {
                canonical: Zeroizing::new("fluffy".into()),
                original: Zeroizing::new("Fluffy".into()),
            }],
            all_base_words: vec![DecryptedBaseWordEntry {
                canonical: Zeroizing::new("fluffy".into()),
                original: Zeroizing::new("Fluffy".into()),
            }],
            site_abbreviations: vec!["Tm".into()],
            era_window: None,
        };
        let s = HistoryStats::default();
        let mut c = RecoverConfig::default();
        c.site = Some("Tumblr".into());
        c.hint = Some("fluffy".into());
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let wrong_site_seed = Candidate {
            password: Zeroizing::new("OldGame123".into()),
            score: 0.0,
            provenance: vec![RuleId::HistorySeed],
            seed_history_id: Some(1),
            breakdown: None,
        };
        let right_site_synth = Candidate {
            password: Zeroizing::new("Fluffy$2014Tm".into()),
            score: 0.0,
            provenance: vec![RuleId::BaseWordPool, RuleId::SpecialSuffix, RuleId::NumberIncrement, RuleId::SiteAffix, RuleId::OriginalCasing],
            seed_history_id: None,
            breakdown: None,
        };
        let s_wrong = score(&wrong_site_seed, &rc, None);
        let s_right = score(&right_site_synth, &rc, None);
        assert!(s_right > s_wrong,
            "right-site synthesis must outrank wrong-site historical seed; got synth={s_right} wrong_seed={s_wrong}");
    }

    #[test]
    fn site_queried_wrong_site_seed_is_demoted() {
        // site queried + seed is NOT an exact-site match (strength 0.5)
        // -> history bonus scaled by HISTORY_SITE_MISMATCH_FACTOR.
        let p = pool_with_seed_strength(0.5);
        let s = HistoryStats::default();
        let mut c = RecoverConfig::default();
        c.site = Some("TacoBell".into());
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let cand = Candidate {
            password: Zeroizing::new("PW".into()),
            score: 0.0,
            provenance: vec![RuleId::HistorySeed],
            seed_history_id: Some(1),
            breakdown: None,
        };
        let (_, bd) = score_with_breakdown(&cand, &rc, None);
        assert!((bd.history_seed - 0.075).abs() < 1e-6,
            "0.5 strength * 0.15 mismatch factor = 0.075, got {}", bd.history_seed);
    }

    #[test]
    fn site_queried_exact_match_seed_keeps_full_bonus() {
        let p = pool_with_seed_strength(1.0);
        let s = HistoryStats::default();
        let mut c = RecoverConfig::default();
        c.site = Some("TacoBell".into());
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let cand = Candidate {
            password: Zeroizing::new("PW".into()),
            score: 0.0,
            provenance: vec![RuleId::HistorySeed],
            seed_history_id: Some(1),
            breakdown: None,
        };
        let (_, bd) = score_with_breakdown(&cand, &rc, None);
        assert!((bd.history_seed - 1.0).abs() < 1e-6,
            "exact-site seed keeps full bonus even under a site query, got {}", bd.history_seed);
    }

    #[test]
    fn used_trailing_symbol_outranks_unused_by_strong_margin() {
        // History establishes '!' as a common trailing symbol; '#' never used.
        // Two otherwise-identical candidates differ only in trailing symbol.
        // After up-weighting W_FREQ the used-symbol margin must be strong.
        let mut s = HistoryStats::default();
        s.trailing_symbol_freq.insert('!', 1.0);
        let p = pool_with_seed_strength(0.5);
        let c = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let mk = |pw: &str| Candidate {
            password: Zeroizing::new(pw.into()),
            score: 0.0,
            provenance: vec![RuleId::SpecialSuffix],
            seed_history_id: None,
            breakdown: None,
        };
        let used   = score(&mk("fluffy!"), &rc, None);
        let unused = score(&mk("fluffy#"), &rc, None);
        let margin = used - unused;
        assert!(margin > 0.2,
            "used-symbol should beat unused by a strong margin (got {margin}); W_FREQ too low?");
    }

    // Phase 4.26 ---------------------------------------------------------------

    #[test]
    fn transform_depth_counts_only_divergence_rules() {
        let c = Candidate {
            password: Zeroizing::new("x".into()), score: 0.0,
            provenance: vec![RuleId::HistorySeed, RuleId::OriginalCasing, RuleId::SiteAffix, RuleId::SpecialSuffix],
            seed_history_id: Some(1), breakdown: None,
        };
        assert_eq!(transform_depth(&c), 2, "SiteAffix + SpecialSuffix count; HistorySeed + OriginalCasing do not");
        let verbatim = Candidate {
            password: Zeroizing::new("x".into()), score: 0.0,
            provenance: vec![RuleId::HistorySeed], seed_history_id: Some(1), breakdown: None,
        };
        assert_eq!(transform_depth(&verbatim), 0);
    }

    fn cand(prov: Vec<RuleId>, seed: Option<i64>) -> Candidate {
        Candidate { password: Zeroizing::new("pw".into()), score: 0.0, provenance: prov, seed_history_id: seed, breakdown: None }
    }

    #[test]
    fn tier_exact_site_seed_is_0() {
        let p = pool_with_seed_strength(1.0);
        let s = HistoryStats::default(); let c = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        assert_eq!(tier(&cand(vec![RuleId::HistorySeed], Some(1)), &rc), 0);
    }

    #[test]
    fn tier_other_site_seed_is_1() {
        let p = pool_with_seed_strength(0.5);
        let s = HistoryStats::default(); let c = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        assert_eq!(tier(&cand(vec![RuleId::HistorySeed], Some(1)), &rc), 1);
    }

    #[test]
    fn tier_one_transform_descendant_is_2() {
        let p = pool_with_seed_strength(1.0);
        let s = HistoryStats::default(); let c = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        assert_eq!(tier(&cand(vec![RuleId::HistoryDescendant, RuleId::SpecialSuffix], Some(1)), &rc), 2);
    }

    #[test]
    fn tier_deep_descendant_is_3() {
        let p = pool_with_seed_strength(1.0);
        let s = HistoryStats::default(); let c = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        assert_eq!(tier(&cand(vec![RuleId::HistoryDescendant, RuleId::SpecialSuffix, RuleId::SiteAffix], Some(1)), &rc), 3);
    }

    #[test]
    fn tier_pure_synthesis_is_3() {
        let p = pool_with_seed_strength(1.0);
        let s = HistoryStats::default(); let c = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        assert_eq!(tier(&cand(vec![RuleId::BaseWordPool, RuleId::SpecialSuffix], None), &rc), 3);
    }

    #[test]
    fn tier_sort_puts_real_seed_above_stacked_synthesis() {
        let p = pool_with_seed_strength(0.5); // seed history_id 1 -> tier 1
        let s = HistoryStats::default(); let c = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let seed = Candidate { password: Zeroizing::new("oldpw".into()), score: 0.10,
            provenance: vec![RuleId::HistorySeed], seed_history_id: Some(1), breakdown: None };
        let synth = Candidate { password: Zeroizing::new("RGpass4word#".into()), score: 0.99,
            provenance: vec![RuleId::BaseWordPool, RuleId::SiteAffix, RuleId::SpecialSuffix, RuleId::HistoryDescendant],
            seed_history_id: Some(1), breakdown: None };
        let sorted = sorted_by_tier(vec![synth, seed], &rc);
        assert_eq!(sorted[0].password.as_str(), "oldpw", "tier-1 seed must beat tier-3 synthesis despite lower score");
        assert_eq!(sorted[1].password.as_str(), "RGpass4word#");
    }
}
