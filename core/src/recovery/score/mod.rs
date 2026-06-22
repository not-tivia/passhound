//! Score modifiers + ranking. EraBoost is currently the only score modifier.

use crate::recovery::{Candidate, RecoverContext};

pub mod ranking;
pub mod era_boost;

pub trait ScoreModifier: Sync {
    fn name(&self) -> &'static str;
    fn adjust(&self, c: &mut Candidate, ctx: &RecoverContext<'_>);
}

pub static SCORE_MODIFIERS: &[&'static dyn ScoreModifier] = &[
    &era_boost::EraBoost,
];

// Core relevance weights. Phase 3.7 lever 3 added W_ORIG_CASING and rebalanced
// others; the goal is to make original-casing-derived synthesis chains (e.g.
// MoonBeam$2019Rd from MoonBeam seed) outrank machine-variant chains (e.g.
// thunder-moon!!) when both contain the user's hint substring.
// NOTE: these are NOT renormalized to sum to 1.0 (they sum to 1.20 since Phase
// 4.25 B2 raised W_FREQ to 0.30). The scorer is a raw weighted sum that already
// exceeds 1.0 via the additive history/clean-pattern bonuses below; only the
// RELATIVE magnitudes matter for ranking.
pub const W_SITE: f32         = 0.30;
pub const W_HINT: f32         = 0.25;
pub const W_FREQ: f32         = 0.30;  // Phase 4.25 B2 (was 0.10): let real symbol/digit habits bury never-used shapes
pub const W_FAV_BASE: f32     = 0.10;
pub const W_LEN: f32          = 0.05;
pub const W_ORIG_CASING: f32  = 0.20;

// Phase 3.8 — additive (not in the convex sum) bonus for candidates whose
// password fully decomposes into recognized segments AND ends in a natural
// terminator (Favorite / DigitRun / Abbrev, or SymbolRun directly after a
// Favorite). See `clean_pattern.rs` for the decomposition rules. Score range
// becomes [0, 1.05] when this bonus fires.
pub const W_CLEAN_PATTERN: f32 = 0.05;

// Phase 4.19 — additive bonus for candidates whose seed came from the user's
// own password history (seed_history_id is Some). Rewards candidates that are
// mutations of real past passwords rather than pure synthesis.
pub const W_HISTORY_SEED: f32 = 1.0;
pub const W_HISTORY_DESCENDANT: f32 = 0.5;

/// Phase 4.25 B1 — when the user queried a specific site, a history seed that
/// is NOT an exact site match has its history bonus scaled by this factor
/// (site-first ranking). Freeform recovery (no site) is unchanged.
pub const HISTORY_SITE_MISMATCH_FACTOR: f32 = 0.15;

#[derive(Debug, Clone)]
pub struct ScoreBreakdown {
    pub site: f32,         pub site_weighted: f32,
    pub hint: f32,         pub hint_weighted: f32,
    pub freq: f32,         pub freq_weighted: f32,
    pub fav: f32,          pub fav_weighted: f32,
    pub len: f32,          pub len_weighted: f32,
    pub orig_casing: f32,  pub orig_casing_weighted: f32,
    pub clean_pattern: f32, pub clean_pattern_weighted: f32,
    pub history_seed: f32, pub history_seed_weighted: f32,
    pub history_descendant: f32, pub history_descendant_weighted: f32,
    pub multiplier: f32,
    pub total: f32,
}
