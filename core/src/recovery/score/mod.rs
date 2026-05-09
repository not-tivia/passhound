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

// Convex weights — sum to 1.0. See spec for rationale.
pub const W_SITE: f32       = 0.30;
pub const W_HINT: f32       = 0.25;
pub const W_FREQ: f32       = 0.20;
pub const W_FAV_BASE: f32   = 0.15;
pub const W_LEN: f32        = 0.10;
