//! Browser-data scanner: history + cookie files (file-level only, no sqlite reads).
use std::path::{Path, PathBuf};
use sweep_core::{home, JunkItem, Progress, Scanner};

pub struct Privacy;

fn push_file(out: &mut Vec<JunkItem>, path: PathBuf, category: &str) {
    if let Ok(m) = std::fs::metadata(&path) {
        if m.is_file() {
            out.push(JunkItem::new(path, m.len(), category));
        }
    }
}

fn scan_home(home: &Path, on_progress: &dyn Fn(Progress)) -> Vec<JunkItem> {
    // (label, profile dir relative to ~, [(file, category suffix)])
    let chromium: &[(&str, &str)] = &[
        ("Chrome", "Library/Application Support/Google/Chrome/Default"),
        ("Edge", "Library/Application Support/Microsoft Edge/Default"),
        ("Brave", "Library/Application Support/BraveSoftware/Brave-Browser/Default"),
        ("Arc", "Library/Application Support/Arc/User Data/Default"),
    ];
    let total = (chromium.len() + 2) as u64;
    let mut out = Vec::new();
    let mut step = 0u64;

    for (browser, rel) in chromium {
        on_progress(Progress {
            done: step,
            total,
            bytes: out.iter().map(|j: &JunkItem| j.size).sum(),
            label: browser.to_string(),
        });
        step += 1;
        let dir = home.join(rel);
        if !dir.is_dir() {
            continue;
        }
        push_file(&mut out, dir.join("History"), &format!("{browser} history"));
        push_file(&mut out, dir.join("History-journal"), &format!("{browser} history"));
        // Chromium ≥96 keeps cookies under Network/; older versions at the profile root.
        for rel in ["Cookies", "Cookies-journal", "Network/Cookies", "Network/Cookies-journal"] {
            push_file(&mut out, dir.join(rel), &format!("{browser} cookies"));
        }
    }

    on_progress(Progress {
        done: step,
        total,
        bytes: out.iter().map(|j: &JunkItem| j.size).sum(),
        label: "Firefox".into(),
    });
    step += 1;
    let profiles = home.join("Library/Application Support/Firefox/Profiles");
    if let Ok(rd) = std::fs::read_dir(&profiles) {
        for entry in rd.filter_map(|e| e.ok()) {
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            push_file(&mut out, p.join("places.sqlite"), "Firefox history");
            push_file(&mut out, p.join("cookies.sqlite"), "Firefox cookies");
        }
    }

    on_progress(Progress {
        done: step,
        total,
        bytes: out.iter().map(|j: &JunkItem| j.size).sum(),
        label: "Safari".into(),
    });
    push_file(&mut out, home.join("Library/Safari/History.db"), "Safari history");
    push_file(&mut out, home.join("Library/Safari/Cookies.binarycookies"), "Safari cookies");
    push_file(&mut out, home.join("Library/Cookies/Cookies.binarycookies"), "Safari cookies");

    out.sort_by(|a, b| b.size.cmp(&a.size));
    on_progress(Progress {
        done: total,
        total,
        bytes: out.iter().map(|j| j.size).sum(),
        label: "done".into(),
    });
    out
}

impl Scanner for Privacy {
    fn id(&self) -> &'static str {
        "privacy"
    }
    fn name(&self) -> &'static str {
        "Browser Data"
    }
    fn scan(&self, on_progress: &dyn Fn(Progress)) -> Vec<JunkItem> {
        scan_home(&home(), on_progress)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_fake_profile_tree() {
        let root = std::env::temp_dir()
            .join(format!("sweep-privacy-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let chrome = root.join("Library/Application Support/Google/Chrome/Default");
        std::fs::create_dir_all(&chrome.join("Network")).unwrap();
        std::fs::write(chrome.join("History"), vec![0u8; 10]).unwrap();
        std::fs::write(chrome.join("Cookies"), vec![0u8; 20]).unwrap(); // pre-96 layout
        std::fs::write(chrome.join("Network/Cookies"), vec![0u8; 25]).unwrap(); // Chromium ≥96
        let ff = root.join("Library/Application Support/Firefox/Profiles/abc.default");
        std::fs::create_dir_all(&ff).unwrap();
        std::fs::write(ff.join("places.sqlite"), vec![0u8; 30]).unwrap();
        std::fs::write(ff.join("cookies.sqlite"), vec![0u8; 40]).unwrap();
        let safari = root.join("Library/Safari");
        std::fs::create_dir_all(&safari).unwrap();
        std::fs::write(safari.join("History.db"), vec![0u8; 50]).unwrap();

        let items = scan_home(&root, &|_| {});
        assert!(items.iter().any(|j| j.category == "Chrome history" && j.size == 10));
        assert!(items.iter().any(|j| j.category == "Chrome cookies" && j.size == 20));
        assert!(items.iter().any(|j| j.category == "Chrome cookies" && j.size == 25)); // Network/Cookies
        assert!(items.iter().any(|j| j.category == "Firefox history" && j.size == 30));
        assert!(items.iter().any(|j| j.category == "Firefox cookies" && j.size == 40));
        assert!(items.iter().any(|j| j.category == "Safari history" && j.size == 50));
        assert!(items.iter().all(|j| j.selected));
        // Absent browsers skipped, not errored.
        assert!(!items.iter().any(|j| j.category.starts_with("Edge")));
        assert!(!items.iter().any(|j| j.category.starts_with("Brave")));
        let _ = std::fs::remove_dir_all(&root);
    }
}
