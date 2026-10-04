//! Search criteria rows and saved searches (smart folders).
use super::fs::{self, Entry};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Attributes a criterion can test.
pub const ATTRS: [&str; 7] =
    ["Kind", "Last modified date", "Created date", "Last opened date", "Name", "Contents", "Size"];
pub const KINDS: [&str; 12] = [
    "Any",
    "Application",
    "Archive",
    "Document",
    "Folder",
    "Image",
    "Movie",
    "Music",
    "PDF",
    "Presentation",
    "Spreadsheet",
    "Text",
];
pub const DATE_OPS: [&str; 7] =
    ["is within last", "is today", "is yesterday", "is this week", "is this month", "is this year", "is before"];
pub const UNITS: [&str; 4] = ["days", "weeks", "months", "years"];
pub const NAME_OPS: [&str; 4] = ["contains", "begins with", "ends with", "is"];
pub const SIZE_OPS: [&str; 2] = ["is greater than", "is less than"];
pub const SIZE_UNITS: [&str; 3] = ["KB", "MB", "GB"];

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Criterion {
    pub attr: i32,
    pub op: i32,
    pub value: String,
    pub unit: i32,
}

impl Criterion {
    pub fn new(attr: i32) -> Self {
        let mut c = Criterion { attr, ..Default::default() };
        if attr == 1 || attr == 2 || attr == 3 {
            c.value = "7".into();
        }
        if attr == 6 {
            c.unit = 1;
            c.value = "1".into();
        }
        c
    }

    /// Operators shown for this attribute.
    pub fn ops(&self) -> &'static [&'static str] {
        match self.attr {
            0 => &KINDS,
            1..=3 => &DATE_OPS,
            4 => &NAME_OPS,
            6 => &SIZE_OPS,
            _ => &[],
        }
    }

    pub fn units(&self) -> &'static [&'static str] {
        match self.attr {
            1..=3 if self.op == 0 => &UNITS,
            6 => &SIZE_UNITS,
            _ => &[],
        }
    }

    pub fn has_value(&self) -> bool {
        match self.attr {
            0 => false,
            1..=3 => self.op == 0 || self.op == 6,
            _ => true,
        }
    }

    /// Whether the criterion needs to read file contents.
    pub fn reads_contents(&self) -> bool {
        self.attr == 5 && !self.value.trim().is_empty()
    }

    pub fn to_value(&self) -> Value {
        json!({"attr": self.attr, "op": self.op, "value": self.value, "unit": self.unit})
    }

    pub fn from_value(v: &Value) -> Criterion {
        let i = |k: &str| v[k].as_i64().unwrap_or(0) as i32;
        Criterion { attr: i("attr"), op: i("op"), unit: i("unit"), value: v["value"].as_str().unwrap_or("").into() }
    }
}

fn kind_matches(e: &Entry, k: i32) -> bool {
    let ext = e.ext.to_lowercase();
    let office = |list: &[&str]| list.contains(&ext.as_str());
    match k {
        1 => e.app.is_some() || e.kind == 5 || e.name.ends_with(".desktop"),
        2 => e.kind == 7,
        3 => !e.is_dir && matches!(e.kind, 1 | 6 | 8) || office(&["doc", "docx", "odt", "rtf", "pages"]),
        4 => e.is_dir,
        5 => e.kind == 2,
        6 => e.kind == 4,
        7 => e.kind == 3,
        8 => e.kind == 8,
        9 => office(&["ppt", "pptx", "odp", "key"]),
        10 => office(&["xls", "xlsx", "ods", "numbers", "csv"]),
        11 => e.kind == 6,
        _ => true,
    }
}

/// Start of the local day containing `t`, plus days since Monday, day of month and day of year (all 0-based).
fn day_start(t: i64) -> (i64, i64, i64, i64) {
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        let tt = t as libc::time_t;
        libc::localtime_r(&tt, &mut tm);
        tm
    };
    let secs = tm.tm_hour as i64 * 3600 + tm.tm_min as i64 * 60 + tm.tm_sec as i64;
    let wday = (tm.tm_wday as i64 + 6).rem_euclid(7);
    (t - secs, wday, tm.tm_mday as i64 - 1, tm.tm_yday as i64)
}

fn parse_date(s: &str) -> Option<i64> {
    let mut it = s.trim().split(['-', '.', '/']);
    let a: i32 = it.next()?.trim().parse().ok()?;
    let b: i32 = it.next()?.trim().parse().ok()?;
    let c: i32 = it.next()?.trim().parse().ok()?;
    let (y, m, d) = if a > 31 { (a, b, c) } else { (c, b, a) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = y - 1900;
    tm.tm_mon = m - 1;
    tm.tm_mday = d;
    tm.tm_isdst = -1;
    let t = unsafe { libc::mktime(&mut tm) };
    (t != -1).then_some(t as i64)
}

pub fn date_matches(t: i64, op: i32, value: &str, unit: i32, now: i64) -> bool {
    let (today, wday, mday, yday) = day_start(now);
    match op {
        0 => {
            let n: i64 = value.trim().parse().unwrap_or(0).max(0);
            let per = [1, 7, 30, 365][unit.clamp(0, 3) as usize];
            t >= today - (n * per - 1).max(0) * 86400
        }
        1 => t >= today,
        2 => t >= today - 86400 && t < today,
        3 => t >= today - wday * 86400,
        4 => t >= today - mday * 86400,
        5 => t >= today - yday * 86400,
        6 => parse_date(value).is_some_and(|d| t < d),
        _ => true,
    }
}

impl Criterion {
    /// Test an entry; `contents` reads the file only when needed.
    pub fn matches(&self, e: &Entry, now: i64, contents: &dyn Fn(&Path, &str) -> bool) -> bool {
        let v = self.value.trim().to_lowercase();
        match self.attr {
            0 => kind_matches(e, self.op),
            1 => date_matches(e.mtime, self.op, &v, self.unit, now),
            2 => date_matches(e.ctime, self.op, &v, self.unit, now),
            3 => date_matches(e.atime, self.op, &v, self.unit, now),
            4 => {
                let n = e.name.to_lowercase();
                match self.op {
                    1 => n.starts_with(&v),
                    2 => {
                        n.ends_with(&v) || Path::new(&n).file_stem().is_some_and(|s| s.to_string_lossy().ends_with(&v))
                    }
                    3 => n == v || Path::new(&n).file_stem().is_some_and(|s| s.to_string_lossy() == v),
                    _ => n.contains(&v),
                }
            }
            5 => v.is_empty() || !e.is_dir && contents(&e.path, &v),
            6 => {
                let n: f64 = v.replace(',', ".").parse().unwrap_or(0.0);
                let bytes = n * [1e3, 1e6, 1e9][self.unit.clamp(0, 2) as usize];
                !e.is_dir && if self.op == 1 { (e.size as f64) < bytes } else { e.size as f64 > bytes }
            }
            _ => true,
        }
    }
}

pub fn all_match(c: &[Criterion], e: &Entry, now: i64, contents: &dyn Fn(&Path, &str) -> bool) -> bool {
    c.iter().all(|c| c.matches(e, now, contents))
}

/// A saved search: where, what and how.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Saved {
    pub root: PathBuf,
    pub query: String,
    pub content: bool,
    pub criteria: Vec<Criterion>,
}

pub fn saved_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| fs::home().join(".local/share")).join("aqua/Saved Searches")
}

impl Saved {
    pub fn load(p: &Path) -> Option<Saved> {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()?;
        Some(Saved {
            root: PathBuf::from(v["root"].as_str().unwrap_or("/")),
            query: v["query"].as_str().unwrap_or("").into(),
            content: v["content"].as_bool().unwrap_or(false),
            criteria: v["criteria"]
                .as_array()
                .map(|a| a.iter().map(Criterion::from_value).collect())
                .unwrap_or_default(),
        })
    }

    pub fn save(&self, dir: &Path, name: &str) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        let clean: String = name.chars().map(|c| if c == '/' { '-' } else { c }).collect();
        let p = dir.join(format!("{}.json", clean.trim()));
        let v = json!({
            "root": self.root.to_string_lossy(),
            "query": self.query,
            "content": self.content,
            "criteria": self.criteria.iter().map(Criterion::to_value).collect::<Vec<_>>(),
        });
        std::fs::write(&p, v.to_string())?;
        Ok(p)
    }
}

/// Display name of a saved search file.
pub fn saved_name(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, size: u64, mtime: i64) -> Entry {
        let (kind, _, label) = fs::classify(name, false);
        Entry {
            name: name.into(),
            path: PathBuf::from(format!("/x/{name}")),
            is_dir: false,
            size,
            mtime,
            ctime: mtime,
            atime: mtime,
            kind,
            ext: Path::new(name).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default(),
            label,
            app: None,
            orig: None,
            mode: 0o644,
        }
    }

    #[test]
    fn kinds_names_and_sizes() {
        let now = fs::now_secs();
        let none = |_: &Path, _: &str| false;
        let img = entry("Photo.JPG", 3_000_000, now);
        let txt = entry("notes.txt", 10, now);
        let kind = |k| Criterion { attr: 0, op: k, ..Default::default() };
        assert!(kind(5).matches(&img, now, &none));
        assert!(!kind(5).matches(&txt, now, &none));
        assert!(kind(11).matches(&txt, now, &none));
        let name = |op, v: &str| Criterion { attr: 4, op, value: v.into(), unit: 0 };
        assert!(name(1, "pho").matches(&img, now, &none));
        assert!(name(2, "photo").matches(&img, now, &none));
        assert!(name(3, "notes").matches(&txt, now, &none));
        assert!(!name(0, "zzz").matches(&txt, now, &none));
        let big = Criterion { attr: 6, op: 0, value: "2".into(), unit: 1 };
        assert!(big.matches(&img, now, &none) && !big.matches(&txt, now, &none));
        let small = Criterion { attr: 6, op: 1, value: "1".into(), unit: 0 };
        assert!(small.matches(&txt, now, &none));
    }

    #[test]
    fn dates() {
        let now = fs::now_secs();
        assert!(date_matches(now, 1, "", 0, now));
        assert!(!date_matches(now - 3 * 86400, 1, "", 0, now));
        assert!(date_matches(now - 3 * 86400, 0, "7", 0, now));
        assert!(!date_matches(now - 30 * 86400, 0, "2", 1, now));
        assert!(date_matches(now - 30 * 86400, 0, "2", 2, now));
        assert!(date_matches(0, 6, "2000-01-01", 0, now));
        assert!(!date_matches(now, 6, "01.01.2000", 0, now));
    }

    #[test]
    fn contents_and_saved_roundtrip() {
        let now = fs::now_secs();
        let c = Criterion { attr: 5, op: 0, value: "Hello".into(), unit: 0 };
        let e = entry("a.txt", 5, now);
        assert!(c.matches(&e, now, &|_, q| q == "hello"));
        assert!(!c.matches(&e, now, &|_, _| false));
        let s = Saved { root: "/tmp".into(), query: "q".into(), content: true, criteria: vec![c, Criterion::new(1)] };
        let dir = std::env::temp_dir().join(format!("aqua-saved-{}", std::process::id()));
        let p = s.save(&dir, "My/Search").unwrap();
        assert_eq!(saved_name(&p), "My-Search");
        assert_eq!(Saved::load(&p).unwrap(), s);
        let _ = std::fs::remove_dir_all(dir);
    }
}
