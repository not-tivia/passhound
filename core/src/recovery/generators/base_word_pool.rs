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
        // more transformer fan-out chances. Each favorite contributes BOTH the
        // lowercase canonical AND the original casing (e.g. "moonbeam" + "MoonBeam"),
        // so the user's actual casing enters the seed set as a privileged variant.
        // Dedup later collapses identical strings (when canonical == original, no
        // duplication beyond the existing x3 weighting).
        for w in &ctx.pool.favorite_base_words {
            // Iterate canonical and original separately so we can tag the
            // original-casing emission with RuleId::OriginalCasing for the
            // ranking bonus. When canonical == original, the bonus still
            // attaches to one of the duplicate strings; dedup downstream
            // collapses them into a single Candidate that retains the bonus.
            for (variant, is_original) in [
                (w.canonical.as_str(), false),
                (w.original.as_str(), true),
            ] {
                for _ in 0..3 {
                    if out.len() >= MAX_OUTPUTS { return out; }
                    let mut prov = vec![RuleId::BaseWordPool];
                    if is_original { prov.push(RuleId::OriginalCasing); }
                    out.push(Candidate {
                        password: Zeroizing::new(variant.to_owned()),
                        score: 0.0,
                        provenance: prov,
                        seed_history_id: None,
                    });
                }
            }
        }

        // All base words by usage_count desc (already sorted by repo). Same
        // canonical+original treatment, but no x3 weighting (only favorites
        // get that privilege).
        for w in &ctx.pool.all_base_words {
            for (variant, is_original) in [
                (w.canonical.as_str(), false),
                (w.original.as_str(), true),
            ] {
                if out.len() >= MAX_OUTPUTS { break; }
                if out.iter().any(|c| c.password.as_str() == variant) { continue; }
                let mut prov = vec![RuleId::BaseWordPool];
                if is_original { prov.push(RuleId::OriginalCasing); }
                out.push(Candidate {
                    password: Zeroizing::new(variant.to_owned()),
                    score: 0.0,
                    provenance: prov,
                    seed_history_id: None,
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recovery::{DecryptedBaseWordEntry, HistoryStats, Pool, RecoverConfig};
    use crate::vault::Vault;
    use tempfile::TempDir;

    fn entry(s: &str) -> DecryptedBaseWordEntry {
        DecryptedBaseWordEntry {
            canonical: Zeroizing::new(s.to_string()),
            original: Zeroizing::new(s.to_string()),
        }
    }

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
            favorite_base_words: vec![
                crate::recovery::DecryptedBaseWordEntry {
                    canonical: Zeroizing::new("apple".into()),
                    original: Zeroizing::new("apple".into()),
                },
                crate::recovery::DecryptedBaseWordEntry {
                    canonical: Zeroizing::new("banana".into()),
                    original: Zeroizing::new("banana".into()),
                },
            ],
            all_base_words: vec![
                crate::recovery::DecryptedBaseWordEntry {
                    canonical: Zeroizing::new("apple".into()),
                    original: Zeroizing::new("apple".into()),
                },
                crate::recovery::DecryptedBaseWordEntry {
                    canonical: Zeroizing::new("banana".into()),
                    original: Zeroizing::new("banana".into()),
                },
                crate::recovery::DecryptedBaseWordEntry {
                    canonical: Zeroizing::new("cherry".into()),
                    original: Zeroizing::new("cherry".into()),
                },
            ],
            site_abbreviations: vec![],
            era_window: None,
        };
        let stats = HistoryStats::default();
        let cfg = RecoverConfig::default();
        let ctx = RecoverContext { vault: dummy_vault(), config: &cfg, pool: &pool, stats: &stats };
        let out = BaseWordPool.generate(&ctx);
        // 2 favorites × 2 variants × 3 emits = 12 (canonical and original collide
        // when equal, but the favorites loop doesn't dedup; that happens later
        // in the pipeline). Plus cherry from all_base_words: canonical pushes
        // once, original (==canonical) is dropped by the all_base_words dedup. = 13.
        assert_eq!(out.len(), 13);
        assert_eq!(out[0].password.as_str(), "apple");
        assert_eq!(out[6].password.as_str(), "banana");
        assert_eq!(out[12].password.as_str(), "cherry");
    }

    #[test]
    fn caps_at_max_outputs() {
        let many: Vec<DecryptedBaseWordEntry> = (0..200).map(|i| entry(&format!("word{i}"))).collect();
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
            favorite_base_words: vec![entry("x")],
            all_base_words: vec![entry("x")],
            site_abbreviations: vec![],
            era_window: None,
        };
        let stats = HistoryStats::default();
        let cfg = RecoverConfig::default();
        let ctx = RecoverContext { vault: dummy_vault(), config: &cfg, pool: &pool, stats: &stats };
        let out = BaseWordPool.generate(&ctx);
        // Emissions are either [BaseWordPool] (canonical) or
        // [BaseWordPool, OriginalCasing] (original casing). Both must start
        // with BaseWordPool.
        assert!(out.iter().all(|c| c.provenance.first() == Some(&RuleId::BaseWordPool)));
        assert!(out.iter().all(|c| c.provenance.iter().all(|r| matches!(r, RuleId::BaseWordPool | RuleId::OriginalCasing))));
    }

    #[test]
    fn emits_original_casing_when_different_from_canonical() {
        let pool = Pool {
            seeds: vec![],
            favorite_base_words: vec![crate::recovery::DecryptedBaseWordEntry {
                canonical: Zeroizing::new("moonbeam".into()),
                original: Zeroizing::new("MoonBeam".into()),
            }],
            all_base_words: vec![crate::recovery::DecryptedBaseWordEntry {
                canonical: Zeroizing::new("moonbeam".into()),
                original: Zeroizing::new("MoonBeam".into()),
            }],
            site_abbreviations: vec![],
            era_window: None,
        };
        let stats = HistoryStats::default();
        let cfg = RecoverConfig::default();
        let ctx = RecoverContext { vault: dummy_vault(), config: &cfg, pool: &pool, stats: &stats };
        let out = BaseWordPool.generate(&ctx);
        let strs: Vec<String> = out.iter().map(|c| c.password.as_str().to_string()).collect();
        assert!(strs.contains(&"moonbeam".to_string()), "canonical present");
        assert!(strs.contains(&"MoonBeam".to_string()), "original present");
    }
}
