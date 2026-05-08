//! Transformer trait and registry. Implementations populated by Tasks 9-13.

use crate::recovery::{Candidate, RecoverContext};

pub trait Transformer: Sync {
    fn name(&self) -> &'static str;
    fn transform(&self, c: &Candidate, ctx: &RecoverContext<'_>) -> Vec<Candidate>;
}

pub static TRANSFORMERS: &[&'static dyn Transformer] = &[];
