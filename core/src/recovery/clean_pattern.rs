//! Structural decomposition of a candidate password into recognized segments.
//!
//! Used by `ranking::score` to detect "clean" compositional patterns
//! (e.g. `MoonBeam$2019Rd` — favorite + favorite + symbol + digits + abbrev)
//! and award a structural bonus. A candidate is "clean" when ALL of:
//!   1. The entire password decomposes into a sequence of recognized segments
//!      (favorite base word, digit run, symbol run, or site abbreviation).
//!   2. Segments contain at least one `DigitRun`. Patterns without digits
//!      (e.g. `Thunder!`, `Rdthundermoon!`) are technically composable but
//!      get out-competed by `*!` variants via W_FREQ; the digit-run
//!      requirement gates the bonus to compositions that encode time/era.
//!   3. The last segment is a "natural terminator": Favorite, DigitRun,
//!      Abbrev, OR SymbolRun immediately following a Favorite (so trailing
//!      punctuation past digits or an abbrev is rejected, e.g.
//!      `MoonBeam$2019Rd!` is dirty — last `!` follows an Abbrev).
//!
//! Greedy left-to-right matcher; priority Favorite > Abbrev > DigitRun > SymbolRun
//! at each position. Favorites and abbreviations are matched case-insensitive
//! against `pool.all_base_words` (canonical) and `pool.site_abbreviations`.

use crate::recovery::RecoverContext;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Favorite,
    DigitRun,
    SymbolRun,
    Abbrev,
}

/// Greedy decomposition. Returns `Some(segments)` if the entire password is
/// consumed by recognized segments, else `None`.
pub fn decompose(password: &str, ctx: &RecoverContext<'_>) -> Option<Vec<Segment>> {
    let bytes_lower: Vec<char> = password.chars().flat_map(|c| c.to_lowercase()).collect();
    // We index by char to support arbitrary unicode safely. Build a parallel
    // raw char vector so symbol/digit checks operate on the original chars.
    let raw: Vec<char> = password.chars().collect();
    if raw.len() != bytes_lower.len() {
        // Lowercasing changed char count (e.g. 'İ' -> "i\u{0307}"); bail.
        // Recovery candidates are ASCII-dominated; this branch is defensive.
        return None;
    }

    let favorites: Vec<&str> = ctx
        .pool
        .all_base_words
        .iter()
        .map(|w| w.canonical.as_str())
        .collect();
    let abbrevs: Vec<&str> = ctx
        .pool
        .site_abbreviations
        .iter()
        .map(|s| s.as_str())
        .collect();

    let mut out: Vec<Segment> = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        // 1. Favorite: longest base-word prefix match.
        if let Some(consumed) = match_longest_alpha(&bytes_lower, i, &favorites) {
            out.push(Segment::Favorite);
            i += consumed;
            continue;
        }
        // 2. Abbrev: longest abbreviation prefix match.
        if let Some(consumed) = match_longest_alpha(&bytes_lower, i, &abbrevs) {
            out.push(Segment::Abbrev);
            i += consumed;
            continue;
        }
        // 3. Digit run.
        if raw[i].is_ascii_digit() {
            while i < raw.len() && raw[i].is_ascii_digit() {
                i += 1;
            }
            out.push(Segment::DigitRun);
            continue;
        }
        // 4. Symbol run (any non-alphanumeric).
        if !raw[i].is_alphanumeric() {
            while i < raw.len() && !raw[i].is_alphanumeric() {
                i += 1;
            }
            out.push(Segment::SymbolRun);
            continue;
        }
        // No matcher fires — decomposition fails.
        return None;
    }
    Some(out)
}

/// Returns the number of chars consumed when one of `candidates` (lowercased
/// alpha tokens) is a prefix of `lowered[start..]`. Picks the LONGEST match.
fn match_longest_alpha(lowered: &[char], start: usize, candidates: &[&str]) -> Option<usize> {
    let mut best: Option<usize> = None;
    for cand in candidates {
        if cand.is_empty() {
            continue;
        }
        let cand_chars: Vec<char> = cand.chars().flat_map(|c| c.to_lowercase()).collect();
        if start + cand_chars.len() > lowered.len() {
            continue;
        }
        if lowered[start..start + cand_chars.len()] == cand_chars[..] {
            if best.map_or(true, |b| cand_chars.len() > b) {
                best = Some(cand_chars.len());
            }
        }
    }
    best
}

/// Cleanliness predicate. See module doc for the rule.
pub fn is_clean(segments: &[Segment]) -> bool {
    let Some(last) = segments.last() else {
        return false;
    };
    // Require at least one DigitRun: compositions encoding time/era
    // (e.g. year suffix) are the user-pattern shape this bonus targets.
    // Without this gate, `Rdthundermoon!`-style patterns score equally
    // and crowd out the actual target during cap truncation.
    if !segments.iter().any(|s| matches!(s, Segment::DigitRun)) {
        return false;
    }
    match last {
        Segment::Favorite | Segment::DigitRun | Segment::Abbrev => true,
        Segment::SymbolRun => matches!(
            segments.iter().rev().nth(1),
            None | Some(Segment::Favorite),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recovery::{DecryptedBaseWordEntry, HistoryStats, Pool, RecoverConfig};
    use crate::vault::Vault;
    use tempfile::TempDir;
    use zeroize::Zeroizing;

    fn dummy_vault() -> &'static Vault {
        // Vault contains rusqlite::Connection (not Sync), so OnceLock<Vault>
        // doesn't compile. Box::leak gives &'static; tests don't touch SQL on
        // this vault and the process is short-lived, so the leak is fine.
        let tmp = Box::leak(Box::new(TempDir::new().unwrap()));
        let path = tmp.path().join("v.db");
        let v = Vault::create(&path, b"x").unwrap();
        Box::leak(Box::new(v))
    }

    fn entry(s: &str) -> DecryptedBaseWordEntry {
        DecryptedBaseWordEntry {
            canonical: Zeroizing::new(s.to_string()),
            original: Zeroizing::new(s.to_string()),
        }
    }

    fn ctx_with(favs: &[&str], abbrevs: &[&str]) -> (Pool, HistoryStats, RecoverConfig) {
        let all_base_words: Vec<DecryptedBaseWordEntry> = favs.iter().map(|s| entry(s)).collect();
        let pool = Pool {
            seeds: vec![],
            favorite_base_words: all_base_words.clone(),
            all_base_words,
            site_abbreviations: abbrevs.iter().map(|s| s.to_string()).collect(),
            era_window: None,
        };
        (pool, HistoryStats::default(), RecoverConfig::default())
    }

    #[test]
    fn decompose_full_chain_with_two_favorites_symbol_digits_abbrev() {
        let (p, s, c) = ctx_with(&["moon", "beam"], &["Rd"]);
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let segs = decompose("MoonBeam$2019Rd", &rc).expect("must decompose");
        assert_eq!(
            segs,
            vec![
                Segment::Favorite,
                Segment::Favorite,
                Segment::SymbolRun,
                Segment::DigitRun,
                Segment::Abbrev,
            ]
        );
        assert!(is_clean(&segs));
    }

    #[test]
    fn decompose_dirty_chain_trailing_symbol_after_abbrev() {
        let (p, s, c) = ctx_with(&["moon", "beam"], &["Rd"]);
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let segs = decompose("MoonBeam$2019Rd!", &rc).expect("must decompose");
        assert_eq!(
            segs,
            vec![
                Segment::Favorite,
                Segment::Favorite,
                Segment::SymbolRun,
                Segment::DigitRun,
                Segment::Abbrev,
                Segment::SymbolRun,
            ]
        );
        assert!(!is_clean(&segs), "trailing sym after abbrev must be dirty");
    }

    #[test]
    fn decompose_unknown_word_fails() {
        let (p, s, c) = ctx_with(&["moon"], &[]);
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        assert!(decompose("Mypassword#1", &rc).is_none());
    }

    #[test]
    fn decompose_adjacent_favorites_no_separator() {
        let (p, s, c) = ctx_with(&["moon", "beam"], &[]);
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let segs = decompose("MoonBeam", &rc).expect("must decompose");
        assert_eq!(segs, vec![Segment::Favorite, Segment::Favorite]);
        // No DigitRun -> not clean under the digit-gated rule.
        assert!(!is_clean(&segs));
    }

    #[test]
    fn decompose_three_word_compound_with_digits() {
        let (p, s, c) = ctx_with(&["pass", "you"], &[]);
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let segs = decompose("Pass4You#5", &rc).expect("must decompose");
        assert_eq!(
            segs,
            vec![
                Segment::Favorite,
                Segment::DigitRun,
                Segment::Favorite,
                Segment::SymbolRun,
                Segment::DigitRun,
            ]
        );
        assert!(is_clean(&segs));
    }

    #[test]
    fn decompose_pure_digits_returns_single_digit_run() {
        let (p, s, c) = ctx_with(&[], &[]);
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let segs = decompose("2019", &rc).expect("must decompose");
        assert_eq!(segs, vec![Segment::DigitRun]);
        assert!(is_clean(&segs));
    }

    #[test]
    fn decompose_favorite_alone_is_dirty_no_digits() {
        let (p, s, c) = ctx_with(&["thunder"], &[]);
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let segs = decompose("Thunder", &rc).expect("must decompose");
        assert_eq!(segs, vec![Segment::Favorite]);
        // Single-favorite, no DigitRun -> not clean.
        assert!(!is_clean(&segs));
    }

    #[test]
    fn decompose_favorite_plus_symbol_is_dirty_no_digits() {
        let (p, s, c) = ctx_with(&["thunder"], &[]);
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let segs = decompose("Thunder!!", &rc).expect("must decompose");
        assert_eq!(segs, vec![Segment::Favorite, Segment::SymbolRun]);
        // No DigitRun -> not clean. (Last-segment rule alone would say clean,
        // but the digit-gated rule rejects this pattern.)
        assert!(!is_clean(&segs));
    }

    #[test]
    fn decompose_favorite_symbol_digits_is_clean() {
        let (p, s, c) = ctx_with(&["thunder"], &[]);
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let segs = decompose("Thunder!@#2017", &rc).expect("must decompose");
        assert_eq!(
            segs,
            vec![Segment::Favorite, Segment::SymbolRun, Segment::DigitRun]
        );
        assert!(is_clean(&segs));
    }

    #[test]
    fn decompose_favorite_digits_abbrev_is_clean() {
        let (p, s, c) = ctx_with(&["fluffy"], &["RS"]);
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let segs = decompose("Fluffy2014RS", &rc).expect("must decompose");
        assert_eq!(
            segs,
            vec![Segment::Favorite, Segment::DigitRun, Segment::Abbrev]
        );
        assert!(is_clean(&segs));
    }

    #[test]
    fn is_clean_last_sym_after_digit_returns_false() {
        let segs = vec![Segment::Favorite, Segment::DigitRun, Segment::SymbolRun];
        assert!(!is_clean(&segs));
    }

    #[test]
    fn is_clean_empty_returns_false() {
        assert!(!is_clean(&[]));
    }

    #[test]
    fn is_clean_pure_symbols_only_segment_returns_false() {
        // No DigitRun -> not clean. (Even ignoring digits, edge case for
        // a pure-punctuation password is unlikely in practice.)
        let segs = vec![Segment::SymbolRun];
        assert!(!is_clean(&segs));
    }
}
