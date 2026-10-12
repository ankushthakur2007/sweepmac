//! Tray autostart via ServiceManagement's `SMAppService` (macOS 13+).
//!
//! Raw `msg_send` against the `SMAppService` class; the ServiceManagement
//! framework itself is linked from `build.rs`. Single-process app: the MAIN
//! app registers itself (no bundled helper needed).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginState {
    NotRegistered,
    Enabled,
    RequiresApproval,
    /// App isn't Developer-ID signed: SMAppService throws instead of
    /// answering, so don't call it at all.
    Unsigned,
    Unknown(i64),
}

/// Team ID of the running binary, if Developer-ID signed. SMAppService and
/// friends throw instead of answering on ad-hoc builds, so callers must
/// check this first. Cached: codesign runs once per process.
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
                    // Ad-hoc/unsigned builds report "not set" (or nothing at
                    // all): SMAppService throws on those, so treat as absent.
                    // Real team IDs are 10 alphanumerics (e.g. S8EX82NJP6).
                    if v.len() == 10 && v.chars().all(|c| c.is_ascii_alphanumeric()) {
                        return Some(v.to_string());
                    }
                }
            }
            None
        })
        .clone()
}
pub fn map_status(raw: i64) -> LoginState {
    match raw {
        0 => LoginState::NotRegistered,
        1 => LoginState::Enabled,
        2 => LoginState::RequiresApproval,
        other => LoginState::Unknown(other),
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use objc::runtime::{Class, Object};
    use objc::{msg_send, sel, sel_impl};

    use super::{map_status, team_id, LoginState};

    fn service() -> Option<*mut Object> {
        // Single-process app now: register the main app itself. (The old
        // loginItemServiceWithIdentifier:… path needs a bundled helper and
        // reports status 3/notFound without one.)
        unsafe {
            let cls: &Class = Class::get("SMAppService")?;
            let svc: *mut Object = msg_send![cls, mainApp];
            if svc.is_null() {
                None
            } else {
                Some(svc)
            }
        }
    }

    pub fn status() -> LoginState {
        if team_id().is_none() {
            return LoginState::Unsigned;
        }
        unsafe {
            match service() {
                None => LoginState::Unknown(-1),
                Some(svc) => {
                    let raw: isize = msg_send![svc, status];
                    map_status(raw as i64)
                }
            }
        }
    }

    pub fn set_enabled(enable: bool) -> Result<String, String> {
        if team_id().is_none() {
            return Err("needs a Developer ID–signed build".to_string());
        }
        unsafe {
            let svc = service().ok_or_else(|| "SMAppService unavailable".to_string())?;
            // NULL error out-param: we report the post-call status instead.
            let err: *mut *mut Object = std::ptr::null_mut();
            if enable {
                let _ok: bool = msg_send![svc, registerAndReturnError: err];
                match status() {
                    LoginState::Enabled => Ok("approved".to_string()),
                    LoginState::RequiresApproval => {
                        Ok("needs approval in System Settings".to_string())
                    }
                    LoginState::Unsigned => Err("needs a Developer ID–signed build".to_string()),
                    LoginState::NotRegistered => Err("registration failed".to_string()),
                    LoginState::Unknown(v) => {
                        Err(format!("unexpected login item status: {v}"))
                    }
                }
            } else {
                let ok: bool = msg_send![svc, unregisterAndReturnError: err];
                if ok {
                    Ok("removed".to_string())
                } else {
                    Err("failed to remove login item".to_string())
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub use imp::{set_enabled, status};

#[cfg(not(target_os = "macos"))]
pub fn status() -> LoginState {
    LoginState::NotRegistered
}

#[cfg(not(target_os = "macos"))]
pub fn set_enabled(_enable: bool) -> Result<String, String> {
    Err("login items require macOS".to_string())
}

// No round-trip tests: register/unregister mutates real system login-item
// state, so only the pure status mapping below is tested.
#[cfg(test)]
mod tests {
    use super::{map_status, team_id, LoginState};

    #[test]
    fn maps_known_statuses() {
        assert_eq!(map_status(0), LoginState::NotRegistered);
        assert_eq!(map_status(1), LoginState::Enabled);
        assert_eq!(map_status(2), LoginState::RequiresApproval);
        assert_eq!(map_status(3), LoginState::Unknown(3));
        assert_eq!(map_status(-1), LoginState::Unknown(-1));
    }
}
