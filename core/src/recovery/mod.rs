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
}

impl RuleId {
    /// Short tag used in why-output. Matches chris's pattern letters where possible.
    pub fn tag(&self) -> &'static str {
        match self {
            RuleId::BaseWordPool    => "G",
            RuleId::WordCombine     => "D",
            RuleId::CaseVariations  => "CASE",
            RuleId::SpecialSuffix   => "B",
            RuleId::SiteAffix       => "E",
            RuleId::NumberIncrement => "F",
            RuleId::LeetSwap        => "LEET",
            RuleId::EraBoost        => "H",
            RuleId::OriginalCasing  => "ORIG",
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            RuleId::BaseWordPool    => "BaseWordPool",
            RuleId::WordCombine     => "WordCombine",
            RuleId::CaseVariations  => "CaseVariations",
            RuleId::SpecialSuffix   => "SpecialSuffix",
            RuleId::SiteAffix       => "SiteAffix",
            RuleId::NumberIncrement => "NumberIncrement",
            RuleId::LeetSwap        => "LeetSwap",
            RuleId::EraBoost        => "EraBoost",
            RuleId::OriginalCasing  => "OriginalCasing",
        }
    }
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
}
