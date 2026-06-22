//! Site-name normalization helpers. Shared by `recovery::pool` (abbreviation
//! derivation + match comparison) and `repo::sites` (one-shot cleanup of
//! URL-shaped stored names). Neutral module to avoid a repo<->recovery cycle.

/// Strip URL noise from a site name while preserving original case.
///
/// Handles two URL families:
/// - Standard web: removes `http(s)://`, path/query/fragment, port, `www.`,
///   and the trailing TLD segment.
/// - Google Password Manager Android entries (`android://<cert>==@<pkg>/`):
///   extracts the package brand segment (e.g. `com.tumblr` -> `tumblr`,
///   `com.jagex.oldscape.android` -> `jagex`).
///
/// "https://www.GitHub.com"                       -> "GitHub"
/// "https://www.amazon.co/login"                  -> "amazon"
/// "github.com:8080"                              -> "github"
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
    match s.rsplit_once('.') {
        Some((root, _tld)) if !root.is_empty() => root.to_string(),
        _ => s.to_string(),
    }
}

fn is_org_tld(s: &str) -> bool {
    matches!(
        s.to_lowercase().as_str(),
        "com" | "org" | "io" | "net" | "co" | "app" | "me" | "edu" | "gov" | "info"
    )
}

/// Canonical lowercase form for site-name equality comparison.
pub fn canonical_site_name(name: &str) -> String {
    strip_url_noise(name).to_lowercase()
}
