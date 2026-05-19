//! EraBoost — multiplies candidate score by 1.0..=1.5 based on seed era proximity.

use crate::recovery::score::ScoreModifier;
use crate::recovery::{Candidate, RecoverContext, RuleId};
use chrono::NaiveDate;

pub struct EraBoost;

impl ScoreModifier for EraBoost {
    fn name(&self) -> &'static str { "EraBoost" }

    fn adjust(&self, c: &mut Candidate, ctx: &RecoverContext<'_>) {
        let Some((start, end)) = ctx.pool.era_window else { return; };
        let Some(seed_id) = c.seed_history_id else { return; };
        let Some(seed) = ctx.pool.seeds.iter().find(|s| s.history_id == seed_id) else { return; };
        let center = era_center(start, end);
        let dist_years = (seed.created_at.date_naive() - center).num_days().abs() as f32 / 365.0;
        let proximity = (1.0 - (dist_years / 5.0)).clamp(0.0, 1.0);
        let multiplier = 1.0 + 0.5 * proximity;
        c.score *= multiplier;
        if !c.provenance.contains(&RuleId::EraBoost) {
            c.provenance.push(RuleId::EraBoost);
        }
    }
}

fn era_center(start: NaiveDate, end: NaiveDate) -> NaiveDate {
    let total = (end - start).num_days();
    start + chrono::Duration::days(total / 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recovery::{HistoryStats, Pool, PoolSeed, RecoverConfig};
    use crate::vault::Vault;
    use chrono::{TimeZone, Utc};
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

    fn pool_with_seed(year: i32, month: u32, day: u32) -> Pool {
        Pool {
            seeds: vec![PoolSeed {
                history_id: 1,
                plaintext: Zeroizing::new("x".into()),
                created_at: Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap(),
                site_id: None,
                site_match_strength: 1.0,
            }],
            favorite_base_words: vec![], all_base_words: vec![], site_abbreviations: vec![],
            era_window: Some((
                NaiveDate::from_ymd_opt(2014, 1, 1).unwrap(),
                NaiveDate::from_ymd_opt(2014, 12, 31).unwrap(),
            )),
        }
    }

    #[test]
    fn matching_era_boosts_score() {
        let p = pool_with_seed(2014, 6, 15);
        let s = HistoryStats::default();
        let c = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let mut cand = Candidate { password: Zeroizing::new("x".into()), score: 1.0, provenance: vec![], seed_history_id: Some(1), breakdown: None };
        EraBoost.adjust(&mut cand, &rc);
        assert!(cand.score > 1.4 && cand.score <= 1.5);
        assert!(cand.provenance.contains(&RuleId::EraBoost));
    }

    #[test]
    fn distant_era_does_not_boost_much() {
        let p = pool_with_seed(2025, 1, 1);
        let s = HistoryStats::default();
        let c = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let mut cand = Candidate { password: Zeroizing::new("x".into()), score: 1.0, provenance: vec![], seed_history_id: Some(1), breakdown: None };
        EraBoost.adjust(&mut cand, &rc);
        assert!((cand.score - 1.0).abs() < 0.01, "10+ years away -> no boost; got {}", cand.score);
    }

    #[test]
    fn no_era_window_no_change() {
        let mut p = pool_with_seed(2014, 6, 15);
        p.era_window = None;
        let s = HistoryStats::default();
        let c = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let mut cand = Candidate { password: Zeroizing::new("x".into()), score: 1.0, provenance: vec![], seed_history_id: Some(1), breakdown: None };
        EraBoost.adjust(&mut cand, &rc);
        assert_eq!(cand.score, 1.0);
        assert!(!cand.provenance.contains(&RuleId::EraBoost));
    }

    #[test]
    fn no_seed_id_no_change() {
        let p = pool_with_seed(2014, 6, 15);
        let s = HistoryStats::default();
        let c = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let mut cand = Candidate { password: Zeroizing::new("x".into()), score: 1.0, provenance: vec![], seed_history_id: None, breakdown: None };
        EraBoost.adjust(&mut cand, &rc);
        assert_eq!(cand.score, 1.0);
    }
}
