//! SpaceLens scanner: top-level folder sizes under fixed roots.
//! Informational only — items default to unselected, the user opts in.
use std::path::{Path, PathBuf};
use sweep_core::{dir_size, home, JunkItem, Progress, Scanner};

fn roots() -> Vec<PathBuf> {
    let h = home();
    vec![
        h.clone(),
        h.join("Library"),
        PathBuf::from("/Applications"),
        h.join("Documents"),
        h.join("Downloads"),
        h.join("Movies"),
    ]
}

/// Drop roots nested under another root; keep the outermost.
/// `canonical` holds (original, canonicalized) pairs; survivors preserve input order.
fn prune_roots_canonical(canonical: &[(PathBuf, PathBuf)]) -> Vec<PathBuf> {
    canonical
        .iter()
        .enumerate()
        .filter(|(i, (_, c))| {
            !canonical.iter().enumerate().any(|(j, (_, o))| {
                j != *i && c.starts_with(o) && (c != o || j < *i)
            })
        })
        .map(|(_, (orig, _))| orig.clone())
        .collect()
}

fn prune_roots(roots: Vec<PathBuf>) -> Vec<PathBuf> {
    let pairs: Vec<(PathBuf, PathBuf)> = roots
        .into_iter()
        .filter_map(|r| std::fs::canonicalize(&r).ok().map(|c| (r, c)))
        .collect();
    prune_roots_canonical(&pairs)
}
/// One unselected JunkItem per top-level child dir of `root`; unreadable skipped.
fn children_sizes(root: &Path) -> Vec<JunkItem> {
    let Ok(rd) = std::fs::read_dir(root) else { return Vec::new() };
    rd.filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir() && !p.is_symlink())
        .filter(|p| !sweep_core::is_protected(p))
        .filter_map(|p| {
            let size = dir_size(&p, 6);
            (size > 0).then(|| JunkItem { path: p, size, category: "Folder".into(), selected: false })
        })
        .collect()
}

pub struct SpaceLens;

impl Scanner for SpaceLens {
    fn id(&self) -> &'static str {
        "spacelens"
    }
    fn name(&self) -> &'static str {
        "Large Folders"
    }
    fn scan(&self, on_progress: &dyn Fn(Progress)) -> Vec<JunkItem> {
        let existing = prune_roots(roots().into_iter().filter(|r| r.is_dir()).collect());
        let total = existing.len() as u64;
        let mut out = Vec::new();
        std::thread::scope(|s| {
            let handles: Vec<_> = existing.iter().map(|r| s.spawn(|| children_sizes(r))).collect();
            for (i, (root, h)) in existing.iter().zip(handles).enumerate() {
                out.extend(h.join().unwrap_or_default());
                on_progress(Progress {
                    done: i as u64 + 1,
                    total,
                    bytes: out.iter().map(|j| j.size).sum(),
                    label: root.display().to_string(),
                });
            }
        });
        out.sort_by(|a, b| b.size.cmp(&a.size));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_root(case: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "spacelens-{}-{}-{}",
            case,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn spacelens_sums_nested_files_per_child_dir() {
        let root = tmp_root("sums");
        let kid = root.join("kid");
        std::fs::create_dir_all(kid.join("nested")).unwrap();
        std::fs::write(kid.join("a.bin"), vec![0u8; 100]).unwrap();
        std::fs::write(kid.join("nested/b.bin"), vec![1u8; 200]).unwrap();
        std::fs::write(root.join("top.bin"), vec![2u8; 50]).unwrap(); // plain file: ignored
        assert_eq!(dir_size(&kid, 6), 300);
        let items = children_sizes(&root);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].path, kid);
        assert_eq!(items[0].size, 300);
        assert!(!items[0].selected);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn spacelens_scanner_meta() {
        let s = SpaceLens;
        assert_eq!(s.id(), "spacelens");
        assert_eq!(s.name(), "Large Folders");
    }

    #[test]
    fn prune_nested_pairs_collapse_to_outermost() {
        let pairs = vec![
            (PathBuf::from("/home/u"), PathBuf::from("/home/u")),
            (PathBuf::from("/home/u/Library"), PathBuf::from("/home/u/Library")),
            (PathBuf::from("/home/u/Documents"), PathBuf::from("/home/u/Documents")),
        ];
        assert_eq!(prune_roots_canonical(&pairs), vec![PathBuf::from("/home/u")]);
    }

    #[test]
    fn prune_disjoint_roots_survive() {
        let pairs = vec![
            (PathBuf::from("/a"), PathBuf::from("/a")),
            (PathBuf::from("/b"), PathBuf::from("/b")),
        ];
        assert_eq!(
            prune_roots_canonical(&pairs),
            vec![PathBuf::from("/a"), PathBuf::from("/b")]
        );
    }

    #[test]
    fn prune_preserves_input_order() {
        let pairs = vec![
            (PathBuf::from("/z"), PathBuf::from("/z")),
            (PathBuf::from("/a"), PathBuf::from("/a")),
            (PathBuf::from("/a/nested"), PathBuf::from("/a/nested")),
        ];
        assert_eq!(
            prune_roots_canonical(&pairs),
            vec![PathBuf::from("/z"), PathBuf::from("/a")]
        );
    }
}
