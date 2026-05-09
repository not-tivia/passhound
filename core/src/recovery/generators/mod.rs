//! Generator trait and registry. Generators produce SEED candidates from the pool.

use crate::recovery::{Candidate, RecoverContext};

pub mod base_word_pool;
pub mod word_combine;

pub trait Generator: Sync {
    fn name(&self) -> &'static str;
    fn generate(&self, ctx: &RecoverContext<'_>) -> Vec<Candidate>;
}

pub static GENERATORS: &[&'static dyn Generator] = &[
    &base_word_pool::BaseWordPool,
    &word_combine::WordCombine,
];
