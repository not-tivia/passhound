//! Filled in by Task 4.

use crate::error::Result;
use crate::vault::Vault;

#[derive(Debug, Default)]
pub struct AnalyzeReport {
    pub tokens_seen: usize,
    pub base_words_written: usize,
    pub favorites_set: usize,
}

/// Stub — full implementation in Task 4.
pub fn extract_base_words_from_history(_vault: &Vault, _top_favorites: usize) -> Result<AnalyzeReport> {
    unimplemented!("filled in by Task 4")
}
