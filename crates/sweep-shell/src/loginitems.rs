//! Autostart via a user LaunchAgent plist + launchctl. Works fully unsigned
//! (SMAppService needs a paid Developer ID and throws without one, so the
//! app manages its own agent instead — same result, no certificate).
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginState {
    NotRegistered,
    Enabled,
    RequiresApproval,
    /// Kept for API compatibility; LaunchAgent path needs no signing.
    Unsigned,
    Unknown(i64),
}

/// Pure mapping from probe results. Kept separate so it stays unit-tested
/// without touching system login-item state.
pub fn map_status(raw: i64) -> LoginState {
    match raw {
        0 => LoginState::NotRegistered,
        1 => LoginState::Enabled,
        2 => LoginState::RequiresApproval,
        other => LoginState::Unknown(other),
    }
}

const LABEL: &str = "com.sweepmac.app";

fn home() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/"))
}

fn agent_plist() -> PathBuf {
    home().join(format!("Library/LaunchAgents/{LABEL}.plist"))
}

fn uid() -> Option<String> {
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn plist_body(exe: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{LABEL}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{exe}</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
</dict>
</plist>
"#
    )
}

fn is_loaded() -> bool {
    let Some(uid) = uid() else { return false };
    std::process::Command::new("launchctl")
        .args(["print", &format!("gui/{uid}/{LABEL}")])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn status() -> LoginState {
    if !agent_plist().is_file() {
        return LoginState::NotRegistered;
    }
    if is_loaded() {
        LoginState::Enabled
    } else {
        // Plist present but not loaded (e.g. after an update): report off;
        // enabling rewrites + bootstraps cleanly.
        LoginState::NotRegistered
    }
}

pub fn set_enabled(enable: bool) -> Result<String, String> {
    let plist = agent_plist();
    let Some(uid) = uid() else {
        return Err("cannot determine user id".to_string());
    };
    if enable {
        // Point at THIS binary, so dev and installed builds both work.
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        if let Some(p) = plist.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        std::fs::write(&plist, plist_body(&exe.to_string_lossy())).map_err(|e| e.to_string())?;
        // Clear any stale registration first (idempotent).
        let _ = std::process::Command::new("launchctl")
            .args(["bootout", &format!("gui/{uid}/{LABEL}")])
            .output();
        let out = std::process::Command::new("launchctl")
            .args(["bootstrap", &format!("gui/{uid}"), &plist.to_string_lossy()])
            .output()
            .map_err(|e| e.to_string())?;
        if is_loaded() {
            Ok("autostart on".to_string())
        } else {
            Err(format!(
                "bootstrap failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    } else {
        let _ = std::process::Command::new("launchctl")
            .args(["bootout", &format!("gui/{uid}/{LABEL}")])
            .output();
        std::fs::remove_file(&plist).ok();
        Ok("autostart off".to_string())
    }
}

// team_id stays for future signed builds (SMAppService path); unused for now.
#[allow(dead_code)]
pub fn team_id() -> Option<String> {
    static CACHE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            let exe = std::env::current_exe().ok()?;
            let out = std::process::Command::new("/usr/bin/codesign")
                .args(["-dv", "--verbose=4"])
                .arg(&exe)
                .output()
                .ok()?;
            let err = String::from_utf8_lossy(&out.stderr);
            for line in err.lines() {
                if let Some(v) = line.strip_prefix("TeamIdentifier=") {
                    let v = v.trim();
                    if v.len() == 10 && v.chars().all(|c| c.is_ascii_alphanumeric()) {
                        return Some(v.to_string());
                    }
                }
            }
            None
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::{map_status, LoginState};

    #[test]
    fn maps_known_statuses() {
        assert_eq!(map_status(0), LoginState::NotRegistered);
        assert_eq!(map_status(1), LoginState::Enabled);
        assert_eq!(map_status(2), LoginState::RequiresApproval);
        assert_eq!(map_status(3), LoginState::Unknown(3));
        assert_eq!(map_status(-1), LoginState::Unknown(-1));
    }

    #[test]
    fn plist_body_points_at_given_binary() {
        let xml = super::plist_body("/Applications/SweepMac.app/Contents/MacOS/sweepmac");
        assert!(xml.contains("<string>com.sweepmac.app</string>"));
        assert!(xml.contains("/Applications/SweepMac.app/Contents/MacOS/sweepmac"));
        assert!(xml.contains("<true/>"));
    }
}
