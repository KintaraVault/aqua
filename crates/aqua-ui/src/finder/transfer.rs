//! Copying and moving in the background, with progress, cancellation and name conflicts.
use super::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Copy,
    Move,
}

/// How to resolve a name that already exists in the destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    KeepBoth,
    Replace,
    Skip,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub src: PathBuf,
    pub dst: PathBuf,
    pub mode: Mode,
    /// an existing item at `dst` is moved to the Trash first
    pub replace: bool,
}

/// Items whose name is already taken in `dest` (copies into their own folder are not conflicts:
/// they get a “copy” name).
pub fn conflicts(srcs: &[PathBuf], dest: &Path, mode: Mode) -> Vec<PathBuf> {
    srcs.iter()
        .filter(|s| {
            let same = s.parent() == Some(dest);
            !same && dest.join(s.file_name().unwrap_or_default()).symlink_metadata().is_ok()
        })
        .filter(|s| !(mode == Mode::Move && s.parent() == Some(dest)))
        .cloned()
        .collect()
}

/// Turn a request into concrete steps. `choice` answers conflicts (missing = keep both).
pub fn plan(srcs: &[PathBuf], dest: &Path, mode: Mode, choice: &dyn Fn(&Path) -> Choice) -> Vec<Step> {
    let mut steps: Vec<Step> = vec![];
    let mut taken: Vec<PathBuf> = vec![];
    for s in srcs {
        let name = s.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let same = s.parent() == Some(dest);
        if same && mode == Mode::Move {
            continue;
        }
        if fs::inside(dest, s) {
            continue;
        }
        let target = dest.join(&name);
        let exists = target.symlink_metadata().is_ok() || taken.contains(&target);
        let (dst, replace) = if same {
            (unique_avoiding(dest, &name, crate::tr("copy"), &taken), false)
        } else if exists {
            match choice(s) {
                Choice::Skip => continue,
                Choice::Replace if !taken.contains(&target) => (target, true),
                _ => (unique_avoiding(dest, &name, "", &taken), false),
            }
        } else {
            (target, false)
        };
        taken.push(dst.clone());
        steps.push(Step { src: s.clone(), dst, mode, replace });
    }
    steps
}

fn unique_avoiding(dir: &Path, name: &str, suffix: &str, taken: &[PathBuf]) -> PathBuf {
    let free = |p: &PathBuf| p.symlink_metadata().is_err() && !taken.contains(p);
    let plain = dir.join(name);
    if suffix.is_empty() && free(&plain) {
        return plain;
    }
    let (stem, ext) = fs::split_ext(name);
    if !suffix.is_empty() {
        let first = dir.join(format!("{stem} {suffix}{ext}"));
        if free(&first) {
            return first;
        }
    }
    (2..)
        .map(|k| {
            if suffix.is_empty() {
                dir.join(format!("{stem} {k}{ext}"))
            } else {
                dir.join(format!("{stem} {suffix} {k}{ext}"))
            }
        })
        .find(free)
        .unwrap()
}

#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub bytes: u64,
    pub total: u64,
    pub items: usize,
    pub count: usize,
    pub current: String,
}

pub enum Msg {
    Progress(Progress),
    Done { done: Vec<Step>, replaced: Vec<(PathBuf, PathBuf)>, error: Option<String>, cancelled: bool },
}

pub struct Handle {
    pub rx: mpsc::Receiver<Msg>,
    pub cancel: Arc<AtomicBool>,
    pub mode: Mode,
    pub dest: PathBuf,
    pub count: usize,
    pub started: std::time::Instant,
    pub last: Progress,
}

impl Handle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub fn start(steps: Vec<Step>, trash: Option<PathBuf>) -> Handle {
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let mode = steps.first().map(|s| s.mode).unwrap_or(Mode::Copy);
    let dest = steps.first().and_then(|s| s.dst.parent()).map(|p| p.to_path_buf()).unwrap_or_default();
    let count = steps.len();
    let c = cancel.clone();
    std::thread::Builder::new()
        .name("finder-transfer".into())
        .spawn(move || {
            let r = run(steps, &c, trash.as_deref(), &mut |p| {
                let _ = tx.send(Msg::Progress(p.clone()));
            });
            let _ = tx.send(r);
        })
        .ok();
    Handle { rx, cancel, mode, dest, count, started: std::time::Instant::now(), last: Progress::default() }
}

fn bytes_of(p: &Path) -> u64 {
    match p.symlink_metadata() {
        Ok(m) if m.is_dir() => fs::tree_size(p, &|| false).0,
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

fn same_volume(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let da = a.symlink_metadata().map(|m| m.dev()).ok();
    let db = b.parent().and_then(|p| std::fs::metadata(p).ok()).map(|m| m.dev());
    da.is_some() && da == db
}

/// Run the steps on the current thread (the worker); `report` gets throttled progress.
pub fn run(steps: Vec<Step>, cancel: &AtomicBool, trash: Option<&Path>, report: &mut dyn FnMut(&Progress)) -> Msg {
    let mut pr = Progress { count: steps.len(), ..Default::default() };
    let fast: Vec<bool> = steps.iter().map(|s| s.mode == Mode::Move && same_volume(&s.src, &s.dst)).collect();
    pr.total = steps.iter().zip(&fast).filter(|(_, f)| !**f).map(|(s, _)| bytes_of(&s.src)).sum();
    let mut done = vec![];
    let mut replaced = vec![];
    let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
    let mut error = None;
    for (s, fast) in steps.into_iter().zip(fast) {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        pr.current = s.src.file_name().unwrap_or_default().to_string_lossy().into_owned();
        report(&pr);
        if s.replace && s.dst.symlink_metadata().is_ok() {
            let r = match trash {
                Some(t) => fs::trash_in(t, &s.dst),
                None => fs::trash(&s.dst),
            };
            match r {
                Ok(t) => replaced.push((s.dst.clone(), t)),
                Err(e) => {
                    error = Some(e);
                    break;
                }
            }
        }
        if fs::inside(s.dst.parent().unwrap_or(&s.dst), &s.src) {
            error = Some(crate::tr("cannot copy a folder into itself").to_string());
            break;
        }
        let renamed = if fast {
            match fs::rename_noreplace(&s.src, &s.dst) {
                // a bind mount or btrfs subvolume looks like the same device but is not
                Err(e) if e.raw_os_error() == Some(libc::EXDEV) => None,
                r => Some(r),
            }
        } else {
            None
        };
        let r = if let Some(r) = renamed {
            r
        } else {
            let mut tick = |n: u64, pr: &mut Progress| {
                pr.bytes += n;
                if last.elapsed().as_millis() > 80 {
                    last = std::time::Instant::now();
                    report(pr);
                }
            };
            let r = copy_tree(&s.src, &s.dst, cancel, &mut pr, &mut tick);
            match (r, s.mode) {
                (Ok(()), Mode::Move) => fs::remove_rec(&s.src),
                // AlreadyExists: something else owns `dst` now, it is not ours to delete
                (Err(e), _) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(e),
                (Err(e), _) => {
                    let _ = fs::remove_rec(&s.dst);
                    Err(e)
                }
                (ok, _) => ok,
            }
        };
        match r {
            Ok(()) => {
                pr.items += 1;
                done.push(s);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => break,
            Err(e) => {
                error = Some(e.to_string());
                break;
            }
        }
    }
    report(&pr);
    Msg::Done { done, replaced, error, cancelled: cancel.load(Ordering::Relaxed) }
}

fn copy_tree(
    src: &Path,
    dst: &Path,
    cancel: &AtomicBool,
    pr: &mut Progress,
    tick: &mut dyn FnMut(u64, &mut Progress),
) -> std::io::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "cancelled"));
    }
    let md = std::fs::symlink_metadata(src)?;
    if md.file_type().is_symlink() {
        std::os::unix::fs::symlink(std::fs::read_link(src)?, dst)?;
        tick(md.len(), pr);
        return Ok(());
    }
    if md.is_dir() {
        if fs::inside(dst, src) {
            return Err(std::io::Error::other("cannot copy a folder into itself"));
        }
        std::fs::create_dir(dst)?;
        for e in std::fs::read_dir(src)?.flatten() {
            copy_tree(&e.path(), &dst.join(e.file_name()), cancel, pr, tick)?;
        }
        let _ = std::fs::set_permissions(dst, md.permissions());
        let _ = set_mtime(dst, &md);
        return Ok(());
    }
    if !md.is_file() {
        // FIFOs would block open() forever; sockets/devices cannot be copied as data
        use std::os::unix::fs::FileTypeExt;
        if md.file_type().is_fifo() {
            use std::os::unix::ffi::OsStrExt;
            use std::os::unix::fs::PermissionsExt;
            let c = std::ffi::CString::new(dst.as_os_str().as_bytes()).map_err(std::io::Error::other)?;
            if unsafe { libc::mkfifo(c.as_ptr(), md.permissions().mode() & 0o7777) } != 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        return Ok(());
    }
    let mut r = std::fs::File::open(src)?;
    let mut w = std::fs::File::options().write(true).create_new(true).open(dst)?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(w);
            let _ = std::fs::remove_file(dst);
            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "cancelled"));
        }
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        w.write_all(&buf[..n])?;
        tick(n as u64, pr);
    }
    drop(w);
    let _ = std::fs::set_permissions(dst, md.permissions());
    let _ = set_mtime(dst, &md);
    Ok(())
}

fn set_mtime(p: &Path, md: &std::fs::Metadata) -> std::io::Result<()> {
    let f = std::fs::File::options().read(true).open(p).or_else(|_| std::fs::File::open(p))?;
    f.set_times(std::fs::FileTimes::new().set_modified(md.modified()?).set_accessed(md.accessed()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aqua-xfer-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn run_now(steps: Vec<Step>, trash: &Path) -> Msg {
        let c = AtomicBool::new(false);
        run(steps, &c, Some(trash), &mut |_| {})
    }

    #[test]
    fn plan_names_and_conflicts() {
        let d = tmp("plan");
        let (a, b) = (d.join("a"), d.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("x.txt"), "a").unwrap();
        std::fs::write(a.join("y.txt"), "a").unwrap();
        std::fs::write(b.join("x.txt"), "b").unwrap();
        let srcs = vec![a.join("x.txt"), a.join("y.txt")];
        assert_eq!(conflicts(&srcs, &b, Mode::Copy), vec![a.join("x.txt")]);
        assert!(conflicts(&srcs, &a, Mode::Copy).is_empty());
        let keep = plan(&srcs, &b, Mode::Copy, &|_| Choice::KeepBoth);
        assert_eq!(keep[0].dst, b.join("x 2.txt"));
        assert_eq!(keep[1].dst, b.join("y.txt"));
        let rep = plan(&srcs, &b, Mode::Move, &|_| Choice::Replace);
        assert!(rep[0].replace && rep[0].dst == b.join("x.txt"));
        let skip = plan(&srcs, &b, Mode::Copy, &|_| Choice::Skip);
        assert_eq!(skip.len(), 1);
        let dup = plan(&srcs, &a, Mode::Copy, &|_| Choice::Replace);
        assert_eq!(dup[0].dst, a.join(format!("x {}.txt", crate::tr("copy"))));
        assert!(plan(&srcs, &a, Mode::Move, &|_| Choice::KeepBoth).is_empty());
        assert!(plan(std::slice::from_ref(&a), &a.join("inner"), Mode::Move, &|_| Choice::KeepBoth).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn same_name_twice_in_one_request() {
        let d = tmp("twice");
        for s in ["p", "q", "out"] {
            std::fs::create_dir_all(d.join(s)).unwrap();
        }
        std::fs::write(d.join("p/n.txt"), "p").unwrap();
        std::fs::write(d.join("q/n.txt"), "q").unwrap();
        let steps = plan(&[d.join("p/n.txt"), d.join("q/n.txt")], &d.join("out"), Mode::Copy, &|_| Choice::Replace);
        assert_eq!(steps.len(), 2);
        assert_ne!(steps[0].dst, steps[1].dst);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn copy_tree_with_progress() {
        let d = tmp("copy");
        std::fs::create_dir_all(d.join("src/deep")).unwrap();
        std::fs::write(d.join("src/a.bin"), vec![7u8; 3 << 20]).unwrap();
        std::fs::write(d.join("src/deep/b.txt"), "hello").unwrap();
        std::os::unix::fs::symlink("a.bin", d.join("src/link")).unwrap();
        std::fs::create_dir_all(d.join("dst")).unwrap();
        let steps = plan(&[d.join("src")], &d.join("dst"), Mode::Copy, &|_| Choice::KeepBoth);
        let c = AtomicBool::new(false);
        let mut seen = vec![];
        let m = run(steps, &c, Some(&d.join(".t")), &mut |p| seen.push(p.clone()));
        let Msg::Done { done, error, cancelled, .. } = m else { panic!() };
        assert!(error.is_none() && !cancelled && done.len() == 1);
        assert_eq!(std::fs::read(d.join("dst/src/a.bin")).unwrap().len(), 3 << 20);
        assert_eq!(std::fs::read_to_string(d.join("dst/src/deep/b.txt")).unwrap(), "hello");
        assert_eq!(std::fs::read_link(d.join("dst/src/link")).unwrap(), PathBuf::from("a.bin"));
        let last = seen.last().unwrap();
        assert_eq!(last.bytes, last.total);
        assert_eq!(last.items, 1);
        assert!(d.join("src/a.bin").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn move_and_replace_keeps_old_in_trash() {
        let d = tmp("move");
        std::fs::create_dir_all(d.join("a")).unwrap();
        std::fs::create_dir_all(d.join("b")).unwrap();
        std::fs::write(d.join("a/f"), "new").unwrap();
        std::fs::write(d.join("b/f"), "old").unwrap();
        let steps = plan(&[d.join("a/f")], &d.join("b"), Mode::Move, &|_| Choice::Replace);
        let Msg::Done { done, replaced, error, .. } = run_now(steps, &d.join(".t")) else { panic!() };
        assert!(error.is_none());
        assert_eq!(done.len(), 1);
        assert_eq!(std::fs::read_to_string(d.join("b/f")).unwrap(), "new");
        assert!(!d.join("a/f").exists());
        assert_eq!(replaced.len(), 1);
        assert_eq!(std::fs::read_to_string(&replaced[0].1).unwrap(), "old");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn cancelled_copy_leaves_no_partial_file() {
        let d = tmp("cancel");
        std::fs::write(d.join("big"), vec![1u8; 4 << 20]).unwrap();
        std::fs::create_dir_all(d.join("out")).unwrap();
        let steps = plan(&[d.join("big")], &d.join("out"), Mode::Copy, &|_| Choice::KeepBoth);
        let c = AtomicBool::new(false);
        let m = run(steps, &c, None, &mut |p| {
            if p.bytes > 0 {
                c.store(true, Ordering::Relaxed);
            }
        });
        let Msg::Done { done, cancelled, .. } = m else { panic!() };
        assert!(cancelled && done.is_empty());
        assert!(!d.join("out/big").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn background_handle_reports_done() {
        let d = tmp("bg");
        std::fs::write(d.join("x"), "1").unwrap();
        std::fs::create_dir_all(d.join("o")).unwrap();
        let h = start(plan(&[d.join("x")], &d.join("o"), Mode::Copy, &|_| Choice::KeepBoth), None);
        let mut ok = false;
        while let Ok(m) = h.rx.recv_timeout(std::time::Duration::from_secs(5)) {
            if let Msg::Done { done, .. } = m {
                ok = done.len() == 1;
                break;
            }
        }
        assert!(ok && d.join("o/x").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn symlinked_destination_inside_source_is_refused() {
        let d = tmp("selfloop");
        std::fs::create_dir_all(d.join("a/sub")).unwrap();
        std::fs::write(d.join("a/f"), "x").unwrap();
        std::os::unix::fs::symlink(d.join("a"), d.join("l")).unwrap();
        // the Finder shows ~/l/sub, which really is ~/a/sub
        assert!(plan(&[d.join("a")], &d.join("l/sub"), Mode::Copy, &|_| Choice::KeepBoth).is_empty());
        let step = Step { src: d.join("a"), dst: d.join("l/sub/a"), mode: Mode::Copy, replace: false };
        let Msg::Done { done, error, .. } = run_now(vec![step], &d.join(".t")) else { panic!() };
        assert!(done.is_empty() && error.is_some());
        assert!(!d.join("a/sub/a").exists());
        // a symlink itself may still go into the folder it points to
        assert!(!fs::inside(&d.join("a"), &d.join("l")));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn fifos_are_recreated_instead_of_read() {
        let d = tmp("fifo");
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::create_dir_all(d.join("out")).unwrap();
        std::fs::write(d.join("src/a"), "a").unwrap();
        let c = std::ffi::CString::new(d.join("src/pipe").to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let (tx, rx) = mpsc::channel();
        let steps = plan(&[d.join("src")], &d.join("out"), Mode::Copy, &|_| Choice::KeepBoth);
        let t = d.join(".t");
        std::thread::spawn(move || {
            let _ = tx.send(run_now(steps, &t));
        });
        let Ok(Msg::Done { done, error, .. }) = rx.recv_timeout(std::time::Duration::from_secs(5)) else {
            panic!("copying a FIFO hung")
        };
        assert!(error.is_none() && done.len() == 1, "{error:?}");
        use std::os::unix::fs::FileTypeExt;
        assert!(d.join("out/src/pipe").symlink_metadata().unwrap().file_type().is_fifo());
        assert_eq!(std::fs::read_to_string(d.join("out/src/a")).unwrap(), "a");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn items_appearing_after_planning_are_never_overwritten() {
        for mode in [Mode::Copy, Mode::Move] {
            let d = tmp(&format!("race{mode:?}"));
            std::fs::create_dir_all(d.join("o")).unwrap();
            std::fs::write(d.join("x"), "new").unwrap();
            std::fs::create_dir_all(d.join("dir")).unwrap();
            std::fs::write(d.join("dir/f"), "new").unwrap();
            let steps = plan(&[d.join("x"), d.join("dir")], &d.join("o"), mode, &|_| Choice::KeepBoth);
            assert_eq!(steps.len(), 2);
            // another program creates the same names before the worker gets there
            std::fs::write(d.join("o/x"), "precious").unwrap();
            std::fs::create_dir_all(d.join("o/dir")).unwrap();
            std::fs::write(d.join("o/dir/mine"), "precious").unwrap();
            let Msg::Done { done, error, .. } = run_now(steps, &d.join(".t")) else { panic!() };
            assert!(done.is_empty() && error.is_some(), "{mode:?}");
            assert_eq!(std::fs::read_to_string(d.join("o/x")).unwrap(), "precious");
            assert_eq!(std::fs::read_to_string(d.join("o/dir/mine")).unwrap(), "precious");
            assert_eq!(std::fs::read_to_string(d.join("x")).unwrap(), "new");
            let _ = std::fs::remove_dir_all(&d);
        }
    }

    #[test]
    fn undo_moves_never_overwrite() {
        let d = tmp("undo");
        std::fs::write(d.join("a"), "a").unwrap();
        std::fs::write(d.join("b"), "b").unwrap();
        assert!(fs::move_to(&d.join("a"), &d.join("b")).is_err());
        assert_eq!(std::fs::read_to_string(d.join("b")).unwrap(), "b");
        assert!(fs::move_to(&d.join("a"), &d.join("c")).is_ok());
        assert_eq!(std::fs::read_to_string(d.join("c")).unwrap(), "a");
        std::fs::write(d.join("e"), "mine").unwrap();
        assert!(fs::copy_rec(&d.join("c"), &d.join("e")).is_err());
        assert_eq!(std::fs::read_to_string(d.join("e")).unwrap(), "mine");
        let _ = std::fs::remove_dir_all(&d);
    }
}
