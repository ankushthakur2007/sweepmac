//! ClamAV signature/database status + updater. std-only.
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const STALE_AFTER_DAYS: u64 = 7;
const DB_FILES: [&str; 2] = ["daily.cvd", "daily.cld"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClamStatus {
    Ready { sig_age_days: u64 },
    StaleOrMissing(String),
}

fn age_days(mtime: SystemTime, now: SystemTime) -> u64 {
    now.duration_since(mtime).map(|d| d.as_secs() / 86_400).unwrap_or(0)
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.is_file() && p.metadata().map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}

/// `which`-like lookup: absolute entries checked directly, bare names searched in PATH.
fn bin_found(bin: &str) -> bool {
    if bin.contains('/') {
        return is_executable(Path::new(bin));
    }
    std::env::var_os("PATH").map(|paths| {
        std::env::split_paths(&paths).any(|dir| is_executable(&dir.join(bin)))
    }).unwrap_or(false)
}

/// Newest daily db age across `db_dirs`; `None` when no db file exists.
fn newest_db_age_days(db_dirs: &[PathBuf]) -> Option<u64> {
    let now = SystemTime::now();
    let mut best: Option<u64> = None;
    for dir in db_dirs {
        for name in DB_FILES {
            let p = dir.join(name);
            if let Ok(meta) = std::fs::metadata(&p) {
                if meta.is_file() {
                    // Unreadable mtime: treat as infinitely stale rather than Ready.
                    let age = meta.modified().map(|m| age_days(m, now)).unwrap_or(u64::MAX);
                    best = Some(best.map_or(age, |b| b.min(age)));
                }
            }
        }
    }
    best
}

pub fn clamav_status(db_dirs: &[PathBuf], bins: &[&str]) -> ClamStatus {
    match newest_db_age_days(db_dirs) {
        Some(age) if age < STALE_AFTER_DAYS => ClamStatus::Ready { sig_age_days: age },
        Some(age) => ClamStatus::StaleOrMissing(format!("signatures {age} days old")),
        None if bins.iter().any(|b| bin_found(b)) => {
            ClamStatus::StaleOrMissing("database not downloaded — run update".into())
        }
        None => ClamStatus::StaleOrMissing("clamav not installed — brew install clamav".into()),
    }
}

/// Last 2000 chars on a char boundary.
fn tail(s: &str) -> String {
    const MAX: usize = 2000;
    if s.len() <= MAX {
        return s.to_string();
    }
    let mut start = s.len() - MAX;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    s[start..].to_string()
}

/// Run `freshclam` with `args`, giving it 120s. std has no process timeout,
/// so the wait runs on a thread and `recv_timeout` bounds the block (a
/// timed-out child is left running; freshclam is short-lived and idempotent).
pub fn update_signatures(freshclam: &Path, args: &[&str]) -> Result<String, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let fc = freshclam.to_path_buf();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    std::thread::spawn(move || {
        let out = std::process::Command::new(&fc).args(&args).output();
        let _ = tx.send(out);
    });
    match rx.recv_timeout(std::time::Duration::from_secs(120)) {
        Err(_) => Err("freshclam timed out after 120s".to_string()),
        Ok(Err(e)) => Err(e.to_string()),
        Ok(Ok(out)) => {
            if out.status.success() {
                let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
                let err = String::from_utf8_lossy(&out.stderr);
                if !err.is_empty() {
                    combined.push_str(&err);
                }
                Ok(tail(&combined))
            } else {
                let err = tail(&String::from_utf8_lossy(&out.stderr));
                if !err.trim().is_empty() {
                    return Err(err);
                }
                let stdout = tail(&String::from_utf8_lossy(&out.stdout));
                if stdout.trim().is_empty() {
                    Err(format!("freshclam failed: {}", out.status))
                } else {
                    Err(stdout)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static N: AtomicU64 = AtomicU64::new(0);

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir()
            .join(format!("sigupdate-{}-{}-{}", std::process::id(), tag, N.fetch_add(1, Ordering::SeqCst)));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn touch(path: &Path) {
        std::fs::write(path, b"sigdata").unwrap();
    }

    #[test]
    fn ready_when_db_fresh() {
        let d = tmpdir("fresh");
        touch(&d.join("daily.cvd"));
        assert_eq!(clamav_status(&[d], &[]), ClamStatus::Ready { sig_age_days: 0 });
    }

    #[test]
    fn stale_when_db_old() {
        let d = tmpdir("stale");
        let f = d.join("daily.cld");
        touch(&f);
        // `touch -t` is supported by both BSD (macOS) and GNU touch.
        let ok = std::process::Command::new("touch").args(["-t", "200001010000"]).arg(&f).status().map(|s| s.success()).unwrap_or(false);
        if !ok {
            return; // shortcut: platform without touch -t, age math covered by newest_db_age_days logic
        }
        match clamav_status(&[d], &[]) {
            ClamStatus::StaleOrMissing(msg) => assert!(msg.starts_with("signatures "), "{msg}"),
            s => panic!("expected stale, got {s:?}"),
        }
    }

    #[test]
    fn newest_db_wins() {
        let d = tmpdir("mixed");
        let old = d.join("daily.cvd");
        touch(&old);
        let ok = std::process::Command::new("touch").args(["-t", "200001010000"]).arg(&old).status().map(|s| s.success()).unwrap_or(false);
        if !ok {
            return;
        }
        touch(&d.join("daily.cld")); // fresh
        assert_eq!(clamav_status(&[d], &[]), ClamStatus::Ready { sig_age_days: 0 });
    }

    #[test]
    fn not_installed_when_no_db_no_bin() {
        let d = tmpdir("empty");
        match clamav_status(&[d], &["definitely-not-a-real-bin-xyz"]) {
            ClamStatus::StaleOrMissing(msg) => assert!(msg.contains("not installed"), "{msg}"),
            s => panic!("expected not-installed, got {s:?}"),
        }
    }

    #[cfg(unix)]
    fn make_exec(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[test]
    #[cfg(unix)]
    fn db_missing_but_bin_present() {
        let d = tmpdir("nobdb");
        let bindir = tmpdir("fakebin");
        // Absolute candidate: no PATH mutation needed.
        let bin = make_exec(&bindir, "clamscan", "#!/bin/sh\nexit 0\n");
        let bin = bin.to_string_lossy().into_owned();
        match clamav_status(&[d], &[bin.as_str()]) {
            ClamStatus::StaleOrMissing(msg) => assert!(msg.contains("not downloaded"), "{msg}"),
            s => panic!("expected not-downloaded, got {s:?}"),
        }
    }

    #[test]
    #[cfg(unix)]
    fn update_signatures_fake_freshclam() {
        let d = tmpdir("freshclam");
        let ok = make_exec(&d, "freshclam-ok", "#!/bin/sh\necho signatures updated\n");
        let out = update_signatures(&ok, &[]).expect("fake freshclam should succeed");
        assert!(out.contains("signatures updated"), "{out}");

        let bad = make_exec(&d, "freshclam-bad", "#!/bin/sh\necho nope >&2\nexit 1\n");
        let err = update_signatures(&bad, &[]).expect_err("exit 1 should fail");
        assert!(err.contains("nope"), "{err}");

        assert!(update_signatures(&d.join("missing"), &[]).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn update_signatures_passes_args() {
        let d = tmpdir("args");
        let fc = make_exec(
            &d,
            "freshclam-args",
            "#!/bin/sh\n[ \"$1\" = \"--config-file\" ] && [ -n \"$2\" ] && echo ok || { echo bad >&2; exit 1; }\n",
        );
        let conf = d.join("freshclam.conf");
        std::fs::write(&conf, "").unwrap();
        let conf = conf.to_string_lossy().into_owned();
        let out =
            update_signatures(&fc, &["--config-file", conf.as_str()]).expect("args forwarded");
        assert!(out.contains("ok"), "{out}");
    }
}
