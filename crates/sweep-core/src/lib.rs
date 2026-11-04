//! SweepMac core: scan → clean pipeline (mirrors CleanMyMac's
//! KnowledgeBase → Tasks → Processor → FileRemover, minus the bloat).
pub mod ignore;
pub mod rules;
pub mod snapshot;

use std::path::PathBuf;

/// One cleanable item found by a scanner.
#[derive(Debug, Clone)]
pub struct JunkItem {
    pub path: PathBuf,
    pub size: u64,
    pub category: String,
    pub selected: bool,
    pub selectable: bool,
}

impl JunkItem {
    pub fn new(path: PathBuf, size: u64, category: &str) -> Self {
        Self {
            path,
            size,
            category: category.into(),
            selected: true,
            selectable: true,
        }
    }
}

/// Progress snapshot streamed from a running scan/clean.
#[derive(Debug, Clone, Default)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
    pub bytes: u64,
    pub label: String,
}

/// Result of a scan pass.
#[derive(Debug, Default)]
pub struct ScanOutcome {
    pub items: Vec<JunkItem>,
    pub incomplete: Option<String>,
}

/// A scanner: enumerate candidates, never delete.
pub trait Scanner: Send {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn scan(&self, on_progress: &dyn Fn(Progress), is_cancelled: &dyn Fn() -> bool) -> ScanOutcome;
}

/// Result of a clean pass.
#[derive(Debug, Default)]
pub struct CleanResult {
    pub removed: u64,
    pub bytes_freed: u64,
    pub errors: Vec<String>,
    pub failed: Vec<PathBuf>,
}

/// Home dir, None if HOME missing/invalid.
pub fn home() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty() && p.is_absolute())
}

/// Sum of file sizes under `dir` (bounded depth, symlink-safe, errors ignored).
/// Sparse files count logical size; callers needing disk usage should use
/// `dir_allocated_size` instead.
pub fn dir_size(dir: &std::path::Path, max_depth: usize) -> u64 {
    walkdir::WalkDir::new(dir)
        .max_depth(max_depth)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

/// Allocated disk bytes for `p` via `st_blocks` (actual blocks), falling back
/// to logical size on non-macOS or stat failure.
#[cfg(target_os = "macos")]
fn allocated_from_meta(m: &std::fs::Metadata) -> u64 {
    use std::os::darwin::fs::MetadataExt;
    m.st_blocks().saturating_mul(512)
}

#[cfg(not(target_os = "macos"))]
fn allocated_from_meta(m: &std::fs::Metadata) -> u64 {
    m.len()
}

/// Actual disk usage (allocated blocks) of `p`, or 0 when unreadable.
pub fn allocated_size(p: &std::path::Path) -> u64 {
    std::fs::metadata(p)
        .map(|m| allocated_from_meta(&m))
        .unwrap_or(0)
}

/// File size or 0 when unreadable (logical `st_size`).
pub fn file_size(p: &std::path::Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

/// Sum of allocated disk bytes under `dir` (bounded depth, symlink-safe, errors
/// ignored).
pub fn dir_allocated_size(dir: &std::path::Path, max_depth: usize) -> u64 {
    walkdir::WalkDir::new(dir)
        .max_depth(max_depth)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| allocated_from_meta(&m))
        .sum()
}

/// True when SIP protects `p` (`stat` flag `SF_RESTRICTED`, 0x80000).
/// Public equivalent of CleanMyMac's `isFileProtectedByRootless`
/// (which forwards to Apple's own check). False on any stat error.
pub fn is_protected(p: &std::path::Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::os::darwin::fs::MetadataExt;
        std::fs::metadata(p)
            .map(|m| m.st_flags() & 0x80000 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = p;
        false
    }
}

/// An explicitly opt-in item (audit-style scanners start unselected).
pub fn unselected(path: PathBuf, size: u64, category: &str) -> JunkItem {
    let mut item = JunkItem::new(path, size, category);
    item.selected = false;
    item
}

/// Move paths to the Trash (never hard-delete in P0).
/// Batch first via trash::delete_all; fall back per-item on failure.
pub fn trash_paths(paths: &[PathBuf]) -> CleanResult {
    let mut out = CleanResult::default();
    if paths.is_empty() {
        return out;
    }
    match trash::delete_all(paths) {
        Ok(()) => {
            out.removed += paths.len() as u64;
            return out;
        }
        Err(_) => {
            for p in paths {
                match trash::delete(p) {
                    Ok(()) => out.removed += 1,
                    Err(e) => {
                        out.errors.push(format!("{}: {e}", p.display()));
                        out.failed.push(p.clone());
                    }
                }
            }
        }
    }
    out
}

pub fn fmt_bytes(n: u64) -> String {
    const U: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u + 1 < U.len() {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", U[u])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn allocated_size_at_least_logical_for_temp_file() {
        let p = std::env::temp_dir().join(format!("sweep-alloc-{}", std::process::id()));
        fs::write(&p, vec![0u8; 100]).unwrap();
        let logical = fs::metadata(&p).unwrap().len();
        let alloc = allocated_size(&p);
        assert!(alloc >= logical, "alloc={alloc} logical={logical}");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn dir_allocated_size_sums() {
        let root = std::env::temp_dir().join(format!("sweep-diralloc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("sub/a.bin"), vec![0u8; 100]).unwrap();
        assert!(dir_allocated_size(&root, 6) >= 100);
        let _ = fs::remove_dir_all(&root);
    }
}
