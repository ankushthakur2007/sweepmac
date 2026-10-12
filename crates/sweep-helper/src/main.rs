use std::env;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

mod bless;
use std::path::{Path, PathBuf};
use std::process::Command;

fn euid() -> Option<u32> {
    let out = Command::new("id").arg("-u").output().ok()?;
    String::from_utf8(out.stdout).ok()?.trim().parse().ok()
}

fn json_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

fn emit(path: &str, ok: bool, err: Option<&str>) {
    let e = match err {
        Some(m) => format!("\"{}\"", json_escape(m)),
        None => "null".to_string(),
    };
    println!(
        "{{\"path\":\"{}\",\"ok\":{},\"error\":{}}}",
        json_escape(path),
        ok,
        e
    );
}

fn normalize_lexically(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            _ => out.push(c.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from("/")
    } else {
        out
    }
}

fn absolutize(p: &str) -> PathBuf {
    let p = Path::new(p);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        env::current_dir().unwrap_or_else(|_| PathBuf::from("/")).join(p)
    };
    normalize_lexically(&abs)
}

fn canon_or_abs(p: &str) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| absolutize(p))
}

fn allowed(target: &Path, allows: &[PathBuf]) -> bool {
    allows.iter().any(|a| target.starts_with(a))
}

// TOCTOU recheck: the type observed at check time must match right before removal.
fn recheck_unchanged(path: &str, was_symlink: bool, was_dir: bool) -> Result<(), String> {
    let now = fs::symlink_metadata(path).map_err(|e| format!("stat failed: {e}"))?;
    if now.file_type().is_symlink() != was_symlink || now.is_dir() != was_dir {
        return Err("refused: file changed between check and removal".into());
    }
    Ok(())
}

fn count_remaining(dir: &Path) -> usize {
    let mut n = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let rd = match fs::read_dir(&d) {
            Ok(r) => r,
            Err(_) => {
                n += 1;
                continue;
            }
        };
        for e in rd {
            match e {
                Ok(en) => {
                    match en.file_type() {
                        Ok(ft) => {
                            n += 1;
                            if ft.is_dir() && !ft.is_symlink() {
                                stack.push(en.path());
                            }
                        }
                        Err(_) => n += 1,
                    }
                }
                Err(_) => n += 1,
            }
        }
    }
    n
}

fn dir_remove_error(path: &str, e: impl std::fmt::Display) -> String {
    format!(
        "remove dir failed: {e} ({} entries remain)",
        count_remaining(Path::new(path))
    )
}

fn do_remove(path: &str, allows: &[PathBuf]) -> Result<(), String> {
    if allows.is_empty() {
        return Err("refused: no --allow prefix given".into());
    }
    let canon = std::fs::canonicalize(path).map_err(|e| format!("canonicalize failed: {e}"))?;
    if !allowed(&canon, allows) {
        return Err("refused: not under --allow prefix".into());
    }
    let meta = fs::symlink_metadata(path).map_err(|e| format!("stat failed: {e}"))?;
    let was_symlink = meta.file_type().is_symlink();
    let was_dir = meta.is_dir();
    // Hardlink bypass: canonicalized path inside an allowed dir may still be
    // an extra name for an outside inode. Refuse multi-link non-dir files.
    #[cfg(unix)]
    if !was_symlink && !was_dir {
        if let Ok(sm) = fs::metadata(&canon) {
            if !sm.is_dir() && sm.nlink() > 1 {
                return Err(format!("refused: hardlink with nlink={}", sm.nlink()));
            }
        }
    }
    recheck_unchanged(path, was_symlink, was_dir)?;
    if was_symlink {
        fs::remove_file(path).map_err(|e| format!("remove symlink failed: {e}"))
    } else if was_dir {
        fs::remove_dir_all(path).map_err(|e| dir_remove_error(path, e))
    } else {
        fs::remove_file(path).map_err(|e| format!("remove file failed: {e}"))
    }
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for e in fs::read_dir(src)? {
        let e = e?;
        let ft = e.file_type()?;
        let d = dst.join(e.file_name());
        if ft.is_symlink() {
            #[cfg(unix)]
            {
                let t = fs::read_link(e.path())?;
                std::os::unix::fs::symlink(t, &d)?;
            }
        } else if ft.is_dir() {
            copy_dir_all(&e.path(), &d)?;
        } else {
            fs::copy(e.path(), &d)?;
        }
    }
    Ok(())
}

fn console_user_home(user: &str) -> Option<PathBuf> {
    let u = user.trim();
    if u.is_empty() || u == "root" || u.contains('/') || u.contains('.') {
        return None;
    }
    Some(PathBuf::from(format!("/Users/{u}")))
}

fn trash_home() -> PathBuf {
    let home_fallback =
        env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/var/root"));
    if euid() != Some(0) {
        return home_fallback;
    }
    match Command::new("stat").args(["-f%Su", "/dev/console"]).output() {
        Ok(out) => {
            let u = String::from_utf8_lossy(&out.stdout).into_owned();
            console_user_home(&u).unwrap_or(home_fallback)
        }
        Err(_) => home_fallback,
    }
}

fn trash_dest(name: &str) -> Result<PathBuf, String> {
    let trash = trash_home().join(".Trash");
    fs::create_dir_all(&trash).map_err(|e| format!("mkdir ~/.Trash failed: {e}"))?;
    let mut dest = trash.join(name);
    if !dest.exists() {
        return Ok(dest);
    }
    for i in 1..1000 {
        let c = trash.join(format!("{name} {i}"));
        if !c.exists() {
            dest = c;
            return Ok(dest);
        }
    }
    Err("trash name collision".into())
}

fn do_trash(path: &str) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("stat failed: {e}"))?;
    let name = Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "bad file name".to_string())?;
    let dest = trash_dest(name)?;
    if fs::rename(path, &dest).is_ok() {
        return Ok(());
    }
    // cross-device fallback: copy + delete
    if meta.file_type().is_symlink() {
        #[cfg(unix)]
        {
            let t = fs::read_link(path).map_err(|e| format!("readlink failed: {e}"))?;
            std::os::unix::fs::symlink(&t, &dest).map_err(|e| format!("copy symlink failed: {e}"))?;
            fs::remove_file(path).map_err(|e| format!("delete after copy failed: {e}"))?;
            return Ok(());
        }
        #[cfg(not(unix))]
        return Err("rename failed (cross-device)".into());
    } else if meta.is_dir() {
        copy_dir_all(Path::new(path), &dest).map_err(|e| format!("copy dir failed: {e}"))?;
        fs::remove_dir_all(path).map_err(|e| format!("delete after copy failed: {e}"))?;
        Ok(())
    } else {
        fs::copy(path, &dest).map_err(|e| format!("copy file failed: {e}"))?;
        fs::remove_file(path).map_err(|e| format!("delete after copy failed: {e}"))?;
        Ok(())
    }
}

const USAGE: &str =
    "usage: sweep-helper [--allow <prefix>]... <remove|trash> [--allow <prefix>]... <path>...";

fn main() {
    let raw: Vec<String> = env::args().skip(1).collect();
    // `sweep-helper bless --team <TEAMID> --label <LABEL>`: print the two
    // SMJobBless plist snippets so the packaging step stays in sync with
    // the code (also keeps bless.rs linked into the binary).
    if raw.first().map(|s| s.as_str()) == Some("bless") {
        let mut team = String::new();
        let mut label = String::from("com.sweepmac.helper");
        let mut it = raw.iter().skip(1);
        while let Some(a) = it.next() {
            if a == "--team" {
                team = it.next().cloned().unwrap_or_default();
            } else if a == "--label" {
                label = it.next().cloned().unwrap_or_default();
            }
        }
        if team.is_empty() {
            eprintln!("usage: sweep-helper bless --team <TEAMID> [--label <LABEL>]");
            std::process::exit(2);
        }
        println!("{}", bless::app_privileged_executables_plist(&team, &label));
        println!("{}", bless::helper_authorized_clients_plist(&team, &label));
        return;
    }
    match euid() {
        Some(0) => {}
        _ => eprintln!("warning: sweep-helper dev mode: euid != 0, no privilege separation"),
    }
    let raw: Vec<String> = env::args().skip(1).collect();
    let mut allows_raw: Vec<String> = vec![];
    let mut cmd: Option<String> = None;
    let mut paths: Vec<String> = vec![];
    let mut i = 0;
    while i < raw.len() {
        let a = &raw[i];
        if a == "--allow" {
            i += 1;
            match raw.get(i) {
                Some(p) => allows_raw.push(p.clone()),
                None => {
                    eprintln!("{USAGE}\n--allow needs a value");
                    std::process::exit(2);
                }
            }
        } else if let Some(p) = a.strip_prefix("--allow=") {
            allows_raw.push(p.to_string());
        } else if cmd.is_none() && (a == "remove" || a == "trash") {
            cmd = Some(a.clone());
        } else if cmd.is_none() && a.starts_with('-') {
            eprintln!("{USAGE}\nunknown flag: {a}");
            std::process::exit(2);
        } else if cmd.is_none() {
            eprintln!("{USAGE}\nneed remove|trash");
            std::process::exit(2);
        } else {
            paths.push(a.clone());
        }
        i += 1;
    }
    let cmd = match cmd {
        Some(c) => c,
        None => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };
    if paths.is_empty() {
        eprintln!("{USAGE}\nno paths given");
        std::process::exit(2);
    }
    let allows: Vec<PathBuf> = allows_raw.iter().map(|a| canon_or_abs(a)).collect();
    let mut failed = false;
    for p in &paths {
        let r = if cmd == "remove" { do_remove(p, &allows) } else { do_trash(p) };
        match &r {
            Ok(()) => emit(p, true, None),
            Err(m) => {
                emit(p, false, Some(m));
                failed = true;
            }
        }
    }
    if failed {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static N: AtomicU64 = AtomicU64::new(0);

    fn tmpdir(tag: &str) -> PathBuf {
        let d = env::temp_dir().join(format!(
            "sweep-helper-{}-{}-{}",
            std::process::id(),
            tag,
            N.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn hardlink_inside_allow_is_refused() {
        let base = tmpdir("hardlink");
        let allow = base.join("allow");
        fs::create_dir_all(&allow).unwrap();
        let outside = base.join("outside.txt");
        fs::write(&outside, b"secret").unwrap();
        let link = allow.join("link.txt");
        #[cfg(unix)]
        fs::hard_link(&outside, &link).unwrap();
        #[cfg(not(unix))]
        fs::copy(&outside, &link).unwrap();
        let allows = vec![std::fs::canonicalize(&allow).unwrap()];
        let err = do_remove(link.to_str().unwrap(), &allows).unwrap_err();
        assert!(err.contains("hardlink"), "unexpected: {err}");
        assert!(outside.exists(), "target must survive refusal");
        assert!(link.exists(), "link must not be deleted on refusal");
    }

    #[test]
    fn symlink_removes_link_not_target_and_recheck_catches_swap() {
        let base = tmpdir("symlink");
        let allow = base.join("allow");
        fs::create_dir_all(&allow).unwrap();
        let target = allow.join("target.txt");
        fs::write(&target, b"data").unwrap();
        let link = allow.join("link.txt");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        #[cfg(not(unix))]
        return;
        // recheck helper: type swap is caught
        assert!(recheck_unchanged(link.to_str().unwrap(), true, false).is_ok());
        assert!(recheck_unchanged(link.to_str().unwrap(), false, false).is_err());
        // removing the symlink removes the link, not the target
        let allows = vec![std::fs::canonicalize(&allow).unwrap()];
        do_remove(link.to_str().unwrap(), &allows).unwrap();
        assert!(!link.exists());
        assert!(target.exists());
    }

    #[test]
    fn trash_home_uses_real_home_when_non_root() {
        if euid() == Some(0) {
            return; // root path needs /dev/console; not testable here
        }
        let home = env::var("HOME").unwrap();
        assert_eq!(trash_home(), PathBuf::from(home));
    }

    #[test]
    fn console_user_home_parsing() {
        assert_eq!(console_user_home("bob"), Some(PathBuf::from("/Users/bob")));
        assert_eq!(console_user_home("root"), None);
        assert_eq!(console_user_home(""), None);
        assert_eq!(console_user_home("../evil"), None);
    }

    #[test]
    fn dotdot_allow_is_normalized() {
        assert_eq!(canon_or_abs("/a/../b"), PathBuf::from("/b"));
        assert!(allowed(&PathBuf::from("/b/x"), &[canon_or_abs("/a/../b")]));
        assert!(!allowed(&PathBuf::from("/a/evil"), &[canon_or_abs("/a/../b")]));
    }

    #[test]
    fn leftover_count_and_trash_collision() {
        let base = tmpdir("leftover");
        let d = base.join("d");
        fs::create_dir_all(d.join("sub")).unwrap();
        fs::write(d.join("a.txt"), b"a").unwrap();
        fs::write(d.join("sub").join("b.txt"), b"b").unwrap();
        assert_eq!(count_remaining(&d), 3); // a.txt, sub, sub/b.txt
        let msg = dir_remove_error(d.to_str().unwrap(), "boom");
        assert!(msg.contains("entries remain"), "unexpected: {msg}");
        // trash collision suffix: pre-create name then expect "name 1"
        let trash = trash_home().join(".Trash");
        let _ = fs::create_dir_all(&trash);
        let probe = base.join("probe-collision-sweep-helper.txt");
        fs::write(&probe, b"x").unwrap();
        let name = probe.file_name().unwrap().to_str().unwrap();
        let first = trash.join(name);
        let _ = fs::write(&first, b"occupant");
        let dest = trash_dest(name).unwrap();
        assert!(dest.to_str().unwrap().ends_with(" 1"));
        let _ = fs::remove_file(&first);
    }
}
