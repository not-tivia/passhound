//! Importer module: parsers (CSV, paste-and-parse) and pipeline (preview/commit/undo).
//!
//! Parsers produce a shared [`ImportEntry`] IR plus [`ParseDiagnostic`] entries
//! for rows that couldn't be parsed. The pipeline takes that IR, classifies
//! each entry against the current vault state, and (on commit) writes a
//! transactional batch tagged with a fresh `imports` row id.

use chrono::{DateTime, Utc};
use zeroize::Zeroizing;

pub mod csv;
pub mod paste;
pub mod pipeline;
pub mod shred;

pub use csv::{parse_file as parse_csv, Mapping};
pub use paste::parse_str as parse_paste;
pub use pipeline::{commit, preview, undo, Classification, ClassifiedEntry, ImportId, Preview, UndoCounts};

/// A single parsed entry from any importer source.
#[derive(Debug, Clone)]
pub struct ImportEntry {
    /// Required, non-empty after trim.
    pub site: String,
    pub url: Option<String>,
    pub username: Option<String>,
    /// Public-facing identity on the service (game handle, screen name).
    /// Distinct from `username` (login credential). Optional.
    pub display_name: Option<String>,
    /// Required, never empty. Wrapped in `Zeroizing` so the plaintext is
    /// wiped from memory when this entry is dropped.
    pub password: Zeroizing<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub notes: Option<String>,
    /// Verbatim source line(s) for `--show-conflicts` diagnostics.
    pub source_row: Option<String>,
}

/// What the parser successfully extracted from a row that ended up as a
/// diagnostic. The missing field(s) are None; everything else reflects
/// what was found. Used by `pipeline::apply_patches` to reconstruct a
/// full ImportEntry when the user supplies the missing value.
#[derive(Debug, Clone, Default)]
pub struct PartialEntry {
    pub site: Option<String>,
    pub url: Option<String>,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub password: Option<Zeroizing<String>>,
    pub notes: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub source_row: Option<String>,
}

/// User-supplied fill-in values for a previously-skipped row.
/// Constructed by the GUI layer from RowPatchArgs IPC payloads.
#[derive(Debug, Clone)]
pub struct RowPatch {
    pub row: usize,
    pub site: Option<String>,
    pub password: Option<String>,
}

/// A row the parser couldn't interpret.
#[derive(Debug, Clone)]
pub struct ParseDiagnostic {
    pub row: usize,
    pub raw: String,
    pub reason: String,
    pub parsed: PartialEntry,
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

    /// Compile-time type check: ImportEntry.password must be Zeroizing<String>.
    /// If the field type regresses to plain String this function will not compile.
    #[test]
    fn import_entry_password_is_zeroizing() {
        fn _check(e: &ImportEntry) -> &zeroize::Zeroizing<String> {
            &e.password
        }
    }
}
