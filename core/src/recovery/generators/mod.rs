//! Generator trait and registry. Implementations populated by Tasks 7-8.

use crate::recovery::{Candidate, RecoverContext};

pub trait Generator: Sync {
    fn name(&self) -> &'static str;
    fn generate(&self, ctx: &RecoverContext<'_>) -> Vec<Candidate>;
}

pub static GENERATORS: &[&'static dyn Generator] = &[];
