use anyhow::{Context, Result};
use passhound_core::Vault;
use std::path::Path;

pub fn run(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("create vault dir")?;
    }
    if path.exists() {
        anyhow::bail!("vault already exists at {}", path.display());
    }
    let pw = rpassword::prompt_password("New master password: ")?;
    let pw2 = rpassword::prompt_password("Confirm master password: ")?;
    if pw != pw2 {
        anyhow::bail!("passwords do not match");
    }
    if pw.len() < 8 {
        anyhow::bail!("master password must be at least 8 characters");
    }
    let _vault = Vault::create(path, pw.as_bytes())?;
    println!("Vault created at {}", path.display());
    Ok(())
}
