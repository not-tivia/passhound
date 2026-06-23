//! Recovery candidate generator.
//!
//! Pipeline: pool::build -> stats::compute -> generators -> transformers
//! -> ranking::score -> score modifiers -> sort/truncate.

use crate::vault::Vault;
use chrono::{DateTime, NaiveDate, Utc};
use std::collections::HashMap;
use zeroize::Zeroizing;

pub mod analyze;
pub mod clean_pattern;
pub mod feedback;
pub mod generators;
pub mod pipeline;
pub mod pool;
pub mod score;
pub mod stats;
pub mod transformers;

pub use analyze::{extract_base_words_from_history, AnalyzeReport};
pub use pipeline::recover;

/// One candidate password produced by the pipeline.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub password: Zeroizing<String>,
    pub score: f32,
    pub provenance: Vec<RuleId>,
    pub seed_history_id: Option<i64>,
    pub breakdown: Option<crate::recovery::score::ScoreBreakdown>,
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum RuleId {
    BaseWordPool,
    WordCombine,
    CaseVariations,
    SpecialSuffix,
    SiteAffix,
    NumberIncrement,
    LeetSwap,
    EraBoost,
    OriginalCasing,
    HistorySeed,
    HistoryDescendant,
}

impl RuleId {
    /// Short tag used in why-output. Matches chris's pattern letters where possible.
    pub fn tag(&self) -> &'static str {
        match self {
            RuleId::BaseWordPool      => "G",
            RuleId::WordCombine       => "D",
            RuleId::CaseVariations    => "CASE",
            RuleId::SpecialSuffix     => "B",
            RuleId::SiteAffix         => "E",
            RuleId::NumberIncrement   => "F",
            RuleId::LeetSwap          => "LEET",
            RuleId::EraBoost          => "H",
            RuleId::OriginalCasing    => "ORIG",
            RuleId::HistorySeed       => "HIST",
            RuleId::HistoryDescendant => "HDESC",
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            RuleId::BaseWordPool      => "BaseWordPool",
            RuleId::WordCombine       => "WordCombine",
            RuleId::CaseVariations    => "CaseVariations",
            RuleId::SpecialSuffix     => "SpecialSuffix",
            RuleId::SiteAffix         => "SiteAffix",
            RuleId::NumberIncrement   => "NumberIncrement",
            RuleId::LeetSwap          => "LeetSwap",
            RuleId::EraBoost          => "EraBoost",
            RuleId::OriginalCasing    => "OriginalCasing",
            RuleId::HistorySeed       => "HistorySeed",
            RuleId::HistoryDescendant => "HistoryDescendant",
        }
    }
    /// Inverse of `tag()`. Returns `None` for unknown strings.
    pub fn from_tag(s: &str) -> Option<RuleId> {
        match s {
            "G"     => Some(RuleId::BaseWordPool),
            "D"     => Some(RuleId::WordCombine),
            "CASE"  => Some(RuleId::CaseVariations),
            "B"     => Some(RuleId::SpecialSuffix),
            "E"     => Some(RuleId::SiteAffix),
            "F"     => Some(RuleId::NumberIncrement),
            "LEET"  => Some(RuleId::LeetSwap),
            "H"     => Some(RuleId::EraBoost),
            "ORIG"  => Some(RuleId::OriginalCasing),
            "HIST"  => Some(RuleId::HistorySeed),
            "HDESC" => Some(RuleId::HistoryDescendant),
            _       => None,
        }
    }
}

/// Compose child provenance for a transformer applying `rule` to `parent`.
///
/// Semantics:
/// - If parent was a HistorySeed (verbatim historical password), the child
///   is downgraded to HistoryDescendant — the child has been mutated by the
///   transformer, so it's no longer the verbatim historical password, but
///   still derives from one.
/// - If parent was already a HistoryDescendant, the child stays a
///   HistoryDescendant (transitivity through multi-pass transformations).
/// - If parent had no history lineage (synthesized from generators), the
///   child has no history lineage either.
/// - The transformer's own RuleId is appended (deduplicated against the
///   existing provenance).
pub fn child_provenance(parent: &Candidate, rule: RuleId) -> Vec<RuleId> {
    let mut prov = parent.provenance.clone();
    let was_history = prov.contains(&RuleId::HistorySeed)
        || prov.contains(&RuleId::HistoryDescendant);
    prov.retain(|r| *r != RuleId::HistorySeed);
    if was_history && !prov.contains(&RuleId::HistoryDescendant) {
        prov.push(RuleId::HistoryDescendant);
    }
    if !prov.contains(&rule) {
        prov.push(rule);
    }
    prov
}

/// Hints from the CLI; everything optional.
#[derive(Debug, Clone, Default)]
pub struct RecoverConfig {
    pub site: Option<String>,
    pub account: Option<String>,
    pub era_name: Option<String>,
    pub hint: Option<String>,
    pub limit: usize,
    pub min_length: Option<usize>,
    pub require_symbol: bool,
    pub require_digit: bool,
}

/// Read-only context passed to every rule. Built once per recover() invocation.
pub struct RecoverContext<'a> {
    pub vault: &'a Vault,
    pub config: &'a RecoverConfig,
    pub pool: &'a Pool,
    pub stats: &'a HistoryStats,
}

/// One base word in the pool: lowercase canonical for matching plus the
/// reconstructed original casing (or canonical itself if mask=0).
#[derive(Debug, Clone)]
pub struct DecryptedBaseWordEntry {
    pub canonical: Zeroizing<String>,
    pub original: Zeroizing<String>,
}

/// Filtered subset of password_history + base_words used for generation.
pub struct Pool {
    pub seeds: Vec<PoolSeed>,
    pub favorite_base_words: Vec<DecryptedBaseWordEntry>,
    pub all_base_words: Vec<DecryptedBaseWordEntry>,
    pub site_abbreviations: Vec<String>,
    pub era_window: Option<(NaiveDate, NaiveDate)>,
}

pub struct PoolSeed {
    pub history_id: i64,
    pub plaintext: Zeroizing<String>,
    pub created_at: DateTime<Utc>,
    pub site_id: Option<i64>,
    pub site_match_strength: f32,
}

/// Pre-computed pattern statistics from the user's whole history.
#[derive(Debug, Default)]
pub struct HistoryStats {
    pub trailing_symbol_freq: HashMap<char, f32>,
    pub trailing_digit_count_freq: HashMap<u8, f32>,
    pub mean_length: f32,
    pub year_suffix_freq: HashMap<u16, f32>,
    pub rule_applicability: std::collections::HashMap<RuleId, f32>,
    pub corpus_size: usize,
    pub rule_fit: std::collections::HashMap<RuleId, f32>,
}

// Stub functions live in submodule files; this is the type-only skeleton.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_id_tag_and_name_round_trip() {
        for r in [
            RuleId::BaseWordPool, RuleId::WordCombine, RuleId::CaseVariations,
            RuleId::SpecialSuffix, RuleId::SiteAffix, RuleId::NumberIncrement,
            RuleId::LeetSwap, RuleId::EraBoost, RuleId::OriginalCasing,
            RuleId::HistorySeed, RuleId::HistoryDescendant,
        ] {
            assert!(!r.tag().is_empty());
            assert!(!r.name().is_empty());
        }
    }

    #[test]
    fn recover_config_default_is_sensible() {
        let c = RecoverConfig::default();
        assert!(c.site.is_none());
        assert_eq!(c.limit, 0);
        assert!(!c.require_symbol);
    }

    #[test]
    fn rule_id_tag_from_tag_round_trip() {
        for r in [
            RuleId::BaseWordPool,
            RuleId::WordCombine,
            RuleId::CaseVariations,
            RuleId::SpecialSuffix,
            RuleId::SiteAffix,
            RuleId::NumberIncrement,
            RuleId::LeetSwap,
            RuleId::EraBoost,
            RuleId::OriginalCasing,
            RuleId::HistorySeed,
            RuleId::HistoryDescendant,
        ] {
            let t = r.tag();
            let back = RuleId::from_tag(t);
            assert_eq!(back, Some(r), "round trip failed for {:?}", r);
        }
        assert_eq!(RuleId::from_tag("nonexistent"), None);
    }

    #[test]
    fn child_provenance_downgrades_history_seed_to_descendant() {
        let parent = Candidate {
            password: Zeroizing::new("test".into()),
            score: 0.0,
            provenance: vec![RuleId::HistorySeed],
            seed_history_id: Some(1),
            breakdown: None,
        };
        let child = child_provenance(&parent, RuleId::NumberIncrement);
        assert!(!child.contains(&RuleId::HistorySeed),
            "HistorySeed must be removed from child provenance after transformation");
        assert!(child.contains(&RuleId::HistoryDescendant),
            "child must carry HistoryDescendant when parent was HistorySeed");
        assert!(child.contains(&RuleId::NumberIncrement),
            "child must include the transformer's own RuleId");
    }

    #[test]
    fn child_provenance_keeps_history_descendant_transitive() {
        let parent = Candidate {
            password: Zeroizing::new("test".into()),
            score: 0.0,
            provenance: vec![RuleId::HistoryDescendant, RuleId::SiteAffix],
            seed_history_id: Some(1),
            breakdown: None,
        };
        let child = child_provenance(&parent, RuleId::NumberIncrement);
        assert!(child.contains(&RuleId::HistoryDescendant),
            "transitive: HistoryDescendant survives subsequent transformations");
        assert!(child.contains(&RuleId::SiteAffix),
            "prior transformer rules survive");
        assert!(child.contains(&RuleId::NumberIncrement),
            "new transformer rule appended");
    }

    #[test]
    fn child_provenance_no_lineage_when_parent_synthesized() {
        let parent = Candidate {
            password: Zeroizing::new("test".into()),
            score: 0.0,
            provenance: vec![RuleId::BaseWordPool],
            seed_history_id: None,
            breakdown: None,
        };
        let child = child_provenance(&parent, RuleId::NumberIncrement);
        assert!(!child.contains(&RuleId::HistorySeed));
        assert!(!child.contains(&RuleId::HistoryDescendant),
            "non-history parent must produce non-history child");
        assert!(child.contains(&RuleId::BaseWordPool));
        assert!(child.contains(&RuleId::NumberIncrement));
    }

    #[test]
    fn child_provenance_appends_rule_only_once() {
        let parent = Candidate {
            password: Zeroizing::new("test".into()),
            score: 0.0,
            provenance: vec![RuleId::BaseWordPool, RuleId::NumberIncrement],
            seed_history_id: None,
            breakdown: None,
        };
        let child = child_provenance(&parent, RuleId::NumberIncrement);
        let count = child.iter().filter(|r| **r == RuleId::NumberIncrement).count();
        assert_eq!(count, 1, "rule must be deduplicated against existing provenance");
    }
}
