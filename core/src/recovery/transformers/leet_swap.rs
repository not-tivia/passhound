//! LeetSwap — applies leet-speak character substitutions.

use crate::recovery::transformers::Transformer;
use crate::recovery::{Candidate, RecoverContext, RuleId};
use zeroize::Zeroizing;

pub struct LeetSwap;

const MAX_OUT: usize = 6;
// (target, replacement). Case-insensitive on the target side.
const MAP: &[(char, char)] = &[
    ('a', '@'),
    ('e', '3'),
    ('i', '1'),
    ('o', '0'),
    ('s', '$'),
];

impl Transformer for LeetSwap {
    fn name(&self) -> &'static str { "LeetSwap" }

    fn transform(&self, c: &Candidate, _ctx: &RecoverContext<'_>) -> Vec<Candidate> {
        let src = c.password.as_str();
        let lower = src.to_lowercase();
        if !MAP.iter().any(|(t, _)| lower.contains(*t)) {
            return Vec::new();
        }
        let mut out: Vec<Candidate> = Vec::new();

        // All-swapped variant.
        let all = swap_all(src);
        if all != src {
            push(&mut out, c, &all);
        }

        // Single-char-swapped variants (one substitution at a time).
        for (target, replacement) in MAP {
            let v = swap_single(src, *target, *replacement);
            if v != src && !out.iter().any(|x| x.password.as_str() == v) {
                push(&mut out, c, &v);
                if out.len() >= MAX_OUT { return out; }
            }
        }
        out
    }
}

fn swap_all(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let lc = c.to_ascii_lowercase();
        match MAP.iter().find(|(t, _)| *t == lc) {
            Some((_, r)) => out.push(*r),
            None => out.push(c),
        }
    }
    out
}

fn swap_single(s: &str, target: char, replacement: char) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.to_ascii_lowercase() == target {
            out.push(replacement);
        } else {
            out.push(c);
        }
    }
    out
}

fn push(out: &mut Vec<Candidate>, parent: &Candidate, s: &str) {
    let mut prov = parent.provenance.clone();
    prov.push(RuleId::LeetSwap);
    out.push(Candidate {
        password: Zeroizing::new(s.to_string()),
        score: 0.0,
        provenance: prov,
        seed_history_id: parent.seed_history_id,
        breakdown: None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recovery::{HistoryStats, Pool, RecoverConfig};
    use crate::vault::Vault;
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
    fn swaps_when_chars_present() {
        let p = Pool { seeds: vec![], favorite_base_words: vec![], all_base_words: vec![], site_abbreviations: vec![], era_window: None };
        let s = HistoryStats::default();
        let c = RecoverConfig::default();
        let cand = Candidate { password: Zeroizing::new("apples".into()), score: 0.0, provenance: vec![], seed_history_id: None, breakdown: None };
        let out = LeetSwap.transform(&cand, &rc(&p, &s, &c));
        let strs: Vec<String> = out.iter().map(|x| x.password.as_str().to_string()).collect();
        assert!(strs.contains(&"@pples".to_string()));
        assert!(strs.contains(&"appl3s".to_string()));
        assert!(strs.contains(&"apple$".to_string()));
        // All-swapped: a@->@; pp->pp; l->l; e->3; s->$.
        assert!(strs.contains(&"@ppl3$".to_string()));
    }

    #[test]
    fn empty_when_no_swap_chars() {
        let p = Pool { seeds: vec![], favorite_base_words: vec![], all_base_words: vec![], site_abbreviations: vec![], era_window: None };
        let s = HistoryStats::default();
        let c = RecoverConfig::default();
        let cand = Candidate { password: Zeroizing::new("xyz".into()), score: 0.0, provenance: vec![], seed_history_id: None, breakdown: None };
        assert!(LeetSwap.transform(&cand, &rc(&p, &s, &c)).is_empty());
    }

    #[test]
    fn caps_at_max_outputs() {
        let p = Pool { seeds: vec![], favorite_base_words: vec![], all_base_words: vec![], site_abbreviations: vec![], era_window: None };
        let s = HistoryStats::default();
        let c = RecoverConfig::default();
        let cand = Candidate { password: Zeroizing::new("aeiosaeio".into()), score: 0.0, provenance: vec![], seed_history_id: None, breakdown: None };
        let out = LeetSwap.transform(&cand, &rc(&p, &s, &c));
        assert!(out.len() <= MAX_OUT);
    }
}
