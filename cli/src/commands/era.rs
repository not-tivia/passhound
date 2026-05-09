use anyhow::{Context, Result};
use chrono::NaiveDate;
use passhound_core::repo::eras;
use passhound_core::Vault;
use std::path::Path;

#[derive(clap::Args)]
pub struct EraArgs {
    #[command(subcommand)]
    pub command: EraCommand,
}

#[derive(clap::Subcommand)]
pub enum EraCommand {
    /// List all defined eras.
    List,
    /// Define a new era.
    Add(AddArgs),
}

#[derive(clap::Args)]
pub struct AddArgs {
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub start: Option<String>,
    #[arg(long)]
    pub end: Option<String>,
    #[arg(long)]
    pub notes: Option<String>,
}

pub fn run(path: &Path, args: EraArgs) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("vault not found at {}", path.display());
    }
    let pw = rpassword::prompt_password("Master password: ")?;
    let mut vault = Vault::open(path)?;
    vault.unlock(pw.as_bytes()).context("unlock failed")?;

    match args.command {
        EraCommand::List => list(&vault),
        EraCommand::Add(a) => {
            let start = a.start.as_deref().map(parse_date).transpose()?;
            let end = a.end.as_deref().map(parse_date).transpose()?;
            let id = eras::add(&vault, &a.name, start, end, a.notes.as_deref())?;
            println!("Added era '{}' as #{id}.", a.name);
            Ok(())
        }
    }
}

fn parse_date(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|e| anyhow::anyhow!("invalid date '{s}' (expected YYYY-MM-DD): {e}"))
}

fn list(vault: &Vault) -> Result<()> {
    let rows = eras::list(vault)?;
    if rows.is_empty() {
        println!("(no eras defined)");
        return Ok(());
    }
    println!("{:<5} {:<24} {:<12} {:<12} NOTES", "ID", "NAME", "START", "END");
    for e in rows {
        println!("{:<5} {:<24} {:<12} {:<12} {}",
            e.id,
            e.name,
            e.start_date.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_else(|| "-".into()),
            e.end_date.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_else(|| "-".into()),
            e.notes.as_deref().unwrap_or(""),
        );
    }
    Ok(())
}
