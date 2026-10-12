//! Uninstaller scanner: app inventory + leftover sweep keyed by bundle id.
//! Mirrors CleanMyMac's ApplicationsService leftover mapping (see REPLICA_PLAN.md).
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use sweep_core::{dir_size, home, JunkItem, Progress, Scanner};

#[derive(Debug, Clone)]
pub struct AppInfo {
    pub name: String,
    pub path: PathBuf,
    pub bid: Option<String>,
}

fn read_bid(app_path: &Path) -> Option<String> {
    let plist = app_path.join("Contents/Info.plist");
    let data = std::fs::read(&plist).ok()?;
    let value: plist::Value = plist::from_bytes(&data).ok()?;
    value.as_dictionary()?.get("CFBundleIdentifier")?.as_string().map(|s| s.to_string())
}

fn push_app(out: &mut Vec<AppInfo>, p: PathBuf) {
    if p.extension().map(|e| e == "app").unwrap_or(false) && p.is_dir() {
        let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or("?").to_string();
        let bid = read_bid(&p);
        out.push(AppInfo { name, path: p, bid });
    }
}

fn scan_apps_dir(dir: &Path, out: &mut Vec<AppInfo>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.filter_map(|e| e.ok()) {
        let p = entry.path();
        if p.extension().map(|e| e == "app").unwrap_or(false) {
            push_app(out, p);
        } else if p.is_dir() {
            // One extra level: /Applications/Utilities, vendor folders, etc.
            let Ok(sub) = std::fs::read_dir(&p) else { continue };
            for child in sub.filter_map(|e| e.ok()) {
                push_app(out, child.path());
            }
        }
    }
}

/// Every .app in /Applications and ~/Applications (plus one level down).
pub fn list_apps() -> Vec<AppInfo> {
    let mut out = Vec::new();
    for dir in [PathBuf::from("/Applications"), home().join("Applications")] {
        scan_apps_dir(&dir, &mut out);
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Candidate leftover locations for one app. Only existing paths are returned;
/// the caller decides whether they are orphans.
fn candidates_for(app: &AppInfo, h: &Path) -> Vec<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Some(bid) = &app.bid {
        let parts: Vec<&str> = bid.split('.').collect();
        let vendor = parts.iter().take(parts.len().saturating_sub(1)).skip(1).cloned().collect::<Vec<_>>().join(".");
        // Short bids (e.g. "com.foo") yield an empty vendor: never flag the bare parent.
        if !vendor.is_empty() {
            cands.push(h.join(format!("Library/Application Support/{vendor}")));
        }
        cands.push(h.join(format!("Library/Caches/{bid}")));
        cands.push(h.join(format!("Library/Preferences/{bid}.plist")));
        cands.push(h.join(format!("Library/Saved Application State/{bid}.savedState")));
        cands.push(h.join(format!("Library/Containers/{bid}")));
        cands.push(h.join(format!("Library/Group Containers/{bid}")));
        cands.push(h.join(format!("Library/Cookies/{bid}.binarycookies")));
        cands.push(h.join(format!("Library/Logs/{bid}")));
    } else {
        // No bundle id: match by app name only (conservative).
        cands.push(h.join(format!("Library/Application Support/{}", app.name)));
        cands.push(h.join(format!("Library/Caches/{}", app.name)));
        cands.push(h.join(format!("Library/Preferences/{}.plist", app.name)));
        cands.push(h.join(format!("Library/Saved Application State/{}.savedState", app.name)));
    }
    cands.into_iter().filter(|p| p.exists()).collect()
}

fn leftovers_for(app: &AppInfo) -> Vec<PathBuf> {
    candidates_for(app, &home())
}

pub struct Uninstaller;

impl Scanner for Uninstaller {
    fn id(&self) -> &'static str {
        "uninstaller"
    }
    fn name(&self) -> &'static str {
        "App Leftovers"
    }
    fn scan(&self, on_progress: &dyn Fn(Progress)) -> Vec<JunkItem> {
        on_progress(Progress { done: 0, total: 2, bytes: 0, label: "Listing apps".into() });
        let apps = list_apps();
        let installed: HashSet<String> = apps.iter().filter_map(|a| a.bid.clone()).collect();
        // Orphan leftovers: parents we sweep for bids no longer installed.
        on_progress(Progress { done: 1, total: 2, bytes: 0, label: "Sweeping leftovers".into() });
        let mut out = Vec::new();
        // Installed apps own their support files; per-app candidates only apply to bid-less apps.
        for app in apps.iter().filter(|a| a.bid.is_none()) {
            for p in leftovers_for(app) {
                if sweep_core::is_protected(&p) {
                    continue;
                }
                let size = if p.is_dir() { dir_size(&p, 8) } else { p.metadata().map(|m| m.len()).unwrap_or(0) };
                if size > 0 {
                    out.push(JunkItem::new(
                        p,
                        size,
                        &format!("Leftover · {}", app.name),
                    ));
                }
            }
        }
        // Also: leftover entries whose app is fully gone (bid on disk, app absent).
        let h = home();
        for parent in [
            "Library/Application Support",
            "Library/Caches",
            "Library/Containers",
            "Library/Preferences",
            "Library/Cookies",
            "Library/Logs",
        ] {
            let Ok(rd) = std::fs::read_dir(h.join(parent)) else { continue };
            for entry in rd.filter_map(|e| e.ok()) {
                let p = entry.path();
                if sweep_core::is_protected(&p) {
                    continue;
                }
                let fname = entry.file_name().to_string_lossy().into_owned();
                // File-based parents store `{bid}.plist` / `{bid}.binarycookies`; compare the bid stem.
                let key = if p.is_file() {
                    match fname.rfind('.') {
                        Some(i) => fname[..i].to_owned(),
                        None => fname,
                    }
                } else {
                    fname
                };
                if installed.contains(&key) {
                    continue;
                }
                // Only flag reverse-DNS names (likely bundle ids), not plain folders.
                if key.matches('.').count() >= 2 {
                    let size = if p.is_dir() { dir_size(&p, 8) } else { p.metadata().map(|m| m.len()).unwrap_or(0) };
                    if size > 1024 * 1024 {
                        out.push(JunkItem::new(p, size, "Orphan data"));
                    }
                }
            }
        }
        out.sort_by(|a, b| b.size.cmp(&a.size));
        on_progress(Progress { done: 2, total: 2, bytes: out.iter().map(|j| j.size).sum(), label: "done".into() });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sweep-uninstaller-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn short_bid_yields_no_bare_parent_candidate() {
        let h = tmp_dir("vendor-guard");
        std::fs::create_dir_all(h.join("Library/Application Support")).unwrap();
        let app = AppInfo { name: "Foo".into(), path: h.join("Foo.app"), bid: Some("com.foo".into()) };
        let got = candidates_for(&app, &h);
        assert!(
            !got.iter().any(|p| *p == h.join("Library/Application Support")),
            "bare parent must never be a candidate: {got:?}"
        );
        std::fs::remove_dir_all(&h).ok();
    }

    #[test]
    fn finds_apps_one_level_down() {
        let root = tmp_dir("subfolder");
        std::fs::create_dir_all(root.join("Utilities/Foo.app/Contents")).unwrap();
        std::fs::create_dir_all(root.join("Bar.app/Contents")).unwrap();
        let mut out = Vec::new();
        scan_apps_dir(&root, &mut out);
        assert!(out.iter().any(|a| a.name == "Foo"), "nested app missing: {out:?}");
        assert!(out.iter().any(|a| a.name == "Bar"), "top-level app missing: {out:?}");
        std::fs::remove_dir_all(&root).ok();
    }
}
