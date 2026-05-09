//! Transformer trait and registry. Transformers fan out new candidates from existing ones.

use crate::recovery::{Candidate, RecoverContext};

pub mod case_variations;
pub mod special_suffix;
pub mod site_affix;
pub mod number_increment;
pub mod leet_swap;

pub trait Transformer: Sync {
    fn name(&self) -> &'static str;
    fn transform(&self, c: &Candidate, ctx: &RecoverContext<'_>) -> Vec<Candidate>;
}

pub static TRANSFORMERS: &[&'static dyn Transformer] = &[
    &case_variations::CaseVariations,
    &special_suffix::SpecialSuffix,
    &site_affix::SiteAffix,
    &number_increment::NumberIncrement,
    &leet_swap::LeetSwap,
];
