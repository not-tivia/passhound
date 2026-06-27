//! Site-name normalization helpers. Shared by `recovery::pool` (abbreviation
//! derivation + match comparison) and `repo::sites` (one-shot cleanup of
//! URL-shaped stored names). Neutral module to avoid a repo<->recovery cycle.

/// Strip URL noise from a site name while preserving original case.
///
/// Handles two URL families:
/// - Standard web: removes `http(s)://`, path/query/fragment, port, `www.`,
///   then looks up the registrable domain (eTLD+1) via the Public Suffix List.
///   Subdomains are collapsed: `auth.riotgames.com` -> `riotgames.com`,
///   `us.battle.net` -> `battle.net`.  If the host has no public suffix
///   (bare names like `zotacstore`, pseudo-hosts like `us.battle`), the cleaned
///   host is kept as-is.
/// - Google Password Manager Android entries (`android://<cert>==@<pkg>/`):
///   extracts the package brand segment (e.g. `com.tumblr` -> `tumblr`,
///   `com.jagex.oldscape.android` -> `jagex`).
///
/// "https://www.GitHub.com"                       -> "GitHub.com"
/// "auth.riotgames.com"                           -> "riotgames.com"
/// "us.battle.net"                                -> "battle.net"
/// "https://www.amazon.co.uk"                     -> "amazon.co.uk"
/// "us.battle"                                    -> "us.battle"
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

    // Registrable domain (eTLD+1) via the Public Suffix List: collapses
    // subdomains (us.battle.net -> battle.net) while keeping the real domain.
    // The returned slice is from the original host, so case is preserved.
    match psl::domain_str(s) {
        Some(registrable) => registrable.to_string(),
        // No public suffix (bare names like "zotacstore", or pseudo-hosts like
        // "us.battle"): keep the cleaned host as-is rather than invent a brand.
        None => s.to_string(),
    }
}

#[cfg(test)]
mod brand_tests {
    use super::*;

    #[test]
    fn brand_label_strips_suffix_and_subdomains() {
        assert_eq!(brand_label("reddit.com"), "reddit");
        assert_eq!(brand_label("https://www.reddit.com"), "reddit");
        assert_eq!(brand_label("reddit"), "reddit");          // bare kept
        assert_eq!(brand_label("amazon.co.uk"), "amazon");    // compound suffix
        assert_eq!(brand_label("us.battle.net"), "battle");   // subdomain collapsed first
        assert_eq!(brand_label("RedDit.COM"), "reddit");      // lowercased
    }

    #[test]
    fn has_public_suffix_distinguishes_domain_from_bare() {
        assert!(has_public_suffix("reddit.com"));
        assert!(has_public_suffix("battle.net"));
        assert!(has_public_suffix("amazon.co.uk"));
        assert!(!has_public_suffix("reddit"));        // bare
        assert!(!has_public_suffix("zotacstore"));    // bare
    }
}

fn is_org_tld(s: &str) -> bool {
    matches!(
        s.to_lowercase().as_str(),
        "com" | "org" | "io" | "net" | "co" | "app" | "me" | "edu" | "gov" | "info"
    )
}

/// The brand stem of a site name: the first label of its registrable domain, or
/// the cleaned bare name when there is no public suffix. Lowercased.
/// `reddit.com` -> "reddit"; `www.reddit.com` -> "reddit"; bare `reddit` -> "reddit";
/// `amazon.co.uk` -> "amazon".
pub fn brand_label(name: &str) -> String {
    let canon = canonical_site_name(name); // registrable domain lowercased, or bare-as-is
    canon.split('.').next().unwrap_or(&canon).to_string()
}

/// True iff the name's canonical form has a public suffix — i.e. it is a real
/// registrable domain rather than a bare brand (`reddit.com` -> true, `reddit` -> false).
pub fn has_public_suffix(name: &str) -> bool {
    psl::domain_str(&canonical_site_name(name)).is_some()
}

/// Canonical equality key: the registrable domain (eTLD+1), lowercased.
/// "us.battle.net" and "eu.battle.net" both canonicalize to "battle.net".
/// Cross-brand grouping (battle.net under Blizzard) is handled by site aliases.
pub fn canonical_site_name(name: &str) -> String {
    strip_url_noise(name).to_lowercase()
}
