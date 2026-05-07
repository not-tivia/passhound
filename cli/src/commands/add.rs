use anyhow::{Context, Result};
use passhound_core::repo::{accounts::{self, NewAccount}, passwords::{self, NewPassword, Confidence}, sites::{self, NewSite}};
use passhound_core::Vault;
use std::path::Path;

#[derive(clap::Args)]
pub struct AddArgs {
    /// Site name (e.g., "RuneScape"). If a site with this name exists, it's reused.
    #[arg(long)]
    pub site: String,

    /// Optional URL.
    #[arg(long)]
    pub url: Option<String>,

    /// Optional category (Gaming, Banking, etc.).
    #[arg(long)]
    pub category: Option<String>,

    /// Username/email/handle for the account.
    #[arg(long)]
    pub username: Option<String>,

    /// Optional alias for the account ("main", "alt").
    #[arg(long)]
    pub alias: Option<String>,
}

pub fn run(path: &Path, args: AddArgs) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("vault not found at {}; run `init` first", path.display());
    }
    let pw = rpassword::prompt_password("Master password: ")?;
    let mut vault = Vault::open(path)?;
    vault.unlock(pw.as_bytes()).context("unlock failed (wrong master password?)")?;

    let secret = rpassword::prompt_password(format!("Password for {}: ", args.site))?;
    if secret.is_empty() {
        anyhow::bail!("password must not be empty");
    }

    // Find-or-create site by name (case-sensitive exact match).
    let site_id = sites::list(&vault)?
        .into_iter()
        .find(|s| s.name == args.site)
        .map(|s| s.id);
    let site_id = match site_id {
        Some(id) => id,
        None => sites::create(&vault, NewSite {
            name: args.site.clone(),
            url: args.url,
            category: args.category,
            abbreviations: vec![],
            notes: None,
        })?.id,
    };

    let account = accounts::create(&vault, NewAccount {
        site_id,
        username: args.username,
        alias: args.alias,
        notes: None,
    })?;
    passwords::insert(&vault, NewPassword {
        account_id: account.id,
        plaintext: &secret,
        source: "manual".into(),
        confidence: Confidence::Certain,
        notes: None,
        created_at: None,
    })?;
    println!("Added password for site '{}' (account #{}).", args.site, account.id);
    Ok(())
}
