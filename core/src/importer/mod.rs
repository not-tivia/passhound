//! Importer module: parsers (CSV, paste-and-parse) and pipeline (preview/commit/undo).
//!
//! Parsers produce a shared [`ImportEntry`] IR plus [`ParseDiagnostic`] entries
//! for rows that couldn't be parsed. The pipeline takes that IR, classifies
//! each entry against the current vault state, and (on commit) writes a
//! transactional batch tagged with a fresh `imports` row id.

use chrono::{DateTime, Utc};

pub mod csv;
pub mod paste;
pub mod shred;

pub use csv::{parse_file as parse_csv, Mapping};
pub use paste::parse_str as parse_paste;

/// A single parsed entry from any importer source.
#[derive(Debug, Clone)]
pub struct ImportEntry {
    /// Required, non-empty after trim.
    pub site: String,
    pub url: Option<String>,
    pub username: Option<String>,
    /// Required, never empty.
    pub password: String,
    pub created_at: Option<DateTime<Utc>>,
    pub notes: Option<String>,
    /// Verbatim source line(s) for `--show-conflicts` diagnostics.
    pub source_row: Option<String>,
}

/// A row the parser couldn't interpret.
#[derive(Debug, Clone)]
pub struct ParseDiagnostic {
    pub row: usize,
    pub raw: String,
    pub reason: String,
}

/// Output of a parser: successful entries plus rows it skipped.
#[derive(Debug, Clone, Default)]
pub struct ParseResult {
    pub entries: Vec<ImportEntry>,
    pub diagnostics: Vec<ParseDiagnostic>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_result_default_is_empty() {
        let r = ParseResult::default();
        assert!(r.entries.is_empty());
        assert!(r.diagnostics.is_empty());
    }
}
