//! Archives in Finder: browse zip / 7z / rar / tar.* / iso … like folders, open files
//! inside them, extract them (`7z`, or libarchive's `bsdtar` for compressed tarballs and as
//! a fallback).
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;

/// One member of an archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArcEntry {
    /// Path inside the archive, `/`-separated, no leading or trailing slash.
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: i64,
}

/// Compressed tarballs: 7z would show the inner `.tar`, libarchive lists the members.
const TARBALL: &[&str] = &[".tar.gz", ".tgz", ".tar.bz2", ".tbz", ".tbz2", ".tar.xz", ".txz", ".tar.zst", ".tzst", ".tar.lz", ".tar.lzma", ".tar.z"];
const ARCHIVE: &[&str] = &[
    ".zip", ".7z", ".rar", ".tar", ".iso", ".cab", ".jar", ".war", ".apk", ".xpi", ".cbz", ".cbr", ".cb7", ".lzh", ".lha",
    ".arj", ".cpio", ".wim", ".deb", ".rpm", ".xar", ".pkg", ".dmg", ".vhd",
];

fn lower(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub fn is_tarball(p: &Path) -> bool {
    let n = lower(p);
    TARBALL.iter().any(|e| n.ends_with(e))
}

/// Can Finder look inside this file?
pub fn is_archive(p: &Path) -> bool {
    let n = lower(p);
    is_tarball(p) || ARCHIVE.iter().any(|e| n.ends_with(e))
}

/// Name without the archive extension(s): `photos.tar.gz` → `photos`.
pub fn stem(p: &Path) -> String {
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let n = name.to_lowercase();
    for e in TARBALL.iter().chain(ARCHIVE) {
        if n.ends_with(e) && n.len() > e.len() {
            return name[..name.len() - e.len()].to_string();
        }
    }
    match name.rfind('.') {
        Some(i) if i > 0 => name[..i].to_string(),
        _ => name,
    }
}

fn have(p: &str) -> bool {
    aqua_sys::have(p)
}

fn seven() -> Option<&'static str> {
    ["7z", "7zz", "7za"].into_iter().find(|p| have(p))
}

/// `7z l -slt` output → members.
pub fn parse_7z_slt(out: &str) -> Vec<ArcEntry> {
    let mut v = vec![];
    // Skip the archive's own header block (before the "----------" line).
    let body = out.split_once("\n----------").map(|(_, b)| b).unwrap_or(out);
    for block in body.split("\n\n") {
        let mut e = ArcEntry { path: String::new(), is_dir: false, size: 0, mtime: 0 };
        for line in block.lines() {
            let Some((k, val)) = line.split_once(" = ") else { continue };
            match k.trim() {
                "Path" => e.path = val.trim_matches('/').replace('\\', "/"),
                "Folder" => e.is_dir = val.trim() == "+",
                "Size" => e.size = val.trim().parse().unwrap_or(0),
                "Modified" => e.mtime = parse_time(val.trim()),
                "Attributes" if val.starts_with('D') => e.is_dir = true,
                _ => {}
            }
        }
        if !e.path.is_empty() {
            v.push(e);
        }
    }
    v
}

/// `bsdtar -tvf` (ls -l style) output → members.
pub fn parse_bsdtar(out: &str) -> Vec<ArcEntry> {
    let mut v = vec![];
    for line in out.lines() {
        // perms links owner group size month day time|year name…
        let mut rest = line;
        let mut fields = vec![];
        for _ in 0..8 {
            rest = rest.trim_start();
            let end = rest.find(' ').unwrap_or(rest.len());
            fields.push(&rest[..end]);
            rest = &rest[end..];
        }
        let name = rest.strip_prefix(' ').unwrap_or(rest);
        if fields.len() < 8 || name.is_empty() {
            continue;
        }
        let name = name.split(" -> ").next().unwrap_or(name);
        let name = name.split(" link to ").next().unwrap_or(name);
        let is_dir = fields[0].starts_with('d') || name.ends_with('/');
        let path = name.trim_start_matches("./").trim_matches('/').to_string();
        if path.is_empty() || path == "." {
            continue;
        }
        v.push(ArcEntry { path, is_dir, size: fields[4].parse().unwrap_or(0), mtime: 0 });
    }
    v
}

/// "2024-05-01 13:45:10" (local time) → unix seconds.
fn parse_time(s: &str) -> i64 {
    let s = s.split('.').next().unwrap_or(s);
    let (d, t) = s.split_once(' ').unwrap_or((s, "00:00:00"));
    let dp: Vec<i64> = d.split('-').filter_map(|x| x.parse().ok()).collect();
    let tp: Vec<i64> = t.split(':').filter_map(|x| x.parse().ok()).collect();
    if dp.len() != 3 {
        return 0;
    }
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = (dp[0] - 1900) as i32;
    tm.tm_mon = (dp[1] - 1) as i32;
    tm.tm_mday = dp[2] as i32;
    tm.tm_hour = *tp.first().unwrap_or(&0) as i32;
    tm.tm_min = *tp.get(1).unwrap_or(&0) as i32;
    tm.tm_sec = *tp.get(2).unwrap_or(&0) as i32;
    tm.tm_isdst = -1;
    unsafe { libc::mktime(&mut tm) as i64 }
}

/// Members directly inside `dir` ("" = top level). Folders that only exist implicitly (as
/// a prefix of member paths) are included.
pub fn children(all: &[ArcEntry], dir: &str) -> Vec<ArcEntry> {
    let dir = dir.trim_matches('/');
    let prefix = if dir.is_empty() { String::new() } else { format!("{dir}/") };
    let mut out: Vec<ArcEntry> = vec![];
    for e in all {
        let Some(rel) = e.path.strip_prefix(&prefix) else { continue };
        if rel.is_empty() {
            continue;
        }
        let (first, nested) = match rel.split_once('/') {
            Some((f, _)) => (f, true),
            None => (rel, false),
        };
        let path = format!("{prefix}{first}");
        if let Some(have) = out.iter_mut().find(|o| o.path == path) {
            if !nested {
                // the explicit entry carries the real size / date
                *have = ArcEntry { is_dir: have.is_dir || e.is_dir, ..e.clone() };
            }
            continue;
        }
        if nested {
            out.push(ArcEntry { path, is_dir: true, size: 0, mtime: e.mtime });
        } else {
            out.push(e.clone());
        }
    }
    out
}

/// Top-level names of an archive's members.
pub fn top_level(all: &[ArcEntry]) -> Vec<String> {
    children(all, "").into_iter().map(|e| e.path).collect()
}

fn run(cmd: &mut Command) -> Result<String, String> {
    let out = cmd.output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
        Err(if line.is_empty() { format!("exit status {}", out.status) } else { line })
    }
}

fn list_uncached(p: &Path) -> Result<Vec<ArcEntry>, String> {
    let seven = seven();
    if !is_tarball(p) {
        if let Some(z) = seven {
            return run(Command::new(z).args(["l", "-slt", "-sccUTF-8", "-p-"]).arg("--").arg(p)).map(|o| parse_7z_slt(&o));
        }
    }
    if have("bsdtar") {
        return run(Command::new("bsdtar").arg("-tvf").arg(p)).map(|o| parse_bsdtar(&o));
    }
    if have("tar") && is_tarball(p) {
        return run(Command::new("tar").arg("-tvf").arg(p)).map(|o| parse_bsdtar(&o));
    }
    Err(crate::tr("Install 7-Zip (7z) or libarchive (bsdtar) to open archives.").into())
}

thread_local! {
    /// (archive, mtime, size) → members; listing big archives is slow.
    static CACHE: RefCell<Option<(PathBuf, i64, u64, Vec<ArcEntry>)>> = const { RefCell::new(None) };
}

fn stamp(p: &Path) -> (i64, u64) {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(p).map(|m| (m.mtime(), m.len())).unwrap_or((0, 0))
}

/// Members of archive `p` (cached while the file is unchanged).
pub fn list(p: &Path) -> Result<Vec<ArcEntry>, String> {
    let (mt, len) = stamp(p);
    if let Some(v) = CACHE.with(|c| {
        c.borrow().as_ref().filter(|(q, m, l, _)| q == p && *m == mt && *l == len).map(|(.., v)| v.clone())
    }) {
        return Ok(v);
    }
    let v = list_uncached(p)?;
    CACHE.with(|c| *c.borrow_mut() = Some((p.to_path_buf(), mt, len, v.clone())));
    Ok(v)
}

/// Extract the whole archive into the existing folder `dest`.
pub fn extract_all(p: &Path, dest: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    if !is_tarball(p) {
        if let Some(z) = seven() {
            let mut o = std::ffi::OsString::from("-o");
            o.push(dest);
            return run(Command::new(z).args(["x", "-y", "-p-", "-aou"]).arg(o).arg("--").arg(p)).map(|_| ());
        }
    }
    if have("bsdtar") {
        return run(Command::new("bsdtar").arg("-xf").arg(p).arg("-C").arg(dest)).map(|_| ());
    }
    run(Command::new("tar").arg("-xf").arg(p).arg("-C").arg(dest)).map(|_| ())
}

/// Extract members `inner` (and everything below them) into `dest`, which receives them by
/// their last path component (like dragging items out of a folder). Returns the new paths.
pub fn extract_items(p: &Path, inner: &[String], dest: &Path) -> Result<Vec<PathBuf>, String> {
    let tmp = scratch_dir("extract")?;
    let res = (|| {
        if !is_tarball(p) {
            if let Some(z) = seven() {
                let mut o = std::ffi::OsString::from("-o");
                o.push(&tmp);
                let mut c = Command::new(z);
                c.args(["x", "-y", "-p-"]).arg(o).arg("--").arg(p);
                for i in inner {
                    c.arg(i);
                }
                return run(&mut c).map(|_| ());
            }
        }
        let mut c = Command::new(if have("bsdtar") { "bsdtar" } else { "tar" });
        c.arg("-xf").arg(p).arg("-C").arg(&tmp);
        for i in inner {
            c.arg(i);
        }
        run(&mut c).map(|_| ())
    })();
    if let Err(e) = res {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }
    let mut out = vec![];
    for i in inner {
        let src = tmp.join(i);
        let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !src.exists() || name.is_empty() {
            continue;
        }
        let dst = super::fs::unique(dest, &name, "");
        if std::fs::rename(&src, &dst).is_err() {
            let ok = Command::new("cp").arg("-a").arg(&src).arg(&dst).status().map(|s| s.success()).unwrap_or(false);
            if !ok {
                continue;
            }
        }
        out.push(dst);
    }
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(out)
}

/// "Extract Here" (Archive Utility): an archive holding a single item extracts to that item
/// next to it; otherwise into a new folder named after the archive. Returns what was created.
pub fn extract_here(p: &Path) -> Result<PathBuf, String> {
    let dir = p.parent().unwrap_or(Path::new("/")).to_path_buf();
    let tops = list(p).map(|v| top_level(&v)).unwrap_or_default();
    let tmp = scratch_dir_in(&dir)?;
    if let Err(e) = extract_all(p, &tmp) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }
    let made: Vec<PathBuf> = std::fs::read_dir(&tmp).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    let res = if made.len() == 1 && tops.len() <= 1 {
        let name = made[0].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let dst = super::fs::unique(&dir, &name, "");
        std::fs::rename(&made[0], &dst).map(|_| dst).map_err(|e| e.to_string())
    } else {
        let dst = super::fs::unique(&dir, &stem(p), "");
        std::fs::rename(&tmp, &dst).map(|_| dst.clone()).map_err(|e| e.to_string())
    };
    let _ = std::fs::remove_dir_all(&tmp);
    res
}

/// A fresh hidden folder inside `dir` (same file system: the final rename is instant).
fn scratch_dir_in(dir: &Path) -> Result<PathBuf, String> {
    for k in 0..1000u32 {
        let d = dir.join(format!(".aqua-extract-{}-{k}", std::process::id()));
        if std::fs::create_dir(&d).is_ok() {
            return Ok(d);
        }
    }
    Err(crate::tr("The folder can't be written to.").into())
}

/// A fresh folder under `~/.cache/aqua/archives/`.
pub fn scratch_dir(what: &str) -> Result<PathBuf, String> {
    let base = dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("aqua/archives");
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let d = base.join(format!("{what}-{}-{ts}", std::process::id()));
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

/// Extract one member to a temporary folder (to open it); returns its path there.
pub fn extract_to_temp(p: &Path, inner: &str) -> Result<PathBuf, String> {
    let d = scratch_dir("open")?;
    let got = extract_items(p, &[inner.to_string()], &d)?;
    got.into_iter().next().ok_or_else(|| crate::tr("The item couldn't be extracted.").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLT: &str = "\
7-Zip [64] 17.05

Listing archive: t.zip

--
Path = t.zip
Type = zip
Physical Size = 400

----------
Path = docs
Folder = +
Size = 0
Modified = 2024-05-01 13:45:10

Path = docs/readme.txt
Folder = -
Size = 12
Modified = 2024-05-01 13:45:10

Path = img/a b.png
Folder = -
Size = 300
Modified = 2024-05-02 08:00:00
";

    #[test]
    fn parses_7z_listing() {
        let v = parse_7z_slt(SLT);
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], ArcEntry { path: "docs".into(), is_dir: true, size: 0, mtime: v[0].mtime });
        assert!(v[0].mtime > 1_700_000_000);
        assert_eq!(v[2].path, "img/a b.png");
        assert_eq!(v[2].size, 300);
    }

    #[test]
    fn parses_bsdtar_listing() {
        let out = "drwxr-xr-x  0 me     me          0 May  1 13:45 proj/\n\
                   -rw-r--r--  0 me     me         42 May  1 13:45 proj/my file.rs\n\
                   lrwxrwxrwx  0 me     me          0 May  1 13:45 proj/link -> my file.rs\n";
        let v = parse_bsdtar(out);
        assert_eq!(v.len(), 3);
        assert!(v[0].is_dir && v[0].path == "proj");
        assert_eq!((v[1].path.as_str(), v[1].size), ("proj/my file.rs", 42));
        assert_eq!(v[2].path, "proj/link");
    }

    #[test]
    fn children_include_implicit_folders() {
        let v = parse_7z_slt(SLT);
        let top = children(&v, "");
        let names: Vec<_> = top.iter().map(|e| (e.path.as_str(), e.is_dir)).collect();
        assert_eq!(names, vec![("docs", true), ("img", true)]);
        let docs = children(&v, "docs");
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].path, "docs/readme.txt");
        assert_eq!(children(&v, "img/")[0].size, 300);
        assert_eq!(top_level(&v), vec!["docs", "img"]);
    }

    #[test]
    fn archive_names() {
        assert!(is_archive(Path::new("/x/a.ZIP")));
        assert!(is_archive(Path::new("a.tar.gz")) && is_tarball(Path::new("a.tar.gz")));
        assert!(!is_archive(Path::new("a.txt")));
        assert_eq!(stem(Path::new("/x/photos.tar.gz")), "photos");
        assert_eq!(stem(Path::new("b.zip")), "b");
    }

    /// Real round trip through the installed tools (skipped when none is installed).
    #[test]
    fn extract_round_trip() {
        if seven().is_none() && !have("bsdtar") {
            return;
        }
        let root = std::env::temp_dir().join(format!("aqua-arc-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let src = root.join("src/proj");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("a.txt"), "hello").unwrap();
        std::fs::write(src.join("sub/b.txt"), "world").unwrap();
        let arc = root.join("proj.tar.gz");
        let ok = Command::new("tar").arg("-czf").arg(&arc).arg("-C").arg(root.join("src")).arg("proj").status().unwrap();
        assert!(ok.success());
        let v = list(&arc).unwrap();
        assert!(v.iter().any(|e| e.path == "proj/sub/b.txt"), "{v:?}");
        assert_eq!(top_level(&v), vec!["proj"]);
        // single top-level folder → extracted next to the archive under its own name
        std::fs::remove_dir_all(root.join("src")).unwrap();
        let out = extract_here(&arc).unwrap();
        assert_eq!(out, root.join("proj"));
        assert_eq!(std::fs::read_to_string(root.join("proj/sub/b.txt")).unwrap(), "world");
        // a second time: unique name, nothing overwritten
        let out2 = extract_here(&arc).unwrap();
        assert_eq!(out2, root.join("proj 2"));
        let got = extract_items(&arc, &["proj/sub".into()], &root).unwrap();
        assert_eq!(got, vec![root.join("sub")]);
        assert!(root.join("sub/b.txt").exists());
        let tmp = extract_to_temp(&arc, "proj/a.txt").unwrap();
        assert_eq!(std::fs::read_to_string(&tmp).unwrap(), "hello");
        if let Some(z) = seven() {
            let zip = root.join("p.zip");
            let ok = Command::new(z).arg("a").arg("-tzip").arg(&zip).arg(root.join("proj")).stdout(std::process::Stdio::null()).status().unwrap();
            assert!(ok.success());
            let v = list(&zip).unwrap();
            assert!(v.iter().any(|e| e.path == "proj/sub/b.txt" && !e.is_dir), "{v:?}");
            let dest = root.join("zipout");
            std::fs::create_dir_all(&dest).unwrap();
            let got = extract_items(&zip, &["proj/sub".into()], &dest).unwrap();
            assert_eq!(got, vec![dest.join("sub")]);
            assert_eq!(std::fs::read_to_string(dest.join("sub/b.txt")).unwrap(), "world");
        }
        let leftovers = std::fs::read_dir(&root).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with(".aqua-extract")).count();
        assert_eq!(leftovers, 0);
        let _ = std::fs::remove_dir_all(&root);
    }
}
