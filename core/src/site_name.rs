//! Site-name normalization helpers. Shared by `recovery::pool` (abbreviation
//! derivation + match comparison) and `repo::sites` (one-shot cleanup of
//! URL-shaped stored names). Neutral module to avoid a repo<->recovery cycle.

/// Recognized single-label public TLDs. Extend as real data requires.
const TLDS: &[&str] = &[
    "com", "org", "io", "net", "co", "app", "me", "edu", "gov", "info",
    "gg", "tv", "us", "uk", "ca", "de", "fr", "jp", "au", "dev", "xyz",
];
/// Second-level labels that precede a ccTLD in a compound TLD (co.uk, com.au).
const COMPOUND_SECOND_LEVEL: &[&str] = &["co", "com", "org", "net", "gov", "ac"];

fn is_tld(s: &str) -> bool {
    TLDS.iter().any(|t| s.eq_ignore_ascii_case(t))
}
fn is_compound_second_level(s: &str) -> bool {
    COMPOUND_SECOND_LEVEL.iter().any(|t| s.eq_ignore_ascii_case(t))
}

/// Strip URL noise from a site name while preserving original case.
///
/// Handles two URL families:
/// - Standard web: removes `http(s)://`, path/query/fragment, port, `www.`,
///   and extracts the brand (rightmost segment after dropping recognized TLD
///   and compound second-level labels). Subdomains are discarded.
/// - Google Password Manager Android entries (`android://<cert>==@<pkg>/`):
///   extracts the package brand segment (e.g. `com.tumblr` -> `tumblr`,
///   `com.jagex.oldscape.android` -> `jagex`).
///
/// "https://www.GitHub.com"                       -> "GitHub"
/// "auth.riotgames.com"                           -> "riotgames"
/// "https://www.amazon.co.uk"                     -> "amazon"
/// "us.battle"                                    -> "battle"
/// "android://abc==@com.tumblr/"                  -> "tumblr"
/// "android://xyz==@com.jagex.oldscape.android/"  -> "jagex"
/// "  Tumblr  "                                   -> "Tumblr"
pub fn strip_url_noise(name: &str) -> String {
    let s = name.trim();

    if let Some(rest) = s.strip_prefix("android://") {
        let pkg = rest.split('@').nth(1).unwrap_or("");
        let pkg = pkg.split('/').next().unwrap_or("");
        let segments: Vec<&str> = pkg.split('.').filter(|s| !s.is_empty()).collect();
        return match segments.as_slice() {
            [first, second, ..] if is_org_tld(first) => second.to_string(),
            [only] => only.to_string(),
            [first, ..] => first.to_string(),
            _ => String::new(),
        };
    }

    let s = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://")).unwrap_or(s);
    let s = s.split(['/', '?', '#']).next().unwrap_or("");
    let s = s.split(':').next().unwrap_or("");
    let s = if s.len() >= 4 && s[..4].eq_ignore_ascii_case("www.") { &s[4..] } else { s };
    let mut segs: Vec<&str> = s.split('.').filter(|p| !p.is_empty()).collect();
    if segs.is_empty() {
        return String::new();
    }
    // Drop a recognized TLD (and a compound second-level label if present),
    // but never reduce to nothing.
    if segs.len() >= 2 && is_tld(segs[segs.len() - 1]) {
        segs.pop();
        if segs.len() >= 2 && is_compound_second_level(segs[segs.len() - 1]) {
            segs.pop();
        }
    }
    // Brand = rightmost remaining segment; earlier segments are subdomains.
    segs.last().unwrap().to_string()
}

fn is_org_tld(s: &str) -> bool {
    matches!(
        s.to_lowercase().as_str(),
        "com" | "org" | "io" | "net" | "co" | "app" | "me" | "edu" | "gov" | "info"
    )
}

/// Canonical equality key: the brand, lowercased, alphanumerics only.
/// "Riot games" and "auth.riotgames.com" both canonicalize to "riotgames".
pub fn canonical_site_name(name: &str) -> String {
    strip_url_noise(name)
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}
