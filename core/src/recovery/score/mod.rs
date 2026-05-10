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

// Convex weights — sum to 1.0. Phase 3.7 lever 3 added W_ORIG_CASING and
// rebalanced others; the goal is to make original-casing-derived synthesis
// chains (e.g. MoonBeam$2019Rd from MoonBeam seed) outrank machine-variant
// chains (e.g. thunder-moon!!) when both contain the user's hint substring.
pub const W_SITE: f32         = 0.30;
pub const W_HINT: f32         = 0.25;
pub const W_FREQ: f32         = 0.10;
pub const W_FAV_BASE: f32     = 0.10;
pub const W_LEN: f32          = 0.05;
pub const W_ORIG_CASING: f32  = 0.20;

// Phase 3.8 — additive (not in the convex sum) bonus for candidates whose
// password fully decomposes into recognized segments AND ends in a natural
// terminator (Favorite / DigitRun / Abbrev, or SymbolRun directly after a
// Favorite). See `clean_pattern.rs` for the decomposition rules. Score range
// becomes [0, 1.05] when this bonus fires.
pub const W_CLEAN_PATTERN: f32 = 0.05;
