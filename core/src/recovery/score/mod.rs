//! Score modifiers + ranking. Implementations populated by Task 14.

use crate::recovery::{Candidate, RecoverContext};

pub mod ranking;
pub mod era_boost;

pub trait ScoreModifier: Sync {
    fn name(&self) -> &'static str;
    fn adjust(&self, c: &mut Candidate, ctx: &RecoverContext<'_>);
}

pub static SCORE_MODIFIERS: &[&'static dyn ScoreModifier] = &[];
