//! Exact-duplicate finder: byte-identical files in ~/Downloads, ~/Desktop,
//! ~/Documents. Groups by size → prefix hash → full hash, then revalidates
//! stat before emitting. Nothing is preselected.
use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::hash::Hasher;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use sweep_core::{home, is_protected, unselected, JunkItem, Progress, ScanOutcome, Scanner};
use walkdir::WalkDir;

const PREFIX: usize = 64 * 1024;
const MAX_DEPTH: usize = 8;
const ROOTS_LABEL: &str = "Duplicate";

fn is_bundle_or_library(p: &Path) -> bool {
    p.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        s.ends_with(".app") || s.ends_with(".photoslibrary")
    })
}

fn is_hidden_dir(e: &walkdir::DirEntry) -> bool {
    e.file_type().is_dir() && e.file_name().to_string_lossy().starts_with('.')
}

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
        "/Volumes",
    ];
    let s = p.to_string_lossy();
    SKIP.iter()
        .any(|k| s == *k || s.starts_with(&format!("{k}/")))
}

#[derive(Clone)]
struct Cand {
    path: PathBuf,
    size: u64,
    mtime: SystemTime,
}

fn hash_file(path: &Path, full: bool) -> Option<u64> {
    let mut f = File::open(path).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut buf = vec![0u8; 64 * 1024];
    if full {
        loop {
            let n = f.read(&mut buf).ok()?;
            if n == 0 {
                break;
            }
            h.write(&buf[..n]);
        }
    } else {
        let plen = PREFIX.min(buf.len());
        let n = f.read(&mut buf[..plen]).ok()?;
        h.write(&buf[..n]);
    }
    Some(h.finish())
}

fn files_equal(a: &Path, b: &Path, size: u64) -> bool {
    if a == b {
        return true;
    }
    let Ok(mut fa) = File::open(a) else {
        return false;
    };
    let Ok(mut fb) = File::open(b) else {
        return false;
    };
    let mut bufa = vec![0u8; 64 * 1024];
    let mut bufb = vec![0u8; 64 * 1024];
    let mut remaining = size;
    while remaining > 0 {
        let chunk = remaining.min(64 * 1024) as usize;
        if fa.read_exact(&mut bufa[..chunk]).is_err() {
            return false;
        }
        if fb.read_exact(&mut bufb[..chunk]).is_err() {
            return false;
        }
        if bufa[..chunk] != bufb[..chunk] {
            return false;
        }
        remaining -= chunk as u64;
    }
    true
}

fn collect(roots: &[PathBuf], is_cancelled: &dyn Fn() -> bool) -> Vec<Cand> {
    let mut out = Vec::new();
    let mut seen_inode: HashSet<(u64, u64)> = HashSet::new();
    for root in roots {
        if is_cancelled() {
            return out;
        }
        if !root.is_dir() {
            continue;
        }
        let walker = WalkDir::new(root)
            .max_depth(MAX_DEPTH)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                !is_hidden_dir(e)
                    && !is_system_skipped(e.path())
                    && !is_protected(e.path())
                    && !is_bundle_or_library(e.path())
            })
            .filter_map(|e| e.ok());
        for entry in walker {
            if is_cancelled() {
                return out;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path().to_path_buf();
            if is_bundle_or_library(&path) {
                continue;
            }
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            if sweep_core::is_protected(&path) {
                continue;
            }
            use std::os::unix::fs::MetadataExt;
            let key = (meta.dev(), meta.ino());
            if !seen_inode.insert(key) {
                // Same (dev,ino): hard link / same physical file — skip.
                continue;
            }
            let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            out.push(Cand {
                path,
                size: meta.len(),
                mtime,
            });
        }
    }
    out
}

/// Scan the given roots and return one JunkItem per file that belongs to a
/// duplicate group (all copies, none preselected).
pub fn scan_roots(roots: &[PathBuf], is_cancelled: &dyn Fn() -> bool) -> Vec<JunkItem> {
    let cands = collect(roots, is_cancelled);
    if is_cancelled() {
        return Vec::new();
    }

    // Group by size.
    let mut by_size: BTreeMap<u64, Vec<Cand>> = BTreeMap::new();
    for c in cands {
        if c.size == 0 { /* keep empty files too */ }
        by_size.entry(c.size).or_default().push(c);
    }

    let mut out: Vec<JunkItem> = Vec::new();
    for (_size, group) in by_size.iter() {
        if is_cancelled() {
            return out;
        }
        if group.len() < 2 {
            continue;
        }
        // Prefix hash.
        let mut by_prefix: BTreeMap<u64, Vec<&Cand>> = BTreeMap::new();
        for c in group {
            if is_cancelled() {
                return out;
            }
            let Some(h) = hash_file(&c.path, false) else {
                continue;
            };
            by_prefix.entry(h).or_default().push(c);
        }
        for (_ph, pg) in by_prefix.iter() {
            if is_cancelled() {
                return out;
            }
            if pg.len() < 2 {
                continue;
            }
            // Full hash.
            let mut by_full: BTreeMap<u64, Vec<&Cand>> = BTreeMap::new();
            for c in pg {
                if is_cancelled() {
                    return out;
                }
                let Some(h) = hash_file(&c.path, true) else {
                    continue;
                };
                by_full.entry(h).or_default().push(*c);
            }
            for (_fh, fg) in by_full.iter() {
                if is_cancelled() {
                    return out;
                }
                if fg.len() < 2 {
                    continue;
                }
                // Revalidate size + mtime; collect surviving.
                let mut valid: Vec<&Cand> = Vec::new();
                for c in fg {
                    if is_cancelled() {
                        return out;
                    }
                    let Ok(m) = std::fs::metadata(&c.path) else {
                        continue;
                    };
                    if m.len() != c.size {
                        continue;
                    }
                    let mt = m.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                    if mt != c.mtime {
                        continue;
                    }
                    valid.push(c);
                }
                if valid.len() < 2 {
                    continue;
                }
                // Byte-verify all against the first survivor.
                let first = valid[0];
                let mut verified: Vec<&Cand> = vec![first];
                for c in &valid[1..] {
                    if is_cancelled() {
                        return out;
                    }
                    if files_equal(&first.path, &c.path, first.size) {
                        verified.push(*c);
                    }
                }
                if verified.len() < 2 {
                    continue;
                }
                for c in verified {
                    out.push(unselected(c.path.clone(), c.size, ROOTS_LABEL));
                }
            }
        }
    }
    out.sort_by(|a, b| b.size.cmp(&a.size).then(a.path.cmp(&b.path)));
    out
}

/// Convenience alias for single-root callers / tests.
pub fn scan_root(root: &Path, is_cancelled: &dyn Fn() -> bool) -> Vec<JunkItem> {
    scan_roots(&[root.to_path_buf()], is_cancelled)
}

/// Alias used by older callers.
pub fn find_duplicates(roots: &[PathBuf], is_cancelled: &dyn Fn() -> bool) -> Vec<JunkItem> {
    scan_roots(roots, is_cancelled)
}

pub fn duplicates_scan(roots: &[PathBuf], is_cancelled: &dyn Fn() -> bool) -> Vec<JunkItem> {
    scan_roots(roots, is_cancelled)
}

pub struct Duplicates;

impl Scanner for Duplicates {
    fn id(&self) -> &'static str {
        "duplicates"
    }
    fn name(&self) -> &'static str {
        "Duplicates"
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
        let roots = [h.join("Downloads"), h.join("Desktop"), h.join("Documents")];
        let roots: Vec<PathBuf> = roots.into_iter().filter(|p| p.is_dir()).collect();
        let total = roots.len() as u64;
        if total == 0 {
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
        }
        for (i, r) in roots.iter().enumerate() {
            if is_cancelled() {
                return ScanOutcome {
                    items: Vec::new(),
                    incomplete: Some("Cancelled".into()),
                };
            }
            on_progress(Progress {
                done: i as u64,
                total,
                bytes: 0,
                label: r.display().to_string(),
            });
        }
        let items = scan_roots(&roots, is_cancelled);
        if is_cancelled() {
            return ScanOutcome {
                items,
                incomplete: Some("Cancelled".into()),
            };
        }
        on_progress(Progress {
            done: total,
            total,
            bytes: items.iter().map(|j| j.size).sum(),
            label: "done".into(),
        });
        ScanOutcome {
            items,
            incomplete: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp(tag: &str) -> PathBuf {
        let p = PathBuf::from("/tmp").join(format!(
            "dup-{}-{}-{}",
            tag,
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn identical_files_found() {
        let root = tmp("ident");
        fs::write(root.join("a.txt"), b"hello world").unwrap();
        fs::write(root.join("b.txt"), b"hello world").unwrap();
        fs::write(root.join("c.txt"), b"different").unwrap();
        let items = scan_roots(&[root.clone()], &|| false);
        assert_eq!(items.len(), 2, "{items:?}");
        assert!(items.iter().all(|i| i.category == "Duplicate"));
        assert!(items.iter().all(|i| !i.selected));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn hardlink_skipped() {
        let root = tmp("hl");
        fs::write(root.join("a.txt"), b"hello world").unwrap();
        let _ = fs::hard_link(root.join("a.txt"), root.join("b.txt"));
        let items = scan_roots(&[root.clone()], &|| false);
        assert!(items.is_empty(), "{items:?}");
        let _ = fs::remove_dir_all(&root);
    }
}
