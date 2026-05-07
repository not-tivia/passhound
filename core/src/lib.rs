//! passhound-core: vault, encryption, schema, repos.

pub mod crypto;
pub mod error;
pub mod schema;
pub mod vault;

pub use error::{Error, Result};
pub use vault::Vault;
