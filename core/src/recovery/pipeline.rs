//! Pipeline orchestrator. Wires pool -> stats -> generators -> transformers -> score
//! -> score modifiers -> constraints -> sort -> truncate.

use crate::error::{Error, Result};
use crate::recovery::generators::GENERATORS;
use crate::recovery::pool;
use crate::recovery::score::{ranking, SCORE_MODIFIERS};
use crate::recovery::transformers::TRANSFORMERS;
use crate::recovery::{Candidate, HistoryStats, RecoverConfig, RecoverContext};
use crate::vault::Vault;
use std::collections::HashMap;

const DEFAULT_LIMIT: usize = 100;
const MAX_INTERMEDIATE: usize = 5_000;

pub fn recover(vault: &Vault, mut config: RecoverConfig) -> Result<Vec<Candidate>> {
    if config.limit == 0 { config.limit = DEFAULT_LIMIT; }

    let pool = pool::build(vault, &config)?;
    if pool.seeds.is_empty() && pool.all_base_words.is_empty() {
        return Err(Error::EmptyVault);
    }
    let stats = HistoryStats::compute(vault)?;
    let ctx = RecoverContext { vault, config: &config, pool: &pool, stats: &stats };

    // Generators -> seed candidates.
    let mut seeds: Vec<Candidate> = Vec::new();
    for g in GENERATORS {
        seeds.extend(g.generate(&ctx));
    }

    // Also seed from pool.seeds themselves so transformers can mutate the user's actual
    // historical passwords (e.g. NumberIncrement on "Fluffy!2014" -> "Fluffy!2015"). Each
    // pool seed becomes one Candidate with seed_history_id set.
    for ps in &pool.seeds {
        seeds.push(Candidate {
            password: zeroize::Zeroizing::new(ps.plaintext.as_str().to_owned()),
            score: 0.0,
            provenance: vec![],
            seed_history_id: Some(ps.history_id),
        });
    }

    // Transformers, additive, with intermediate cap.
    let mut fan: Vec<Candidate> = seeds;
    for t in TRANSFORMERS {
        let mut new: Vec<Candidate> = Vec::new();
        for c in &fan {
            new.extend(t.transform(c, &ctx));
        }
        fan.extend(new);
        dedup_exact(&mut fan);
        if fan.len() > MAX_INTERMEDIATE {
            // Promise score: provenance length * recency rank.
            fan.sort_by(|a, b| {
                let pa = promise_score(a, &ctx);
                let pb = promise_score(b, &ctx);
                pb.partial_cmp(&pa).unwrap_or(std::cmp::Ordering::Equal)
            });
            fan.truncate(MAX_INTERMEDIATE);
        }
    }

    // Score.
    for c in &mut fan {
        c.score = ranking::score(c, &ctx);
    }
    for m in SCORE_MODIFIERS {
        for c in &mut fan {
            m.adjust(c, &ctx);
        }
    }

    // Constraints filter.
    fan.retain(|c| satisfies_constraints(c, &config));

    // Sort by score desc.
    fan.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    fan.truncate(config.limit);
    Ok(fan)
}

fn dedup_exact(v: &mut Vec<Candidate>) {
    let mut by_str: HashMap<String, usize> = HashMap::new();
    let mut keep: Vec<bool> = vec![true; v.len()];
    // Collect merge ops first (target index, provenance to merge, seed_history_id) so
    // we can mutate v afterwards without overlapping borrows.
    let mut merges: Vec<(usize, Vec<crate::recovery::RuleId>, Option<i64>)> = Vec::new();
    for (i, c) in v.iter().enumerate() {
        let key = c.password.as_str().to_string();
        match by_str.get(&key).copied() {
            Some(j) => {
                // Merge i's provenance into j; mark i for removal.
                keep[i] = false;
                merges.push((j, c.provenance.clone(), c.seed_history_id));
            }
            None => { by_str.insert(key, i); }
        }
    }
    for (j, prov, sid) in merges {
        for r in prov {
            if !v[j].provenance.contains(&r) {
                v[j].provenance.push(r);
            }
        }
        // Prefer the entry with a seed_history_id (some > none).
        if v[j].seed_history_id.is_none() && sid.is_some() {
            v[j].seed_history_id = sid;
        }
    }
    let mut idx = 0;
    v.retain(|_| { let k = keep[idx]; idx += 1; k });
}

fn promise_score(c: &Candidate, ctx: &RecoverContext<'_>) -> f32 {
    let prov = c.provenance.len() as f32;
    let recency = match c.seed_history_id {
        Some(id) => {
            let pos = ctx.pool.seeds.iter().position(|s| s.history_id == id);
            match pos {
                Some(i) => 1.0 - (i as f32 / ctx.pool.seeds.len().max(1) as f32) * 0.9,
                None => 0.5,
            }
        }
        None => 0.5,
    };
    prov * recency
}

fn satisfies_constraints(c: &Candidate, cfg: &RecoverConfig) -> bool {
    if let Some(min_len) = cfg.min_length {
        if c.password.len() < min_len { return false; }
    }
    if cfg.require_symbol && !c.password.chars().any(|ch| !ch.is_alphanumeric()) {
        return false;
    }
    if cfg.require_digit && !c.password.chars().any(|ch| ch.is_ascii_digit()) {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::accounts::{self, NewAccount};
    use crate::repo::sites::{self, NewSite};
    use crate::recovery::extract_base_words_from_history;
    use tempfile::TempDir;

    fn vault_with_history(passwords: &[&str]) -> (TempDir, Vault) {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("v.db");
        let v = Vault::create(&path, b"hunter2").unwrap();
        let s = sites::create(&v, NewSite { name: "S".into(), ..Default::default() }).unwrap();
        let a = accounts::create(&v, NewAccount { site_id: s.id, ..Default::default() }).unwrap();
        for pw in passwords {
            crate::repo::passwords::insert(&v, crate::repo::passwords::NewPassword {
                account_id: a.id, plaintext: pw, source: "m".into(),
                confidence: crate::repo::passwords::Confidence::Certain, notes: None, created_at: None,
            }).unwrap();
        }
        (tmp, v)
    }

    #[test]
    fn empty_vault_returns_empty_vault_error() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("v.db");
        let v = Vault::create(&path, b"hunter2").unwrap();
        let err = recover(&v, RecoverConfig::default()).unwrap_err();
        assert!(matches!(err, Error::EmptyVault));
    }

    #[test]
    fn returns_at_most_limit_candidates() {
        let (_t, v) = vault_with_history(&["Fluffy!2014", "MoonBeam$2018"]);
        extract_base_words_from_history(&v, 2).unwrap();
        let cfg = RecoverConfig { limit: 5, ..Default::default() };
        let out = recover(&v, cfg).unwrap();
        assert!(out.len() <= 5);
    }

    #[test]
    fn constraints_filter_short_candidates() {
        let (_t, v) = vault_with_history(&["abcd", "efgh"]);
        extract_base_words_from_history(&v, 2).unwrap();
        let cfg = RecoverConfig { limit: 100, min_length: Some(20), ..Default::default() };
        let out = recover(&v, cfg).unwrap();
        for c in &out {
            assert!(c.password.len() >= 20);
        }
    }

    #[test]
    fn require_symbol_filters_out_alphanumeric_only() {
        let (_t, v) = vault_with_history(&["alpha", "beta"]);
        extract_base_words_from_history(&v, 2).unwrap();
        let cfg = RecoverConfig { limit: 100, require_symbol: true, ..Default::default() };
        let out = recover(&v, cfg).unwrap();
        for c in &out {
            assert!(c.password.chars().any(|ch| !ch.is_alphanumeric()));
        }
    }

    #[test]
    fn dedup_collapses_identical_candidates() {
        // Two seeds tokenize to the same word -> deduped to one base word in pool.
        let (_t, v) = vault_with_history(&["Fluffy!", "Fluffy!", "Fluffy!"]);
        extract_base_words_from_history(&v, 1).unwrap();
        let cfg = RecoverConfig { limit: 100, ..Default::default() };
        let out = recover(&v, cfg).unwrap();
        // Distinct strings only.
        let strs: std::collections::HashSet<String> = out.iter().map(|c| c.password.as_str().to_string()).collect();
        assert_eq!(strs.len(), out.len());
    }
}
