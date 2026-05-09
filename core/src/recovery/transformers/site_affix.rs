//! SiteAffix — prepends/appends site abbreviations.

use crate::recovery::transformers::Transformer;
use crate::recovery::{Candidate, RecoverContext, RuleId};
use zeroize::Zeroizing;

pub struct SiteAffix;

const MAX_OUT: usize = 12;
const MAX_ABBREVS: usize = 3;

impl Transformer for SiteAffix {
    fn name(&self) -> &'static str { "SiteAffix" }

    fn transform(&self, c: &Candidate, ctx: &RecoverContext<'_>) -> Vec<Candidate> {
        let src = c.password.as_str();
        let mut out: Vec<Candidate> = Vec::new();
        for abbr in ctx.pool.site_abbreviations.iter().take(MAX_ABBREVS) {
            for cased in [abbr.to_uppercase(), abbr.to_lowercase()] {
                push(&mut out, c, &format!("{cased}{src}"));
                if out.len() >= MAX_OUT { return out; }
                push(&mut out, c, &format!("{src}{cased}"));
                if out.len() >= MAX_OUT { return out; }
            }
        }
        out
    }
}

fn push(out: &mut Vec<Candidate>, parent: &Candidate, s: &str) {
    let mut prov = parent.provenance.clone();
    prov.push(RuleId::SiteAffix);
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

    #[test]
    fn affixes_with_upper_and_lower() {
        let pool = Pool {
            seeds: vec![], favorite_base_words: vec![], all_base_words: vec![],
            site_abbreviations: vec!["RS".into()], era_window: None,
        };
        let stats = HistoryStats::default();
        let cfg = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &cfg, pool: &pool, stats: &stats };
        let cand = Candidate { password: Zeroizing::new("Fluffy".into()), score: 0.0, provenance: vec![], seed_history_id: None };
        let out = SiteAffix.transform(&cand, &rc);
        let strs: Vec<String> = out.iter().map(|c| c.password.as_str().to_string()).collect();
        assert!(strs.contains(&"RSFluffy".to_string()));
        assert!(strs.contains(&"FluffyRS".to_string()));
        assert!(strs.contains(&"rsFluffy".to_string()));
        assert!(strs.contains(&"Fluffyrs".to_string()));
    }

    #[test]
    fn no_abbreviations_yields_empty_output() {
        let pool = Pool {
            seeds: vec![], favorite_base_words: vec![], all_base_words: vec![],
            site_abbreviations: vec![], era_window: None,
        };
        let stats = HistoryStats::default();
        let cfg = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &cfg, pool: &pool, stats: &stats };
        let cand = Candidate { password: Zeroizing::new("Fluffy".into()), score: 0.0, provenance: vec![], seed_history_id: None };
        assert!(SiteAffix.transform(&cand, &rc).is_empty());
    }

    #[test]
    fn caps_abbreviations_at_three() {
        let pool = Pool {
            seeds: vec![], favorite_base_words: vec![], all_base_words: vec![],
            site_abbreviations: vec!["A".into(), "B".into(), "C".into(), "D".into()],
            era_window: None,
        };
        let stats = HistoryStats::default();
        let cfg = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &cfg, pool: &pool, stats: &stats };
        let cand = Candidate { password: Zeroizing::new("x".into()), score: 0.0, provenance: vec![], seed_history_id: None };
        let out = SiteAffix.transform(&cand, &rc);
        let strs: Vec<String> = out.iter().map(|c| c.password.as_str().to_string()).collect();
        // 3 abbrevs * 2 cases * 2 positions = 12 (cap) — D should not appear.
        assert!(!strs.iter().any(|s| s.contains('D') || s.contains('d')));
    }
}
