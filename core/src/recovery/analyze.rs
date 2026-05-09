use crate::error::{Error, Result};
use crate::repo::base_words::{self, AggregatedToken};
use crate::repo::passwords;
use crate::repo::accounts;
use crate::repo::sites;
use crate::vault::Vault;
use chrono::{DateTime, Utc};
use rusqlite::params;
use std::collections::HashMap;
use zeroize::Zeroizing;

#[derive(Debug, Default)]
pub struct AnalyzeReport {
    pub tokens_seen: usize,
    pub base_words_written: usize,
    pub favorites_set: usize,
}

/// Tokenize every password in password_history (current AND retired), aggregate counts,
/// upsert into base_words, and refresh auto-favorites (preserving manual_override).
///
/// Vault must be unlocked.
pub fn extract_base_words_from_history(vault: &Vault, top_favorites: usize) -> Result<AnalyzeReport> {
    // 1. SELECT every password_history row (id + decrypted plaintext + created_at).
    let history = decrypt_all_history(vault)?;
    if history.is_empty() {
        return Ok(AnalyzeReport::default());
    }

    // 2. Tokenize each plaintext, aggregate {word -> count, first_seen, last_seen}.
    let mut agg: HashMap<String, Aggregate> = HashMap::new();
    for (created_at, plaintext) in &history {
        for token in tokenize(plaintext.as_str()) {
            let entry = agg.entry(token.canonical).or_insert(Aggregate {
                count: 0,
                first_seen: *created_at,
                last_seen: *created_at,
                casing_mask: token.casing_mask,
            });
            entry.count += 1;
            if *created_at < entry.first_seen { entry.first_seen = *created_at; }
            if *created_at > entry.last_seen  { entry.last_seen  = *created_at; }
        }
    }

    let tokens_seen = agg.len();

    // 3. Upsert each token in a single transaction. base_words::upsert_aggregated reads
    //    fetch_decrypted on every call which is O(n_words) — for analyze, we instead
    //    inline a faster path: pre-fetch once, then use direct INSERT/UPDATE.
    let existing = base_words::fetch_decrypted(vault)?;
    let mut existing_by_word: HashMap<String, i64> = existing.iter()
        .map(|w| (w.word.as_str().to_string(), w.id))
        .collect();

    let tx = vault.conn().unchecked_transaction()?;
    let mut written = 0usize;
    for (word, ag) in &agg {
        if let Some(&id) = existing_by_word.get(word) {
            tx.execute(
                "UPDATE base_words SET usage_count = ?1, last_seen_at = ?2 WHERE id = ?3",
                params![ag.count as i64, ag.last_seen.to_rfc3339(), id],
            )?;
        } else {
            // Insert via the repo helper which encrypts under the vault key.
            // We can't go through upsert_aggregated inside this tx because it does
            // its own fetch_decrypted; replicate the insert path directly.
            insert_encrypted(vault, &tx, word, ag)?;
            existing_by_word.insert(word.clone(), -1); // mark as inserted
        }
        written += 1;
    }
    tx.commit()?;

    // 4. Refresh auto-favorites; preserves manual_override.
    base_words::refresh_auto_favorites(vault, top_favorites)?;

    let favorites_set = base_words::list(vault)?.into_iter().filter(|b| b.is_favorite).count();

    Ok(AnalyzeReport {
        tokens_seen,
        base_words_written: written,
        favorites_set,
    })
}

struct Aggregate {
    count: usize,
    first_seen: DateTime<Utc>,
    last_seen: DateTime<Utc>,
    casing_mask: u64,
}

fn insert_encrypted(
    vault: &Vault,
    tx: &rusqlite::Transaction<'_>,
    word: &str,
    ag: &Aggregate,
) -> Result<()> {
    use crate::crypto::aead;
    let key = vault.require_key()?;
    let (ct, nonce) = aead::encrypt(key.as_bytes(), word.as_bytes())?;
    tx.execute(
        "INSERT INTO base_words (word_encrypted, word_nonce, is_favorite, first_seen_at, last_seen_at, usage_count, manual_override)
         VALUES (?1, ?2, 0, ?3, ?4, ?5, 0)",
        params![
            ct,
            nonce.as_slice(),
            ag.first_seen.to_rfc3339(),
            ag.last_seen.to_rfc3339(),
            ag.count as i64,
        ],
    )?;
    Ok(())
}

fn decrypt_all_history(vault: &Vault) -> Result<Vec<(DateTime<Utc>, Zeroizing<String>)>> {
    // Iterate every account, every history row, decrypt. Reuses passwords::decrypt_record.
    let sites = sites::list(vault)?;
    let mut out: Vec<(DateTime<Utc>, Zeroizing<String>)> = Vec::new();
    for s in sites {
        let accs = accounts::list_for_site(vault, s.id)?;
        for a in accs {
            let history = passwords::list_history(vault, a.id)?;
            for rec in history {
                let pt = passwords::decrypt_record(vault, rec.id)?;
                out.push((rec.created_at, pt));
            }
        }
    }
    Ok(out)
}

/// One tokenized base-word candidate plus its first-seen casing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// Lowercase canonical form, used as the dedup key in analyze and the
    /// stored ciphertext in base_words.
    pub canonical: String,
    /// Bitmask: bit `i` = 1 if char `i` of `canonical` is uppercase in the
    /// password fragment that produced this token. See
    /// `repo::base_words::apply_casing_mask` for reconstruction.
    pub casing_mask: u64,
}

/// Tokenize a password into candidate base words plus first-seen casing.
/// Rules: insert spaces at camelCase boundaries, replace digits and symbols
/// with spaces, keep alphabetic tokens with len in [4, 24]. The original
/// casing of each token segment (before lowercasing) becomes its bitmask.
pub fn tokenize(password: &str) -> Vec<Token> {
    // Step 1: insert a space at every camelCase boundary so MoonBeam → "Moon Beam".
    let mut camel_split = String::with_capacity(password.len() + 4);
    let chars: Vec<char> = password.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && c.is_ascii_uppercase() && chars[i - 1].is_ascii_lowercase() {
            camel_split.push(' ');
        }
        camel_split.push(*c);
    }
    // Step 2: replace every non-alphabetic char with a space (preserving case).
    let cleaned: String = camel_split
        .chars()
        .map(|c| if c.is_ascii_alphabetic() { c } else { ' ' })
        .collect();
    // Step 3: emit each whitespace-separated segment as a Token.
    cleaned
        .split_whitespace()
        .filter(|t| t.chars().count() >= 4 && t.chars().count() <= 24)
        .map(|original| {
            let canonical: String = original.chars().flat_map(|c| c.to_lowercase()).collect();
            let casing_mask = crate::repo::base_words::compute_casing_mask(original);
            Token { canonical, casing_mask }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::accounts::{self, NewAccount};
    use crate::repo::sites::{self, NewSite};
    use tempfile::TempDir;

    fn vault() -> (TempDir, Vault) {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("v.db");
        let v = Vault::create(&path, b"hunter2").unwrap();
        (tmp, v)
    }

    #[test]
    fn tokenize_basics() {
        let canonicals = |s: &str| -> Vec<String> {
            tokenize(s).into_iter().map(|t| t.canonical).collect()
        };
        assert_eq!(canonicals("Fluffy!2014"), vec!["fluffy"]);
        assert_eq!(canonicals("MoonBeam$2018"), vec!["moon", "beam"]);
        assert_eq!(canonicals("hi!"), Vec::<String>::new(), "tokens shorter than 4 are dropped");
        assert_eq!(canonicals("Thunder!@#2020"), vec!["thunder"]);
        assert_eq!(canonicals("snake_case_word"), vec!["snake", "case", "word"]);
    }

    #[test]
    fn tokenize_drops_pure_digits_and_short_tokens() {
        let canonicals: Vec<String> = tokenize("a 12345 abcd").into_iter().map(|t| t.canonical).collect();
        assert_eq!(canonicals, vec!["abcd"]);
    }

    #[test]
    fn tokenize_captures_casing_mask() {
        let toks = tokenize("MoonBeam$2018");
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[0].canonical, "moon");
        assert_eq!(toks[0].casing_mask, 0b0001, "first char of 'Moon' is upper");
        assert_eq!(toks[1].canonical, "beam");
        assert_eq!(toks[1].casing_mask, 0b0001, "first char of 'Beam' is upper");
    }

    #[test]
    fn tokenize_lowercase_input_yields_zero_mask() {
        let toks = tokenize("fluffy!2014");
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].canonical, "fluffy");
        assert_eq!(toks[0].casing_mask, 0);
    }

    #[test]
    fn tokenize_uppercase_input_yields_full_mask() {
        let toks = tokenize("FLUFFY!2014");
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].canonical, "fluffy");
        assert_eq!(toks[0].casing_mask, 0b00111111);
    }

    #[test]
    fn analyze_empty_vault_returns_default() {
        let (_t, v) = vault();
        let report = extract_base_words_from_history(&v, 5).unwrap();
        assert_eq!(report.tokens_seen, 0);
        assert_eq!(report.base_words_written, 0);
    }

    #[test]
    fn analyze_extracts_and_aggregates() {
        let (_t, v) = vault();
        let s = sites::create(&v, NewSite { name: "S".into(), ..Default::default() }).unwrap();
        let a = accounts::create(&v, NewAccount { site_id: s.id, ..Default::default() }).unwrap();
        crate::repo::passwords::insert(&v, crate::repo::passwords::NewPassword {
            account_id: a.id,
            plaintext: "Fluffy!2010",
            source: "manual".into(),
            confidence: crate::repo::passwords::Confidence::Certain,
            notes: None,
            created_at: None,
        }).unwrap();
        crate::repo::passwords::insert(&v, crate::repo::passwords::NewPassword {
            account_id: a.id,
            plaintext: "Fluffy!2014",
            source: "manual".into(),
            confidence: crate::repo::passwords::Confidence::Certain,
            notes: None,
            created_at: None,
        }).unwrap();
        let report = extract_base_words_from_history(&v, 1).unwrap();
        assert!(report.tokens_seen >= 1);
        assert_eq!(report.favorites_set, 1, "top-1 favorite expected");
        let words = base_words::fetch_decrypted(&v).unwrap();
        let fluffy = words.iter().find(|w| w.word.as_str() == "fluffy").unwrap();
        assert_eq!(fluffy.usage_count, 2);
    }

    #[test]
    fn analyze_preserves_manual_override() {
        let (_t, v) = vault();
        let s = sites::create(&v, NewSite { name: "S".into(), ..Default::default() }).unwrap();
        let a = accounts::create(&v, NewAccount { site_id: s.id, ..Default::default() }).unwrap();
        // Insert with multiple distinct base words; ensure top-1 picks the most-used.
        for pw in &["Apple!2020", "Apple!2021", "Apple!2022", "Banana!2023"] {
            crate::repo::passwords::insert(&v, crate::repo::passwords::NewPassword {
                account_id: a.id,
                plaintext: pw,
                source: "manual".into(),
                confidence: crate::repo::passwords::Confidence::Certain,
                notes: None,
                created_at: None,
            }).unwrap();
        }
        // First analyze — top-1 should be "apple".
        extract_base_words_from_history(&v, 1).unwrap();
        let words_before = base_words::list(&v).unwrap();
        let apple_id = base_words::fetch_decrypted(&v).unwrap()
            .iter().find(|w| w.word.as_str() == "apple").unwrap().id;
        let banana_id = base_words::fetch_decrypted(&v).unwrap()
            .iter().find(|w| w.word.as_str() == "banana").unwrap().id;
        // Manually demote apple, manually promote banana.
        base_words::demote(&v, apple_id).unwrap();
        base_words::promote(&v, banana_id).unwrap();
        // Re-run analyze — manual flags must NOT be clobbered.
        extract_base_words_from_history(&v, 1).unwrap();
        let words_after = base_words::list(&v).unwrap();
        let apple_after = words_after.iter().find(|w| w.id == apple_id).unwrap();
        let banana_after = words_after.iter().find(|w| w.id == banana_id).unwrap();
        assert!(!apple_after.is_favorite, "apple manually demoted; analyze must not auto-favorite it");
        assert!(banana_after.is_favorite, "banana manually promoted; analyze must keep favorite");
        let _ = words_before; // silence unused
    }
}
