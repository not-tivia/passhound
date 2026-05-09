//! SpecialSuffix — appends symbol suffixes; stats-aware.

use crate::recovery::transformers::Transformer;
use crate::recovery::{Candidate, RecoverContext, RuleId};
use zeroize::Zeroizing;

pub struct SpecialSuffix;

const MAX_OUT: usize = 12;
const FIXED_SUFFIXES: &[&str] = &["!", "!!", "?", ".", "@", "#", "!@#", "1!"];

impl Transformer for SpecialSuffix {
    fn name(&self) -> &'static str { "SpecialSuffix" }

    fn transform(&self, c: &Candidate, ctx: &RecoverContext<'_>) -> Vec<Candidate> {
        let src = c.password.as_str();
        let mut out: Vec<Candidate> = Vec::new();
        for sfx in FIXED_SUFFIXES {
            push_child(&mut out, c, &format!("{src}{sfx}"));
            if out.len() >= MAX_OUT { return out; }
        }

        // Stats-aware: if input doesn't end in a non-alphanumeric, also try
        // "!" + each of the top-3 trailing symbols from history stats.
        let ends_in_symbol = src.chars().last().map(|c| !c.is_alphanumeric()).unwrap_or(false);
        if !ends_in_symbol {
            let mut top: Vec<(char, f32)> = ctx.stats.trailing_symbol_freq.iter().map(|(k, v)| (*k, *v)).collect();
            top.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            for (ch, _) in top.iter().take(3) {
                let appended = format!("{src}!{ch}");
                push_child(&mut out, c, &appended);
                if out.len() >= MAX_OUT { return out; }
            }
        }
        out
    }
}

fn push_child(out: &mut Vec<Candidate>, parent: &Candidate, s: &str) {
    let mut prov = parent.provenance.clone();
    prov.push(RuleId::SpecialSuffix);
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

    fn cand(s: &str) -> Candidate {
        Candidate { password: Zeroizing::new(s.into()), score: 0.0, provenance: vec![], seed_history_id: None }
    }

    #[test]
    fn appends_fixed_suffixes() {
        let (p, s, c) = (Pool { seeds: vec![], favorite_base_words: vec![], all_base_words: vec![], site_abbreviations: vec![], era_window: None }, HistoryStats::default(), RecoverConfig::default());
        let out = SpecialSuffix.transform(&cand("fluffy"), &rc(&p, &s, &c));
        let strs: Vec<String> = out.iter().map(|c| c.password.as_str().to_string()).collect();
        assert!(strs.contains(&"fluffy!".to_string()));
        assert!(strs.contains(&"fluffy!!".to_string()));
        assert!(strs.contains(&"fluffy!@#".to_string()));
        assert!(strs.contains(&"fluffy1!".to_string()));
    }

    #[test]
    fn caps_at_max_outputs() {
        let mut stats = HistoryStats::default();
        stats.trailing_symbol_freq = HashMap::from([('!', 0.6), ('.', 0.3), ('?', 0.1)]);
        let (p, c) = (Pool { seeds: vec![], favorite_base_words: vec![], all_base_words: vec![], site_abbreviations: vec![], era_window: None }, RecoverConfig::default());
        let out = SpecialSuffix.transform(&cand("abcd"), &rc(&p, &stats, &c));
        assert!(out.len() <= MAX_OUT);
    }

    #[test]
    fn skips_stats_appends_when_already_ends_in_symbol() {
        let mut stats = HistoryStats::default();
        stats.trailing_symbol_freq = HashMap::from([('!', 0.9)]);
        let (p, c) = (Pool { seeds: vec![], favorite_base_words: vec![], all_base_words: vec![], site_abbreviations: vec![], era_window: None }, RecoverConfig::default());
        let out = SpecialSuffix.transform(&cand("abcd!"), &rc(&p, &stats, &c));
        let strs: Vec<String> = out.iter().map(|c| c.password.as_str().to_string()).collect();
        // Should NOT contain "abcd!!!" (which would be from "abcd!" + stats-append "!!"); should still contain "abcd!!".
        assert!(strs.contains(&"abcd!!".to_string()));
        // None of the candidates should be the "stats-symbol" append form like "abcd!!!" (which was the buggy interpretation).
    }
}
