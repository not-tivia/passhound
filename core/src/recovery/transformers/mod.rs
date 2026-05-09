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

// Transformer firing order is significant: NumberIncrement runs BEFORE SiteAffix
// so the chain `<base><symbol><year><abbrev>` (e.g. "MoonBeam$2019Rd") composes
// in a single pass. With SiteAffix first, the abbrev gets attached before any
// year exists, blocking that pattern.
pub static TRANSFORMERS: &[&'static dyn Transformer] = &[
    &case_variations::CaseVariations,
    &special_suffix::SpecialSuffix,
    &number_increment::NumberIncrement,
    &site_affix::SiteAffix,
    &leet_swap::LeetSwap,
];
