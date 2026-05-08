//! Paste-and-parse: line-prefix parser over markdown / plain text.
//!
//! Splits input on blank lines into "blocks". Within each block, looks for
//! `<key>:` line prefixes (case-insensitive) for site / url / username /
//! password / notes. A block missing both `site:` and `url:`, or missing
//! `password:`, becomes a `ParseDiagnostic`.

use super::{ImportEntry, ParseDiagnostic, ParseResult};

/// Parse the input string into entries and diagnostics.
pub fn parse_str(input: &str) -> ParseResult {
    let mut result = ParseResult::default();
    let mut block_lines: Vec<&str> = Vec::new();
    let mut block_start_line: usize = 1;
    let mut current_line: usize = 0;

    for line in input.lines() {
        current_line += 1;
        if line.trim().is_empty() {
            if !block_lines.is_empty() {
                process_block(&block_lines, block_start_line, &mut result);
                block_lines.clear();
            }
            block_start_line = current_line + 1;
        } else {
            if block_lines.is_empty() {
                block_start_line = current_line;
            }
            block_lines.push(line);
        }
    }
    if !block_lines.is_empty() {
        process_block(&block_lines, block_start_line, &mut result);
    }
    result
}

fn process_block(lines: &[&str], start_row: usize, result: &mut ParseResult) {
    let raw = lines.join("\n");
    let mut site: Option<String> = None;
    let mut url: Option<String> = None;
    let mut username: Option<String> = None;
    let mut password: Option<String> = None;
    let mut notes: Option<String> = None;
    let mut last_field: Option<&'static str> = None;

    for line in lines {
        let lower = line.trim_start().to_ascii_lowercase();
        let (key, value): (Option<&'static str>, &str) = if let Some(v) = strip_prefix_ci(line, "site:") {
            (Some("site"), v)
        } else if let Some(v) = strip_prefix_ci(line, "name:") {
            (Some("site"), v)
        } else if let Some(v) = strip_prefix_ci(line, "website:") {
            (Some("site"), v)
        } else if let Some(v) = strip_prefix_ci(line, "title:") {
            (Some("site"), v)
        } else if let Some(v) = strip_prefix_ci(line, "service:") {
            (Some("site"), v)
        } else if let Some(v) = strip_prefix_ci(line, "url:") {
            (Some("url"), v)
        } else if let Some(v) = strip_prefix_ci(line, "username:") {
            (Some("username"), v)
        } else if let Some(v) = strip_prefix_ci(line, "user:") {
            (Some("username"), v)
        } else if let Some(v) = strip_prefix_ci(line, "login:") {
            (Some("username"), v)
        } else if let Some(v) = strip_prefix_ci(line, "email:") {
            (Some("username"), v)
        } else if let Some(v) = strip_prefix_ci(line, "password:") {
            (Some("password"), v)
        } else if let Some(v) = strip_prefix_ci(line, "pass:") {
            (Some("password"), v)
        } else if let Some(v) = strip_prefix_ci(line, "notes:") {
            (Some("notes"), v)
        } else if let Some(v) = strip_prefix_ci(line, "note:") {
            (Some("notes"), v)
        } else if let Some(v) = strip_prefix_ci(line, "comment:") {
            (Some("notes"), v)
        } else {
            (None, line)
        };
        let _ = lower; // silence unused warning if rustc gets clever

        match key {
            Some("site") => {
                site = Some(value.trim().to_string());
                last_field = Some("site");
            }
            Some("url") => {
                url = Some(value.trim().to_string());
                last_field = Some("url");
            }
            Some("username") => {
                username = Some(value.trim().to_string());
                last_field = Some("username");
            }
            Some("password") => {
                password = Some(value.trim().to_string());
                last_field = Some("password");
            }
            Some("notes") => {
                notes = Some(value.trim().to_string());
                last_field = Some("notes");
            }
            _ => {
                // Continuation of the previous field.
                if let Some(field) = last_field {
                    let extra = line.trim();
                    let target = match field {
                        "site" => &mut site,
                        "url" => &mut url,
                        "username" => &mut username,
                        "password" => &mut password,
                        "notes" => &mut notes,
                        _ => continue,
                    };
                    if let Some(existing) = target.as_mut() {
                        existing.push('\n');
                        existing.push_str(extra);
                    }
                }
            }
        }
    }

    // Derive site from URL hostname if site is missing.
    if site.is_none() {
        if let Some(u) = url.as_ref() {
            site = derive_site_from_url(u);
        }
    }

    let final_site = match site {
        Some(s) if !s.is_empty() => s,
        _ => {
            result.diagnostics.push(ParseDiagnostic {
                row: start_row,
                raw,
                reason: "block missing site/url".to_string(),
            });
            return;
        }
    };
    let final_password = match password {
        Some(p) if !p.is_empty() => p,
        _ => {
            result.diagnostics.push(ParseDiagnostic {
                row: start_row,
                raw,
                reason: "block missing password".to_string(),
            });
            return;
        }
    };

    result.entries.push(ImportEntry {
        site: final_site,
        url,
        username,
        password: final_password,
        created_at: None,
        notes,
        source_row: Some(raw),
    });
}

fn strip_prefix_ci<'a>(line: &'a str, prefix_lower: &str) -> Option<&'a str> {
    let trimmed = line.trim_start();
    if trimmed.len() < prefix_lower.len() {
        return None;
    }
    let head = &trimmed[..prefix_lower.len()];
    if head.eq_ignore_ascii_case(prefix_lower) {
        Some(&trimmed[prefix_lower.len()..])
    } else {
        None
    }
}

fn derive_site_from_url(url: &str) -> Option<String> {
    let s = url.trim();
    let without_scheme = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .unwrap_or(s);
    let host = without_scheme.split('/').next().unwrap_or("");
    let host = host.strip_prefix("www.").unwrap_or(host);
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_two_blocks_separated_by_blank_line() {
        let input = "\
site: RuneScape
username: chris
password: Fluffy!2014

site: Amazon
username: chris@example.com
password: Bezos$Buy1
";
        let r = parse_str(input);
        assert_eq!(r.entries.len(), 2);
        assert_eq!(r.diagnostics.len(), 0);
        assert_eq!(r.entries[0].site, "RuneScape");
        assert_eq!(r.entries[0].password, "Fluffy!2014");
        assert_eq!(r.entries[1].site, "Amazon");
    }

    #[test]
    fn case_insensitive_prefix() {
        let input = "\
SITE: Foo
USERNAME: bar
PASSWORD: baz
";
        let r = parse_str(input);
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].site, "Foo");
        assert_eq!(r.entries[0].username.as_deref(), Some("bar"));
        assert_eq!(r.entries[0].password, "baz");
    }

    #[test]
    fn block_missing_password_becomes_diagnostic() {
        let input = "\
site: NoPass
username: chris
";
        let r = parse_str(input);
        assert_eq!(r.entries.len(), 0);
        assert_eq!(r.diagnostics.len(), 1);
        assert!(r.diagnostics[0].reason.contains("password"));
    }

    #[test]
    fn block_with_only_url_derives_site_from_hostname() {
        let input = "\
url: https://www.example.com/login
username: chris
password: pw
";
        let r = parse_str(input);
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].site, "example.com");
    }

    #[test]
    fn trailing_whitespace_trimmed() {
        let input = "\
site:   RuneScape
password:   Fluffy!2014
";
        let r = parse_str(input);
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].site, "RuneScape");
        assert_eq!(r.entries[0].password, "Fluffy!2014");
    }

    #[test]
    fn block_missing_both_site_and_url_becomes_diagnostic() {
        let input = "\
username: chris
password: pw
";
        let r = parse_str(input);
        assert_eq!(r.entries.len(), 0);
        assert_eq!(r.diagnostics.len(), 1);
        assert!(r.diagnostics[0].reason.contains("site/url"));
    }
}
