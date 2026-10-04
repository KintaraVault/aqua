//! Undo / redo of file operations.
use super::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Rename { from: PathBuf, to: PathBuf },
    Move { pairs: Vec<(PathBuf, PathBuf)> },
    Copy { pairs: Vec<(PathBuf, PathBuf)> },
    Trash { items: Vec<(PathBuf, PathBuf)> },
    PutBack { items: Vec<(PathBuf, PathBuf)> },
    NewFolder { path: PathBuf },
    Alias { pairs: Vec<(PathBuf, PathBuf)> },
    Tag { paths: Vec<PathBuf>, tag: String, on: bool },
}

fn name(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn what(n: usize, first: &Path) -> String {
    if n == 1 {
        format!("“{}”", name(first))
    } else {
        crate::ntr("{n} Item", "{n} Items", n as i64)
    }
}

impl Op {
    pub fn label(&self) -> String {
        let (key, n, first): (&str, usize, PathBuf) = match self {
            Op::Rename { from, .. } => ("Rename", 1, from.clone()),
            Op::Move { pairs } => ("Move", pairs.len(), pairs.first().map(|p| p.0.clone()).unwrap_or_default()),
            Op::Copy { pairs } => ("Copy", pairs.len(), pairs.first().map(|p| p.0.clone()).unwrap_or_default()),
            Op::Trash { items } => {
                ("Move to Trash", items.len(), items.first().map(|p| p.0.clone()).unwrap_or_default())
            }
            Op::PutBack { items } => ("Put Back", items.len(), items.first().map(|p| p.1.clone()).unwrap_or_default()),
            Op::NewFolder { path } => ("New Folder", 1, path.clone()),
            Op::Alias { pairs } => ("Make Alias", pairs.len(), pairs.first().map(|p| p.0.clone()).unwrap_or_default()),
            Op::Tag { paths, .. } => ("Tags", paths.len(), paths.first().cloned().unwrap_or_default()),
        };
        crate::trf("{action} of {what}", &[("action", &crate::tr(key)), ("what", &what(n, &first))])
    }
}

/// What applying an operation in one direction left behind.
#[derive(Debug, Default)]
pub struct Outcome {
    pub select: Vec<PathBuf>,
    pub moved: Vec<(PathBuf, PathBuf)>,
    pub tags: Vec<(PathBuf, String, bool)>,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct History {
    undo: Vec<Op>,
    redo: Vec<Op>,
    pub trash: Option<PathBuf>,
}

const LIMIT: usize = 100;

impl History {
    pub fn push(&mut self, op: Op) {
        let empty = match &op {
            Op::Move { pairs } | Op::Copy { pairs } | Op::Alias { pairs } => pairs.is_empty(),
            Op::Trash { items } | Op::PutBack { items } => items.is_empty(),
            Op::Tag { paths, .. } => paths.is_empty(),
            _ => false,
        };
        if empty {
            return;
        }
        self.undo.push(op);
        if self.undo.len() > LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn can_undo(&self) -> Option<String> {
        self.undo.last().map(|o| o.label())
    }

    pub fn can_redo(&self) -> Option<String> {
        self.redo.last().map(|o| o.label())
    }

    pub fn undo(&mut self) -> Option<Outcome> {
        let op = self.undo.pop()?;
        let (out, back) = self.revert(op);
        if let Some(b) = back {
            self.redo.push(b);
        }
        Some(out)
    }

    pub fn redo(&mut self) -> Option<Outcome> {
        let op = self.redo.pop()?;
        let (out, again) = self.apply(op);
        if let Some(a) = again {
            self.undo.push(a);
        }
        Some(out)
    }

    fn trash(&self, p: &Path) -> Result<PathBuf, String> {
        match &self.trash {
            Some(t) => fs::trash_in(t, p),
            None => fs::trash(p),
        }
    }

    /// Undo `op`; returns the outcome and the operation that redoes it.
    fn revert(&self, op: Op) -> (Outcome, Option<Op>) {
        let mut out = Outcome::default();
        let redo = match op {
            Op::Rename { from, to } => match fs::move_to(&to, &from) {
                Ok(()) => {
                    out.moved.push((to.clone(), from.clone()));
                    out.select.push(from.clone());
                    Some(Op::Rename { from, to })
                }
                Err(e) => {
                    out.error = Some(e.to_string());
                    None
                }
            },
            Op::Move { pairs } => {
                let mut done = vec![];
                for (from, to) in pairs.into_iter().rev() {
                    if from.symlink_metadata().is_ok() {
                        out.error = Some(crate::trf("“{name}” already exists.", &[("name", &name(&from))]));
                        continue;
                    }
                    match fs::move_to(&to, &from) {
                        Ok(()) => {
                            out.moved.push((to.clone(), from.clone()));
                            out.select.push(from.clone());
                            done.push((from, to));
                        }
                        Err(e) => out.error = Some(e.to_string()),
                    }
                }
                done.reverse();
                Some(Op::Move { pairs: done })
            }
            Op::Copy { pairs } => {
                let mut done = vec![];
                for (src, dst) in pairs {
                    match self.trash(&dst) {
                        Ok(_) => done.push((src, dst)),
                        Err(e) => out.error = Some(e),
                    }
                }
                Some(Op::Copy { pairs: done })
            }
            Op::Alias { pairs } => {
                let mut done = vec![];
                for (src, link) in pairs {
                    if std::fs::remove_file(&link).is_ok() {
                        done.push((src, link));
                    }
                }
                Some(Op::Alias { pairs: done })
            }
            Op::Trash { items } => {
                let mut done = vec![];
                for (orig, trashed) in items {
                    match fs::untrash(&trashed, &orig) {
                        Ok(dst) => {
                            out.select.push(dst.clone());
                            done.push((dst, trashed));
                        }
                        Err(e) => out.error = Some(e),
                    }
                }
                Some(Op::Trash { items: done })
            }
            Op::PutBack { items } => {
                let mut done = vec![];
                for (_, orig) in items {
                    match self.trash(&orig) {
                        Ok(t) => done.push((t, orig)),
                        Err(e) => out.error = Some(e),
                    }
                }
                Some(Op::PutBack { items: done })
            }
            Op::NewFolder { path } => {
                let empty = std::fs::read_dir(&path).map(|mut r| r.next().is_none()).unwrap_or(false);
                let r = if empty {
                    std::fs::remove_dir(&path).map_err(|e| e.to_string())
                } else {
                    self.trash(&path).map(|_| ())
                };
                match r {
                    Ok(()) => Some(Op::NewFolder { path }),
                    Err(e) => {
                        out.error = Some(e);
                        None
                    }
                }
            }
            Op::Tag { paths, tag, on } => {
                for p in &paths {
                    out.tags.push((p.clone(), tag.clone(), !on));
                }
                out.select = paths.clone();
                Some(Op::Tag { paths, tag, on })
            }
        };
        (out, redo)
    }

    /// Redo `op` (as originally done); returns the outcome and the operation that undoes it.
    fn apply(&self, op: Op) -> (Outcome, Option<Op>) {
        let mut out = Outcome::default();
        let undo = match op {
            Op::Rename { from, to } => match fs::move_to(&from, &to) {
                Ok(()) => {
                    out.moved.push((from.clone(), to.clone()));
                    out.select.push(to.clone());
                    Some(Op::Rename { from, to })
                }
                Err(e) => {
                    out.error = Some(e.to_string());
                    None
                }
            },
            Op::Move { pairs } => {
                let mut done = vec![];
                for (from, to) in pairs {
                    if to.symlink_metadata().is_ok() {
                        out.error = Some(crate::trf("“{name}” already exists.", &[("name", &name(&to))]));
                        continue;
                    }
                    match fs::move_to(&from, &to) {
                        Ok(()) => {
                            out.moved.push((from.clone(), to.clone()));
                            out.select.push(to.clone());
                            done.push((from, to));
                        }
                        Err(e) => out.error = Some(e.to_string()),
                    }
                }
                Some(Op::Move { pairs: done })
            }
            Op::Copy { pairs } => {
                let mut done = vec![];
                for (src, dst) in pairs {
                    let dst = if dst.symlink_metadata().is_ok() {
                        fs::unique(dst.parent().unwrap_or(Path::new("/")), &name(&dst), "")
                    } else {
                        dst
                    };
                    match fs::copy_rec(&src, &dst) {
                        Ok(()) => {
                            out.select.push(dst.clone());
                            done.push((src, dst));
                        }
                        Err(e) => out.error = Some(e.to_string()),
                    }
                }
                Some(Op::Copy { pairs: done })
            }
            Op::Alias { pairs } => {
                let mut done = vec![];
                for (src, link) in pairs {
                    if std::os::unix::fs::symlink(&src, &link).is_ok() {
                        out.select.push(link.clone());
                        done.push((src, link));
                    }
                }
                Some(Op::Alias { pairs: done })
            }
            Op::Trash { items } => {
                let mut done = vec![];
                for (orig, _) in items {
                    match self.trash(&orig) {
                        Ok(t) => done.push((orig, t)),
                        Err(e) => out.error = Some(e),
                    }
                }
                Some(Op::Trash { items: done })
            }
            Op::PutBack { items } => {
                let mut done = vec![];
                for (trashed, orig) in items {
                    match fs::untrash(&trashed, &orig) {
                        Ok(dst) => {
                            out.select.push(dst.clone());
                            done.push((trashed, dst));
                        }
                        Err(e) => out.error = Some(e),
                    }
                }
                Some(Op::PutBack { items: done })
            }
            Op::NewFolder { path } => match std::fs::create_dir(&path) {
                Ok(()) => {
                    out.select.push(path.clone());
                    Some(Op::NewFolder { path })
                }
                Err(e) => {
                    out.error = Some(e.to_string());
                    None
                }
            },
            Op::Tag { paths, tag, on } => {
                for p in &paths {
                    out.tags.push((p.clone(), tag.clone(), on));
                }
                out.select = paths.clone();
                Some(Op::Tag { paths, tag, on })
            }
        };
        (out, undo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aqua-undo-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn hist(d: &Path) -> History {
        History { trash: Some(d.join(".trash")), ..Default::default() }
    }

    #[test]
    fn rename_round_trip() {
        let d = tmp("rename");
        let (a, b) = (d.join("a.txt"), d.join("b.txt"));
        std::fs::write(&a, "x").unwrap();
        std::fs::rename(&a, &b).unwrap();
        let mut h = hist(&d);
        h.push(Op::Rename { from: a.clone(), to: b.clone() });
        assert!(h.can_undo().is_some());
        let o = h.undo().unwrap();
        assert!(a.exists() && !b.exists());
        assert_eq!(o.select, vec![a.clone()]);
        assert_eq!(o.moved, vec![(b.clone(), a.clone())]);
        assert!(h.can_undo().is_none() && h.can_redo().is_some());
        h.redo().unwrap();
        assert!(!a.exists() && b.exists());
        assert!(h.can_undo().is_some() && h.can_redo().is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn move_back_and_forth() {
        let d = tmp("move");
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("f"), "1").unwrap();
        std::fs::rename(d.join("f"), d.join("sub/f")).unwrap();
        let mut h = hist(&d);
        h.push(Op::Move { pairs: vec![(d.join("f"), d.join("sub/f"))] });
        h.undo().unwrap();
        assert!(d.join("f").exists() && !d.join("sub/f").exists());
        h.redo().unwrap();
        assert!(!d.join("f").exists() && d.join("sub/f").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn move_undo_does_not_clobber() {
        let d = tmp("clobber");
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("sub/f"), "moved").unwrap();
        std::fs::write(d.join("f"), "new").unwrap();
        let mut h = hist(&d);
        h.push(Op::Move { pairs: vec![(d.join("f"), d.join("sub/f"))] });
        let o = h.undo().unwrap();
        assert!(o.error.is_some());
        assert_eq!(std::fs::read_to_string(d.join("f")).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(d.join("sub/f")).unwrap(), "moved");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn copy_undo_trashes_the_copy() {
        let d = tmp("copy");
        std::fs::write(d.join("a"), "1").unwrap();
        std::fs::write(d.join("a copy"), "1").unwrap();
        let mut h = hist(&d);
        h.push(Op::Copy { pairs: vec![(d.join("a"), d.join("a copy"))] });
        h.undo().unwrap();
        assert!(d.join("a").exists() && !d.join("a copy").exists());
        assert!(d.join(".trash/files/a copy").exists());
        h.redo().unwrap();
        assert!(d.join("a copy").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn trash_and_put_back() {
        let d = tmp("trash");
        let f = d.join("doc.txt");
        std::fs::write(&f, "hello").unwrap();
        let t = fs::trash_in(&d.join(".trash"), &f).unwrap();
        assert!(!f.exists() && t.exists());
        assert!(fs::info_of(&t).exists());
        let mut h = hist(&d);
        h.push(Op::Trash { items: vec![(f.clone(), t.clone())] });
        let o = h.undo().unwrap();
        assert!(f.exists() && !t.exists() && !fs::info_of(&t).exists());
        assert_eq!(o.select, vec![f.clone()]);
        h.redo().unwrap();
        assert!(!f.exists());
        h.undo().unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "hello");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn new_folder_and_tags() {
        let d = tmp("folder");
        let p = d.join("untitled folder");
        std::fs::create_dir(&p).unwrap();
        let mut h = hist(&d);
        h.push(Op::NewFolder { path: p.clone() });
        h.undo().unwrap();
        assert!(!p.exists());
        h.redo().unwrap();
        assert!(p.is_dir());
        h.push(Op::Tag { paths: vec![p.clone()], tag: "Red".into(), on: true });
        let o = h.undo().unwrap();
        assert_eq!(o.tags, vec![(p.clone(), "Red".to_string(), false)]);
        let o = h.redo().unwrap();
        assert_eq!(o.tags, vec![(p.clone(), "Red".to_string(), true)]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn empty_ops_are_ignored_and_new_ops_clear_redo() {
        let mut h = History::default();
        h.push(Op::Move { pairs: vec![] });
        assert!(h.can_undo().is_none());
        h.push(Op::Tag { paths: vec!["/x".into()], tag: "Red".into(), on: true });
        h.undo();
        assert!(h.can_redo().is_some());
        h.push(Op::Tag { paths: vec!["/y".into()], tag: "Red".into(), on: true });
        assert!(h.can_redo().is_none());
    }

    #[test]
    fn labels() {
        let op = Op::Rename { from: "/a/old.txt".into(), to: "/a/new.txt".into() };
        assert!(op.label().contains("old.txt"), "{}", op.label());
        let op = Op::Move { pairs: vec![("/a".into(), "/b/a".into()), ("/c".into(), "/b/c".into())] };
        assert!(op.label().contains('2'), "{}", op.label());
    }
}
