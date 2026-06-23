//! `audit` — local diagnostic that flags current passwords which look like
//! site-affixed copies of ANOTHER stored password (the pollution pattern behind
//! recovery surfacing `RGIm2iquit`-style "HIST" candidates). Runs entirely
//! locally; prints nothing to any network. Plaintext is shown on YOUR terminal
//! only so you can eyeball the junk.

use anyhow::{Context, Result};
use passhound_core::crypto::aead::{self, NONCE_LEN};
use passhound_core::recovery::RuleId;
use passhound_core::{recover, RecoverConfig, Vault};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

struct Row {
    pw: String,
    import: Option<i64>,
    account_id: i64,
    username: String,
    site: String,
}

pub fn run(path: &Path) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("vault not found at {}", path.display());
    }
    let pw = zeroize::Zeroizing::new(rpassword::prompt_password("Master password: ")?);
    let mut vault = Vault::open(path)?;
    vault.unlock(pw.as_bytes()).context("unlock failed")?;
    let key = vault.require_key()?;

    // All CURRENT passwords (retired_at IS NULL) — exactly the rows that become
    // recovery seeds — with their account/site/import metadata.
    let mut stmt = vault.conn().prepare(
        "SELECT ph.password_encrypted, ph.password_nonce, ph.source_import_id,
                a.id, a.username, s.name
         FROM password_history ph
         JOIN accounts a ON a.id = ph.account_id
         JOIN sites s ON s.id = a.site_id
         WHERE ph.retired_at IS NULL",
    )?;
    let rows: Vec<Row> = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, Vec<u8>>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?
        .filter_map(|x| x.ok())
        .filter_map(|(ct, nv, import, account_id, username, site)| {
            if nv.len() != NONCE_LEN {
                return None;
            }
            let mut nonce = [0u8; NONCE_LEN];
            nonce.copy_from_slice(&nv);
            let pt = aead::decrypt(key.as_bytes(), &ct, &nonce).ok()?;
            let s = String::from_utf8_lossy(&pt).to_string();
            Some(Row { pw: s, import, account_id, username: username.unwrap_or_default(), site })
        })
        .collect();

    // Set of all current plaintexts (lowercased) for "is this an affix of another?" lookups.
    let set: HashSet<String> = rows.iter().map(|r| r.pw.to_lowercase()).collect();

    // A password is flagged if removing a short (2-4 char) ASCII-alphabetic
    // prefix OR suffix yields ANOTHER stored current password of length >= 3.
    // That is exactly "base + site-tag" / "site-tag + base" — the generated shape.
    let reduces_to = |p: &str| -> Option<String> {
        let pl = p.to_lowercase();
        if !pl.is_ascii() {
            return None;
        }
        for n in 2..=4 {
            if pl.len() <= n + 2 {
                continue;
            }
            let pre = &pl[..n];
            let rest = &pl[n..];
            if pre.bytes().all(|b| b.is_ascii_alphabetic()) && rest.len() >= 3 && set.contains(rest) {
                return Some(rest.to_string());
            }
            let suf = &pl[pl.len() - n..];
            let rest = &pl[..pl.len() - n];
            if suf.bytes().all(|b| b.is_ascii_alphabetic()) && rest.len() >= 3 && set.contains(rest) {
                return Some(rest.to_string());
            }
        }
        None
    };

    let mut flagged: Vec<(&Row, String)> = Vec::new();
    for r in &rows {
        if let Some(base) = reduces_to(&r.pw) {
            flagged.push((r, base));
        }
    }

    println!("Total current passwords: {}", rows.len());
    println!(
        "Flagged as site-affix copies of another stored password: {}",
        flagged.len()
    );

    let mut by_import: BTreeMap<String, usize> = BTreeMap::new();
    for (r, _) in &flagged {
        let k = r.import.map(|i| format!("import {i}")).unwrap_or_else(|| "manual/none".into());
        *by_import.entry(k).or_default() += 1;
    }
    println!("Flagged by import: {by_import:?}");

    println!("\nExamples (stored password  ->  reduces to base that is ALSO stored):");
    for (r, base) in flagged.iter().take(40) {
        println!(
            "  acct#{:<5} site={:<16} user={:<16} import={:<3}  {}  ->  {}",
            r.account_id,
            truncate(&r.site, 16),
            truncate(&r.username, 16),
            r.import.map(|i| i.to_string()).unwrap_or_else(|| "-".into()),
            r.pw,
            base
        );
    }
    if flagged.len() > 40 {
        println!("  ... and {} more", flagged.len() - 40);
    }

    // --- Recovery cross-check: the actual bug detector. -------------------
    // Run the recovery engine and count candidates tagged HistorySeed ("HIST")
    // whose plaintext is NOT in the stored current-password set. A HistorySeed
    // tag claims "this is a verbatim stored password"; if the plaintext isn't
    // actually stored, the engine fabricated it (the bug). Prints counts only.
    println!("\n=== Recovery cross-check (the bug detector) ===");
    for site in [None, Some("Runescape".to_string())] {
        let label = site.clone().unwrap_or_else(|| "(no site / broad)".into());
        let cfg = RecoverConfig { site, limit: 500, ..Default::default() };
        match recover(&vault, cfg) {
            Ok(cands) => {
                let mut hist_total = 0usize;
                let mut hist_fabricated = 0usize;
                for c in &cands {
                    if c.provenance.contains(&RuleId::HistorySeed) {
                        hist_total += 1;
                        if !set.contains(&c.password.as_str().to_lowercase()) {
                            hist_fabricated += 1;
                        }
                    }
                }
                println!(
                    "  site={label:<22} candidates={:<4} HIST-tagged={hist_total:<4} HIST-but-NOT-stored(fabricated)={hist_fabricated}",
                    cands.len()
                );
            }
            Err(e) => println!("  site={label:<22} recover error: {e}"),
        }
    }
    println!("  (fabricated > 0  => engine bug confirmed;  fabricated = 0  => HIST tags are all real, issue is elsewhere)");
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n.saturating_sub(1)).collect::<String>() + "\u{2026}"
    }
}
