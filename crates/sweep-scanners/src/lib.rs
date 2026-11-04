//! P0 scanners: system junk + trash. Knowledge-base paths cribbed from
//! CleanMyMac's SystemJunkScanning/TrashJunkScanning (see REPLICA_PLAN.md).
use std::path::{Path, PathBuf};
use sweep_core::{dir_allocated_size, dir_size, home, JunkItem, Progress, ScanOutcome, Scanner};

pub mod clamsetup;
pub mod clutter;
pub mod duplicates;
pub mod malware;
pub mod performance;
pub mod privacy;
pub mod protection;
pub mod sigupdate;
pub mod spacelens;
pub mod uninstaller;

/// One item per top-level subdirectory of a big container root.
/// Pure over `path` so tests can point it at a temp dir.
fn items_for_root(name: &str, path: &Path) -> Vec<JunkItem> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(path) else {
        return out;
    };
    for entry in rd.filter_map(|e| e.ok()) {
        let p = entry.path();
        if sweep_core::is_protected(&p) {
            continue;
        }
        if !p.is_dir() {
            continue;
        }
        let size = dir_allocated_size(&p, 6);
        if size > 0 {
            let child = p
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            out.push(JunkItem::new(p, size, &format!("{name} · {child}")));
        }
    }
    out.sort_by(|a, b| b.size.cmp(&a.size));
    out
}
/// Hardcoded per-app junk locations (mirrors their CMCachesKnowledgeBase rows).
fn app_junk_roots(h: &Path) -> Vec<(&'static str, PathBuf)> {
    let subs = [
        (
            "Spotify cache",
            "Library/Application Support/Spotify/PersistentCache/Storage",
        ),
        (
            "Telegram cache",
            "Library/Application Support/Telegram Desktop/tdata/user_data",
        ),
        (
            "Adobe media cache",
            "Library/Application Support/Adobe/Common/Media Cache Files",
        ),
        (
            "Tencent WeChat",
            "Library/Containers/com.tencent.xinWeChat/Data/Library/Application Support",
        ),
        (
            "Tencent QQMusic",
            "Library/Containers/com.tencent.QQMusicMac/Data/Library/Application Support",
        ),
    ];
    subs.into_iter().map(|(n, s)| (n, h.join(s))).collect()
}

/// Declarative list of system junk roots: (label, path, expand_per_subdir, selected).
fn system_junk_roots(h: &Path) -> Vec<(String, PathBuf, bool, bool)> {
    let mut roots: Vec<(String, PathBuf, bool, bool)> = vec![
        ("User caches".into(), h.join("Library/Caches"), true, true),
        ("User logs".into(), h.join("Library/Logs"), true, true),
        (
            "Xcode derived data".into(),
            h.join("Library/Developer/Xcode/DerivedData"),
            false,
            true,
        ),
        // review-level Xcode artifacts
        (
            "Xcode archives".into(),
            h.join("Library/Developer/Xcode/Archives"),
            false,
            false,
        ),
        (
            "Xcode device support".into(),
            h.join("Library/Developer/Xcode/iOS DeviceSupport"),
            false,
            false,
        ),
        (
            "CoreSimulator devices".into(),
            h.join("Library/Developer/CoreSimulator/Devices"),
            false,
            false,
        ),
    ];
    roots.extend([
        // developer tool caches (safe)
        (
            "Homebrew cache".into(),
            h.join("Library/Caches/Homebrew"),
            false,
            true,
        ),
        (
            "Homebrew cache".into(),
            h.join(".cache/Homebrew"),
            false,
            true,
        ),
        ("npm cache".into(), h.join(".npm/_cacache"), false, true),
        ("pnpm store".into(), h.join(".pnpm-store"), false, true),
        (
            "pnpm store".into(),
            h.join("Library/pnpm/store"),
            false,
            true,
        ),
        (
            "Yarn cache".into(),
            h.join("Library/Caches/Yarn"),
            false,
            true,
        ),
        ("Yarn cache".into(), h.join(".yarn/cache"), false, true),
        (
            "Cargo registry cache".into(),
            h.join(".cargo/registry/cache"),
            false,
            true,
        ),
        (
            "Cargo registry src".into(),
            h.join(".cargo/registry/src"),
            false,
            true,
        ),
        (
            "pip cache".into(),
            h.join("Library/Caches/pip"),
            false,
            true,
        ),
        ("pip cache".into(), h.join(".cache/pip"), false, true),
        ("Gradle cache".into(), h.join(".gradle/caches"), false, true),
        // review-level AI model caches
        (
            "Hugging Face cache".into(),
            h.join(".cache/huggingface"),
            false,
            false,
        ),
        ("Ollama models".into(), h.join(".ollama"), false, false),
        // review-level Mail downloads
        (
            "Mail downloads".into(),
            h.join("Library/Containers/com.apple.mail/Data/Library/Mail Downloads"),
            false,
            false,
        ),
    ]);
    // shortcut: docker build cache needs docker CLI + permissions; enumerated would also
    // touch VM disks. Skipped per plan — leave a comment only.
    // docker: ~/Library/Containers/com.docker.docker/Data/vms (not scanned here).
    for (n, p) in app_junk_roots(h) {
        roots.push((n.to_string(), p, false, true));
    }
    roots
}

pub struct SystemJunk;

impl Scanner for SystemJunk {
    fn id(&self) -> &'static str {
        "system_junk"
    }
    fn name(&self) -> &'static str {
        "System Junk"
    }
    fn scan(&self, on_progress: &dyn Fn(Progress), is_cancelled: &dyn Fn() -> bool) -> ScanOutcome {
        let Some(h) = home() else {
            on_progress(Progress {
                done: 0,
                total: 0,
                bytes: 0,
                label: "done".into(),
            });
            return ScanOutcome {
                items: Vec::new(),
                incomplete: None,
            };
        };
        let roots = system_junk_roots(&h);
        let total = roots.len() as u64;
        let mut out = Vec::new();
        for (i, (name, path, expand, selected)) in roots.iter().enumerate() {
            if is_cancelled() {
                out.sort_by(|a: &JunkItem, b| b.size.cmp(&a.size));
                return ScanOutcome {
                    items: out,
                    incomplete: Some("Cancelled".into()),
                };
            }
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
                    if *selected {
                        out.push(JunkItem::new(path.clone(), size, name));
                    } else {
                        out.push(sweep_core::unselected(path.clone(), size, name));
                    }
                }
            }
        }
        out.sort_by(|a: &JunkItem, b| b.size.cmp(&a.size));
        if is_cancelled() {
            return ScanOutcome {
                items: out,
                incomplete: Some("Cancelled".into()),
            };
        }
        on_progress(Progress {
            done: total,
            total,
            bytes: out.iter().map(|j| j.size).sum(),
            label: "done".into(),
        });
        ScanOutcome {
            items: out,
            incomplete: None,
        }
    }
}

/// All P0 scanners, in run order.
pub fn all() -> Vec<Box<dyn Scanner>> {
    vec![Box::new(SystemJunk)]
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
    vec![
        Box::new(protection::Protection),
        Box::new(malware::Malware::default()),
    ]
}

/// P3: old-and-large-file scanners.
pub fn clutter() -> Vec<Box<dyn Scanner>> {
    vec![Box::new(clutter::Clutter)]
}

/// P3: exact-duplicate finder.
pub fn duplicates() -> Vec<Box<dyn Scanner>> {
    vec![Box::new(duplicates::Duplicates)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_root(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "sweep-test-{}-{}-{tag}",
            std::process::id(),
            line!()
        ));
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
        fs::write(root.join("com.apple.Safari/f"), vec![0u8; 9000]).unwrap();
        fs::write(root.join("com.example.small/f"), vec![0u8; 100]).unwrap();

        let items = items_for_root("User caches", &root);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].category, "User caches · com.apple.Safari");
        assert_eq!(
            items[0].size,
            sweep_core::dir_allocated_size(&root.join("com.apple.Safari"), 6)
        );
        assert_eq!(items[1].category, "User caches · com.example.small");
        assert_eq!(
            items[1].size,
            sweep_core::dir_allocated_size(&root.join("com.example.small"), 6)
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn items_for_root_empty_or_missing_is_empty() {
        let root = tmp_root("empty");
        assert!(items_for_root("User caches", &root).is_empty());
        assert!(items_for_root("User caches", &root.join("nope")).is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn system_junk_roots_includes_npm_cache() {
        let h = PathBuf::from("/tmp/home");
        let roots = system_junk_roots(&h);
        assert!(roots
            .iter()
            .any(|(name, path, _, _)| name == "npm cache" && path == &h.join(".npm/_cacache")));
    }
}
