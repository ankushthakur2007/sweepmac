//! Update check: GitHub Releases API (`/releases/latest`) — the standard
//! plumbing for apps shipped from a GitHub repo, so there is no self-hosted
//! feed to maintain. `tag_name` (v-prefixed) is compared against CURRENT and
//! `html_url` is the release page; the top-level `html_url` serializes before
//! any nested ones (author/assets), so first-match extraction is correct.
//! Drafts/prereleases are excluded by `latest` itself.

use std::process::Command;

/// Version comparison base — derived so Cargo and the updater can't drift.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// Latest-release endpoint; the "Check for updates" button renders only
/// while this is Some (set to None to hide the button again).
pub const FEED: Option<&str> =
    Some("https://api.github.com/repos/ankushthakur2007/sweepmac/releases/latest");

#[derive(Debug)]
pub struct Release {
    pub version: String,
    pub url: String,
}

pub fn check(endpoint: &str) -> Result<Option<Release>, String> {
    let out = Command::new("/usr/bin/curl")
        .args([
            "-sL",
            "--max-time",
            "15",
            "-H",
            "User-Agent: SweepMac-updater",
            endpoint,
        ])
        .output()
        .map_err(|e| format!("curl failed: {e}"))?;
    if !out.status.success() {
        return Err(format!("curl exited with {}", out.status));
    }
    let body = String::from_utf8(out.stdout).map_err(|e| format!("malformed response: {e}"))?;
    // A 403/404 payload carries GitHub's own "message" — surface it verbatim
    // (rate-limit explanations are the common case for an anonymous client).
    let version = extract(&body, "tag_name").ok_or_else(|| {
        let why = extract(&body, "message").unwrap_or_else(|| "unexpected response".into());
        format!("no release found: {why}")
    })?;
    let url = extract(&body, "html_url").ok_or("malformed response: missing \"html_url\"")?;
    if cmp_version(&version, CURRENT) > 0 {
        Ok(Some(Release { version, url }))
    } else {
        Ok(None)
    }
}

/// Numeric dot-separated compare; strips leading 'v', ignores build metadata
/// after '+'/'-'. Non-numeric parts compare as 0. Returns -1/0/1.
fn cmp_version(a: &str, b: &str) -> i32 {
    let norm = |s: &str| {
        let s = s.strip_prefix('v').unwrap_or(s);
        let s = s.split(['+', '-']).next().unwrap_or(s);
        s.split('.')
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    let (a, b) = (norm(a), norm(b));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (*a.get(i).unwrap_or(&0), *b.get(i).unwrap_or(&0));
        if x != y {
            return if x > y { 1 } else { -1 };
        }
    }
    0
}

/// Extract the string value for a top-level `"key": "value"` pair.
fn extract(json: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\"");
    let mut rest = json;
    loop {
        let i = rest.find(&pat)?;
        let after = &rest[i + pat.len()..];
        let after = after.trim_start();
        if !after.starts_with(':') {
            rest = after;
            continue;
        }
        let mut chars = after[1..].trim_start().chars();
        if chars.next()? != '"' {
            // Non-string value (e.g. nested object holding the same key):
            // keep scanning instead of failing the whole feed.
            rest = &after[1..];
            continue;
        }
        let mut val = String::new();
        let mut esc = false;
        for c in chars {
            if esc {
                match c {
                    'n' => val.push('\n'),
                    't' => val.push('\t'),
                    'r' => val.push('\r'),
                    other => val.push(other),
                }
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                return Some(val);
            } else {
                val.push(c);
            }
        }
        return None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn gh_response(body: &str) -> String {
        let mut p = std::env::temp_dir();
        p.push(format!("sweep-gh-{}.json", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        format!("file://{}", p.display())
    }

    #[test]
    fn newer_tag_yields_release() {
        let url = gh_response(r#"{"html_url":"https://github.com/u/r/releases/tag/v9.9.9","tag_name":"v9.9.9","body":"notes"}"#);
        let r = check(&url).unwrap().expect("expected Some");
        assert_eq!(r.version, "v9.9.9");
        assert_eq!(r.url, "https://github.com/u/r/releases/tag/v9.9.9");
    }

    #[test]
    fn same_tag_returns_none() {
        let url = gh_response(&format!(r#"{{"html_url":"https://x","tag_name":"v{CURRENT}"}}"#));
        assert!(check(&url).unwrap().is_none());
    }

    #[test]
    fn real_key_order_prefers_toplevel_html_url() {
        // GitHub serializes url/assets_url/upload_url/html_url/id/author… —
        // author's nested html_url must not win over the top-level one.
        let url = gh_response(
            r#"{"url":"https://api.github.com/repos/u/r/releases/1","html_url":"https://github.com/u/r/releases/tag/v9.9.9","author":{"login":"u","html_url":"https://github.com/u"},"tag_name":"v9.9.9"}"#,
        );
        let r = check(&url).unwrap().expect("Some");
        assert_eq!(r.url, "https://github.com/u/r/releases/tag/v9.9.9");
    }

    #[test]
    fn rate_limit_message_is_surfaced() {
        let url = gh_response(r#"{"message":"API rate limit exceeded for 1.2.3.4.","documentation_url":"https://docs.github.com"}"#);
        let err = check(&url).unwrap_err();
        assert!(err.contains("API rate limit exceeded"), "{err}");
    }

    #[test]
    fn missing_html_url_is_err() {
        let url = gh_response(r#"{"tag_name":"v9.9.9"}"#);
        assert!(check(&url).unwrap_err().contains("html_url"));
    }

    #[test]
    fn malformed_non_json_is_err() {
        let url = gh_response("<html>gateway error</html>");
        assert!(check(&url).unwrap_err().contains("no release found"));
    }

    /// Live call against the real endpoint — run manually:
    /// `cargo test -p sweepmac updater::tests::live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live() {
        let r = check(FEED.expect("feed enabled")).expect("endpoint reachable");
        match r {
            Some(rel) => println!("update available: {} {}", rel.version, rel.url),
            None => println!("up to date against {CURRENT}"),
        }
    }
}
