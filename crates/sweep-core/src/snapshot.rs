//! Pre-clean snapshots: JSON array of `{path,size,category}` in
//! `~/.sweepmac/snapshots/<unix-ts>.json` (never blocks a clean on read).
use std::path::{Path, PathBuf};

/// Real snapshots dir.
pub fn dir() -> Option<PathBuf> {
    crate::home().map(|h| h.join(".sweepmac/snapshots"))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

/// Write a snapshot into an explicit dir (tests use temp dirs).
pub fn save_to(dir: &Path, items: &[(PathBuf, u64, String)]) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let ts = now_secs();
    let mut dest = dir.join(format!("{ts}.json"));
    let mut n = 0u32;
    while dest.exists() {
        n += 1;
        dest = dir.join(format!("{ts}-{n}.json"));
    }
    let mut out = String::from("[");
    for (i, (p, size, cat)) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"path\":\"{}\",\"size\":{},\"category\":\"{}\"}}",
            esc(&p.to_string_lossy()),
            size,
            esc(cat)
        ));
    }
    out.push(']');
    std::fs::write(&dest, out).map_err(|e| e.to_string())?;
    prune(dir);
    Ok(dest)
}

/// Snapshots are write-only pre-clean records: keep only the newest files so
/// they can't pile up forever (listings stay internal — open the JSON in
/// ~/.sweepmac/snapshots/ to inspect).
const KEEP: usize = 10;

fn prune(dir: &Path) {
    let mut all = files_in(dir); // newest-first
    if all.len() > KEEP {
        for old in all.split_off(KEEP) {
            let _ = std::fs::remove_file(old);
        }
    }
}

/// Snapshot files in an explicit dir, newest-first.
fn files_in(dir: &Path) -> Vec<PathBuf> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .ok()
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names.reverse();
    names.into_iter().map(|n| dir.join(n)).collect()
}

/// Snapshot to the real dir.
pub fn save(items: &[(PathBuf, u64, String)]) -> Result<PathBuf, String> {
    match dir() {
        Some(d) => save_to(&d, items),
        None => Err("HOME is not set".into()),
    }
}

/// Most recent snapshot file, if any.
pub fn latest() -> Option<PathBuf> {
    files_in(&dir()?).first().cloned()
}

/// Best-effort undo: move every path recorded in the most recent snapshot
/// back from the Trash to its original location. Returns the number restored.
pub fn restore_latest() -> Result<usize, String> {
    let Some(latest) = latest() else {
        return Ok(0);
    };
    let body = std::fs::read_to_string(&latest).map_err(|e| e.to_string())?;
    let paths = extract_paths(&body);
    if paths.is_empty() {
        return Ok(0);
    }
    restore_from_trash(&paths)
}

/// Move the given original paths back out of the Trash, if still present.
/// On macOS the trash crate does not expose os_limited restore, so this is a
/// best-effort move from ~/.Trash by original filename.
pub fn restore_from_trash(paths: &[PathBuf]) -> Result<usize, String> {
    let trash_dir = crate::home()
        .map(|h| h.join(".Trash"))
        .ok_or_else(|| "HOME is not set".to_string())?;
    let mut restored = 0usize;
    for original in paths {
        let name = match original.file_name() {
            Some(n) => n,
            None => continue,
        };
        let trashed = trash_dir.join(name);
        if !trashed.exists() {
            continue;
        }
        if let Some(parent) = original.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!("restore: cannot create parent {}: {e}", parent.display());
                continue;
            }
        }
        if original.exists() {
            // A live file already exists; refuse to overwrite.
            continue;
        }
        if std::fs::rename(&trashed, original).is_ok() {
            restored += 1;
        }
    }
    Ok(restored)
}

/// Hand-rolled extractor for {"path":"..."} entries in snapshot JSON.
fn extract_paths(body: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut rest = body;
    loop {
        let Some(i) = rest.find("\"path\":\"") else { break };
        let after = &rest[i + "\"path\":\"".len()..];
        let mut val = String::new();
        let mut esc = false;
        let mut end = None;
        for (idx, c) in after.chars().enumerate() {
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
                end = Some(idx);
                break;
            } else {
                val.push(c);
            }
        }
        if end.is_some() {
            out.push(PathBuf::from(val));
        } else {
            break;
        }
        rest = &after[end.unwrap() + 1..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmpdir() -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let d = std::env::temp_dir().join(format!("sweep-snap-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn save_writes_json_array() {
        let d = tmpdir();
        let items = vec![(PathBuf::from("/a/b"), 42u64, "Cache".to_string())];
        let dest = save_to(&d, &items).unwrap();
        let body = std::fs::read_to_string(&dest).unwrap();
        assert!(body.contains("/a/b") && body.contains("42") && body.contains("Cache"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn files_in_newest_first() {
        let d = tmpdir();
        for name in ["100.json", "300.json", "200.json"] {
            std::fs::write(d.join(name), "[]").unwrap();
        }
        let got: Vec<String> = files_in(&d)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(got, vec!["300.json", "200.json", "100.json"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn missing_dir_lists_empty() {
        assert!(files_in(&tmpdir().join("nope")).is_empty());
    }

    #[test]
    fn save_prunes_to_keep_newest() {
        let d = tmpdir();
        // 10-digit names sort like real <unix-ts>.json names.
        for i in 1_000_000_000..1_000_000_015u32 {
            std::fs::write(d.join(format!("{i}.json")), "[]").unwrap();
        }
        let dest = save_to(&d, &[(PathBuf::from("/a"), 1, "c".into())]).unwrap();
        let left = files_in(&d);
        assert_eq!(left.len(), KEEP, "prune should keep only {KEEP}");
        assert!(dest.exists(), "the just-written snapshot must survive");
        let _ = std::fs::remove_dir_all(&d);
    }
}
