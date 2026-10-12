//! Update feed check: curls a JSON feed, compares against CURRENT.
//!
//! Feed format (ours): a flat JSON object with exactly three string keys:
//! `{"version": "0.3.0", "url": "https://…/SweepMac-0.3.0.dmg", "notes": "…"}`
//! Whitespace/order irrelevant; extra keys ignored; `\"`/`\\`/`\n` etc.
//! escapes inside values are unescaped. Anything else is malformed.

use std::process::Command;

pub const CURRENT: &str = "0.2.0";

#[derive(Debug)]
pub struct Release {
    pub version: String,
    pub url: String,
    pub notes: String,
}

pub fn check(feed_url: &str) -> Result<Option<Release>, String> {
    let out = Command::new("/usr/bin/curl")
        .args(["-sL", "--max-time", "15", feed_url])
        .output()
        .map_err(|e| format!("curl failed: {e}"))?;
    if !out.status.success() {
        return Err(format!("curl exited with {}", out.status));
    }
    let body = String::from_utf8(out.stdout).map_err(|e| format!("malformed feed: {e}"))?;
    let version = extract(&body, "version").ok_or("malformed feed: missing \"version\"")?;
    let url = extract(&body, "url").ok_or("malformed feed: missing \"url\"")?;
    let notes = extract(&body, "notes").ok_or("malformed feed: missing \"notes\"")?;
    if cmp_version(&version, CURRENT) > 0 {
        Ok(Some(Release { version, url, notes }))
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

    fn feed_file(body: &str) -> String {
        let mut p = std::env::temp_dir();
        p.push(format!("sweep-feed-{}.json", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        format!("file://{}", p.display())
    }

    #[test]
    fn newer_returns_release() {
        let url = feed_file(r#"{"version":"9.9.9","url":"https://x/y.dmg","notes":"hi"}"#);
        let r = check(&url).unwrap().expect("expected Some");
        assert_eq!(r.version, "9.9.9");
    }

    #[test]
    fn same_returns_none() {
        let url = feed_file(&format!(r#"{{"notes":"n","url":"https://x","version":"{CURRENT}"}}"#));
        assert!(check(&url).unwrap().is_none());
    }

    #[test]
    fn malformed_is_err() {
        let url = feed_file(r#"{"version": 123}"#);
        assert!(check(&url).unwrap_err().contains("malformed"));
    }

    #[test]
    fn nested_same_name_key_is_skipped() {
        let url = feed_file(r#"{"meta": {"version": 2}, "version": "9.9.9", "url": "https://x/y.dmg", "notes": "n"}"#);
        let r = check(&url).unwrap().expect("real version wins");
        assert_eq!(r.version, "9.9.9");
    }
}
