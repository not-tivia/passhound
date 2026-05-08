//! BaseWordPool — emits favorite + non-favorite base words as seed candidates.

use crate::recovery::generators::Generator;
use crate::recovery::{Candidate, RecoverContext, RuleId};
use zeroize::Zeroizing;

pub struct BaseWordPool;

const MAX_OUTPUTS: usize = 100;

impl Generator for BaseWordPool {
    fn name(&self) -> &'static str { "BaseWordPool" }

    fn generate(&self, ctx: &RecoverContext<'_>) -> Vec<Candidate> {
        let mut out: Vec<Candidate> = Vec::new();

        // Favorites first; weighted x3 by emitting them three times so they get
        // more transformer fan-out chances. (Dedup pass after each transformer
        // collapses identical strings, but the multiple emits give the seed extra
        // provenance weight when collisions merge.)
        for w in &ctx.pool.favorite_base_words {
            for _ in 0..3 {
                if out.len() >= MAX_OUTPUTS { return out; }
                out.push(Candidate {
                    password: Zeroizing::new(w.as_str().to_owned()),
                    score: 0.0,
                    provenance: vec![RuleId::BaseWordPool],
                    seed_history_id: None,
                });
            }
        }

        // All base words by usage_count desc (already sorted by repo).
        for w in &ctx.pool.all_base_words {
            if out.len() >= MAX_OUTPUTS { break; }
            // Skip exact duplicates of favorites already pushed.
            if out.iter().any(|c| c.password.as_str() == w.as_str()) { continue; }
            out.push(Candidate {
                password: Zeroizing::new(w.as_str().to_owned()),
                score: 0.0,
                provenance: vec![RuleId::BaseWordPool],
                seed_history_id: None,
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recovery::{HistoryStats, Pool, RecoverConfig};
    use crate::vault::Vault;
    use tempfile::TempDir;

    fn dummy_vault() -> &'static Vault {
        // We only need a Vault reference for the context; tests don't touch SQL.
        // Build a fresh temp vault and leak it; Vault isn't Sync so a OnceLock
        // doesn't work. Each test gets its own.
        let tmp = Box::leak(Box::new(TempDir::new().unwrap()));
        let path = tmp.path().join("v.db");
        let v = Vault::create(&path, b"x").unwrap();
        Box::leak(Box::new(v))
    }

    #[test]
    fn emits_favorites_first() {
        let pool = Pool {
            seeds: vec![],
            favorite_base_words: vec![Zeroizing::new("apple".into()), Zeroizing::new("banana".into())],
            all_base_words: vec![
                Zeroizing::new("apple".into()),
                Zeroizing::new("banana".into()),
                Zeroizing::new("cherry".into()),
            ],
            site_abbreviations: vec![],
            era_window: None,
        };
        let stats = HistoryStats::default();
        let cfg = RecoverConfig::default();
        let ctx = RecoverContext { vault: dummy_vault(), config: &cfg, pool: &pool, stats: &stats };
        let out = BaseWordPool.generate(&ctx);
        // 3 emits of each of 2 favorites = 6, plus cherry = 7.
        assert_eq!(out.len(), 7);
        assert_eq!(out[0].password.as_str(), "apple");
        assert_eq!(out[3].password.as_str(), "banana");
        assert_eq!(out[6].password.as_str(), "cherry");
    }

    #[test]
    fn caps_at_max_outputs() {
        let many: Vec<Zeroizing<String>> = (0..200).map(|i| Zeroizing::new(format!("word{i}"))).collect();
        let pool = Pool {
            seeds: vec![],
            favorite_base_words: vec![],
            all_base_words: many,
            site_abbreviations: vec![],
            era_window: None,
        };
        let stats = HistoryStats::default();
        let cfg = RecoverConfig::default();
        let ctx = RecoverContext { vault: dummy_vault(), config: &cfg, pool: &pool, stats: &stats };
        let out = BaseWordPool.generate(&ctx);
        assert_eq!(out.len(), MAX_OUTPUTS);
    }

    #[test]
    fn provenance_marks_base_word_pool() {
        let pool = Pool {
            seeds: vec![],
            favorite_base_words: vec![Zeroizing::new("x".into())],
            all_base_words: vec![Zeroizing::new("x".into())],
            site_abbreviations: vec![],
            era_window: None,
        };
        let stats = HistoryStats::default();
        let cfg = RecoverConfig::default();
        let ctx = RecoverContext { vault: dummy_vault(), config: &cfg, pool: &pool, stats: &stats };
        let out = BaseWordPool.generate(&ctx);
        assert!(out.iter().all(|c| c.provenance == vec![RuleId::BaseWordPool]));
    }
}
