//! CaseVariations — emits lower / UPPER / TitleCase / cAmElCaSe of input.

use crate::recovery::transformers::Transformer;
use crate::recovery::{Candidate, RecoverContext, RuleId};
use zeroize::Zeroizing;

pub struct CaseVariations;

impl Transformer for CaseVariations {
    fn name(&self) -> &'static str { "CaseVariations" }

    fn transform(&self, c: &Candidate, _ctx: &RecoverContext<'_>) -> Vec<Candidate> {
        let src = c.password.as_str();
        let lower = src.to_lowercase();
        let upper = src.to_uppercase();
        let title = title_case(src);
        let camel = alternating_case(src);

        let mut variants: Vec<String> = Vec::new();
        for v in [lower, upper, title, camel] {
            if v != src && !variants.contains(&v) {
                variants.push(v);
            }
        }
        variants.into_iter().map(|s| make_child(c, s)).collect()
    }
}

fn make_child(parent: &Candidate, s: String) -> Candidate {
    let mut prov = parent.provenance.clone();
    prov.push(RuleId::CaseVariations);
    Candidate {
        password: Zeroizing::new(s),
        score: 0.0,
        provenance: prov,
        seed_history_id: parent.seed_history_id,
        breakdown: None,
    }
}

fn title_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut at_start = true;
    for c in s.chars() {
        if c.is_alphabetic() {
            if at_start { out.extend(c.to_uppercase()); at_start = false; }
            else { out.extend(c.to_lowercase()); }
        } else {
            out.push(c);
            at_start = true;
        }
    }
    out
}

fn alternating_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut upper = false;
    for c in s.chars() {
        if c.is_alphabetic() {
            if upper { out.extend(c.to_uppercase()); }
            else { out.extend(c.to_lowercase()); }
            upper = !upper;
        } else {
            out.push(c);
        }
    }
    out
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
    fn ctx() -> (Pool, HistoryStats, RecoverConfig) {
        let p = Pool { seeds: vec![], favorite_base_words: vec![], all_base_words: vec![], site_abbreviations: vec![], era_window: None };
        (p, HistoryStats::default(), RecoverConfig::default())
    }

    fn cand(s: &str) -> Candidate {
        Candidate { password: Zeroizing::new(s.into()), score: 0.0, provenance: vec![], seed_history_id: None, breakdown: None }
    }

    #[test]
    fn emits_distinct_variants() {
        let (p, s, c) = ctx();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let out = CaseVariations.transform(&cand("fluffy"), &rc);
        let strs: Vec<String> = out.iter().map(|x| x.password.as_str().to_string()).collect();
        assert!(strs.contains(&"FLUFFY".to_string()));
        assert!(strs.contains(&"Fluffy".to_string()));
        assert!(strs.iter().any(|s| s.chars().any(|c| c.is_uppercase()) && s.chars().any(|c| c.is_lowercase())));
        // "fluffy" itself is the input, must NOT appear in outputs.
        assert!(!strs.contains(&"fluffy".to_string()));
    }

    #[test]
    fn caps_at_four_outputs() {
        let (p, s, c) = ctx();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let out = CaseVariations.transform(&cand("fluffy"), &rc);
        assert!(out.len() <= 4);
    }

    #[test]
    fn dedups_identical_to_input() {
        // ALL-UPPERCASE input: lower != input, upper == input, title != input, camel != input
        // -> at most 3 variants.
        let (p, s, c) = ctx();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let out = CaseVariations.transform(&cand("FLUFFY"), &rc);
        assert!(out.iter().all(|x| x.password.as_str() != "FLUFFY"));
    }

    #[test]
    fn appends_provenance() {
        let (p, s, c) = ctx();
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let mut parent = cand("fluffy");
        parent.provenance = vec![RuleId::BaseWordPool];
        let out = CaseVariations.transform(&parent, &rc);
        assert!(out.iter().all(|x| x.provenance == vec![RuleId::BaseWordPool, RuleId::CaseVariations]));
    }
}
