//! Linear weighted-sum scorer.

use crate::recovery::score::{W_FAV_BASE, W_FREQ, W_HINT, W_LEN, W_SITE};
use crate::recovery::stats::{count_trailing_digits, trailing_year};
use crate::recovery::{Candidate, RecoverContext};

pub fn score(c: &Candidate, ctx: &RecoverContext<'_>) -> f32 {
    let site = c.seed_history_id
        .and_then(|id| ctx.pool.seeds.iter().find(|s| s.history_id == id))
        .map(|s| s.site_match_strength)
        .unwrap_or(0.5); // candidates with no seed (BaseWordPool/WordCombine) get neutral site signal.
    let hint = match &ctx.config.hint {
        Some(h) if c.password.to_lowercase().contains(&h.to_lowercase()) => 1.0,
        Some(_) => 0.0,
        None => 0.5,
    };
    let freq = pattern_freq_match(c, ctx);
    let fav  = if contains_any_favorite(c, ctx) { 1.0 } else { 0.0 };
    let len  = length_match(c, ctx);
    W_SITE * site + W_HINT * hint + W_FREQ * freq + W_FAV_BASE * fav + W_LEN * len
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

fn length_match(c: &Candidate, ctx: &RecoverContext<'_>) -> f32 {
    if ctx.stats.mean_length == 0.0 { return 0.5; }
    let dist = (c.password.len() as f32 - ctx.stats.mean_length).abs();
    (1.0 - (dist / 10.0)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recovery::{DecryptedBaseWordEntry, HistoryStats, Pool, PoolSeed, RecoverConfig};
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
}
