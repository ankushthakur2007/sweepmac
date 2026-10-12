//! Ignore list: exact paths (one per line) in `~/.sweepmac/ignored`.
//! A path matches itself or anything under it (parent-dir prefix).
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Real ignore-list file.
pub fn file() -> PathBuf {
    crate::home().join(".sweepmac/ignored")
}

fn norm(s: &str) -> String {
    let t = s.trim();
    if t.len() > 1 {
        t.trim_end_matches('/').to_string()
    } else {
        t.to_string()
    }
}

/// Load from an explicit file (tests use temp files; shell passes [`file`]).
pub fn load_from(file: &Path) -> HashSet<String> {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .map(norm)
        .filter(|l| !l.is_empty())
        .collect()
}

/// Persist to an explicit file.
pub fn save_to(file: &Path, set: &HashSet<String>) -> Result<(), String> {
    if let Some(p) = file.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    let mut v: Vec<&String> = set.iter().collect();
    v.sort();
    let body = if v.is_empty() {
        String::new()
    } else {
        v.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n") + "\n"
    };
    std::fs::write(file, body).map_err(|e| e.to_string())
}

/// Load the real ignore list (missing file → empty).
pub fn load() -> HashSet<String> {
    load_from(&file())
}

/// Persist the real ignore list.
pub fn save(set: &HashSet<String>) -> Result<(), String> {
    save_to(&file(), set)
}

/// Add one path to an explicit file.
pub fn add_to(file: &Path, path_str: &str) -> Result<(), String> {
    let t = norm(path_str);
    if t.is_empty() {
        return Ok(());
    }
    let mut set = load_from(file);
    set.insert(t);
    save_to(file, &set)
}

/// Add one path to the real ignore list.
pub fn add(path_str: &str) -> Result<(), String> {
    add_to(&file(), path_str)
}

/// Pure match: exact path or anything under an ignored dir.
pub fn is_ignored(path: &Path, set: &HashSet<String>) -> bool {
    let owned = path.to_string_lossy().into_owned();
    let p = if owned.len() > 1 {
        owned.trim_end_matches('/')
    } else {
        &owned
    };
    set.iter()
        .filter(|e| !e.is_empty())
        .any(|e| p == *e || p.starts_with(&format!("{e}/")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp() -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "sweep-ignore-{}-{}.txt",
            std::process::id(),
            n
        ))
    }

    #[test]
    fn roundtrip() {
        let f = tmp();
        let mut set = HashSet::new();
        set.insert("/a/b".to_string());
        set.insert("/c".to_string());
        save_to(&f, &set).unwrap();
        assert_eq!(load_from(&f), set);
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn missing_file_is_empty() {
        assert!(load_from(&tmp()).is_empty());
    }

    #[test]
    fn exact_and_parent_prefix_match() {
        let set: HashSet<String> = ["/a/b", "/c"].iter().map(|s| s.to_string()).collect();
        assert!(is_ignored(Path::new("/a/b"), &set));
        assert!(is_ignored(Path::new("/a/b/c/d.txt"), &set));
        assert!(!is_ignored(Path::new("/a/bc"), &set));
        assert!(!is_ignored(Path::new("/a"), &set));
        assert!(is_ignored(Path::new("/c/x"), &set));
        assert!(!is_ignored(Path::new("/d"), &set));
    }

    #[test]
    fn add_to_persists() {
        let f = tmp();
        add_to(&f, "/x/y/").unwrap();
        add_to(&f, "/x/y/z").unwrap();
        let set = load_from(&f);
        assert!(is_ignored(Path::new("/x/y/z/w"), &set));
        assert!(is_ignored(Path::new("/x/y"), &set));
        let _ = std::fs::remove_file(&f);
    }
}
