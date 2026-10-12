//! First-run ClamAV setup: install → configure → freshclam → launch clamd.
//! The user never touches a terminal; the Protection page drives this.
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

fn home() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/"))
}

pub fn conf_dir() -> PathBuf {
    home().join(".sweepmac/clamav")
}

pub fn db_dir() -> PathBuf {
    conf_dir().join("db")
}

fn find_bin(name: &str) -> Option<PathBuf> {
    for dir in ["/opt/homebrew/sbin", "/opt/homebrew/bin", "/usr/local/sbin", "/usr/local/bin", "/opt/local/bin", "/opt/local/sbin"] {
        let p = Path::new(dir).join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    std::env::var("PATH").ok().and_then(|path| {
        std::env::split_paths(&path)
            .map(|d| d.join(name))
            .find(|p| p.is_file())
    })
}

pub fn brew() -> Option<PathBuf> {
    find_bin("brew")
}

/// Which install step is still needed (pure seam for tests; no env access).
/// Returns the log line `setup()` would send, or "" when nothing blocks it.
fn needs_install(clamd_found: bool, brew_found: bool) -> &'static str {
    if clamd_found {
        ""
    } else if brew_found {
        "Installing ClamAV (this takes a minute)…"
    } else {
        "Homebrew not found — install it from https://brew.sh first."
    }
}

fn clamd_conf(db: &Path, dir: &Path) -> String {
    format!(
        "DatabaseDirectory {}\nLogFile {}/clamd.log\nTCPSocket 3310\nTCPAddr 127.0.0.1\nForeground false\n",
        db.display(),
        dir.display()
    )
}

fn freshclam_conf(db: &Path, dir: &Path) -> String {
    format!(
        "DatabaseDirectory {}\nUpdateLogFile {}/freshclam.log\nDatabaseMirror database.clamav.net\n",
        db.display(),
        dir.display()
    )
}

/// Write only when missing or content differs; Ok(true) = wrote, Ok(false) = skipped.
fn write_if_different(path: &Path, content: &str) -> std::io::Result<bool> {
    if std::fs::read(path).ok().as_deref() == Some(content.as_bytes()) {
        return Ok(false);
    }
    std::fs::write(path, content)?;
    Ok(true)
}

/// Human-readable protection status for the banner.
pub fn status() -> String {
    if ping_clamd() {
        return match crate::sigupdate::clamav_status(&[db_dir()], &["clamd"]) {
            crate::sigupdate::ClamStatus::Ready { sig_age_days } => {
                format!("Protected — signatures {sig_age_days}d old")
            }
            crate::sigupdate::ClamStatus::StaleOrMissing(m) => format!("Setup needed: {m}"),
        };
    }
    if find_bin("clamd").is_none() {
        if brew().is_none() {
            return "Needs Homebrew + ClamAV (one click installs both)".into();
        }
        return "ClamAV not installed".into();
    }
    "clamd installed but not running".into()
}

fn ping_clamd() -> bool {
    crate::malware::ClamClient::new("127.0.0.1:3310").ping()
}

fn run(cmd: &Path, args: &[&str], tx: &Sender<String>) -> bool {
    tx.send(format!("$ {} {}", cmd.display(), args.join(" "))).ok();
    match std::process::Command::new(cmd).args(args).output() {
        Ok(o) => {
            let tail = String::from_utf8_lossy(&o.stdout);
            let tail: String = tail.lines().rev().take(3).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
            if !tail.is_empty() {
                tx.send(tail).ok();
            }
            o.status.success()
        }
        Err(e) => {
            tx.send(format!("failed to run {}: {e}", cmd.display())).ok();
            false
        }
    }
}

/// Runs the full setup on a worker thread, streaming log lines. Idempotent:
// skips steps already done.
pub fn setup(tx: Sender<String>) {
    let dir = conf_dir();
    let db = db_dir();
    let _ = std::fs::create_dir_all(&db);

    // 1. Install.
    if find_bin("clamd").is_none() {
        let b = brew();
        tx.send(needs_install(false, b.is_some()).into()).ok();
        match b {
            Some(b) => {
                if !run(&b, &["install", "clamav"], &tx) && find_bin("clamd").is_none() {
                    tx.send("brew install failed — see log above.".into()).ok();
                    return;
                }
            }
            None => return,
        }
    }

    // 2. Configs (user-local, no sudo). Skipped when already matching.
    let clamd_path = dir.join("clamd.conf");
    let fresh_path = dir.join("freshclam.conf");
    let cfg = clamd_conf(&db, &dir);
    let fcfg = freshclam_conf(&db, &dir);
    if write_if_different(&clamd_path, &cfg).is_err()
        || write_if_different(&fresh_path, &fcfg).is_err()
    {
        tx.send("Cannot write config files.".into()).ok();
        return;
    }

    // 3. Signatures.
    let Some(freshclam) = find_bin("freshclam") else {
        tx.send("freshclam not found — reinstall ClamAV.".into()).ok();
        return;
    };
    tx.send("Downloading signatures (first run is large)…".into()).ok();
    run(&freshclam, &["--config-file", &fresh_path.to_string_lossy()], &tx);

    // 4. Daemon.
    if ping_clamd() {
        tx.send("clamd already running.".into()).ok();
        return;
    }
    let Some(clamd) = find_bin("clamd") else {
        tx.send("clamd not found — reinstall ClamAV.".into()).ok();
        return;
    };
    tx.send("Starting clamd…".into()).ok();
    match std::process::Command::new(&clamd)
        .arg("--config-file")
        .arg(&clamd_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(_) => {
            for _ in 0..30 {
                std::thread::sleep(std::time::Duration::from_secs(2));
                if ping_clamd() {
                    tx.send("Protected — clamd is up.".into()).ok();
                    return;
                }
            }
            tx.send("clamd started but not answering yet — retry shortly.".into()).ok();
        }
        Err(e) => {
            tx.send(format!("cannot launch clamd: {e}")).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_reports_homebrew_missing_when_nothing_found() {
        assert_eq!(
            needs_install(false, false),
            "Homebrew not found — install it from https://brew.sh first."
        );
        assert_eq!(needs_install(false, true), "Installing ClamAV (this takes a minute)…");
        assert_eq!(needs_install(true, false), "");
        assert_eq!(needs_install(true, true), "");
    }

    #[test]
    fn conf_renders_exact_strings() {
        let db = Path::new("/Users/x/.sweepmac/clamav/db");
        let dir = Path::new("/Users/x/.sweepmac/clamav");
        assert_eq!(
            clamd_conf(db, dir),
            "DatabaseDirectory /Users/x/.sweepmac/clamav/db\n\
             LogFile /Users/x/.sweepmac/clamav/clamd.log\n\
             TCPSocket 3310\nTCPAddr 127.0.0.1\nForeground false\n"
        );
        assert_eq!(
            freshclam_conf(db, dir),
            "DatabaseDirectory /Users/x/.sweepmac/clamav/db\n\
             UpdateLogFile /Users/x/.sweepmac/clamav/freshclam.log\n\
             DatabaseMirror database.clamav.net\n"
        );
    }

    #[test]
    fn config_write_skipped_when_content_matches() {
        let p = std::env::temp_dir().join(format!(
            "clamsetup-test-{}-{}.conf",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        assert!(write_if_different(&p, "a\n").unwrap()); // missing → write
        assert!(!write_if_different(&p, "a\n").unwrap()); // matching → skip
        assert!(write_if_different(&p, "b\n").unwrap()); // differs → write
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "b\n");
        std::fs::remove_file(&p).ok();
    }
}
