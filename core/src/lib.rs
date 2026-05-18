//! passhound-core: vault, encryption, schema, repos, importer, recovery.

pub mod crypto;
pub mod error;
pub mod generator;
pub mod importer;
pub mod recovery;
pub mod repo;
pub mod schema;
pub mod settings;
pub mod vault;

pub use error::{Error, Result};
pub use vault::Vault;
pub use vault::chmod_journal_if_present as vault_chmod_journal_if_present;
pub use recovery::{
    extract_base_words_from_history, recover, AnalyzeReport, Candidate, RecoverConfig, RuleId,
};
