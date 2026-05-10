//! Structural decomposition of a candidate password into recognized segments.
//!
//! Used by `ranking::score` to detect "clean" compositional patterns
//! (e.g. `MoonBeam$2019Rd` — favorite + favorite + symbol + digits + abbrev)
//! and award a structural bonus. A candidate is "clean" when ALL of:
//!   1. The entire password decomposes into a sequence of recognized segments
//!      (favorite base word, digit run, symbol run, or site abbreviation).
//!   2. Segments contain at least one `DigitRun(n)` with `n >= 4` — i.e. a
//!      year-shaped digit run. Single/short digit-runs (e.g. `moonfluffy1!Rd`,
//!      `pass#42`) are not clean. Phase 3.9 narrowed this from "any DigitRun"
//!      to ">=4 digits" so the bonus discriminates against single-digit
//!      junk runs that out-score year-encoded targets via len_match.
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
    /// Number of consecutive ASCII digits in the run. Phase 3.9's cleanliness
    /// rule requires at least one `DigitRun(n)` with `n >= 4`.
    DigitRun(usize),
    SymbolRun,
    Abbrev,
}

/// Greedy decomposition. Returns `Some(segments)` if the entire password is
/// consumed by recognized segments, else `None`. ASCII fast-path: non-ASCII
/// passwords bail early (decomposition fails -> no clean bonus, same
/// conservative behavior as a fully-failed match). Zero heap allocations:
/// candidate sets are iterated directly from the pool.
pub fn decompose(password: &str, ctx: &RecoverContext<'_>) -> Option<Vec<Segment>> {
    if !password.is_ascii() {
        return None;
    }
    let bytes = password.as_bytes();

    // Build iterators over ASCII candidate byte-slices from the pool — no Vec
    // allocation. Non-ASCII entries are skipped (eq_ignore_ascii_case only
    // handles A-Z <-> a-z; non-ASCII would silently mis-match).
    let fav_iter = || {
        ctx.pool.all_base_words.iter().filter_map(|w| {
            let s = w.canonical.as_str();
            if s.is_ascii() && !s.is_empty() { Some(s.as_bytes()) } else { None }
        })
    };
    let abbrev_iter = || {
        ctx.pool.site_abbreviations.iter().filter_map(|s| {
            let s = s.as_str();
            if s.is_ascii() && !s.is_empty() { Some(s.as_bytes()) } else { None }
        })
    };

    let mut out: Vec<Segment> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        // 1. Favorite: longest base-word prefix match (case-insensitive).
        if let Some(consumed) = match_longest_ascii(bytes, i, fav_iter()) {
            out.push(Segment::Favorite);
            i += consumed;
            continue;
        }
        // 2. Abbrev: longest abbreviation prefix match (case-insensitive).
        if let Some(consumed) = match_longest_ascii(bytes, i, abbrev_iter()) {
            out.push(Segment::Abbrev);
            i += consumed;
            continue;
        }
        // 3. Digit run — track length for the year-shaped cleanliness check.
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            out.push(Segment::DigitRun(i - start));
            continue;
        }
        // 4. Symbol run (anything that's not alphanumeric ASCII at this point).
        if !(bytes[i] as char).is_ascii_alphanumeric() {
            while i < bytes.len() && !(bytes[i] as char).is_ascii_alphanumeric() {
                i += 1;
            }
            out.push(Segment::SymbolRun);
            continue;
        }
        // ASCII letter that didn't match any favorite/abbrev — decomposition fails.
        return None;
    }
    Some(out)
}

/// Returns the number of bytes consumed when one of `candidates` is a prefix
/// of `haystack[start..]` (ASCII case-insensitive). Picks the LONGEST match.
/// Both `haystack` and each yielded `cand` must be ASCII.
fn match_longest_ascii<'a>(
    haystack: &[u8],
    start: usize,
    candidates: impl Iterator<Item = &'a [u8]>,
) -> Option<usize> {
    let mut best: Option<usize> = None;
    for cand in candidates {
        let len = cand.len();
        if start + len > haystack.len() {
            continue;
        }
        if haystack[start..start + len].eq_ignore_ascii_case(cand) {
            if best.map_or(true, |b| len > b) {
                best = Some(len);
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
    // Require at least one year-shaped DigitRun (length >= 4). Single-digit
    // and 2-3-digit runs (e.g. `moonfluffy1!Rd`, `pass#42`) get no bonus —
    // they cluster above year-encoded targets via len_match without this
    // discriminator. Phase 3.9 narrowed from "any DigitRun" to ">=4 digits".
    if !segments.iter().any(|s| matches!(s, Segment::DigitRun(n) if *n >= 4)) {
        return false;
    }
    match last {
        Segment::Favorite | Segment::DigitRun(_) | Segment::Abbrev => true,
        Segment::SymbolRun => matches!(
            segments.iter().rev().nth(1),
            None | Some(Segment::Favorite),
        ),
    }
}

/// Combined decompose + is_clean check with zero heap allocations.
/// Returns `true` iff the password fully decomposes into recognized segments
/// AND passes the is_clean predicate. Use this in the hot scoring path instead
/// of `decompose(..).map(|s| is_clean(&s)).unwrap_or(false)`.
pub fn is_clean_pattern(password: &str, ctx: &RecoverContext<'_>) -> bool {
    if !password.is_ascii() {
        return false;
    }
    let bytes = password.as_bytes();
    if bytes.is_empty() {
        return false;
    }

    let fav_iter = || {
        ctx.pool.all_base_words.iter().filter_map(|w| {
            let s = w.canonical.as_str();
            if s.is_ascii() && !s.is_empty() { Some(s.as_bytes()) } else { None }
        })
    };
    let abbrev_iter = || {
        ctx.pool.site_abbreviations.iter().filter_map(|s| {
            let s = s.as_str();
            if s.is_ascii() && !s.is_empty() { Some(s.as_bytes()) } else { None }
        })
    };

    // Track only what is_clean needs: presence of any year-shaped DigitRun
    // (length >= 4), last segment, and second-to-last segment (for the
    // trailing-SymbolRun check).
    let mut has_year_digit_run = false;
    let mut prev_seg: Option<Segment> = None; // second-to-last segment
    let mut last_seg: Option<Segment> = None; // last segment

    let mut i = 0;
    while i < bytes.len() {
        let seg;
        if let Some(consumed) = match_longest_ascii(bytes, i, fav_iter()) {
            i += consumed;
            seg = Segment::Favorite;
        } else if let Some(consumed) = match_longest_ascii(bytes, i, abbrev_iter()) {
            i += consumed;
            seg = Segment::Abbrev;
        } else if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let run_len = i - start;
            if run_len >= 4 {
                has_year_digit_run = true;
            }
            seg = Segment::DigitRun(run_len);
        } else if !(bytes[i] as char).is_ascii_alphanumeric() {
            while i < bytes.len() && !(bytes[i] as char).is_ascii_alphanumeric() {
                i += 1;
            }
            seg = Segment::SymbolRun;
        } else {
            return false; // unmatched alpha
        }
        prev_seg = last_seg;
        last_seg = Some(seg);
    }

    // Apply is_clean rules.
    if !has_year_digit_run { return false; }
    match last_seg {
        None => false,
        Some(Segment::Favorite) | Some(Segment::DigitRun(_)) | Some(Segment::Abbrev) => true,
        Some(Segment::SymbolRun) => matches!(prev_seg, None | Some(Segment::Favorite)),
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
                Segment::DigitRun(4),
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
                Segment::DigitRun(4),
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
    fn decompose_three_word_compound_short_digits_is_dirty_under_year_rule() {
        let (p, s, c) = ctx_with(&["pass", "you"], &[]);
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let segs = decompose("Pass4You#5", &rc).expect("must decompose");
        assert_eq!(
            segs,
            vec![
                Segment::Favorite,
                Segment::DigitRun(1),
                Segment::Favorite,
                Segment::SymbolRun,
                Segment::DigitRun(1),
            ]
        );
        // Phase 3.9: no DigitRun has length >= 4 -> not clean even though
        // the structural decomposition succeeds.
        assert!(!is_clean(&segs));
    }

    #[test]
    fn decompose_pure_year_digits_returns_single_digit_run() {
        let (p, s, c) = ctx_with(&[], &[]);
        let rc = RecoverContext { vault: dummy_vault(), config: &c, pool: &p, stats: &s };
        let segs = decompose("2019", &rc).expect("must decompose");
        assert_eq!(segs, vec![Segment::DigitRun(4)]);
        // 4-digit run; clean under the year-shaped rule.
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
            vec![Segment::Favorite, Segment::SymbolRun, Segment::DigitRun(4)]
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
            vec![Segment::Favorite, Segment::DigitRun(4), Segment::Abbrev]
        );
        assert!(is_clean(&segs));
    }

    #[test]
    fn is_clean_last_sym_after_digit_returns_false() {
        // DigitRun(4) ensures the year-rule passes; the failure reason here
        // is the last-seg=Sym after DigitRun, not the digit-length check.
        let segs = vec![Segment::Favorite, Segment::DigitRun(4), Segment::SymbolRun];
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

    #[test]
    fn is_clean_short_digit_run_returns_false() {
        // Last seg = Abbrev would be clean under the last-seg rule, but the
        // only DigitRun is length 1 -> dirty under Phase 3.9 year rule.
        let segs = vec![
            Segment::Favorite,
            Segment::SymbolRun,
            Segment::DigitRun(1),
            Segment::Abbrev,
        ];
        assert!(!is_clean(&segs));
    }
}
