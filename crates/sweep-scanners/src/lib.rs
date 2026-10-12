//! P0 scanners: system junk + trash. Knowledge-base paths cribbed from
//! CleanMyMac's SystemJunkScanning/TrashJunkScanning (see REPLICA_PLAN.md).
use std::path::{Path, PathBuf};
use sweep_core::{dir_size, home, JunkItem, Progress, Scanner};

pub mod uninstaller;
pub mod spacelens;
pub mod privacy;
pub mod performance;
pub mod protection;
pub mod clutter;
pub mod sigupdate;
pub mod malware;
pub mod clamsetup;

/// One item per top-level subdirectory of a big container root.
/// Pure over `path` so tests can point it at a temp dir.
fn items_for_root(name: &str, path: &Path) -> Vec<JunkItem> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(path) else { return out };
    for entry in rd.filter_map(|e| e.ok()) {
        let p = entry.path();
        if sweep_core::is_protected(&p) {
            continue;
        }
        if !p.is_dir() {
            continue;
        }
        let size = dir_size(&p, 6);
        if size > 0 {
            let child = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            out.push(JunkItem::new(p, size, &format!("{name} · {child}")));
        }
    }
    out.sort_by(|a, b| b.size.cmp(&a.size));
    out
}
/// Hardcoded per-app junk locations (mirrors their CMCachesKnowledgeBase rows).
fn app_junk_roots(h: &Path) -> Vec<(&'static str, PathBuf)> {
    let subs = [
        ("Spotify cache", "Library/Application Support/Spotify/PersistentCache/Storage"),
        ("Telegram cache", "Library/Application Support/Telegram Desktop/tdata/user_data"),
        ("Adobe media cache", "Library/Application Support/Adobe/Common/Media Cache Files"),
        ("Tencent WeChat", "Library/Containers/com.tencent.xinWeChat/Data/Library/Application Support"),
        ("Tencent QQMusic", "Library/Containers/com.tencent.QQMusicMac/Data/Library/Application Support"),
    ];
    subs.into_iter().map(|(n, s)| (n, h.join(s))).collect()
}

pub struct SystemJunk;

impl Scanner for SystemJunk {
    fn id(&self) -> &'static str {
        "system_junk"
    }
    fn name(&self) -> &'static str {
        "System Junk"
    }
    fn scan(&self, on_progress: &dyn Fn(Progress)) -> Vec<JunkItem> {
        let h = home();
        let mut roots: Vec<(String, PathBuf, bool)> = vec![
            ("User caches".into(), h.join("Library/Caches"), true),
            ("User logs".into(), h.join("Library/Logs"), true),
            ("Xcode derived data".into(), h.join("Library/Developer/Xcode/DerivedData"), true),
        ];
        for (n, p) in app_junk_roots(&h) {
            roots.push((n.to_string(), p, false));
        }
        let total = roots.len() as u64;
        let mut out = Vec::new();
        for (i, (name, path, expand)) in roots.iter().enumerate() {
            on_progress(Progress {
                done: i as u64,
                total,
                bytes: out.iter().map(|j: &JunkItem| j.size).sum(),
                label: name.clone(),
            });
            if !path.is_dir() {
                continue;
            }
            if sweep_core::is_protected(path) {
                continue;
            }
            if *expand {
                out.extend(items_for_root(name, path));
            } else {
                let size = dir_size(path, 6);
                if size > 0 {
                    out.push(JunkItem::new(path.clone(), size, name));
                }
            }
        }
        on_progress(Progress { done: total, total, bytes: out.iter().map(|j| j.size).sum(), label: "done".into() });
        out.sort_by(|a, b| b.size.cmp(&a.size));
        out
    }
}

pub struct TrashBin;

impl Scanner for TrashBin {
    fn id(&self) -> &'static str {
        "trash"
    }
    fn name(&self) -> &'static str {
        "Trash"
    }
    fn scan(&self, on_progress: &dyn Fn(Progress)) -> Vec<JunkItem> {
        let trash = home().join(".Trash");
        on_progress(Progress { done: 0, total: 1, bytes: 0, label: "Trash".into() });
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&trash) {
            for entry in rd.filter_map(|e| e.ok()) {
                let p = entry.path();
                if sweep_core::is_protected(&p) {
                    continue;
                }
                let size = if p.is_dir() { dir_size(&p, 8) } else { entry.metadata().map(|m| m.len()).unwrap_or(0) };
                if size > 0 {
                    out.push(JunkItem::new(p, size, "Trash"));
                }
            }
        }
        out.sort_by(|a, b| b.size.cmp(&a.size));
        on_progress(Progress { done: 1, total: 1, bytes: out.iter().map(|j| j.size).sum(), label: "done".into() });
        out
    }
}

/// All P0 scanners, in run order.
pub fn all() -> Vec<Box<dyn Scanner>> {
    vec![Box::new(SystemJunk), Box::new(TrashBin)]
}

/// P1: app-leftover scanners.
pub fn uninstallers() -> Vec<Box<dyn Scanner>> {
    vec![Box::new(uninstaller::Uninstaller)]
}

/// P1: disk-usage scanners.
pub fn spacelens() -> Vec<Box<dyn Scanner>> {
    vec![Box::new(spacelens::SpaceLens)]
}

/// P1: browser-data scanners.
pub fn privacy() -> Vec<Box<dyn Scanner>> {
    vec![Box::new(privacy::Privacy)]
}

/// P1: startup-item scanners.
pub fn performance() -> Vec<Box<dyn Scanner>> {
    vec![Box::new(performance::Performance)]
}

/// P2: suspicious-file scanners (audit-only).
pub fn protection() -> Vec<Box<dyn Scanner>> {
    vec![Box::new(protection::Protection), Box::new(malware::Malware::default())]
}

/// P3: old-and-large-file scanners.
pub fn clutter() -> Vec<Box<dyn Scanner>> {
    vec![Box::new(clutter::Clutter)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_root(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("sweep-test-{}-{}-{tag}", std::process::id(), line!()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn items_for_root_splits_per_subdir() {
        let root = tmp_root("split");
        fs::create_dir_all(root.join("com.apple.Safari")).unwrap();
        fs::create_dir_all(root.join("com.example.small")).unwrap();
        fs::create_dir_all(root.join("empty")).unwrap();
        fs::write(root.join("com.apple.Safari/f"), vec![0u8; 300]).unwrap();
        fs::write(root.join("com.example.small/f"), vec![0u8; 100]).unwrap();

        let items = items_for_root("User caches", &root);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].category, "User caches · com.apple.Safari");
        assert_eq!(items[0].size, 300);
        assert_eq!(items[1].category, "User caches · com.example.small");
        assert_eq!(items[1].size, 100);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn items_for_root_empty_or_missing_is_empty() {
        let root = tmp_root("empty");
        assert!(items_for_root("User caches", &root).is_empty());
        assert!(items_for_root("User caches", &root.join("nope")).is_empty());
        let _ = fs::remove_dir_all(&root);
    }
}
