use crate::crypto::aead::{self, NONCE_LEN};
use crate::error::{Error, Result};
use crate::repo::common;
use crate::vault::Vault;
use chrono::{DateTime, Utc};
use rusqlite::params;
use zeroize::Zeroizing;

#[derive(Debug, Clone)]
pub struct BaseWord {
    pub id: i64,
    pub is_favorite: bool,
    pub manual_override: bool,
    pub first_seen_at: Option<DateTime<Utc>>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub usage_count: i64,
    pub casing_mask: u64,
}

#[derive(Debug, Clone)]
pub struct DecryptedBaseWord {
    pub id: i64,
    pub word: Zeroizing<String>,
    pub is_favorite: bool,
    pub usage_count: i64,
    pub casing_mask: u64,
}

/// Aggregated counts for a single token, fed into `upsert_aggregated`.
pub struct AggregatedToken<'a> {
    pub word: &'a str,
    pub usage_count: i64,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub casing_mask: u64,
}

/// List metadata for every base word. Does NOT decrypt; returns counts only.
pub fn list(vault: &Vault) -> Result<Vec<BaseWord>> {
    let mut stmt = vault.conn().prepare(
        "SELECT id, is_favorite, manual_override, first_seen_at, last_seen_at, usage_count, casing_mask
         FROM base_words ORDER BY usage_count DESC, id ASC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            let first: Option<String> = r.get(3)?;
            let last: Option<String> = r.get(4)?;
            Ok(BaseWord {
                id: r.get(0)?,
                is_favorite: r.get::<_, i64>(1)? != 0,
                manual_override: r.get::<_, i64>(2)? != 0,
                first_seen_at: first.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|d| d.with_timezone(&Utc))),
                last_seen_at: last.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|d| d.with_timezone(&Utc))),
                usage_count: r.get(5)?,
                casing_mask: r.get::<_, i64>(6)? as u64,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Manually mark a base word as a favorite. Sets manual_override so re-runs of analyze
/// will not clobber this setting.
pub fn promote(vault: &Vault, id: i64) -> Result<()> {
    let n = vault.conn().execute(
        "UPDATE base_words SET is_favorite = 1, manual_override = 1 WHERE id = ?1",
        params![id],
    )?;
    common::ensure_affected(n)
}

/// Manually mark a base word as NOT a favorite. Same manual_override semantics as promote.
pub fn demote(vault: &Vault, id: i64) -> Result<()> {
    let n = vault.conn().execute(
        "UPDATE base_words SET is_favorite = 0, manual_override = 1 WHERE id = ?1",
        params![id],
    )?;
    common::ensure_affected(n)
}

/// Manually add a base word to the favorites pool. Encrypts under the vault key,
/// inserts a row with is_favorite=1, manual_override=1, usage_count=0, and
/// a casing_mask derived from the typed text. Returns the new BaseWord.
pub fn manual_insert(vault: &Vault, text: &str) -> Result<BaseWord> {
    let key = vault.require_key()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(Error::InvalidInput("base word must not be empty".into()));
    }
    if trimmed.len() > 64 {
        return Err(Error::InvalidInput("base word must be 64 chars or fewer".into()));
    }
    let canonical = trimmed.to_lowercase();
    // Reject duplicates by decrypted-word equality (mirrors upsert_aggregated's dedup).
    let existing = fetch_decrypted(vault)?;
    if existing.iter().any(|w| w.word.as_str() == canonical) {
        return Err(Error::AlreadyExists);
    }
    let casing_mask = compute_casing_mask(trimmed);
    let now = Utc::now();
    let (ct, nonce) = aead::encrypt(key.as_bytes(), canonical.as_bytes())?;
    vault.conn().execute(
        "INSERT INTO base_words (word_encrypted, word_nonce, is_favorite,
         first_seen_at, last_seen_at, usage_count, manual_override, casing_mask)
         VALUES (?1, ?2, 1, ?3, ?4, 0, 1, ?5)",
        params![
            ct,
            nonce.as_slice(),
            now.to_rfc3339(),
            now.to_rfc3339(),
            casing_mask as i64,
        ],
    )?;
    let id = vault.conn().last_insert_rowid();
    Ok(BaseWord {
        id,
        is_favorite: true,
        manual_override: true,
        first_seen_at: Some(now),
        last_seen_at: Some(now),
        usage_count: 0,
        casing_mask,
    })
}

/// Upsert a token into the base_words table. Encrypts the word under the vault key.
/// Existing rows (matched by decrypted-word equality) get usage_count overwritten and
/// last_seen_at updated. The caller (analyze) is responsible for transaction boundaries.
pub fn upsert_aggregated(vault: &Vault, tok: AggregatedToken<'_>) -> Result<()> {
    let key = vault.require_key()?;
    let existing = fetch_decrypted(vault)?;
    if let Some(found) = existing.iter().find(|w| w.word.as_str() == tok.word) {
        vault.conn().execute(
            "UPDATE base_words SET usage_count = ?1, last_seen_at = ?2 WHERE id = ?3",
            params![tok.usage_count, tok.last_seen_at.to_rfc3339(), found.id],
        )?;
        return Ok(());
    }
    let (ct, nonce) = aead::encrypt(key.as_bytes(), tok.word.as_bytes())?;
    vault.conn().execute(
        "INSERT INTO base_words (word_encrypted, word_nonce, is_favorite, first_seen_at, last_seen_at, usage_count, casing_mask)
         VALUES (?1, ?2, 0, ?3, ?4, ?5, ?6)",
        params![
            ct,
            nonce.as_slice(),
            tok.first_seen_at.to_rfc3339(),
            tok.last_seen_at.to_rfc3339(),
            tok.usage_count,
            tok.casing_mask as i64,
        ],
    )?;
    Ok(())
}

/// Set is_favorite for the given ids without touching manual_override. Used by analyze
/// after upserting tokens — sets favorites to the top-N USAGE rows EXCEPT those with
/// manual_override=1 (whose flag is preserved).
pub fn refresh_auto_favorites(vault: &Vault, top_n: usize) -> Result<()> {
    // Reset auto favorites only.
    vault.conn().execute(
        "UPDATE base_words SET is_favorite = 0 WHERE manual_override = 0",
        params![],
    )?;
    // Set top_n by usage (excluding manual rows).
    vault.conn().execute(
        "UPDATE base_words SET is_favorite = 1
         WHERE manual_override = 0
           AND id IN (
             SELECT id FROM base_words
             WHERE manual_override = 0
             ORDER BY usage_count DESC, id ASC
             LIMIT ?1
           )",
        params![top_n as i64],
    )?;
    Ok(())
}

/// Decrypt every base_words row. Vault must be unlocked.
pub fn fetch_decrypted(vault: &Vault) -> Result<Vec<DecryptedBaseWord>> {
    let key = vault.require_key()?;
    let mut stmt = vault.conn().prepare(
        "SELECT id, word_encrypted, word_nonce, is_favorite, usage_count, casing_mask FROM base_words ORDER BY usage_count DESC, id ASC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Vec<u8>>(1)?,
            r.get::<_, Vec<u8>>(2)?,
            r.get::<_, i64>(3)? != 0,
            r.get::<_, i64>(4)?,
            r.get::<_, i64>(5)? as u64,
        ))
    })?.collect::<std::result::Result<Vec<_>, _>>()?;

    let mut out = Vec::with_capacity(rows.len());
    for (id, ct, nonce_vec, is_favorite, usage_count, casing_mask) in rows {
        if nonce_vec.len() != NONCE_LEN {
            return Err(Error::InvalidInput("malformed base_word nonce".into()));
        }
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&nonce_vec);
        let pt = Zeroizing::new(aead::decrypt(key.as_bytes(), &ct, &nonce)?);
        let s = std::str::from_utf8(&pt).map_err(|_| Error::InvalidInput("non-utf8 base word".into()))?;
        out.push(DecryptedBaseWord {
            id,
            word: Zeroizing::new(s.to_owned()),
            is_favorite,
            usage_count,
            casing_mask,
        });
    }
    Ok(out)
}

/// Reconstruct the original-cased form of a lowercase canonical word using a
/// bitmask. Bit `i` of `mask` corresponds to position `i` (0-indexed) of
/// `canonical`: bit set means that character is uppercase in the original.
///
/// Examples:
/// - canonical="moonbeam", mask=0b00010001 (=17) → "MoonBeam"
///   (bit 0 set → upper M, bit 4 set → upper B)
/// - canonical="iphone",   mask=0b00000010 (=2)  → "iPhone"
///   (bit 1 set → upper P)
/// - canonical="hello",    mask=0                → "hello"
///
/// Words shorter than 64 chars use the low bits; bits beyond the word length
/// are ignored. Tokenize already caps tokens at 24 chars, well under 64.
pub fn apply_casing_mask(canonical: &str, mask: u64) -> String {
    let mut out = String::with_capacity(canonical.len());
    for (i, ch) in canonical.chars().enumerate() {
        if i < 64 && (mask >> i) & 1 == 1 {
            out.extend(ch.to_uppercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Compute the casing bitmask of `original` against its lowercase canonical.
/// Bit `i` is set if `original`'s character at position `i` is uppercase.
/// Characters at positions >= 64 do not contribute to the mask.
///
/// Examples:
/// - "MoonBeam" → 0b00010001 (= 17)
/// - "iPhone"   → 0b00000010 (= 2)
/// - "hello"    → 0
pub fn compute_casing_mask(original: &str) -> u64 {
    let mut mask: u64 = 0;
    for (i, ch) in original.chars().enumerate() {
        if i >= 64 { break; }
        if ch.is_uppercase() {
            mask |= 1u64 << i;
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn vault() -> (TempDir, Vault) {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("v.db");
        let v = Vault::create(&path, b"hunter2").unwrap();
        (tmp, v)
    }

    #[test]
    fn upsert_inserts_then_updates() {
        let (_t, v) = vault();
        let now = Utc::now();
        upsert_aggregated(&v, AggregatedToken { word: "fluffy", usage_count: 3, first_seen_at: now, last_seen_at: now, casing_mask: 0 }).unwrap();
        upsert_aggregated(&v, AggregatedToken { word: "fluffy", usage_count: 5, first_seen_at: now, last_seen_at: now, casing_mask: 0 }).unwrap();
        let words = fetch_decrypted(&v).unwrap();
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].word.as_str(), "fluffy");
        assert_eq!(words[0].usage_count, 5);
    }

    #[test]
    fn promote_sets_favorite_and_manual_override() {
        let (_t, v) = vault();
        let now = Utc::now();
        upsert_aggregated(&v, AggregatedToken { word: "moonbeam", usage_count: 1, first_seen_at: now, last_seen_at: now, casing_mask: 0 }).unwrap();
        let id = list(&v).unwrap()[0].id;
        promote(&v, id).unwrap();
        let row = list(&v).unwrap().into_iter().find(|w| w.id == id).unwrap();
        assert!(row.is_favorite);
        assert!(row.manual_override);
    }

    #[test]
    fn refresh_auto_favorites_preserves_manual_overrides() {
        let (_t, v) = vault();
        let now = Utc::now();
        for (w, c) in [("a", 10), ("b", 9), ("c", 8), ("d", 1)] {
            upsert_aggregated(&v, AggregatedToken { word: w, usage_count: c, first_seen_at: now, last_seen_at: now, casing_mask: 0 }).unwrap();
        }
        // Manually demote 'a' (which would otherwise be favorited by top-2).
        let a_id = fetch_decrypted(&v).unwrap().iter().find(|x| x.word.as_str() == "a").unwrap().id;
        demote(&v, a_id).unwrap();
        // Manually promote 'd' (which would NOT be favorited by top-2).
        let d_id = fetch_decrypted(&v).unwrap().iter().find(|x| x.word.as_str() == "d").unwrap().id;
        promote(&v, d_id).unwrap();
        // Now run auto-favorite top-2.
        refresh_auto_favorites(&v, 2).unwrap();
        let rows = list(&v).unwrap();
        let a = rows.iter().find(|r| r.id == a_id).unwrap();
        let d = rows.iter().find(|r| r.id == d_id).unwrap();
        // Manual overrides preserved:
        assert!(!a.is_favorite, "a was manually demoted");
        assert!(d.is_favorite, "d was manually promoted");
        // 'b' and 'c' should be auto-favorites (top 2 by usage among manual_override=0 rows).
        let b = rows.iter().find(|r| !r.manual_override && r.usage_count == 9).unwrap();
        let c = rows.iter().find(|r| !r.manual_override && r.usage_count == 8).unwrap();
        assert!(b.is_favorite);
        assert!(c.is_favorite);
    }

    #[test]
    fn promote_unknown_id_returns_not_found() {
        let (_t, v) = vault();
        let err = promote(&v, 9999).unwrap_err();
        assert!(matches!(err, Error::NotFound));
    }

    #[test]
    fn apply_casing_mask_basic() {
        assert_eq!(apply_casing_mask("moonbeam", 0b00010001), "MoonBeam");
        assert_eq!(apply_casing_mask("iphone", 0b00000010), "iPhone");
        assert_eq!(apply_casing_mask("hello", 0), "hello");
        assert_eq!(apply_casing_mask("abc", 0b00000111), "ABC");
    }

    #[test]
    fn compute_casing_mask_basic() {
        assert_eq!(compute_casing_mask("MoonBeam"), 0b00010001);
        assert_eq!(compute_casing_mask("iPhone"), 0b00000010);
        assert_eq!(compute_casing_mask("hello"), 0);
        assert_eq!(compute_casing_mask("ABC"), 0b00000111);
    }

    #[test]
    fn casing_mask_round_trip() {
        for word in &["MoonBeam", "iPhone", "hello", "ABC", "fLuFfY"] {
            let canonical: String = word.chars().flat_map(|c| c.to_lowercase()).collect();
            let mask = compute_casing_mask(word);
            let roundtrip = apply_casing_mask(&canonical, mask);
            assert_eq!(&roundtrip, *word, "round trip failed for '{word}'");
        }
    }

    #[test]
    fn manual_insert_round_trip() {
        let (_tmp, v) = vault();
        let new = manual_insert(&v, "MoonBeam").unwrap();
        assert!(new.is_favorite);
        assert!(new.manual_override);
        assert_eq!(new.usage_count, 0);
        assert_eq!(new.casing_mask, 0b00010001, "MoonBeam casing mask should match");

        let words = list(&v).unwrap();
        assert!(words.iter().any(|w| w.id == new.id && w.is_favorite));

        let decrypted = fetch_decrypted(&v).unwrap();
        assert!(decrypted.iter().any(|w| w.word.as_str() == "moonbeam"));
    }

    #[test]
    fn manual_insert_rejects_duplicate() {
        let (_tmp, v) = vault();
        manual_insert(&v, "MoonBeam").unwrap();
        let err = manual_insert(&v, "moonbeam").unwrap_err();
        assert!(matches!(err, Error::AlreadyExists));
    }

    #[test]
    fn manual_insert_rejects_empty() {
        let (_tmp, v) = vault();
        let err = manual_insert(&v, "   ").unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[test]
    fn upsert_aggregated_persists_casing_mask() {
        let (_t, v) = vault();
        let now = Utc::now();
        upsert_aggregated(&v, AggregatedToken {
            word: "moonbeam",
            usage_count: 1,
            first_seen_at: now,
            last_seen_at: now,
            casing_mask: 0b00010001,
        }).unwrap();
        let words = fetch_decrypted(&v).unwrap();
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].word.as_str(), "moonbeam");
        assert_eq!(words[0].casing_mask, 0b00010001);
        assert_eq!(apply_casing_mask(words[0].word.as_str(), words[0].casing_mask), "MoonBeam");
    }
}
