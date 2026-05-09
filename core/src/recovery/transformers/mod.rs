//! Transformer trait and registry. Transformers fan out new candidates from existing ones.

use crate::recovery::{Candidate, RecoverContext};

pub mod case_variations;
pub mod special_suffix;

pub trait Transformer: Sync {
    fn name(&self) -> &'static str;
    fn transform(&self, c: &Candidate, ctx: &RecoverContext<'_>) -> Vec<Candidate>;
}

pub static TRANSFORMERS: &[&'static dyn Transformer] = &[
    &case_variations::CaseVariations,
    &special_suffix::SpecialSuffix,
];
