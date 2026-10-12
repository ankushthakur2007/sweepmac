//! Suspicious-files audit: quarantined downloads + non-Apple system daemons.
//! Read-only enumeration (audit, user opts in); everything ships with
//! `selected = false`.
use std::path::Path;

use sweep_core::{file_size, home, unselected, JunkItem, Progress, Scanner};

fn is_candidate_ext(p: &Path) -> bool {
    p.extension()
        .and_then(|s| s.to_str())
        .map(|s| matches!(s.to_ascii_lowercase().as_str(), "dmg" | "pkg" | "sh" | "command"))
        .unwrap_or(false)
}

/// True when the first 4 bytes match Mach-O magic (LE64 / FAT).
fn is_macho(p: &Path) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(p) else {
        return false;
    };
    let mut magic = [0u8; 4];
    if f.read_exact(&mut magic).is_err() {
        return false;
    }
    magic == [0xCF, 0xFA, 0xED, 0xFE] || magic == [0xCA, 0xFE, 0xBA, 0xBE]
}

// getxattr presence-probe declarations (macOS takes position+options).
#[cfg(target_os = "macos")]
extern "C" {
    fn getxattr(
        path: *const std::os::raw::c_char,
        name: *const std::os::raw::c_char,
        value: *mut std::os::raw::c_void,
        size: usize,
        position: u32,
        options: i32,
    ) -> isize;
}
#[cfg(not(target_os = "macos"))]
extern "C" {
    fn getxattr(
        path: *const std::os::raw::c_char,
        name: *const std::os::raw::c_char,
        value: *mut std::os::raw::c_void,
        size: usize,
    ) -> isize;
}
fn has_quarantine(p: &Path) -> bool {
    // True when `com.apple.quarantine` xattr is present (single getxattr
    // call, no process spawn). Any failure = false.
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = CString::new(p.as_os_str().as_bytes()) else {
        return false;
    };
    let Ok(name) = CString::new("com.apple.quarantine") else {
        return false;
    };
    // Probe presence only (size 0 / NULL value); >= 0 means present.
    unsafe {
        #[cfg(target_os = "macos")]
        return getxattr(path.as_ptr(), name.as_ptr(), std::ptr::null_mut(), 0, 0, 0) >= 0;
        #[cfg(not(target_os = "macos"))]
        return getxattr(path.as_ptr(), name.as_ptr(), std::ptr::null_mut(), 0) >= 0;
    }
}

fn items_from_downloads(dir: &Path) -> Vec<JunkItem> {
    walkdir::WalkDir::new(dir)
        .max_depth(3)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| !sweep_core::is_protected(e.path()))
        .filter(|e| is_candidate_ext(e.path()) || is_macho(e.path()))
        .filter(|e| has_quarantine(e.path()))
        .map(|e| unselected(e.path().to_path_buf(), file_size(e.path()), "Quarantined download"))
        .collect()
}

/// Minimal XML-plist Label extractor (std only).
/// shortcut: binary bplist Labels skipped (no <string> tags), upgrade to plist crate if needed.
fn plist_label(data: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(data);
    let key = "<key>Label</key>";
    let pos = text.find(key)?;
    let rest = &text[pos + key.len()..];
    let open = "<string>";
    let close = "</string>";
    let s = rest.find(open)? + open.len();
    let e = rest[s..].find(close)?;
    Some(rest[s..s + e].trim().to_string())
}

/// One item per `*.plist` in `dir` (non-recursive) whose Label does NOT
/// start with `com.apple.`. Path is the plist itself.
fn items_from_daemon_dir(dir: &Path) -> Vec<JunkItem> {
    walkdir::WalkDir::new(dir)
        .min_depth(1)
        .max_depth(1)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| !sweep_core::is_protected(e.path()))
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("plist"))
        .filter_map(|e| {
            let data = std::fs::read(e.path()).ok()?;
            let label = plist_label(&data)?;
            if label.starts_with("com.apple.") {
                return None;
            }
            Some(unselected(e.path().to_path_buf(), file_size(e.path()), "System daemon"))
        })
        .collect()
}

pub struct Protection;

impl Scanner for Protection {
    fn id(&self) -> &'static str {
        "protection"
    }
    fn name(&self) -> &'static str {
        "Suspicious Files"
    }
    fn scan(&self, on_progress: &dyn Fn(Progress)) -> Vec<JunkItem> {
        const TOTAL: u64 = 3;
        let mut out = Vec::new();
        let total_bytes = |out: &[JunkItem]| out.iter().map(|j: &JunkItem| j.size).sum();

        on_progress(Progress { done: 0, total: TOTAL, bytes: total_bytes(&out), label: "Downloads".into() });
        out.extend(items_from_downloads(&home().join("Downloads")));

        on_progress(Progress { done: 1, total: TOTAL, bytes: total_bytes(&out), label: "LaunchDaemons".into() });
        out.extend(items_from_daemon_dir(Path::new("/Library/LaunchDaemons")));

        on_progress(Progress { done: 2, total: TOTAL, bytes: total_bytes(&out), label: "LaunchAgents".into() });
        out.extend(items_from_daemon_dir(Path::new("/Library/LaunchAgents")));

        out.sort_by(|a, b| b.size.cmp(&a.size));
        on_progress(Progress { done: TOTAL, total: TOTAL, bytes: total_bytes(&out), label: "done".into() });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sweep-protection-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn has_xattr() -> bool {
        std::process::Command::new("xattr")
            .arg("-h")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn set_quarantine(p: &Path) -> bool {
        std::process::Command::new("xattr")
            .arg("-w")
            .arg("com.apple.quarantine")
            .arg("0081;00000000;Chrome;")
            .arg(p)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    const DAEMON_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>Label</key><string>LABEL</string></dict></plist>"#;

    #[test]
    fn finds_quarantined_script_not_plain() {
        if !has_xattr() {
            return; // skip gracefully where xattr is missing
        }
        let dir = tmp_dir("quar");
        let hit = dir.join("evil.sh");
        let plain = dir.join("plain.sh");
        std::fs::write(&hit, b"#!/bin/sh\necho hi\n").unwrap();
        std::fs::write(&plain, b"#!/bin/sh\necho hi\n").unwrap();
        if !set_quarantine(&hit) {
            std::fs::remove_dir_all(&dir).ok();
            return; // skip if quarantine xattr cannot be set (e.g. tmpfs)
        }
        let items = items_from_downloads(&dir);
        assert!(items.iter().any(|i| i.path == hit && i.category == "Quarantined download" && !i.selected));
        assert!(!items.iter().any(|i| i.path == plain));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn finds_quarantined_macho_without_extension() {
        if !has_xattr() {
            return;
        }
        let dir = tmp_dir("macho");
        let macho = dir.join("tool-noext");
        let other = dir.join("notes-noext");
        std::fs::write(&macho, [0xCF, 0xFA, 0xED, 0xFE, 0x00, 0x01]).unwrap();
        std::fs::write(&other, b"just text").unwrap();
        if !set_quarantine(&macho) || !set_quarantine(&other) {
            std::fs::remove_dir_all(&dir).ok();
            return;
        }
        let items = items_from_downloads(&dir);
        assert!(items.iter().any(|i| i.path == macho && !i.selected));
        assert!(!items.iter().any(|i| i.path == other));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn filters_apple_daemon_labels() {
        let dir = tmp_dir("daemon");
        std::fs::write(dir.join("com.apple.foo.plist"), DAEMON_PLIST.replace("LABEL", "com.apple.foo")).unwrap();
        std::fs::write(dir.join("com.example.bar.plist"), DAEMON_PLIST.replace("LABEL", "com.example.bar")).unwrap();
        std::fs::write(dir.join("broken.plist"), b"not a plist").unwrap();
        let items = items_from_daemon_dir(&dir);
        assert_eq!(items.len(), 1);
        assert!(items[0].path.ends_with("com.example.bar.plist"));
        assert_eq!(items[0].category, "System daemon");
        assert!(!items[0].selected);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn full_scan_never_panics() {
        let items = Protection.scan(&|_| {});
        assert!(items.iter().all(|i| !i.selected));
    }
}
