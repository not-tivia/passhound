use anyhow::{Context, Result};
use passhound_core::repo::passwords;
use passhound_core::Vault;
use std::path::Path;

#[derive(clap::Args)]
pub struct GetArgs {
    /// Account id (find via `list`).
    #[arg(long)]
    pub account: i64,

    /// Print the password to stdout. Default: copy to clipboard would go here, but
    /// for v1 we just print. Future: clipboard integration.
    #[arg(long, default_value_t = true)]
    pub print: bool,
}

pub fn run(path: &Path, args: GetArgs) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("vault not found at {}", path.display());
    }
    let pw = rpassword::prompt_password("Master password: ")?;
    let mut vault = Vault::open(path)?;
    vault.unlock(pw.as_bytes()).context("unlock failed")?;

    let pt = passwords::current_plaintext(&vault, args.account)?
        .ok_or_else(|| anyhow::anyhow!("no current password for account #{}", args.account))?;
    if args.print {
        println!("{}", pt.as_str());
    }
    Ok(())
}
