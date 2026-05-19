//! SiteAffix — prepends/appends site abbreviations.

use crate::recovery::transformers::Transformer;
use crate::recovery::{Candidate, RecoverContext, RuleId};
use zeroize::Zeroizing;

pub struct SiteAffix;

// 3 casings (original, UPPER, lower deduped) x 2 positions x 3 abbrevs = 18.
const MAX_OUT: usize = 18;
const MAX_ABBREVS: usize = 3;

impl Transformer for SiteAffix {
    fn name(&self) -> &'static str { "SiteAffix" }

    fn transform(&self, c: &Candidate, ctx: &RecoverContext<'_>) -> Vec<Candidate> {
        let src = c.password.as_str();
        let mut out: Vec<Candidate> = Vec::new();
        for abbr in ctx.pool.site_abbreviations.iter().take(MAX_ABBREVS) {
            // Emit original casing first (e.g. "Rd" for Reddit) so the user's
            // declared abbreviation casing is preferred. Tag it with
            // RuleId::OriginalCasing so ranking::score awards W_ORIG_CASING.
            // Then emit UPPER and lower, deduped against the original.
            let original = abbr.as_str().to_owned();
            let upper = abbr.to_uppercase();
            let lower = abbr.to_lowercase();
            let mut casings: Vec<(String, bool)> = vec![(original.clone(), true)];
            if upper != original { casings.push((upper, false)); }
            if lower != original && casings.iter().all(|(s, _)| s != &lower) {
                casings.push((lower, false));
            }
            for (cased, is_original) in casings {
                push(&mut out, c, &format!("{cased}{src}"), is_original);
                if out.len() >= MAX_OUT { return out; }
                push(&mut out, c, &format!("{src}{cased}"), is_original);
                if out.len() >= MAX_OUT { return out; }
            }
        }
        out
    }
}

fn push(out: &mut Vec<Candidate>, parent: &Candidate, s: &str, is_original: bool) {
    let mut prov = parent.provenance.clone();
    prov.push(RuleId::SiteAffix);
    if is_original && !prov.contains(&RuleId::OriginalCasing) {
        prov.push(RuleId::OriginalCasing);
    }
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

    #[test]
    fn affixes_with_upper_and_lower() {
        let pool = Pool {
            seeds: vec![], favorite_base_words: vec![], all_base_words: vec![],
            site_abbreviations: vec!["RS".into()], era_window: None,
        };
        let stats = HistoryStats::default();
        let cfg = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &cfg, pool: &pool, stats: &stats };
        let cand = Candidate { password: Zeroizing::new("Fluffy".into()), score: 0.0, provenance: vec![], seed_history_id: None, breakdown: None };
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
        let cand = Candidate { password: Zeroizing::new("Fluffy".into()), score: 0.0, provenance: vec![], seed_history_id: None, breakdown: None };
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
        let cand = Candidate { password: Zeroizing::new("x".into()), score: 0.0, provenance: vec![], seed_history_id: None, breakdown: None };
        let out = SiteAffix.transform(&cand, &rc);
        let strs: Vec<String> = out.iter().map(|c| c.password.as_str().to_string()).collect();
        // Single-letter abbrevs: original == upper, so 2 unique casings.
        // 3 abbrevs * 2 casings * 2 positions = 12 — D should not appear.
        assert!(!strs.iter().any(|s| s.contains('D') || s.contains('d')));
    }

    #[test]
    fn preserves_original_abbrev_casing_and_tags_it() {
        let pool = Pool {
            seeds: vec![], favorite_base_words: vec![], all_base_words: vec![],
            site_abbreviations: vec!["Rd".into()],
            era_window: None,
        };
        let stats = HistoryStats::default();
        let cfg = RecoverConfig::default();
        let rc = RecoverContext { vault: dummy_vault(), config: &cfg, pool: &pool, stats: &stats };
        let cand = Candidate {
            password: Zeroizing::new("MoonBeam$2019".into()),
            score: 0.0,
            provenance: vec![],
            seed_history_id: None,
            breakdown: None,
        };
        let out = SiteAffix.transform(&cand, &rc);
        let strs: Vec<String> = out.iter().map(|c| c.password.as_str().to_string()).collect();
        assert!(strs.contains(&"MoonBeam$2019Rd".to_string()), "expected original-cased 'Rd' suffix");
        assert!(strs.contains(&"MoonBeam$2019RD".to_string()), "expected uppercase 'RD' suffix");
        assert!(strs.contains(&"MoonBeam$2019rd".to_string()), "expected lowercase 'rd' suffix");
        // Original-cased emission must be tagged with RuleId::OriginalCasing.
        let orig = out.iter().find(|c| c.password.as_str() == "MoonBeam$2019Rd").unwrap();
        assert!(orig.provenance.contains(&RuleId::OriginalCasing),
            "original-cased emission must carry OriginalCasing tag; got {:?}", orig.provenance);
        // Upper/lower-cased emissions must NOT carry OriginalCasing.
        let upper = out.iter().find(|c| c.password.as_str() == "MoonBeam$2019RD").unwrap();
        assert!(!upper.provenance.contains(&RuleId::OriginalCasing),
            "uppercase emission must not carry OriginalCasing tag");
    }
}
