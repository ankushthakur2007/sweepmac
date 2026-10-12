//! Startup-items audit: LaunchAgents plists + loginwindow auto-launch apps.
//! Read-only enumeration (audit, user opts in); disabling login items is out
//! of scope for now, so every item ships with `selected = false`.
use std::path::{Path, PathBuf};

use sweep_core::{file_size, home, unselected, JunkItem, Progress, Scanner};

fn disabled_dir() -> PathBuf {
    home().join(".sweepmac/disabled")
}

fn current_uid() -> Option<String> {
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// Suffix index of a backup file name for `label`: `<label>.plist` is 1,
/// `<label>-N.plist` (N >= 2) is N, anything else is `None`.
fn backup_index(file_name: &str, label: &str) -> Option<u64> {
    if file_name == format!("{label}.plist") {
        return Some(1);
    }
    let (prefix, suffix) = (format!("{label}-"), ".plist");
    let n = file_name.strip_prefix(prefix.as_str())?.strip_suffix(suffix)?;
    n.parse::<u64>().ok().filter(|&n| n >= 2)
}

fn backup_path(dir: &Path, label: &str, n: u64) -> PathBuf {
    if n <= 1 {
        dir.join(format!("{label}.plist"))
    } else {
        dir.join(format!("{label}-{n}.plist"))
    }
}

/// Validate one LaunchAgent plist, touching Label + ProgramArguments[0] +
/// RunAtLoad. True when the plist looks like a launchd job.
fn is_launch_agent(data: &[u8]) -> bool {
    let Ok(value) = plist::from_bytes::<plist::Value>(data) else {
        return false;
    };
    let Some(dict) = value.as_dictionary() else {
        return false;
    };
    // Label is mandatory for a launchd job; the rest are best-effort.
    if dict.get("Label").and_then(|v| v.as_string()).is_none() {
        return false;
    }
    let _program = dict
        .get("ProgramArguments")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_string());
    let _run_at_load = dict.get("RunAtLoad").and_then(|v| v.as_boolean()).unwrap_or(false);
    true
}

/// One item per parseable `*.plist` in `dir` (non-recursive). Path is the
/// plist itself; size is the plist file size (job cost unknown).
fn items_from_launch_agents(dir: &Path) -> Vec<JunkItem> {
    walkdir::WalkDir::new(dir)
        .min_depth(1)
        .max_depth(1)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().map(|x| x == "plist").unwrap_or(false))
        .filter_map(|e| {
            let data = std::fs::read(e.path()).ok()?;
            is_launch_agent(&data)
                .then(|| unselected(e.path().to_path_buf(), file_size(e.path()), "Login item"))
        })
        .collect()
}

/// One item per `AutoLaunchedApplicationDictionary` entry (Name + Path) in
/// the loginwindow plist. Path prefers the app's Path, falling back to the
/// plist itself; size is the plist file size (app cost unknown).
fn items_from_loginwindow(plist_path: &Path) -> Vec<JunkItem> {
    let Ok(data) = std::fs::read(plist_path) else {
        return Vec::new();
    };
    let Ok(value) = plist::from_bytes::<plist::Value>(&data) else {
        return Vec::new();
    };
    let size = file_size(plist_path);
    value
        .as_dictionary()
        .and_then(|d| d.get("AutoLaunchedApplicationDictionary"))
        .and_then(|v| v.as_array())
        .map(|apps| {
            apps.iter()
                .filter_map(|v| v.as_dictionary())
                .filter(|d| d.get("Name").and_then(|v| v.as_string()).is_some())
                .map(|d| {
                    let path = d
                        .get("Path")
                        .and_then(|v| v.as_string())
                        .filter(|s| !s.is_empty())
                        .map(PathBuf::from)
                        .unwrap_or_else(|| plist_path.to_path_buf());
                    unselected(path, size, "Login item")
                })
                .collect()
        })
        .unwrap_or_default()
}

pub struct Performance;

/// Disable a LaunchAgent: move its plist under `~/.sweepmac/disabled/` and
/// best-effort `launchctl bootout` it. Loginwindow entries are app paths,
/// not plists, and must go through System Settings.
pub fn disable(item: &JunkItem) -> Result<String, String> {
    if item.path.extension().map(|x| x != "plist").unwrap_or(true) || !item.path.is_file() {
        return Err("loginwindow entries must be removed in System Settings".into());
    }
    let label = item
        .path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "loginwindow entries must be removed in System Settings".to_string())?;
    let dest_dir = disabled_dir();
    std::fs::create_dir_all(&dest_dir).map_err(|e| e.to_string())?;
    let dest = (1..=100)
        .map(|n| backup_path(&dest_dir, &label, n))
        .find(|p| !p.exists())
        .ok_or_else(|| format!("too many backups for {label} (limit 100)"))?;
    std::fs::rename(&item.path, &dest).map_err(|e| e.to_string())?;
    let Some(uid) = current_uid() else {
        return Ok(format!("Disabled {label} (moved to {}; uid lookup failed, skipped bootout)", dest.display()));
    };
    let target = format!("gui/{uid}/{label}");
    match std::process::Command::new("launchctl").args(["bootout", &target]).output() {
        Ok(o) if o.status.success() => Ok(format!("Disabled {label}")),
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr).trim().to_owned();
            Ok(format!("Disabled {label} (moved to {}; bootout warning: {err})", dest.display()))
        }
        Err(e) => Ok(format!("Disabled {label} (moved to {}; bootout warning: {e})", dest.display())),
    }
}

/// Undo a `disable`: move the newest backup for `label` back to
/// `~/Library/LaunchAgents/<label>.plist` (never overwriting a live
/// agent) and best-effort `launchctl bootstrap` it.
pub fn restore(label: &str) -> Result<String, String> {
    if label.is_empty() || label.contains('/') || label.contains('\\') {
        return Err("invalid label".into());
    }
    let dir = disabled_dir();
    let entries = std::fs::read_dir(&dir).map_err(|_| format!("no backup found for {label}"))?;
    let newest = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter_map(|p| {
            let n = backup_index(&p.file_name()?.to_string_lossy(), label)?;
            let mtime = std::fs::metadata(&p)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos());
            Some((n, mtime, p))
        })
        .max_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)))
        .map(|(_, _, p)| p)
        .ok_or_else(|| format!("no backup found for {label}"))?;
    let dest = home().join("Library/LaunchAgents").join(format!("{label}.plist"));
    if dest.exists() {
        return Err(format!("{} already exists, refusing to overwrite", dest.display()));
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&newest, &dest).map_err(|e| e.to_string())?;
    if let Some(uid) = current_uid() {
        let _ = std::process::Command::new("launchctl")
            .args(["bootstrap", &format!("gui/{uid}"), &dest.to_string_lossy()])
            .output();
    }
    Ok(format!("Restored {label}"))
}

impl Scanner for Performance {
    fn id(&self) -> &'static str {
        "performance"
    }
    fn name(&self) -> &'static str {
        "Startup Items"
    }
    fn scan(&self, on_progress: &dyn Fn(Progress)) -> Vec<JunkItem> {
        on_progress(Progress { done: 0, total: 1, bytes: 0, label: "Startup Items".into() });
        let h = home();
        let mut out = Vec::new();
        out.extend(items_from_launch_agents(&h.join("Library/LaunchAgents")));
        out.extend(items_from_launch_agents(&PathBuf::from("/Library/LaunchAgents")));
        out.extend(items_from_loginwindow(
            &h.join("Library/Preferences/com.apple.loginwindow.plist"),
        ));
        out.sort_by(|a, b| b.size.cmp(&a.size));
        on_progress(Progress {
            done: 1,
            total: 1,
            bytes: out.iter().map(|j| j.size).sum(),
            label: "done".into(),
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENT_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.example.testagent</string>
  <key>ProgramArguments</key><array><string>/usr/bin/true</string></array>
  <key>RunAtLoad</key><true/>
</dict>
</plist>"#;

    const LOGINWINDOW_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>AutoLaunchedApplicationDictionary</key>
  <array>
    <dict>
      <key>Name</key><string>TestApp</string>
      <key>Path</key><string>/Applications/TestApp.app</string>
    </dict>
  </array>
</dict>
</plist>"#;

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sweep-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    // HOME is process-global; serialize tests that stub it.
    fn home_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(())).lock().unwrap()
    }

    struct HomeGuard {
        old: Option<String>,
    }
    impl HomeGuard {
        fn set(path: &Path) -> (HomeGuard, std::sync::MutexGuard<'static, ()>) {
            let guard = home_lock();
            let old = std::env::var("HOME").ok();
            std::env::set_var("HOME", path);
            (HomeGuard { old }, guard)
        }
    }
    impl Drop for HomeGuard {
        fn drop(&mut self) {
            match &self.old {
                Some(h) => std::env::set_var("HOME", h),
                None => std::env::remove_var("HOME"),
            }
        }
    }

    #[test]
    fn finds_fake_launch_agent() {
        let dir = tmp_dir("perf-agent");
        std::fs::write(dir.join("com.example.test.plist"), AGENT_PLIST).unwrap();
        let items = items_from_launch_agents(&dir);
        assert_eq!(items.len(), 1);
        assert!(items.iter().all(|i| i.category == "Login item" && !i.selected));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn finds_fake_loginwindow_entry() {
        let dir = tmp_dir("perf-login");
        let p = dir.join("com.apple.loginwindow.plist");
        std::fs::write(&p, LOGINWINDOW_PLIST).unwrap();
        let items = items_from_loginwindow(&p);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].path, PathBuf::from("/Applications/TestApp.app"));
        assert!(!items[0].selected);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn full_scan_never_panics() {
        // No HOME stubbing: passes on any machine, real entries or not.
        let items = Performance.scan(&|_| {});
        assert!(items.iter().all(|i| i.category == "Login item" && !i.selected));
    }

    #[test]
    fn disable_moves_plist_to_disabled_dir() {
        let fake_home = tmp_dir("perf-disable-home");
        let (_home, _lock) = HomeGuard::set(&fake_home);
        let agent_dir = fake_home.join("Library/LaunchAgents");
        std::fs::create_dir_all(&agent_dir).unwrap();
        let src = agent_dir.join("com.example.disableme.plist");
        std::fs::write(&src, AGENT_PLIST).unwrap();
        let item = unselected(src.clone(), file_size(&src), "Login item");
        let msg = disable(&item).expect("disable should succeed");
        assert!(msg.contains("com.example.disableme"), "msg: {msg}");
        assert!(!src.exists(), "original plist must be gone");
        assert!(fake_home.join(".sweepmac/disabled/com.example.disableme.plist").is_file());
        std::fs::remove_dir_all(&fake_home).ok();
    }

    #[test]
    fn disable_twice_keeps_both_backups() {
        let fake_home = tmp_dir("perf-disable-twice");
        let (_home, _lock) = HomeGuard::set(&fake_home);
        let agent_dir = fake_home.join("Library/LaunchAgents");
        std::fs::create_dir_all(&agent_dir).unwrap();
        let label = "com.example.twice";
        let src = agent_dir.join(format!("{label}.plist"));
        std::fs::write(&src, AGENT_PLIST).unwrap();
        disable(&unselected(src.clone(), file_size(&src), "Login item")).unwrap();
        std::fs::write(&src, AGENT_PLIST).unwrap();
        disable(&unselected(src.clone(), file_size(&src), "Login item")).unwrap();
        let first = fake_home.join(format!(".sweepmac/disabled/{label}.plist"));
        let second = fake_home.join(format!(".sweepmac/disabled/{label}-2.plist"));
        assert!(first.is_file(), "first backup must be kept");
        assert!(second.is_file(), "second disable must use a distinct backup");
        assert!(!src.exists(), "original plist must be gone");
        std::fs::remove_dir_all(&fake_home).ok();
    }

    #[test]
    fn restore_brings_back_newest_and_refuses_when_live() {
        let fake_home = tmp_dir("perf-restore");
        let (_home, _lock) = HomeGuard::set(&fake_home);
        let agent_dir = fake_home.join("Library/LaunchAgents");
        std::fs::create_dir_all(&agent_dir).unwrap();
        let label = "com.example.restoreme";
        let src = agent_dir.join(format!("{label}.plist"));
        std::fs::write(&src, AGENT_PLIST).unwrap();
        disable(&unselected(src.clone(), file_size(&src), "Login item")).unwrap();
        std::fs::write(&src, "v2").unwrap();
        disable(&unselected(src.clone(), file_size(&src), "Login item")).unwrap();
        let msg = restore(label).expect("restore should succeed");
        assert!(msg.contains(label), "msg: {msg}");
        assert_eq!(std::fs::read_to_string(&src).unwrap(), "v2", "newest backup wins");
        assert!(restore(label).is_err(), "second restore must refuse (already live)");
        std::fs::remove_dir_all(&fake_home).ok();
    }

    #[test]
    fn disable_rejects_loginwindow_entries() {
        let item = unselected(PathBuf::from("/Applications/TestApp.app"), 42, "Login item");
        assert_eq!(disable(&item), Err("loginwindow entries must be removed in System Settings".into()));
    }
}
