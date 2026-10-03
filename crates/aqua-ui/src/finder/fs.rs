//! File-system side of Finder: listing, classification, trash, copy/move, tags and
//! folder customisation storage, recents.
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub enum Loc {
    Dir(PathBuf),
    Recents,
    Apps,
    Tag(String),
    Trash,
    Search(PathBuf, String),
}

impl Loc {
    pub fn key(&self) -> String {
        match self {
            Loc::Dir(p) => p.to_string_lossy().into_owned(),
            Loc::Recents => "recents:".into(),
            Loc::Apps => "apps:".into(),
            Loc::Tag(t) => format!("tag:{t}"),
            Loc::Trash => "trash:".into(),
            Loc::Search(p, _) => p.to_string_lossy().into_owned(),
        }
    }
    pub fn parse(s: &str) -> Loc {
        let s = s.trim();
        match s {
            "recents:" | "recents://" => Loc::Recents,
            "apps:" | "applications:" | "applications://" => Loc::Apps,
            "trash:" | "trash://" | "trash:///" => Loc::Trash,
            _ if s.starts_with("tag:") => Loc::Tag(s[4..].to_string()),
            _ => {
                let p = s.strip_prefix("file://").map(percent_decode).unwrap_or_else(|| s.to_string());
                let p = match p.strip_prefix('~') {
                    Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("{}{}", home().display(), rest),
                    _ => p,
                };
                Loc::Dir(PathBuf::from(p))
            }
        }
    }
    pub fn dir(&self) -> Option<&Path> {
        match self {
            Loc::Dir(p) | Loc::Search(p, _) => Some(p),
            _ => None,
        }
    }
}

pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| "/".into())
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: i64,
    pub ctime: i64,
    pub atime: i64,
    pub kind: i32,
    pub ext: String,
    pub label: String,
    /// Applications: (command, desktop id, Icon=)
    pub app: Option<(String, String, String)>,
    /// Trash: original location
    pub orig: Option<PathBuf>,
    pub mode: u32,
}

pub const TEXT_EXT: &[&str] = &[
    "txt",
    "md",
    "markdown",
    "rs",
    "toml",
    "json",
    "yaml",
    "yml",
    "xml",
    "plist",
    "html",
    "htm",
    "css",
    "js",
    "ts",
    "tsx",
    "jsx",
    "py",
    "rb",
    "go",
    "c",
    "h",
    "cpp",
    "hpp",
    "cc",
    "java",
    "kt",
    "swift",
    "sh",
    "bash",
    "zsh",
    "fish",
    "conf",
    "cfg",
    "ini",
    "log",
    "csv",
    "tsv",
    "sql",
    "lua",
    "vim",
    "desktop",
    "service",
    "slint",
    "svg",
    "tex",
    "org",
    "rst",
    "nix",
    "patch",
    "diff",
    "lock",
    "env",
    "gitignore",
    "srt",
    "vtt",
];

pub fn classify(name: &str, is_dir: bool) -> (i32, String, String) {
    if is_dir {
        let l = if name.ends_with(".app") { "Application" } else { "Folder" };
        return (0, String::new(), crate::tr(l).into());
    }
    let ext = Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let up = ext.to_uppercase();
    let (kind, label): (i32, String) = match ext.as_str() {
        "png" => (2, "PNG image".into()),
        "jpg" | "jpeg" => (2, "JPEG image".into()),
        "gif" => (2, "GIF image".into()),
        "webp" => (2, "WebP image".into()),
        "bmp" => (2, "BMP image".into()),
        "tif" | "tiff" => (2, "TIFF image".into()),
        "heic" | "heif" => (2, "HEIF image".into()),
        "avif" => (2, "AVIF image".into()),
        "ico" => (2, "Windows icon".into()),
        "icns" => (2, "Apple icon image".into()),
        "svg" => (2, "SVG image".into()),
        "mp3" => (3, "MP3 audio".into()),
        "flac" => (3, "FLAC audio".into()),
        "ogg" | "oga" | "opus" => (3, "Ogg audio".into()),
        "wav" => (3, "Waveform audio".into()),
        "m4a" | "aac" => (3, "MPEG-4 audio".into()),
        "mp4" | "m4v" => (4, "MPEG-4 movie".into()),
        "mov" => (4, "QuickTime movie".into()),
        "mkv" => (4, "Matroska video".into()),
        "webm" => (4, "WebM video".into()),
        "avi" => (4, "AVI movie".into()),
        "zip" => (7, "ZIP archive".into()),
        "gz" | "tgz" => (7, "gzip compressed archive".into()),
        "xz" | "zst" | "bz2" | "7z" | "rar" | "tar" => (7, crate::trf("{ext} archive", &[("ext", &up)])),
        "deb" | "rpm" | "pkg" => (7, "Installer package".into()),
        "appimage" => (5, "AppImage application".into()),
        "pdf" => (8, "PDF document".into()),
        "plist" => (6, "property list".into()),
        "txt" => (6, "Plain Text Document".into()),
        "md" | "markdown" => (6, "Markdown document".into()),
        "rs" => (6, "Rust source".into()),
        "py" => (6, "Python script".into()),
        "sh" | "bash" | "zsh" | "fish" => (6, "Shell script".into()),
        "json" => (6, "JSON document".into()),
        "toml" => (6, "TOML document".into()),
        "yaml" | "yml" => (6, "YAML document".into()),
        "xml" => (6, "XML document".into()),
        "html" | "htm" => (6, "HTML document".into()),
        "css" => (6, "CSS stylesheet".into()),
        "js" | "ts" | "tsx" | "jsx" => (6, "JavaScript source".into()),
        "c" | "h" | "cpp" | "hpp" | "cc" => (6, "C source".into()),
        "desktop" => (6, "Desktop entry".into()),
        "log" => (6, "Log file".into()),
        "csv" => (6, "CSV document".into()),
        "conf" | "cfg" | "ini" => (6, "Configuration file".into()),
        "doc" | "docx" | "odt" | "rtf" | "pages" => (1, "Text document".into()),
        "xls" | "xlsx" | "ods" | "numbers" => (1, "Spreadsheet".into()),
        "ppt" | "pptx" | "odp" | "key" => (1, "Presentation".into()),
        "iso" | "img" | "dmg" => (7, "Disk image".into()),
        "ttf" | "otf" | "woff" | "woff2" => (1, "Font".into()),
        "" => (1, "Document".into()),
        e if TEXT_EXT.contains(&e) => (6, crate::trf("{ext} document", &[("ext", &up)])),
        _ => (1, crate::trf("{ext} file", &[("ext", &up)])),
    };
    let label = crate::tr(&label).to_string();
    (kind, if ext.len() <= 5 { up } else { String::new() }, label)
}

pub fn entry(path: &Path) -> Option<Entry> {
    use std::os::unix::fs::MetadataExt;
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let lmd = std::fs::symlink_metadata(path).ok()?;
    let md = std::fs::metadata(path).unwrap_or(lmd.clone());
    let is_dir = md.is_dir();
    let (mut kind, ext, mut label) = classify(&name, is_dir);
    if !is_dir && kind == 1 && md.mode() & 0o111 != 0 && ext.is_empty() {
        label = crate::tr("Unix executable").into();
    }
    if lmd.file_type().is_symlink() {
        label = crate::trf("Alias ({kind})", &[("kind", &label)]);
    }
    if is_dir && is_mount_point(path) && path != Path::new("/") && kind == 0 {
        kind = 0;
    }
    let ctime = md
        .created()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(md.ctime());
    Some(Entry {
        name,
        path: path.to_path_buf(),
        is_dir,
        size: md.len(),
        mtime: md.mtime(),
        ctime,
        atime: md.atime(),
        kind,
        ext,
        label,
        app: None,
        orig: None,
        mode: md.mode(),
    })
}

fn is_mount_point(p: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(p), p.parent().map(std::fs::metadata)) {
        (Ok(a), Some(Ok(b))) => a.dev() != b.dev(),
        _ => false,
    }
}

pub fn list_dir(dir: &Path) -> Result<Vec<Entry>, String> {
    let rd = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
    Ok(rd.flatten().filter_map(|e| entry(&e.path())).collect())
}

pub fn list_apps() -> Vec<Entry> {
    let mut v: Vec<Entry> = aqua_apps::scan()
        .into_iter()
        .map(|a| {
            let cmd = if a.terminal { aqua_apps::terminal_wrap(&a.command()) } else { a.command() };
            let mut e = entry(&a.path).unwrap_or(Entry {
                name: String::new(),
                path: a.path.clone(),
                is_dir: false,
                size: 0,
                mtime: 0,
                ctime: 0,
                atime: 0,
                kind: 5,
                ext: String::new(),
                label: String::new(),
                app: None,
                orig: None,
                mode: 0,
            });
            e.name = a.name.clone();
            e.kind = 5;
            e.ext.clear();
            e.label = crate::tr("Application").into();
            e.app = Some((cmd, a.id.clone(), a.icon.clone()));
            e
        })
        .collect();
    v.sort_by(|a, b| natural(&a.name, &b.name));
    v.dedup_by(|a, b| a.name == b.name);
    v
}

/// GTK's recently-used list, newest first; falls back to recently modified files.
pub fn list_recents() -> Vec<Entry> {
    let mut v: Vec<(String, PathBuf)> = vec![];
    let xbel = dirs::data_dir().unwrap_or_else(|| home().join(".local/share")).join("recently-used.xbel");
    if let Ok(s) = std::fs::read_to_string(&xbel) {
        for chunk in s.split("<bookmark ").skip(1) {
            let attr = |n: &str| {
                chunk.split(&format!("{n}=\"")).nth(1).and_then(|r| r.split('"').next()).map(|s| s.to_string())
            };
            if let Some(href) = attr("href").filter(|h| h.starts_with("file://")) {
                let p = PathBuf::from(percent_decode(&xml_unescape(&href["file://".len()..])));
                let when = attr("visited").or_else(|| attr("modified")).unwrap_or_default();
                v.push((when, p));
            }
        }
    }
    v.sort_by(|a, b| b.0.cmp(&a.0));
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<Entry> = v
        .into_iter()
        .filter(|(_, p)| seen.insert(p.clone()))
        .filter_map(|(_, p)| entry(&p))
        .filter(|e| !e.is_dir)
        .take(200)
        .collect();
    if out.len() < 12 {
        let now = now_secs();
        let mut more = vec![];
        for d in
            [dirs::desktop_dir(), dirs::document_dir(), dirs::download_dir(), dirs::picture_dir()].into_iter().flatten()
        {
            walk(&d, 2, &mut |e: Entry| {
                if !e.is_dir && now - e.mtime < 60 * 86400 && !e.name.starts_with('.') {
                    more.push(e);
                }
                more.len() < 2000
            });
        }
        more.sort_by_key(|e| std::cmp::Reverse(e.mtime));
        for e in more {
            if out.len() >= 100 {
                break;
            }
            if seen.insert(e.path.clone()) {
                out.push(e);
            }
        }
    }
    out
}

pub fn walk(dir: &Path, depth: usize, f: &mut dyn FnMut(Entry) -> bool) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else { return true };
    let mut subdirs = vec![];
    for e in rd.flatten() {
        let Some(en) = entry(&e.path()) else { continue };
        let is_dir = en.is_dir && !e.file_type().map(|t| t.is_symlink()).unwrap_or(false);
        let hidden = en.name.starts_with('.');
        let p = en.path.clone();
        if !f(en) {
            return false;
        }
        if is_dir && !hidden {
            subdirs.push(p);
        }
    }
    if depth > 0 {
        for d in subdirs {
            if !walk(&d, depth - 1, f) {
                return false;
            }
        }
    }
    true
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn xml_unescape(s: &str) -> String {
    s.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'")
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn percent_encode(p: &str) -> String {
    let mut s = String::new();
    for b in p.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

pub fn file_uri(p: &Path) -> String {
    format!("file://{}", percent_encode(&p.to_string_lossy()))
}

/// Natural, case-insensitive order ("file2" < "file10").
pub fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let mut n1 = String::new();
                while let Some(c) = x.peek().copied().filter(|c| c.is_ascii_digit()) {
                    n1.push(c);
                    x.next();
                }
                let mut n2 = String::new();
                while let Some(c) = y.peek().copied().filter(|c| c.is_ascii_digit()) {
                    n2.push(c);
                    y.next();
                }
                let (t1, t2) = (n1.trim_start_matches('0'), n2.trim_start_matches('0'));
                let o = t1.len().cmp(&t2.len()).then(t1.cmp(t2));
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
            }
            (Some(c), Some(d)) => {
                let o = c.to_lowercase().cmp(d.to_lowercase());
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
                x.next();
                y.next();
            }
        }
    }
}

pub fn human(n: u64) -> String {
    match n {
        0..=999 => crate::ntr("{n} byte", "{n} bytes", n as i64),
        1000..=999_999 => format!("{:.0} {}", (n as f64 / 1e3).max(1.0), crate::tr("KB")),
        1_000_000..=999_999_999 => format!("{:.1} {}", n as f64 / 1e6, crate::tr("MB")),
        _ => format!("{:.2} {}", n as f64 / 1e9, crate::tr("GB")),
    }
}

fn tm(secs: i64) -> libc::tm {
    unsafe {
        let t = secs as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    }
}

/// Day-month-year patterns per month (translated as a whole: languages such as Russian need
/// the genitive month name, "18 июня 2025 г.").
const LONG_DATES: [&str; 12] = [
    "{day} January {year}",
    "{day} February {year}",
    "{day} March {year}",
    "{day} April {year}",
    "{day} May {year}",
    "{day} June {year}",
    "{day} July {year}",
    "{day} August {year}",
    "{day} September {year}",
    "{day} October {year}",
    "{day} November {year}",
    "{day} December {year}",
];
const SHORT_DATES: [&str; 12] = [
    "{day} Jan {year}",
    "{day} Feb {year}",
    "{day} Mar {year}",
    "{day} Apr {year}",
    "{day} May {year}",
    "{day} Jun {year}",
    "{day} Jul {year}",
    "{day} Aug {year}",
    "{day} Sep {year}",
    "{day} Oct {year}",
    "{day} Nov {year}",
    "{day} Dec {year}",
];

fn date_at(patterns: &[&str; 12], t: &libc::tm) -> String {
    let date =
        crate::trf(patterns[t.tm_mon.clamp(0, 11) as usize], &[("day", &t.tm_mday), ("year", &(t.tm_year + 1900))]);
    crate::trf("{date} at {time}", &[("date", &date), ("time", &format!("{:02}:{:02}", t.tm_hour, t.tm_min))])
}

/// "18 June 2025 at 12:57"
pub fn long_date(secs: i64) -> String {
    date_at(&LONG_DATES, &tm(secs))
}

/// Day number of `secs` in local time (days since the epoch).
fn local_day(secs: i64, t: &libc::tm) -> i64 {
    (secs + t.tm_gmtoff).div_euclid(86400)
}

/// "Today at 10:32", "Yesterday at 09:10", "18 Jun 2025 at 12:57"
pub fn short_date(secs: i64) -> String {
    if secs <= 0 {
        return "--".into();
    }
    let t = tm(secs);
    let n = tm(now_secs());
    let d = local_day(now_secs(), &n) - local_day(secs, &t);
    match d {
        0 => crate::trf("Today at {time}", &[("time", &format!("{:02}:{:02}", t.tm_hour, t.tm_min))]),
        1 => crate::trf("Yesterday at {time}", &[("time", &format!("{:02}:{:02}", t.tm_hour, t.tm_min))]),
        _ => date_at(&SHORT_DATES, &t),
    }
}

pub fn perm_string(mode: u32) -> String {
    let mut s = String::new();
    for (i, c) in "rwxrwxrwx".chars().enumerate() {
        s.push(if mode & (1 << (8 - i)) != 0 { c } else { '-' });
    }
    s
}

/// "name", "name copy", "name copy 2" … (extension kept)
pub fn unique(dir: &Path, name: &str, suffix: &str) -> PathBuf {
    let p = dir.join(name);
    if !p.exists() && suffix.is_empty() {
        return p;
    }
    let (stem, ext) = match name.rfind('.').filter(|&i| i > 0) {
        Some(i) => (&name[..i], &name[i..]),
        None => (name, ""),
    };
    let first = if suffix.is_empty() { None } else { Some(dir.join(format!("{stem} {suffix}{ext}"))) };
    if let Some(f) = first.filter(|f| !f.exists()) {
        return f;
    }
    for k in 2.. {
        let c = if suffix.is_empty() {
            dir.join(format!("{stem} {k}{ext}"))
        } else {
            dir.join(format!("{stem} {suffix} {k}{ext}"))
        };
        if !c.exists() {
            return c;
        }
    }
    unreachable!()
}

pub fn copy_rec(src: &Path, dst: &Path) -> std::io::Result<()> {
    let md = std::fs::symlink_metadata(src)?;
    if md.file_type().is_symlink() {
        let t = std::fs::read_link(src)?;
        std::os::unix::fs::symlink(t, dst)
    } else if md.is_dir() {
        if dst.starts_with(src) {
            return Err(std::io::Error::other("cannot copy a folder into itself"));
        }
        std::fs::create_dir_all(dst)?;
        for e in std::fs::read_dir(src)?.flatten() {
            copy_rec(&e.path(), &dst.join(e.file_name()))?;
        }
        let _ = std::fs::set_permissions(dst, md.permissions());
        Ok(())
    } else {
        std::fs::copy(src, dst).map(|_| ())
    }
}

pub fn move_to(src: &Path, dst: &Path) -> std::io::Result<()> {
    if dst.starts_with(src) {
        return Err(std::io::Error::other("cannot move a folder into itself"));
    }
    match std::fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            copy_rec(src, dst)?;
            remove_rec(src)
        }
        Err(e) => Err(e),
    }
}

pub fn remove_rec(p: &Path) -> std::io::Result<()> {
    let md = std::fs::symlink_metadata(p)?;
    if md.is_dir() {
        std::fs::remove_dir_all(p)
    } else {
        std::fs::remove_file(p)
    }
}

pub fn trash_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| home().join(".local/share")).join("Trash")
}

/// Move to the freedesktop.org trash (with a .trashinfo for "Put Back").
pub fn trash(p: &Path) -> Result<(), String> {
    let t = trash_dir();
    let (files, info) = (t.join("files"), t.join("info"));
    std::fs::create_dir_all(&files).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&info).map_err(|e| e.to_string())?;
    let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "item".into());
    let mut dst_name = name.clone();
    let mut k = 2;
    while files.join(&dst_name).exists() || info.join(format!("{dst_name}.trashinfo")).exists() {
        dst_name = format!("{name}.{k}");
        k += 1;
    }
    let abs = std::fs::canonicalize(p.parent().unwrap_or(Path::new("/"))).unwrap_or_default().join(&name);
    let n = tm(now_secs());
    let inf = format!(
        "[Trash Info]\nPath={}\nDeletionDate={:04}-{:02}-{:02}T{:02}:{:02}:{:02}\n",
        percent_encode(&abs.to_string_lossy()),
        n.tm_year + 1900,
        n.tm_mon + 1,
        n.tm_mday,
        n.tm_hour,
        n.tm_min,
        n.tm_sec
    );
    let ipath = info.join(format!("{dst_name}.trashinfo"));
    std::fs::write(&ipath, inf).map_err(|e| e.to_string())?;
    match std::fs::rename(p, files.join(&dst_name)) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            let _ = std::fs::remove_file(&ipath);
            let ok =
                std::process::Command::new("gio").arg("trash").arg(p).status().map(|s| s.success()).unwrap_or(false);
            if ok {
                Ok(())
            } else {
                Err("This item is on another volume and can't be moved to the Trash. Delete it immediately instead?"
                    .into())
            }
        }
        Err(e) => {
            let _ = std::fs::remove_file(&ipath);
            Err(e.to_string())
        }
    }
}

pub fn list_trash() -> Vec<Entry> {
    let t = trash_dir();
    let mut v = list_dir(&t.join("files")).unwrap_or_default();
    for e in &mut v {
        let inf = std::fs::read_to_string(t.join("info").join(format!("{}.trashinfo", e.name))).unwrap_or_default();
        e.orig = inf.lines().find_map(|l| l.strip_prefix("Path=")).map(|p| PathBuf::from(percent_decode(p)));
    }
    v
}

pub fn put_back(e: &Entry) -> Result<PathBuf, String> {
    let orig = e.orig.clone().ok_or("The original location of this item is unknown.")?;
    let dir = orig.parent().unwrap_or(Path::new("/"));
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let dst = if orig.exists() {
        unique(dir, &orig.file_name().unwrap_or_default().to_string_lossy(), "")
    } else {
        orig.clone()
    };
    move_to(&e.path, &dst).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(trash_dir().join("info").join(format!("{}.trashinfo", e.name)));
    Ok(dst)
}

pub fn empty_trash() {
    let t = trash_dir();
    for sub in ["files", "info", "expunged"] {
        if let Ok(rd) = std::fs::read_dir(t.join(sub)) {
            for e in rd.flatten() {
                let _ = remove_rec(&e.path());
            }
        }
    }
}

pub fn trash_count() -> usize {
    std::fs::read_dir(trash_dir().join("files")).map(|r| r.count()).unwrap_or(0)
}

pub const TAGS: [(&str, u32); 7] = [
    ("Red", 0xff453a),
    ("Orange", 0xff9f0a),
    ("Yellow", 0xffd60a),
    ("Green", 0x32d74b),
    ("Blue", 0x0a84ff),
    ("Purple", 0xbf5af2),
    ("Gray", 0x98989d),
];
/// Customize Folder colours (index 0 = default blue)
pub const FOLDER_COLORS: [u32; 9] = [0, 0xf19a37, 0xff453a, 0xffcc00, 0x34c759, 0x0a84ff, 0xbf5af2, 0x8e8e93, 0x5ebcf7];

#[derive(Default)]
pub struct Meta {
    pub tags: std::collections::HashMap<String, Vec<String>>,
    pub folders: std::collections::HashMap<String, (u32, String)>,
}

fn meta_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| home().join(".local/share")).join("aqua")
}

impl Meta {
    pub fn load() -> Self {
        let mut m = Meta::default();
        if let Ok(v) = std::fs::read_to_string(meta_dir().join("tags.json"))
            .map(|s| serde_json::from_str::<serde_json::Value>(&s).unwrap_or_default())
        {
            if let Some(o) = v.as_object() {
                for (k, t) in o {
                    let tags: Vec<String> = t
                        .as_array()
                        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                        .unwrap_or_default();
                    if !tags.is_empty() {
                        m.tags.insert(k.clone(), tags);
                    }
                }
            }
        }
        if let Ok(v) = std::fs::read_to_string(meta_dir().join("folders.json"))
            .map(|s| serde_json::from_str::<serde_json::Value>(&s).unwrap_or_default())
        {
            if let Some(o) = v.as_object() {
                for (k, f) in o {
                    let c = f.get("color").and_then(|c| c.as_u64()).unwrap_or(0) as u32;
                    let s = f.get("symbol").and_then(|c| c.as_str()).unwrap_or("").to_string();
                    m.folders.insert(k.clone(), (c, s));
                }
            }
        }
        m
    }
    pub fn save(&self) {
        let _ = std::fs::create_dir_all(meta_dir());
        let tags: serde_json::Map<String, serde_json::Value> =
            self.tags.iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| (k.clone(), serde_json::json!(v))).collect();
        let _ = std::fs::write(meta_dir().join("tags.json"), serde_json::to_string_pretty(&tags).unwrap_or_default());
        let f: serde_json::Map<String, serde_json::Value> = self
            .folders
            .iter()
            .filter(|(_, v)| v.0 != 0 || !v.1.is_empty())
            .map(|(k, v)| (k.clone(), serde_json::json!({ "color": v.0, "symbol": v.1 })))
            .collect();
        let _ = std::fs::write(meta_dir().join("folders.json"), serde_json::to_string_pretty(&f).unwrap_or_default());
    }
    pub fn tags_of(&self, p: &Path) -> Vec<String> {
        self.tags.get(&*p.to_string_lossy()).cloned().unwrap_or_default()
    }
    pub fn toggle_tag(&mut self, p: &Path, tag: &str, on: bool) {
        let k = p.to_string_lossy().into_owned();
        let v = self.tags.entry(k).or_default();
        v.retain(|t| t != tag);
        if on {
            v.push(tag.to_string());
        }
        let val = v.join(",");
        set_xattr(p, "user.xdg.tags", &val);
    }
    /// Keep tags / looks attached when an item is renamed or moved.
    pub fn moved(&mut self, from: &Path, to: &Path) {
        let (f, t) = (from.to_string_lossy().into_owned(), to.to_string_lossy().into_owned());
        let re = |k: &String| -> Option<String> {
            if k == &f {
                Some(t.clone())
            } else {
                k.strip_prefix(&format!("{f}/")).map(|rest| format!("{t}/{rest}"))
            }
        };
        let tags: Vec<(String, Vec<String>)> = self.tags.drain().map(|(k, v)| (re(&k).unwrap_or(k), v)).collect();
        self.tags = tags.into_iter().collect();
        let fo: Vec<(String, (u32, String))> = self.folders.drain().map(|(k, v)| (re(&k).unwrap_or(k), v)).collect();
        self.folders = fo.into_iter().collect();
    }
}

fn set_xattr(p: &Path, name: &str, val: &str) {
    use std::os::unix::ffi::OsStrExt;
    let Ok(cp) = std::ffi::CString::new(p.as_os_str().as_bytes()) else { return };
    let Ok(cn) = std::ffi::CString::new(name) else { return };
    unsafe {
        if val.is_empty() {
            libc::removexattr(cp.as_ptr(), cn.as_ptr());
        } else {
            libc::setxattr(cp.as_ptr(), cn.as_ptr(), val.as_ptr() as *const libc::c_void, val.len(), 0);
        }
    }
}

pub fn sh_quote(p: &Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn locations_parse_and_roundtrip() {
        assert!(matches!(Loc::parse("recents://"), Loc::Recents));
        assert!(matches!(Loc::parse(" trash:/// "), Loc::Trash));
        assert!(matches!(Loc::parse("applications:"), Loc::Apps));
        assert!(matches!(Loc::parse("tag:Red"), Loc::Tag(t) if t == "Red"));
        assert_eq!(Loc::parse("file:///tmp/a%20b").dir(), Some(Path::new("/tmp/a b")));
        assert_eq!(Loc::parse("~/Docs").dir(), Some(home().join("Docs").as_path()));
        assert_eq!(Loc::parse("~").dir(), Some(home().as_path()));
        assert_eq!(Loc::parse("~other/x").dir(), Some(Path::new("~other/x")), "~user is not expanded to $HOME");
        for k in ["recents:", "apps:", "trash:", "tag:Blue", "/usr/share"] {
            assert_eq!(Loc::parse(k).key(), k);
        }
    }

    #[test]
    fn classification() {
        assert_eq!(classify("Photos", true).2, crate::tr("Folder"));
        assert_eq!(classify("Thing.app", true).2, crate::tr("Application"));
        assert_eq!(classify("a.PNG", false).0, 2);
        assert_eq!(classify("song.flac", false).0, 3);
        assert_eq!(classify("movie.mkv", false).0, 4);
        assert_eq!(classify("x.tar", false).2, crate::trf("{ext} archive", &[("ext", &"TAR")]));
        assert_eq!(classify("doc.pdf", false).0, 8);
    }

    #[test]
    fn percent_coding_roundtrip() {
        for s in ["/home/u/My File (1).txt", "/tmp/Пример/ü", "/a/b%c"] {
            assert_eq!(percent_decode(&percent_encode(s)), s);
        }
        assert_eq!(percent_encode("/a b"), "/a%20b");
        assert_eq!(file_uri(Path::new("/x y")), "file:///x%20y");
        assert_eq!(percent_decode("%4"), "%4");
    }

    #[test]
    fn natural_order() {
        let mut v = vec!["file10", "File2", "file1", "file02", "alpha", "Beta"];
        v.sort_by(|a, b| natural(a, b));
        assert_eq!(v, vec!["alpha", "Beta", "file1", "File2", "file02", "file10"]);
        assert_eq!(natural("a", "a"), Ordering::Equal);
        assert_eq!(natural("a", "ab"), Ordering::Less);
    }

    #[test]
    fn sizes_and_permissions() {
        assert_eq!(human(0), crate::ntr("{n} byte", "{n} bytes", 0));
        assert_eq!(human(999), crate::ntr("{n} byte", "{n} bytes", 999));
        assert_eq!(human(1000), format!("1 {}", crate::tr("KB")));
        assert_eq!(human(2_500_000), format!("2.5 {}", crate::tr("MB")));
        assert_eq!(human(3_000_000_000), format!("3.00 {}", crate::tr("GB")));
        assert_eq!(perm_string(0o755), "rwxr-xr-x");
        assert_eq!(perm_string(0o640), "rw-r-----");
        assert_eq!(perm_string(0o100644), "rw-r--r--");
    }

    #[test]
    fn unique_names() {
        let d = std::env::temp_dir().join(format!("aqua-finder-test-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(unique(&d, "a.txt", ""), d.join("a.txt"));
        std::fs::write(d.join("a.txt"), "").unwrap();
        assert_eq!(unique(&d, "a.txt", ""), d.join("a 2.txt"));
        assert_eq!(unique(&d, "a.txt", "copy"), d.join("a copy.txt"));
        std::fs::write(d.join("a copy.txt"), "").unwrap();
        assert_eq!(unique(&d, "a.txt", "copy"), d.join("a copy 2.txt"));
        assert_eq!(unique(&d, ".bashrc", "copy"), d.join(".bashrc copy"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn relative_dates() {
        let now = now_secs();
        let today = crate::trf("Today at {time}", &[("time", &"")]);
        let yesterday = crate::trf("Yesterday at {time}", &[("time", &"")]);
        assert!(short_date(now).starts_with(&today));
        let t = tm(now);
        let since_midnight = (t.tm_hour * 3600 + t.tm_min * 60 + t.tm_sec) as i64;
        assert!(short_date(now - since_midnight - 3600).starts_with(&yesterday));
        assert!(!short_date(now - 10 * 86400).starts_with(&yesterday));
        assert_eq!(short_date(0), "--");
        let long = long_date(now);
        assert!(long.contains(&(t.tm_year + 1900).to_string()), "{long}");
        assert!(!long.contains('{'), "{long}");
    }
}
