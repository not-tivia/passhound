use anyhow::{Context, Result};
use passhound_core::{extract_base_words_from_history, Vault};
use std::path::Path;

#[derive(clap::Args)]
pub struct AnalyzeArgs {
    /// How many top-usage tokens to auto-mark as favorites.
    #[arg(long, default_value_t = 10)]
    pub top_favorites: usize,
}

pub fn run(path: &Path, args: AnalyzeArgs) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("vault not found at {}", path.display());
    }
    let pw = rpassword::prompt_password("Master password: ")?;
    let mut vault = Vault::open(path)?;
    vault.unlock(pw.as_bytes()).context("unlock failed")?;

    let report = extract_base_words_from_history(&vault, args.top_favorites)?;
    println!(
        "Analyzed: {} unique tokens seen, {} base_word rows written, {} favorites.",
        report.tokens_seen, report.base_words_written, report.favorites_set,
    );
    Ok(())
}
