use anyhow::{Context, Result};
use passhound_core::{recover as core_recover, Candidate, RecoverConfig, RuleId, Vault};
use std::path::Path;

#[derive(clap::Args)]
pub struct RecoverArgs {
    #[arg(long)]
    pub site: Option<String>,
    #[arg(long)]
    pub account: Option<String>,
    #[arg(long)]
    pub era: Option<String>,
    #[arg(long)]
    pub hint: Option<String>,
    #[arg(long, default_value_t = 100)]
    pub limit: usize,
    #[arg(long)]
    pub min_length: Option<usize>,
    #[arg(long, default_value_t = false)]
    pub require_symbol: bool,
    #[arg(long, default_value_t = false)]
    pub require_digit: bool,
}

pub fn run(path: &Path, args: RecoverArgs) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("vault not found at {}", path.display());
    }
    let pw = rpassword::prompt_password("Master password: ")?;
    let mut vault = Vault::open(path)?;
    vault.unlock(pw.as_bytes()).context("unlock failed")?;

    let cfg = RecoverConfig {
        site: args.site,
        account: args.account,
        era_name: args.era,
        hint: args.hint,
        limit: args.limit,
        min_length: args.min_length,
        require_symbol: args.require_symbol,
        require_digit: args.require_digit,
    };

    let candidates = match core_recover(&vault, cfg) {
        Ok(v) => v,
        Err(passhound_core::Error::EmptyVault) => {
            println!("Vault has no history to learn from. Run `passhound import` first.");
            return Ok(());
        }
        Err(passhound_core::Error::EraNotFound(name)) => {
            anyhow::bail!("no era named '{name}'. Run `passhound era list` to see defined eras.");
        }
        Err(e) => return Err(e.into()),
    };

    if candidates.is_empty() {
        println!("No candidates produced. Vault has too little history; run `passhound analyze` after importing more entries.");
        return Ok(());
    }

    print_table(&candidates);
    Ok(())
}

fn print_table(candidates: &[Candidate]) {
    println!("{:<5} {:<6} {:<30} WHY", "RANK", "SCORE", "CANDIDATE");
    for (i, c) in candidates.iter().enumerate() {
        let rank = i + 1;
        let cand_disp = truncate(c.password.as_str(), 30);
        let why = format_why(&c.provenance);
        println!("{rank:<5} {:<6.2} {cand_disp:<30} {why}", c.score);
    }
}

fn format_why(provenance: &[RuleId]) -> String {
    if provenance.is_empty() {
        return "(no rules)".into();
    }
    let tags: Vec<&str> = provenance.iter().map(|r| r.tag()).collect();
    let names: Vec<&str> = provenance.iter().map(|r| r.name()).collect();
    format!("{}: {}", tags.join("+"), names.join(" + "))
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let cut: String = s.chars().take(n.saturating_sub(3)).collect();
        format!("{cut}...")
    }
}
