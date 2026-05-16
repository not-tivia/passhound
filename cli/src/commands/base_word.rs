use anyhow::{Context, Result};
use passhound_core::repo::base_words;
use passhound_core::Vault;
use std::path::Path;

#[derive(clap::Args)]
pub struct BaseWordArgs {
    #[command(subcommand)]
    pub command: BaseWordCommand,
}

#[derive(clap::Subcommand)]
pub enum BaseWordCommand {
    /// List all base words sorted by usage count.
    List,
    /// Mark a base word as a favorite (preserved across analyze re-runs).
    Promote(IdArgs),
    /// Mark a base word as NOT a favorite (preserved across analyze re-runs).
    Demote(IdArgs),
}

#[derive(clap::Args)]
pub struct IdArgs {
    pub id: i64,
}

pub fn run(path: &Path, args: BaseWordArgs) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("vault not found at {}", path.display());
    }
    let pw = zeroize::Zeroizing::new(
        rpassword::prompt_password("Master password: ")?
    );
    let mut vault = Vault::open(path)?;
    vault.unlock(pw.as_bytes()).context("unlock failed")?;

    match args.command {
        BaseWordCommand::List => list(&vault),
        BaseWordCommand::Promote(a) => {
            base_words::promote(&vault, a.id)?;
            println!("Promoted #{}.", a.id);
            Ok(())
        }
        BaseWordCommand::Demote(a) => {
            base_words::demote(&vault, a.id)?;
            println!("Demoted #{}.", a.id);
            Ok(())
        }
    }
}

fn list(vault: &Vault) -> Result<()> {
    let rows = base_words::list(vault)?;
    let decrypted = base_words::fetch_decrypted(vault)?;
    println!("{:<5} {:<6} {:<5} {:<5} WORD", "ID", "USAGE", "FAV", "MAN");
    for r in &rows {
        let word = decrypted.iter().find(|w| w.id == r.id).map(|w| w.word.as_str().to_string()).unwrap_or_else(|| "(decrypt failed)".into());
        println!("{:<5} {:<6} {:<5} {:<5} {word}",
            r.id,
            r.usage_count,
            if r.is_favorite { "yes" } else { "no" },
            if r.manual_override { "yes" } else { "no" },
        );
    }
    Ok(())
}
