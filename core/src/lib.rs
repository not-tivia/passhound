//! passhound-core: vault, encryption, schema, repos, importer, recovery.

pub mod crypto;
pub mod error;
pub mod importer;
pub mod recovery;
pub mod repo;
pub mod schema;
pub mod vault;

pub use error::{Error, Result};
pub use vault::Vault;
pub use recovery::{
    extract_base_words_from_history, recover, AnalyzeReport, Candidate, RecoverConfig, RuleId,
};
