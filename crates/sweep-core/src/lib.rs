//! SweepMac core: scan → clean pipeline (mirrors CleanMyMac's
//! KnowledgeBase → Tasks → Processor → FileRemover, minus the bloat).
pub mod ignore;
pub mod snapshot;

use std::path::PathBuf;

/// One cleanable item found by a scanner.
#[derive(Debug, Clone)]
pub struct JunkItem {
    pub path: PathBuf,
    pub size: u64,
    pub category: String,
    pub selected: bool,
}

impl JunkItem {
    pub fn new(path: PathBuf, size: u64, category: &str) -> Self {
        Self { path, size, category: category.into(), selected: true }
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

/// A scanner: enumerate candidates, never delete.
pub trait Scanner: Send {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn scan(&self, on_progress: &dyn Fn(Progress)) -> Vec<JunkItem>;
}

/// Result of a clean pass.
#[derive(Debug, Default)]
pub struct CleanResult {
    pub removed: u64,
    pub bytes_freed: u64,
    pub errors: Vec<String>,
}

/// Home dir, `/` fallback (keeps scanners total: no panics on odd envs).
pub fn home() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/"))
}

/// Sum of file sizes under `dir` (bounded depth, symlink-safe, errors ignored).
/// Sparse files count logical size; callers needing disk usage should use
/// `st_blocks` instead (see SpaceLens).
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

/// File size or 0 when unreadable.
pub fn file_size(p: &std::path::Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

/// True when SIP protects `p` (`stat` flag `SF_RESTRICTED`, 0x80000).
/// Public equivalent of CleanMyMac's `isFileProtectedByRootless`
/// (which forwards to Apple's own check). False on any stat error.
pub fn is_protected(p: &std::path::Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::os::darwin::fs::MetadataExt;
        std::fs::metadata(p).map(|m| m.st_flags() & 0x80000 != 0).unwrap_or(false)
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
pub fn trash_paths(paths: &[PathBuf]) -> CleanResult {
    let mut out = CleanResult::default();
    for p in paths {
        match trash::delete(p) {
            Ok(()) => out.removed += 1,
            Err(e) => out.errors.push(format!("{}: {e}", p.display())),
        }
    }
    out
}

pub fn fmt_bytes(n: u64) -> String {    const U: &[&str] = &["B", "KB", "MB", "GB", "TB"];
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
