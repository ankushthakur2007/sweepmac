//! Versioned safety rules: which paths are safe/review/informational to clean.
//! Embedded at compile time via `include_str!` so the app works offline;
//! `~/.sweepmac/rules/safety-rules.json` may override the embedded copy.
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    Safe,
    Review,
    Informational,
}

impl Confidence {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "safe" => Some(Self::Safe),
            "review" => Some(Self::Review),
            "informational" => Some(Self::Informational),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Review => "review",
            Self::Informational => "informational",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub id: String,
    pub bundle_id: Option<String>,
    pub paths: Vec<String>,
    pub confidence: Confidence,
    pub preselected: bool,
    pub reason: String,
    pub protected_descendants: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Rules {
    pub version: String,
    rules: Vec<Rule>,
    protected_prefixes: Vec<String>,
    never_preselect_contains: Vec<String>,
    home: Option<PathBuf>,
}

// Minimal JSON value.
#[derive(Debug, Clone)]
enum Jv {
    Null,
    Bool(bool),
    Str(String),
    Num,
    Arr(Vec<Jv>),
    Obj(Vec<(String, Jv)>),
}

struct Parser {
    chars: Vec<char>,
    i: usize,
}

impl Parser {
    fn skip_ws(&mut self) {
        while self.i < self.chars.len() && self.chars[self.i].is_whitespace() {
            self.i += 1;
        }
    }
    fn parse(&mut self) -> Result<Jv, String> {
        self.skip_ws();
        let v = self.parse_val()?;
        self.skip_ws();
        Ok(v)
    }
    fn parse_val(&mut self) -> Result<Jv, String> {
        self.skip_ws();
        if self.i >= self.chars.len() {
            return Err("unexpected eof".into());
        }
        match self.chars[self.i] {
            '{' => self.parse_obj(),
            '[' => self.parse_arr(),
            '"' => Ok(Jv::Str(self.parse_str()?)),
            't' | 'f' => self.parse_bool(),
            'n' => self.parse_null(),
            '-' | '0'..='9' => self.parse_num(),
            c => Err(format!("unexpected char {:?}", c)),
        }
    }
    fn parse_str(&mut self) -> Result<String, String> {
        if self.chars.get(self.i) != Some(&'"') {
            return Err("expected string".into());
        }
        self.i += 1;
        let mut out = String::new();
        while self.i < self.chars.len() {
            let c = self.chars[self.i];
            self.i += 1;
            if c == '"' {
                return Ok(out);
            }
            if c == '\\' {
                if self.i >= self.chars.len() {
                    return Err("bad escape".into());
                }
                let e = self.chars[self.i];
                self.i += 1;
                match e {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    '/' => out.push('/'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    'b' => out.push('\u{0008}'),
                    'f' => out.push('\u{000c}'),
                    'u' => {
                        if self.i + 4 > self.chars.len() {
                            return Err("bad unicode escape".into());
                        }
                        let hex: String = self.chars[self.i..self.i + 4].iter().collect();
                        self.i += 4;
                        let cp = u32::from_str_radix(&hex, 16).map_err(|_| "bad hex".to_string())?;
                        if let Some(ch) = char::from_u32(cp) {
                            out.push(ch);
                        }
                    }
                    _ => return Err("unknown escape".into()),
                }
            } else {
                out.push(c);
            }
        }
        Err("unterminated string".into())
    }
    fn parse_obj(&mut self) -> Result<Jv, String> {
        if self.chars.get(self.i) != Some(&'{') {
            return Err("expected object".into());
        }
        self.i += 1;
        self.skip_ws();
        let mut items = Vec::new();
        if self.chars.get(self.i) == Some(&'}') {
            self.i += 1;
            return Ok(Jv::Obj(items));
        }
        loop {
            self.skip_ws();
            let key = self.parse_str()?;
            self.skip_ws();
            if self.chars.get(self.i) != Some(&':') {
                return Err("expected colon".into());
            }
            self.i += 1;
            let val = self.parse_val()?;
            items.push((key, val));
            self.skip_ws();
            match self.chars.get(self.i) {
                Some(&',') => {
                    self.i += 1;
                }
                Some(&'}') => {
                    self.i += 1;
                    break;
                }
                _ => return Err("expected , or }".into()),
            }
        }
        Ok(Jv::Obj(items))
    }
    fn parse_arr(&mut self) -> Result<Jv, String> {
        if self.chars.get(self.i) != Some(&'[') {
            return Err("expected array".into());
        }
        self.i += 1;
        self.skip_ws();
        let mut items = Vec::new();
        if self.chars.get(self.i) == Some(&']') {
            self.i += 1;
            return Ok(Jv::Arr(items));
        }
        loop {
            let v = self.parse_val()?;
            items.push(v);
            self.skip_ws();
            match self.chars.get(self.i) {
                Some(&',') => {
                    self.i += 1;
                }
                Some(&']') => {
                    self.i += 1;
                    break;
                }
                _ => return Err("expected , or ]".into()),
            }
        }
        Ok(Jv::Arr(items))
    }
    fn parse_bool(&mut self) -> Result<Jv, String> {
        if self.chars[self.i..].starts_with(&['t', 'r', 'u', 'e']) {
            self.i += 4;
            Ok(Jv::Bool(true))
        } else if self.chars[self.i..].starts_with(&['f', 'a', 'l', 's', 'e']) {
            self.i += 5;
            Ok(Jv::Bool(false))
        } else {
            Err("bad bool".into())
        }
    }
    fn parse_null(&mut self) -> Result<Jv, String> {
        if self.chars[self.i..].starts_with(&['n', 'u', 'l', 'l']) {
            self.i += 4;
            Ok(Jv::Null)
        } else {
            Err("bad null".into())
        }
    }
    fn parse_num(&mut self) -> Result<Jv, String> {
        let start = self.i;
        while self.i < self.chars.len()
            && (self.chars[self.i].is_ascii_digit()
                || self.chars[self.i] == '.'
                || self.chars[self.i] == '-'
                || self.chars[self.i] == '+'
                || self.chars[self.i] == 'e'
                || self.chars[self.i] == 'E')
        {
            self.i += 1;
        }
        let s: String = self.chars[start..self.i].iter().collect();
        let _ = s.parse::<f64>().map_err(|_| "bad number".to_string())?;
        Ok(Jv::Num)
    }
}

fn parse_json(s: &str) -> Result<Jv, String> {
    let mut p = Parser { chars: s.chars().collect(), i: 0 };
    p.parse()
}

fn obj_get<'a>(obj: &'a [(String, Jv)], key: &str) -> Option<&'a Jv> {
    obj.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn as_str(v: &Jv) -> Option<String> {
    if let Jv::Str(s) = v { Some(s.clone()) } else { None }
}

fn as_bool(v: &Jv) -> Option<bool> {
    if let Jv::Bool(b) = v { Some(*b) } else { None }
}

fn as_arr(v: &Jv) -> Option<&Vec<Jv>> {
    if let Jv::Arr(a) = v { Some(a) } else { None }
}

fn as_str_vec(v: &Jv) -> Vec<String> {
    as_arr(v).map(|a| a.iter().filter_map(as_str).collect()).unwrap_or_default()
}

impl Rules {
    pub fn from_str(s: &str) -> Result<Self, String> {
        let v = parse_json(s)?;
        let obj = match v {
            Jv::Obj(o) => o,
            _ => return Err("root not object".into()),
        };
        let version = obj_get(&obj, "version")
            .and_then(as_str)
            .unwrap_or_else(|| "1.0".to_string());
        let protected_prefixes = obj_get(&obj, "protected_prefixes")
            .map(as_str_vec)
            .unwrap_or_default();
        let never_preselect_contains = obj_get(&obj, "never_preselect_contains")
            .map(as_str_vec)
            .unwrap_or_default();
        let mut rules = Vec::new();
        if let Some(Jv::Arr(arr)) = obj_get(&obj, "rules") {
            for r in arr {
                if let Jv::Obj(o) = r {
                    let id = obj_get(o, "id").and_then(as_str).unwrap_or_default();
                    let bundle_id = obj_get(o, "bundle_id").and_then(as_str);
                    let paths = obj_get(o, "paths").map(as_str_vec).unwrap_or_default();
                    let confidence = obj_get(o, "confidence")
                        .and_then(as_str)
                        .and_then(|s| Confidence::from_str(&s))
                        .unwrap_or(Confidence::Informational);
                    let preselected = obj_get(o, "preselected").and_then(as_bool).unwrap_or(false);
                    let reason = obj_get(o, "reason").and_then(as_str).unwrap_or_default();
                    let protected_descendants = obj_get(o, "protected_descendants")
                        .map(as_str_vec)
                        .unwrap_or_default();
                    rules.push(Rule {
                        id,
                        bundle_id,
                        paths,
                        confidence,
                        preselected,
                        reason,
                        protected_descendants,
                    });
                }
            }
        }
        Ok(Self { version, rules, protected_prefixes, never_preselect_contains, home: None })
    }

    pub fn load() -> Self {
        if let Some(p) = Self::override_path() {
            if let Ok(s) = std::fs::read_to_string(&p) {
                if let Ok(r) = Self::from_str(&s) {
                    return r;
                }
            }
        }
        Self::from_str(include_str!("../../../rules/safety-rules.json"))
            .unwrap_or_default()
    }

    pub fn load_from(path: &Path) -> Self {
        if let Ok(s) = std::fs::read_to_string(path) {
            if let Ok(r) = Self::from_str(&s) {
                return r;
            }
        }
        Self::default()
    }

    fn override_path() -> Option<PathBuf> {
        crate::home().map(|h| h.join(".sweepmac/rules/safety-rules.json"))
    }

    pub fn with_home(mut self, home: PathBuf) -> Self {
        self.home = Some(home);
        self
    }

    fn expand(&self, raw: &str, home: &Option<PathBuf>) -> String {
        let h = home.as_ref().or(self.home.as_ref()).cloned().or_else(crate::home);
        if let Some(h) = h {
            if raw == "~" {
                return h.to_string_lossy().into_owned();
            }
            if let Some(rest) = raw.strip_prefix("~/") {
                return h.join(rest).to_string_lossy().into_owned();
            }
        }
        raw.to_string()
    }

    fn norm_path(p: &Path) -> String {
        let s = p.to_string_lossy().into_owned();
        if s.len() > 1 { s.trim_end_matches('/').to_string() } else { s }
    }

    fn is_under(path: &str, base: &str) -> bool {
        let p = if path.len() > 1 { path.trim_end_matches('/') } else { path };
        let b = if base.len() > 1 { base.trim_end_matches('/') } else { base };
        p == b || p.starts_with(&format!("{}/", b))
    }

    pub fn is_safe(&self, path: &Path) -> bool {
        self.is_safe_with_home(path, &self.home)
    }

    pub fn is_safe_with_home(&self, path: &Path, home: &Option<PathBuf>) -> bool {
        let p = Self::norm_path(path);
        // protected prefixes
        for pref in &self.protected_prefixes {
            let exp = self.expand(pref, home);
            if Self::is_under(&p, &exp) {
                return false;
            }
        }
        // protected descendants per rule
        for rule in &self.rules {
            for d in &rule.protected_descendants {
                let exp = self.expand(d, home);
                if Self::is_under(&p, &exp) {
                    return false;
                }
            }
        }
        true
    }

    pub fn confidence(&self, path: &Path) -> Option<Confidence> {
        self.confidence_with_home(path, &None)
    }

    pub fn confidence_with_home(&self, path: &Path, home: &Option<PathBuf>) -> Option<Confidence> {
        let h = home.clone().or(self.home.clone());
        let p = Self::norm_path(path);
        for rule in &self.rules {
            for raw in &rule.paths {
                let base = self.expand(raw, &h);
                if Self::is_under(&p, &base) {
                    return Some(rule.confidence);
                }
            }
        }
        None
    }

    pub fn reason(&self, path: &Path) -> Option<&str> {
        self.reason_with_home(path, &None)
    }

    pub fn reason_with_home(&self, path: &Path, home: &Option<PathBuf>) -> Option<&str> {
        let h = home.clone().or(self.home.clone());
        let p = Self::norm_path(path);
        for rule in &self.rules {
            for raw in &rule.paths {
                let base = self.expand(raw, &h);
                if Self::is_under(&p, &base) {
                    return Some(&rule.reason);
                }
            }
        }
        None
    }

    pub fn version_str(&self) -> &str {
        &self.version
    }

    pub fn rules_count(&self) -> usize {
        self.rules.len()
    }

    #[allow(dead_code)]
    pub fn never_preselect(&self, id_or_bundle: &str) -> bool {
        self.never_preselect_contains.iter().any(|s| id_or_bundle.contains(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmpdir() -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let d = std::env::temp_dir().join(format!("sweep-rules-{}-{}", std::process::id(), n));
        fs::create_dir_all(&d).unwrap();
        d
    }

    const EMBEDDED: &str = include_str!("../../../rules/safety-rules.json");

    #[test]
    fn version_parses() {
        let r = Rules::from_str(EMBEDDED).unwrap();
        assert_eq!(r.version, "1.0");
        assert!(r.rules_count() >= 15);
    }

    #[test]
    fn confidence_safe_for_known_cache() {
        let home = tmpdir();
        let r = Rules::from_str(EMBEDDED).unwrap().with_home(home.clone());
        let p = home.join("Library/Caches/com.google.Chrome/Cache_Data/foo");
        assert_eq!(r.confidence(&p), Some(Confidence::Safe));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn confidence_review_for_mail() {
        let home = tmpdir();
        let r = Rules::from_str(EMBEDDED).unwrap().with_home(home.clone());
        let p = home.join("Library/Mail Downloads/attachment.zip");
        assert_eq!(r.confidence(&p), Some(Confidence::Review));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn confidence_none_for_unknown() {
        let home = tmpdir();
        let r = Rules::from_str(EMBEDDED).unwrap().with_home(home.clone());
        let p = home.join("random/unknown/path");
        assert_eq!(r.confidence(&p), None);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn is_safe_false_for_protected_prefixes() {
        let r = Rules::from_str(EMBEDDED).unwrap();
        assert!(!r.is_safe(Path::new("/System/Library/Caches/foo")));
        assert!(!r.is_safe(Path::new("/usr/bin/x")));
        assert!(!r.is_safe(Path::new("/private/etc")));
        assert!(r.is_safe(Path::new("/tmp/foo")));
    }

    #[test]
    fn load_from_temp_file() {
        let d = tmpdir();
        let f = d.join("safety-rules.json");
        fs::write(&f, EMBEDDED).unwrap();
        let r = Rules::load_from(&f);
        assert_eq!(r.version, "1.0");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn reason_returns_some() {
        let home = tmpdir();
        let r = Rules::from_str(EMBEDDED).unwrap().with_home(home.clone());
        let p = home.join("Library/Caches/com.google.Chrome/x");
        assert!(r.reason(&p).is_some());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn override_path_loads() {
        // ensure load() doesn't panic even if no home
        let _ = Rules::load();
    }
}
