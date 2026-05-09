//! WordCombine — joins 2 base words with separators and case patterns.

use crate::recovery::generators::Generator;
use crate::recovery::{Candidate, RecoverContext, RuleId};
use zeroize::Zeroizing;

pub struct WordCombine;

const MAX_OUTPUTS: usize = 200;
const SEPARATORS: &[&str] = &["", "-", "_"];

impl Generator for WordCombine {
    fn name(&self) -> &'static str { "WordCombine" }

    fn generate(&self, ctx: &RecoverContext<'_>) -> Vec<Candidate> {
        let mut out: Vec<Candidate> = Vec::new();
        // Use canonicals for the "no-same-word" filter and as the as-is base.
        let favorites: Vec<(&str, &str)> = ctx.pool.favorite_base_words.iter()
            .map(|w| (w.canonical.as_str(), w.original.as_str()))
            .collect();
        let fav_canonicals: Vec<&str> = favorites.iter().map(|(c, _)| *c).collect();
        // Top 20 non-favorites (by repo order, which is usage_count desc).
        let non_fav: Vec<(&str, &str)> = ctx.pool.all_base_words.iter()
            .map(|w| (w.canonical.as_str(), w.original.as_str()))
            .filter(|(c, _)| !fav_canonicals.contains(c))
            .take(20)
            .collect();

        // favorites x favorites (skip same-canonical).
        for (a_canon, a_orig) in &favorites {
            for (b_canon, b_orig) in &favorites {
                if a_canon == b_canon { continue; }
                push_pair(&mut out, a_canon, b_canon, a_orig, b_orig);
                if out.len() >= MAX_OUTPUTS { return out; }
            }
        }
        // favorites x top-20 non-favorites.
        for (a_canon, a_orig) in &favorites {
            for (b_canon, b_orig) in &non_fav {
                push_pair(&mut out, a_canon, b_canon, a_orig, b_orig);
                if out.len() >= MAX_OUTPUTS { return out; }
            }
        }
        out
    }
}

fn push_pair(out: &mut Vec<Candidate>, a_canon: &str, b_canon: &str, a_orig: &str, b_orig: &str) {
    for sep in SEPARATORS {
        // as-is (canonical, lowercase).
        out.push(Candidate {
            password: Zeroizing::new(format!("{a_canon}{sep}{b_canon}")),
            score: 0.0,
            provenance: vec![RuleId::WordCombine],
            seed_history_id: None,
        });
        // TitleCase each word.
        let title_a = title_case(a_canon);
        let title_b = title_case(b_canon);
        out.push(Candidate {
            password: Zeroizing::new(format!("{title_a}{sep}{title_b}")),
            score: 0.0,
            provenance: vec![RuleId::WordCombine],
            seed_history_id: None,
        });
        // Original casing — privileged variant. When canonical == original
        // (legacy rows or all-lowercase favorites), this duplicates the as-is
        // emission and gets collapsed by dedup downstream.
        out.push(Candidate {
            password: Zeroizing::new(format!("{a_orig}{sep}{b_orig}")),
            score: 0.0,
            provenance: vec![RuleId::WordCombine],
            seed_history_id: None,
        });
    }
}

fn title_case(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str().to_lowercase().as_str(),
        None => String::new(),
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
        // Vault contains rusqlite::Connection (not Sync), so OnceLock<Vault>
        // doesn't compile. Box::leak gives &'static; tests don't touch SQL on
        // this vault and the process is short-lived, so the leak is fine.
        let tmp = Box::leak(Box::new(TempDir::new().unwrap()));
        let path = tmp.path().join("v.db");
        let v = Vault::create(&path, b"x").unwrap();
        Box::leak(Box::new(v))
    }

    fn ctx_with_pool<'a>(pool: &'a Pool, stats: &'a HistoryStats, cfg: &'a RecoverConfig) -> RecoverContext<'a> {
        RecoverContext { vault: dummy_vault(), config: cfg, pool, stats }
    }

    #[test]
    fn combines_two_favorites_with_all_seps_and_cases() {
        let pool = Pool {
            seeds: vec![],
            favorite_base_words: vec![
                crate::recovery::DecryptedBaseWordEntry {
                    canonical: Zeroizing::new("blue".into()),
                    original: Zeroizing::new("blue".into()),
                },
                crate::recovery::DecryptedBaseWordEntry {
                    canonical: Zeroizing::new("fish".into()),
                    original: Zeroizing::new("fish".into()),
                },
            ],
            all_base_words: vec![
                crate::recovery::DecryptedBaseWordEntry {
                    canonical: Zeroizing::new("blue".into()),
                    original: Zeroizing::new("blue".into()),
                },
                crate::recovery::DecryptedBaseWordEntry {
                    canonical: Zeroizing::new("fish".into()),
                    original: Zeroizing::new("fish".into()),
                },
            ],
            site_abbreviations: vec![], era_window: None,
        };
        let stats = HistoryStats::default();
        let cfg = RecoverConfig::default();
        let out = WordCombine.generate(&ctx_with_pool(&pool, &stats, &cfg));
        // 2 ordered pairs × 3 separators × 3 case patterns (as-is, TitleCase, original) = 18.
        // Dedup happens later in the pipeline; the generator emits all 18 even if
        // some collide (canonical == original here).
        assert_eq!(out.len(), 18);
        let strs: Vec<String> = out.iter().map(|c| c.password.as_str().to_string()).collect();
        assert!(strs.contains(&"bluefish".to_string()));
        assert!(strs.contains(&"BlueFish".to_string()));
        assert!(strs.contains(&"blue-fish".to_string()));
        assert!(strs.contains(&"Blue_Fish".to_string()));
    }

    #[test]
    fn caps_at_max_outputs() {
        // Generate enough favorites that the cartesian product would exceed the cap.
        let favs: Vec<crate::recovery::DecryptedBaseWordEntry> = (0..30)
            .map(|i| crate::recovery::DecryptedBaseWordEntry {
                canonical: Zeroizing::new(format!("w{i}")),
                original: Zeroizing::new(format!("w{i}")),
            })
            .collect();
        let pool = Pool {
            seeds: vec![],
            favorite_base_words: favs.clone(),
            all_base_words: favs,
            site_abbreviations: vec![], era_window: None,
        };
        let stats = HistoryStats::default();
        let cfg = RecoverConfig::default();
        let out = WordCombine.generate(&ctx_with_pool(&pool, &stats, &cfg));
        // push_pair adds 9 entries per call (3 seps × 3 case patterns); the cap fires
        // AFTER push_pair returns. Worst-case overshoot is 8.
        assert!(out.len() <= MAX_OUTPUTS + 9);
    }

    #[test]
    fn emits_original_casing_pair() {
        let pool = Pool {
            seeds: vec![],
            favorite_base_words: vec![
                crate::recovery::DecryptedBaseWordEntry {
                    canonical: Zeroizing::new("moon".into()),
                    original: Zeroizing::new("Moon".into()),
                },
                crate::recovery::DecryptedBaseWordEntry {
                    canonical: Zeroizing::new("beam".into()),
                    original: Zeroizing::new("Beam".into()),
                },
            ],
            all_base_words: vec![
                crate::recovery::DecryptedBaseWordEntry {
                    canonical: Zeroizing::new("moon".into()),
                    original: Zeroizing::new("Moon".into()),
                },
                crate::recovery::DecryptedBaseWordEntry {
                    canonical: Zeroizing::new("beam".into()),
                    original: Zeroizing::new("Beam".into()),
                },
            ],
            site_abbreviations: vec![], era_window: None,
        };
        let stats = HistoryStats::default();
        let cfg = RecoverConfig::default();
        let out = WordCombine.generate(&ctx_with_pool(&pool, &stats, &cfg));
        let strs: Vec<String> = out.iter().map(|c| c.password.as_str().to_string()).collect();
        // Original-casing pair "Moon" + "Beam" with empty separator → "MoonBeam".
        assert!(strs.contains(&"MoonBeam".to_string()), "original-cased pair emitted");
        // Original with dash → "Moon-Beam".
        assert!(strs.contains(&"Moon-Beam".to_string()), "original-cased dash pair emitted");
    }
}
