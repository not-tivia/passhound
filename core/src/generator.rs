//! Cryptographically-random password generator. Uses `rand::rngs::OsRng`
//! for entropy — NEVER `thread_rng` or anything seedable.
//!
//! The output is a `Zeroizing<String>` so the generated plaintext zeros
//! when it drops; callers should consume it quickly (clipboard or set_current).

use crate::error::{Error, Result};
use rand::rngs::OsRng;
use rand::RngCore;
use zeroize::Zeroizing;

/// Configurable options for `generate`. See `default()` for sensible defaults.
#[derive(Debug, Clone)]
pub struct GeneratorOptions {
    pub length: u8,
    pub lowercase: bool,
    pub uppercase: bool,
    pub digits: bool,
    pub symbols: bool,
    pub avoid_ambiguous: bool,
}

impl Default for GeneratorOptions {
    fn default() -> Self {
        Self {
            length: 16,
            lowercase: true,
            uppercase: true,
            digits: true,
            symbols: true,
            avoid_ambiguous: false,
        }
    }
}

const LOWERCASE: &str = "abcdefghijklmnopqrstuvwxyz";
const UPPERCASE: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: &str = "0123456789";
/// Curated 12-symbol set. Excludes shell-special / quote-special chars:
/// the `'"\` family + the `\`{}|<>;:?,./()[]` family.
const SYMBOLS: &str = "!@#$%^&*-_+=";

/// Characters omitted when `avoid_ambiguous = true`.
/// Lowercase `l`, uppercase `I L O`, digits `0 1`.
const AMBIGUOUS: &[char] = &['l', 'I', 'L', 'O', '0', '1'];

const MIN_LENGTH: u8 = 8;
const MAX_LENGTH: u8 = 64;

/// Generate a password from the given options. Returns `Error::InvalidInput`
/// if `length` is out of range or no charset is enabled.
pub fn generate(opts: GeneratorOptions) -> Result<Zeroizing<String>> {
    if opts.length < MIN_LENGTH || opts.length > MAX_LENGTH {
        return Err(Error::InvalidInput(format!(
            "length must be between {MIN_LENGTH} and {MAX_LENGTH}, got {}",
            opts.length
        )));
    }
    let mut charset: Vec<char> = Vec::new();
    if opts.lowercase { charset.extend(LOWERCASE.chars()); }
    if opts.uppercase { charset.extend(UPPERCASE.chars()); }
    if opts.digits    { charset.extend(DIGITS.chars()); }
    if opts.symbols   { charset.extend(SYMBOLS.chars()); }
    if opts.avoid_ambiguous {
        charset.retain(|c| !AMBIGUOUS.contains(c));
    }
    if charset.is_empty() {
        return Err(Error::InvalidInput("no charset enabled".into()));
    }

    let mut rng = OsRng;
    let mut out = String::with_capacity(opts.length as usize);
    for _ in 0..opts.length {
        let idx = (rng.next_u32() as usize) % charset.len();
        out.push(charset[idx]);
    }
    Ok(Zeroizing::new(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_respects_length() {
        for len in [8, 12, 16, 32, 64].iter().copied() {
            let opts = GeneratorOptions { length: len, ..Default::default() };
            let pw = generate(opts).unwrap();
            assert_eq!(pw.len(), len as usize, "length mismatch for {len}");
        }
    }

    #[test]
    fn generate_respects_charset_flags() {
        // Only digits enabled.
        let opts = GeneratorOptions {
            length: 32,
            lowercase: false, uppercase: false, digits: true, symbols: false,
            avoid_ambiguous: false,
        };
        let pw = generate(opts).unwrap();
        assert!(pw.chars().all(|c| c.is_ascii_digit()), "non-digit in digits-only: {pw:?}");

        // Only symbols enabled.
        let opts = GeneratorOptions {
            length: 32,
            lowercase: false, uppercase: false, digits: false, symbols: true,
            avoid_ambiguous: false,
        };
        let pw = generate(opts).unwrap();
        assert!(pw.chars().all(|c| SYMBOLS.contains(c)), "non-symbol in symbols-only: {pw:?}");
    }

    #[test]
    fn generate_avoids_ambiguous_when_requested() {
        let opts = GeneratorOptions {
            length: 64,
            avoid_ambiguous: true,
            ..Default::default()
        };
        // Generate many times to reduce the chance of a false negative.
        for _ in 0..10 {
            let pw = generate(opts.clone()).unwrap();
            for amb in AMBIGUOUS {
                assert!(!pw.contains(*amb), "ambiguous char {amb} appeared in {pw:?}");
            }
        }
    }

    #[test]
    fn generate_returns_distinct_values_across_calls() {
        let opts = GeneratorOptions::default();
        let a = generate(opts.clone()).unwrap();
        let b = generate(opts).unwrap();
        assert_ne!(*a, *b, "two calls produced identical passwords (entropy failure)");
    }

    #[test]
    fn generate_rejects_length_below_minimum() {
        let opts = GeneratorOptions { length: 4, ..Default::default() };
        assert!(matches!(generate(opts), Err(Error::InvalidInput(_))));
    }

    #[test]
    fn generate_rejects_empty_charset() {
        let opts = GeneratorOptions {
            length: 16,
            lowercase: false, uppercase: false, digits: false, symbols: false,
            avoid_ambiguous: false,
        };
        assert!(matches!(generate(opts), Err(Error::InvalidInput(_))));
    }
}
