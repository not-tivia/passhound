use anyhow::{Context, Result};
use passhound_core::repo::{accounts, sites};
use passhound_core::Vault;
use std::path::Path;

pub fn run(path: &Path) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("vault not found at {}", path.display());
    }
    let pw = rpassword::prompt_password("Master password: ")?;
    let mut vault = Vault::open(path)?;
    vault.unlock(pw.as_bytes()).context("unlock failed")?;

    let mut printed_any = false;
    for site in sites::list(&vault)? {
        let accs = accounts::list_for_site(&vault, site.id, &[])?;
        if accs.is_empty() { continue; }
        printed_any = true;
        println!("{} [{}]", site.name, site.category.as_deref().unwrap_or("-"));
        for a in accs {
            let user = a.username.as_deref().unwrap_or("-");
            let alias = a.alias.as_deref().unwrap_or("-");
            println!("  #{:<4} {} (alias: {})", a.id, user, alias);
        }
    }
    if !printed_any { println!("(empty vault)"); }
    Ok(())
}
