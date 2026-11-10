//! Finding PDFs, the archive rules, and the saved settings.

use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

pub const APP_NAME: &str = "RapidPrint";

// --------------------------------------------------------------------------
// Finding PDF files
// --------------------------------------------------------------------------

fn natural_key(s: &str) -> Vec<(u8, u64, String)> {
    let mut out = Vec::new();
    let mut num = String::new();
    let mut text = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            if !text.is_empty() {
                out.push((1, 0, std::mem::take(&mut text).to_lowercase()));
            }
            num.push(c);
        } else {
            if !num.is_empty() {
                out.push((0, num.parse().unwrap_or(u64::MAX), String::new()));
                num.clear();
            }
            text.push(c);
        }
    }
    if !num.is_empty() {
        out.push((0, num.parse().unwrap_or(u64::MAX), String::new()));
    }
    if !text.is_empty() {
        out.push((1, 0, text.to_lowercase()));
    }
    out
}

fn is_pdf(p: &Path) -> bool {
    p.extension().map(|e| e.eq_ignore_ascii_case("pdf")).unwrap_or(false)
}

fn walk(dir: &Path, deep: bool, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            if deep {
                walk(&p, true, out);
            }
        } else if is_pdf(&p) {
            out.push(p);
        }
    }
}

/// PDFs in the given files and folders (folders sorted naturally: 2 before 10).
pub fn collect_pdfs(paths: &[PathBuf], subfolders: bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for p in paths {
        if p.is_dir() {
            let mut found = Vec::new();
            walk(p, subfolders, &mut found);
            found.sort_by_key(|f| natural_key(&f.to_string_lossy()));
            out.extend(found);
        } else if p.is_file() && is_pdf(p) {
            out.push(p.clone());
        }
    }
    out
}

pub fn short_folder(folder: &str) -> String {
    let parts: Vec<&str> = folder.split(['\\', '/']).filter(|p| !p.is_empty()).collect();
    if parts.len() <= 3 {
        folder.to_string()
    } else {
        format!("…\\{}", parts[parts.len() - 2..].join("\\"))
    }
}

// --------------------------------------------------------------------------
// Archive: after a file prints, it can be copied or moved to a folder chosen
// by rules on its name ("starts with", "contains", "ends with").
// --------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Match {
    Starts,
    Contains,
    Ends,
}

impl Match {
    pub const ALL: [Match; 3] = [Match::Starts, Match::Contains, Match::Ends];

    pub fn key(self) -> &'static str {
        match self {
            Match::Starts => "starts",
            Match::Contains => "contains",
            Match::Ends => "ends",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Match::Starts => "Name starts with",
            Match::Contains => "Name contains",
            Match::Ends => "Name ends with",
        }
    }

    pub fn from_key(k: &str) -> Match {
        Match::ALL.into_iter().find(|m| m.key() == k).unwrap_or(Match::Starts)
    }
}

#[derive(Clone, Debug)]
pub struct Rule {
    pub how: Match,
    pub text: String,
    pub folder: String,
}

impl Rule {
    /// Ignores case and treats spaces, underscores and dashes alike.
    pub fn matches(&self, file_name: &str) -> bool {
        let want = norm_name(&self.text);
        if want.is_empty() {
            return false;
        }
        let stem = Path::new(file_name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        match self.how {
            Match::Starts => norm_name(file_name).starts_with(&want),
            Match::Contains => norm_name(file_name).contains(&want),
            Match::Ends => norm_name(&stem).ends_with(&want) || norm_name(file_name).ends_with(&want),
        }
    }
}

/// Where a printed file goes: the first rule that matches, else the "other files" folder.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Rule(usize),
    Other,
}

pub fn target_for(rules: &[Rule], other: &str, file_name: &str) -> Option<Target> {
    if let Some(k) = rules.iter().position(|r| r.matches(file_name)) {
        return Some(Target::Rule(k));
    }
    if !other.trim().is_empty() {
        return Some(Target::Other);
    }
    None
}

fn norm_name(s: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in s.chars() {
        if c.is_whitespace() || c == '_' || c == '-' {
            gap = true;
        } else {
            if gap && !out.is_empty() {
                out.push(' ');
            }
            gap = false;
            out.extend(c.to_lowercase());
        }
    }
    out
}

/// The last part of a folder path, for the list ("→ Invoices").
pub fn folder_label(folder: &str) -> String {
    let f = folder.trim().trim_end_matches(['\\', '/']);
    f.rsplit(['\\', '/']).next().filter(|s| !s.is_empty()).unwrap_or(f).to_string()
}

/// This month as "2026-11" (local time), for monthly subfolders.
pub fn year_month() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = (secs as i64 + local_offset_secs()).div_euclid(86_400);
    // Days since 1970 to a calendar date (the "civil from days" method).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}")
}

/// Seconds to add to UTC for local time (Windows time zone settings).
fn local_offset_secs() -> i64 {
    // TIME_ZONE_INFORMATION: Bias (i32), then names and dates; DaylightBias is the last i32.
    #[repr(C)]
    struct Tzi {
        bias: i32,
        rest: [u8; 168],
    }
    type F = unsafe extern "system" fn(*mut Tzi) -> u32;
    unsafe {
        let Ok(lib) = libloading::Library::new("kernel32.dll") else { return 0 };
        let Ok(f) = lib.get::<F>(b"GetTimeZoneInformation\0") else { return 0 };
        let mut tz = Tzi { bias: 0, rest: [0; 168] };
        let r = f(&mut tz);
        let mut bias = tz.bias as i64; // minutes; UTC = local + bias
        if r == 2 {
            let db = i32::from_le_bytes([tz.rest[164], tz.rest[165], tz.rest[166], tz.rest[167]]);
            bias += db as i64;
        }
        -bias * 60
    }
}

fn same_content(a: &Path, b: &Path) -> bool {
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(ma), Ok(mb)) if ma.len() == mb.len() => {}
        _ => return false,
    }
    match (std::fs::read(a), std::fs::read(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// Copy (or move) src into folder, optionally into a subfolder per month.
/// Never overwrites a different file: a second, different file with the same
/// name is saved as "name (2).pdf". Returns (ok, message).
pub fn file_to_archive(src: &Path, folder: &str, move_it: bool, monthly: bool) -> (bool, String) {
    if folder.trim().is_empty() {
        return (false, "no folder chosen".into());
    }
    let mut folder = PathBuf::from(folder.trim());
    if !folder.is_dir() {
        return (false, "folder not found".into());
    }
    if monthly {
        folder = folder.join(year_month());
        if let Err(e) = std::fs::create_dir_all(&folder) {
            return (false, e.to_string());
        }
    }
    let name = src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut dest = folder.join(&name);
    if dest.exists() {
        if same_content(src, &dest) {
            if move_it && !same_file(src, &dest) {
                let _ = std::fs::remove_file(src);
            }
            return (true, "already there".into());
        }
        let stem = src.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let ext = src.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        let mut n = 2;
        while dest.exists() {
            dest = folder.join(format!("{stem} ({n}){ext}"));
            n += 1;
        }
    }
    let done = if move_it {
        std::fs::rename(src, &dest).or_else(|_| std::fs::copy(src, &dest).and_then(|_| std::fs::remove_file(src)))
    } else {
        std::fs::copy(src, &dest).map(|_| ())
    };
    let verb = if move_it { "moved" } else { "copied" };
    match done {
        Ok(()) => {
            let new = dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if new == name {
                (true, verb.into())
            } else {
                (true, format!("{verb} as {new}"))
            }
        }
        Err(e) => (false, e.to_string()),
    }
}

// --------------------------------------------------------------------------
// Settings: %APPDATA%\<edition's folder>\settings.json
// --------------------------------------------------------------------------

pub fn settings_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("RAPIDPRINT_SETTINGS_DIR") {
        return PathBuf::from(dir); // a separate settings folder (tests, portable use)
    }
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join(crate::brand::SETTINGS_FOLDER)
}

pub fn settings_file() -> PathBuf {
    settings_dir().join("settings.json")
}

pub struct Settings(pub Map<String, Value>);

impl Settings {
    pub fn load() -> Settings {
        let v = std::fs::read_to_string(settings_file())
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        Settings(v)
    }

    pub fn save(&self) {
        let _ = std::fs::create_dir_all(settings_dir());
        if let Ok(text) = serde_json::to_string_pretty(&Value::Object(self.0.clone())) {
            let _ = std::fs::write(settings_file(), text);
        }
    }

    pub fn str(&self, key: &str) -> String {
        self.0.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
    }

    pub fn bool(&self, key: &str, default: bool) -> bool {
        self.0.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
    }

    pub fn int(&self, key: &str, default: i64) -> i64 {
        self.0.get(key).and_then(|v| v.as_i64()).unwrap_or(default)
    }

    pub fn set(&mut self, key: &str, v: Value) {
        self.0.insert(key.to_string(), v);
    }

    pub fn obj(&self, key: &str) -> Map<String, Value> {
        self.0.get(key).and_then(|v| v.as_object().cloned()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules() {
        let r = |how, text: &str| Rule { how, text: text.into(), folder: "x".into() };
        assert!(r(Match::Starts, "Invoice ").matches("invoice_2024-01.pdf"));
        assert!(!r(Match::Starts, "Invoice").matches("My invoice.pdf"));
        assert!(r(Match::Contains, "order").matches("Big ORDER 12.pdf"));
        assert!(r(Match::Ends, "-signed").matches("contract signed.pdf"));
        assert!(!r(Match::Ends, "signed").matches("signed contract.pdf"));
        assert!(!r(Match::Contains, "  ").matches("a.pdf"));
        let rules = vec![r(Match::Starts, "a"), r(Match::Contains, "b")];
        assert_eq!(target_for(&rules, "", "abc.pdf"), Some(Target::Rule(0)));
        assert_eq!(target_for(&rules, "", "xbz.pdf"), Some(Target::Rule(1)));
        assert_eq!(target_for(&rules, "", "zzz.pdf"), None);
        assert_eq!(target_for(&rules, r"D:\Other", "zzz.pdf"), Some(Target::Other));
        assert_eq!(folder_label(r"D:\Archive\Invoices\"), "Invoices");
        assert_eq!(year_month().len(), 7);
    }

    #[test]
    fn copy_and_move() {
        let base = std::env::temp_dir().join(format!("rapidprint-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (src_dir, dst) = (base.join("in"), base.join("out"));
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        let f = src_dir.join("a.pdf");
        std::fs::write(&f, b"one").unwrap();
        let d = dst.to_string_lossy().to_string();
        assert_eq!(file_to_archive(&f, &d, false, false), (true, "copied".to_string()));
        assert_eq!(file_to_archive(&f, &d, false, false), (true, "already there".to_string()));
        std::fs::write(&f, b"two").unwrap();
        assert_eq!(file_to_archive(&f, &d, false, false), (true, "copied as a (2).pdf".to_string()));
        assert!(file_to_archive(&f, &d, true, true).0);
        assert!(!f.exists());
        assert!(dst.join(year_month()).join("a.pdf").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn natural() {
        let mut v = vec!["Invoice 10.pdf", "Invoice 2.pdf"];
        v.sort_by_key(|s| natural_key(s));
        assert_eq!(v, vec!["Invoice 2.pdf", "Invoice 10.pdf"]);
    }
}
