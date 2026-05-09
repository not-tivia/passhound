//! NumberIncrement — finds trailing numeric segments and shifts them; also appends top years.

use crate::recovery::transformers::Transformer;
use crate::recovery::{Candidate, RecoverContext, RuleId};
use chrono::Datelike;
use regex::Regex;
use std::sync::OnceLock;
use zeroize::Zeroizing;

pub struct NumberIncrement;

const MAX_OUT: usize = 15;
const SHIFTS: &[i64] = &[-1, 1, -2, 2, -3, 3, 5, 10];

fn trailing_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^(?P<head>.*?)(?P<tail>\d+)$").unwrap())
}

impl Transformer for NumberIncrement {
    fn name(&self) -> &'static str { "NumberIncrement" }

    fn transform(&self, c: &Candidate, ctx: &RecoverContext<'_>) -> Vec<Candidate> {
        let src = c.password.as_str();
        let mut out: Vec<Candidate> = Vec::new();
        let top_years = top_years(ctx, 3);

        if let Some(caps) = trailing_re().captures(src) {
            let head = caps.name("head").map(|m| m.as_str()).unwrap_or("");
            let tail = caps.name("tail").map(|m| m.as_str()).unwrap_or("");
            if let Ok(n) = tail.parse::<i64>() {
                for d in SHIFTS {
                    let new_n = n + d;
                    if new_n < 0 { continue; }
                    let v = format!("{head}{new_n}");
                    push(&mut out, c, &v);
                    if out.len() >= MAX_OUT { return out; }
                }
                for y in &top_years {
                    let v = format!("{head}{y}");
                    push(&mut out, c, &v);
                    if out.len() >= MAX_OUT { return out; }
                }
            }
        } else {
            // No trailing digits — append top-3 years from stats AND every year in
            // the era window (when set), so a candidate produced inside an era
            // can pick up the right year suffix even if that year is not in
            // the top-3 stats. Bounded by MAX_OUT.
            let mut years_to_try: Vec<u16> = top_years.clone();
            if let Some((start, end)) = ctx.pool.era_window {
                let start_y = start.year() as u16;
                let end_y = end.year() as u16;
                let lo = start_y.saturating_sub(1);
                let hi = end_y.saturating_add(1);
                for y in lo..=hi {
                    if !years_to_try.contains(&y) {
                        years_to_try.push(y);
                    }
                }
            }
            for y in &years_to_try {
                let v = format!("{src}{y}");
                push(&mut out, c, &v);
                if out.len() >= MAX_OUT { return out; }
            }
        }
        out
    }
}

fn top_years(ctx: &RecoverContext<'_>, n: usize) -> Vec<u16> {
    let mut yfreq: Vec<(u16, f32)> = ctx.stats.year_suffix_freq.iter().map(|(k, v)| (*k, *v)).collect();
    yfreq.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    yfreq.into_iter().take(n).map(|(y, _)| y).collect()
}

fn push(out: &mut Vec<Candidate>, parent: &Candidate, s: &str) {
    let mut prov = parent.provenance.clone();
    prov.push(RuleId::NumberIncrement);
    out.push(Candidate {
        password: Zeroizing::new(s.to_string()),
        score: 0.0,
        provenance: prov,
        seed_history_id: parent.seed_history_id,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recovery::{HistoryStats, Pool, RecoverConfig};
    use crate::vault::Vault;
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn dummy_vault() -> &'static Vault {
        // Vault contains rusqlite::Connection (not Sync), so OnceLock<Vault>
        // doesn't compile. Box::leak gives &'static; tests don't touch SQL on
        // this vault and the process is short-lived, so the leak is fine.
        let tmp = Box::leak(Box::new(TempDir::new().unwrap()));
        let path = tmp.path().join("v.db");
        let v = Vault::create(&path, b"x").unwrap();
        Box::leak(Box::new(v))
    }

    fn rc<'a>(p: &'a Pool, s: &'a HistoryStats, c: &'a RecoverConfig) -> RecoverContext<'a> {
        RecoverContext { vault: dummy_vault(), config: c, pool: p, stats: s }
    }

    #[test]
    fn shifts_trailing_number() {
        let p = Pool { seeds: vec![], favorite_base_words: vec![], all_base_words: vec![], site_abbreviations: vec![], era_window: None };
        let s = HistoryStats::default();
        let c = RecoverConfig::default();
        let cand = Candidate { password: Zeroizing::new("pw2014".into()), score: 0.0, provenance: vec![], seed_history_id: None };
        let out = NumberIncrement.transform(&cand, &rc(&p, &s, &c));
        let strs: Vec<String> = out.iter().map(|x| x.password.as_str().to_string()).collect();
        assert!(strs.contains(&"pw2013".to_string()));
        assert!(strs.contains(&"pw2015".to_string()));
        assert!(strs.contains(&"pw2016".to_string()));
        assert!(strs.contains(&"pw2019".to_string()));
        assert!(strs.contains(&"pw2024".to_string()));
    }

    #[test]
    fn appends_top_years_when_no_trailing_digits() {
        let p = Pool { seeds: vec![], favorite_base_words: vec![], all_base_words: vec![], site_abbreviations: vec![], era_window: None };
        let mut s = HistoryStats::default();
        s.year_suffix_freq = HashMap::from([(2014_u16, 0.5), (2018_u16, 0.3), (2020_u16, 0.2), (2010_u16, 0.05)]);
        let c = RecoverConfig::default();
        let cand = Candidate { password: Zeroizing::new("plain".into()), score: 0.0, provenance: vec![], seed_history_id: None };
        let out = NumberIncrement.transform(&cand, &rc(&p, &s, &c));
        let strs: Vec<String> = out.iter().map(|x| x.password.as_str().to_string()).collect();
        assert!(strs.contains(&"plain2014".to_string()));
        assert!(strs.contains(&"plain2018".to_string()));
        assert!(strs.contains(&"plain2020".to_string()));
        // Top 3 only — 2010 should NOT appear.
        assert!(!strs.contains(&"plain2010".to_string()));
    }

    #[test]
    fn no_shift_below_zero() {
        let p = Pool { seeds: vec![], favorite_base_words: vec![], all_base_words: vec![], site_abbreviations: vec![], era_window: None };
        let s = HistoryStats::default();
        let c = RecoverConfig::default();
        let cand = Candidate { password: Zeroizing::new("x1".into()), score: 0.0, provenance: vec![], seed_history_id: None };
        let out = NumberIncrement.transform(&cand, &rc(&p, &s, &c));
        for o in &out {
            // Should never produce x-1, x-2, etc. (negatives skipped).
            assert!(!o.password.as_str().contains('-'));
        }
    }

    #[test]
    fn appends_era_window_years() {
        use chrono::NaiveDate;
        let p = Pool {
            seeds: vec![], favorite_base_words: vec![], all_base_words: vec![], site_abbreviations: vec![],
            era_window: Some((
                NaiveDate::from_ymd_opt(2016, 1, 1).unwrap(),
                NaiveDate::from_ymd_opt(2019, 12, 31).unwrap(),
            )),
        };
        let s = HistoryStats::default(); // no stats years
        let c = RecoverConfig::default();
        let cand = Candidate { password: Zeroizing::new("plain".into()), score: 0.0, provenance: vec![], seed_history_id: None };
        let out = NumberIncrement.transform(&cand, &rc(&p, &s, &c));
        let strs: Vec<String> = out.iter().map(|x| x.password.as_str().to_string()).collect();
        // Era window 2016-2019 with a 1-year buffer on each side -> 2015..=2020.
        assert!(strs.contains(&"plain2015".to_string()), "expected 'plain2015'; got {strs:?}");
        assert!(strs.contains(&"plain2017".to_string()), "expected 'plain2017'; got {strs:?}");
        assert!(strs.contains(&"plain2018".to_string()), "expected 'plain2018'; got {strs:?}");
        assert!(strs.contains(&"plain2019".to_string()), "expected 'plain2019'; got {strs:?}");
        assert!(strs.contains(&"plain2020".to_string()), "expected 'plain2020'; got {strs:?}");
    }
}
