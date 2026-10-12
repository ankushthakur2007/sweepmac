//! Clutter scanner: oversized files, stale installers, and untouched downloads
//! — system-scoped (whole drive minus SIP paths), like every other module.
//! Conservative by default — only old installers are pre-selected.
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use sweep_core::{home, JunkItem, Progress, Scanner};

const LARGE_BYTES: u64 = 500 * 1024 * 1024;
const INSTALLER_DAYS: u64 = 30;
const STALE_DAYS: u64 = 180;

fn is_installer(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => matches!(ext.to_ascii_lowercase().as_str(), "dmg" | "pkg" | "mpkg" | "zip"),
        None => false,
    }
}

fn age_days(mtime: SystemTime) -> Option<u64> {
    SystemTime::now().duration_since(mtime).ok().map(|d| d.as_secs() / 86400)
}

/// Classify one file; first match wins: large > old installer > stale download.
fn classify(path: &Path, len: u64, age: Option<u64>, in_downloads: bool) -> Option<(&'static str, bool)> {
    if len > LARGE_BYTES {
        return Some(("Large file", false));
    }
    let old_enough = |days: u64| age.map(|a| a >= days).unwrap_or(false);
    if is_installer(path) && old_enough(INSTALLER_DAYS) {
        return Some(("Old installer", true));
    }
    if in_downloads && old_enough(STALE_DAYS) {
        return Some(("Stale download", false));
    }
    None
}

fn is_hidden_dir(entry: &walkdir::DirEntry) -> bool {
    entry.file_type().is_dir() && entry.file_name().to_string_lossy().starts_with('.')
}

/// Subtrees that are SIP-protected or never user-actionable: walk past them
/// instead of flagging undeletable system files.
fn is_system_skipped(p: &Path) -> bool {
    const SKIP: &[&str] = &[
        "/System",
        "/private",
        "/bin",
        "/sbin",
        "/usr/bin",
        "/usr/sbin",
        "/usr/lib",
        "/dev",
        "/etc",
        "/var",
    ];
    let s = p.to_string_lossy();
    SKIP.iter().any(|k| s == *k || s.starts_with(&format!("{k}/")))
}

fn scan_root(root: &Path, in_downloads: bool) -> Vec<JunkItem> {
    let mut out = Vec::new();
    let walker = walkdir::WalkDir::new(root)
        .max_depth(4)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !is_hidden_dir(e) && !is_system_skipped(e.path()) && !sweep_core::is_protected(e.path()))
        .filter_map(|e| e.ok());
    for entry in walker {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(md) = entry.metadata() else { continue };
        let age = md.modified().ok().and_then(age_days);
        let Some((category, selected)) = classify(entry.path(), md.len(), age, in_downloads) else { continue };
        out.push(JunkItem { path: entry.path().to_path_buf(), size: md.len(), category: category.into(), selected });
    }
    out
}

pub struct Clutter;

impl Scanner for Clutter {
    fn id(&self) -> &'static str {
        "clutter"
    }
    fn name(&self) -> &'static str {
        "Old & Large Files"
    }
    fn scan(&self, on_progress: &dyn Fn(Progress)) -> Vec<JunkItem> {
        let h = home();
        let downloads = h.join("Downloads");
        // System-scoped like every other module: whole drive (SIP paths
        // pruned in scan_root), all users, apps, libraries — not just home.
        let roots = [
            PathBuf::from("/"),
            PathBuf::from("/Applications"),
            PathBuf::from("/Library"),
            PathBuf::from("/Users"),
            PathBuf::from("/opt"),
            downloads.clone(),
            h.join("Desktop"),
            h.join("Documents"),
            h.join("Movies"),
        ];
        let total = roots.len() as u64;
        let mut out = Vec::new();
        for (i, root) in roots.iter().enumerate() {
            on_progress(Progress {
                done: i as u64,
                total,
                bytes: out.iter().map(|j: &JunkItem| j.size).sum(),
                label: root.display().to_string(),
            });
            if root.is_dir() {
                out.extend(scan_root(root, root == &downloads));
            }
        }
        on_progress(Progress { done: total, total, bytes: out.iter().map(|j| j.size).sum(), label: "done".into() });
        out.sort_by(|a, b| b.size.cmp(&a.size));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_root(case: &str) -> PathBuf {
        // NOTE: plain /tmp, not temp_dir(): macOS temp lives under /var,
        // which the SIP filter prunes by design.
        let p = PathBuf::from("/tmp").join(format!(
            "clutter-{}-{}-{}",
            case,
            std::process::id(),
            SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn clutter_scanner_meta() {
        let s = Clutter;
        assert_eq!(s.id(), "clutter");
        assert_eq!(s.name(), "Old & Large Files");
    }

    #[test]
    fn large_file_rule_fires_on_fresh_sparse_file() {
        let root = tmp_root("large");
        let big = root.join("movie.mkv");
        std::fs::File::create(&big).unwrap().set_len(LARGE_BYTES + 1).unwrap();
        std::fs::write(root.join("small.txt"), b"hi").unwrap();
        let items = scan_root(&root, false);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].path, big);
        assert_eq!(items[0].category, "Large file");
        assert!(!items[0].selected);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn installer_extension_match_but_fresh_files_not_flagged() {
        for (name, want) in [("x.dmg", true), ("x.pkg", true), ("x.mpkg", true), ("x.zip", true), ("x.DMG", true), ("x.txt", false), ("x.app", false), ("noext", false)] {
            assert_eq!(is_installer(Path::new(name)), want, "{name}");
        }
        assert_eq!(classify(Path::new("x.dmg"), 100, Some(31), false), Some(("Old installer", true)));
        assert_eq!(classify(Path::new("x.dmg"), 100, Some(0), false), None);
        assert_eq!(classify(Path::new("x.dmg"), 100, None, false), None);
        // End to end: a fresh installer on disk is recognised but not flagged.
        let root = tmp_root("fresh-installer");
        std::fs::write(root.join("setup.dmg"), b"fake").unwrap();
        assert!(is_installer(&root.join("setup.dmg")));
        assert!(scan_root(&root, false).is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn stale_download_needs_downloads_root_and_age() {
        let p = Path::new("old.pdf");
        assert_eq!(classify(p, 100, Some(200), true), Some(("Stale download", false)));
        assert_eq!(classify(p, 100, Some(200), false), None);
        assert_eq!(classify(p, 100, Some(10), true), None);
        assert_eq!(classify(p, 100, None, true), None);
    }

    #[test]
    fn hidden_dot_dirs_are_skipped() {
        let root = tmp_root("hidden");
        let hidden = root.join(".hidden");
        std::fs::create_dir_all(&hidden).unwrap();
        std::fs::File::create(hidden.join("big.bin")).unwrap().set_len(LARGE_BYTES + 1).unwrap();
        assert!(scan_root(&root, false).is_empty());
        std::fs::remove_dir_all(&root).ok();
    }
}
