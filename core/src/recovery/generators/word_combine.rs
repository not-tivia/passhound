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
        let favorites: Vec<&str> = ctx.pool.favorite_base_words.iter().map(|w| w.as_str()).collect();
        // Top 20 non-favorites (by repo order, which is usage_count desc).
        let non_fav: Vec<&str> = ctx.pool.all_base_words.iter()
            .map(|w| w.as_str())
            .filter(|w| !favorites.contains(w))
            .take(20)
            .collect();

        // favorites x favorites (skip same-word).
        for a in &favorites {
            for b in &favorites {
                if a == b { continue; }
                push_pair(&mut out, a, b);
                if out.len() >= MAX_OUTPUTS { return out; }
            }
        }
        // favorites x top-20 non-favorites.
        for a in &favorites {
            for b in &non_fav {
                push_pair(&mut out, a, b);
                if out.len() >= MAX_OUTPUTS { return out; }
            }
        }
        out
    }
}

fn push_pair(out: &mut Vec<Candidate>, a: &str, b: &str) {
    for sep in SEPARATORS {
        // as-is.
        out.push(Candidate {
            password: Zeroizing::new(format!("{a}{sep}{b}")),
            score: 0.0,
            provenance: vec![RuleId::WordCombine],
            seed_history_id: None,
        });
        // TitleCase each word.
        let title_a = title_case(a);
        let title_b = title_case(b);
        out.push(Candidate {
            password: Zeroizing::new(format!("{title_a}{sep}{title_b}")),
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

    fn ctx_with_pool<'a>(pool: &'a Pool, stats: &'a HistoryStats, cfg: &'a RecoverConfig) -> RecoverContext<'a> {
        RecoverContext { vault: dummy_vault(), config: cfg, pool, stats }
    }

    #[test]
    fn combines_two_favorites_with_all_seps_and_cases() {
        let pool = Pool {
            seeds: vec![],
            favorite_base_words: vec![Zeroizing::new("blue".into()), Zeroizing::new("fish".into())],
            all_base_words: vec![Zeroizing::new("blue".into()), Zeroizing::new("fish".into())],
            site_abbreviations: vec![], era_window: None,
        };
        let stats = HistoryStats::default();
        let cfg = RecoverConfig::default();
        let out = WordCombine.generate(&ctx_with_pool(&pool, &stats, &cfg));
        // 2 ordered pairs (blue,fish) and (fish,blue), each producing 6 outputs (3 seps x 2 case patterns) = 12.
        assert_eq!(out.len(), 12);
        let strs: Vec<String> = out.iter().map(|c| c.password.as_str().to_string()).collect();
        assert!(strs.contains(&"bluefish".to_string()));
        assert!(strs.contains(&"BlueFish".to_string()));
        assert!(strs.contains(&"blue-fish".to_string()));
        assert!(strs.contains(&"Blue_Fish".to_string()));
    }

    #[test]
    fn caps_at_max_outputs() {
        // Generate enough favorites that the cartesian product would exceed the cap.
        let favs: Vec<Zeroizing<String>> = (0..30).map(|i| Zeroizing::new(format!("w{i}"))).collect();
        let pool = Pool {
            seeds: vec![],
            favorite_base_words: favs.clone(),
            all_base_words: favs,
            site_abbreviations: vec![], era_window: None,
        };
        let stats = HistoryStats::default();
        let cfg = RecoverConfig::default();
        let out = WordCombine.generate(&ctx_with_pool(&pool, &stats, &cfg));
        // push_pair adds 6 entries per call; the cap fires AFTER push_pair returns.
        // Worst-case overshoot is 5 (we hit MAX_OUTPUTS-1 before the last pair, then add 6).
        assert!(out.len() <= MAX_OUTPUTS + 6);
    }
}
