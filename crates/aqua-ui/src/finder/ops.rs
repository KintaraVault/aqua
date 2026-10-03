//! File operations: rename, trash, copy/paste, duplicate, alias, compress, drop.
use super::*;

impl App {
    pub(super) fn alert(&mut self, msg: &str, detail: &str, cancel: bool) {
        let ui = self.ui();
        let f = ui.global::<F>();
        f.set_alert_detail(crate::tr(detail).into());
        f.set_alert_cancel(cancel);
        f.set_alert_ok(crate::tr("OK").into());
        f.set_alert(crate::tr(msg).into());
    }

    pub(super) fn reload_keep(&mut self, select: Vec<PathBuf>) {
        let q = self.ui().global::<F>().get_query();
        let loc = self.loc.clone();
        self.loc = loc;
        self.load();
        if !q.is_empty() {
            self.ui().global::<F>().set_query(q);
            self.sort_and_filter();
            self.refresh();
        }
        let sel: Vec<usize> = select.iter().filter_map(|p| self.row_of.get(p).copied()).collect();
        if !sel.is_empty() {
            self.focus = sel.last().copied();
            self.anchor = sel.first().copied();
            self.sel = sel;
            self.update_selection();
        }
    }

    pub(super) fn new_folder(&mut self) {
        let Some(dir) = self.loc.dir().map(|d| d.to_path_buf()) else { return };
        let p = fs::unique(&dir, crate::tr("untitled folder"), "");
        if let Err(e) = std::fs::create_dir(&p) {
            return self.alert("The folder couldn't be created.", &e.to_string(), false);
        }
        self.reload_keep(vec![p.clone()]);
        if let Some(&vi) = self.row_of.get(&p) {
            self.ui().global::<F>().set_renaming(vi as i32);
        }
    }

    pub(super) fn rename(&mut self, vi: usize, name: &str) {
        let ui_ = self.ui();
        let f = ui_.global::<F>();
        f.set_renaming(-1);
        let Some(&i) = self.shown.get(vi) else { return };
        let e = self.all[i].clone();
        let name = name.trim();
        if name.is_empty() || name == e.name {
            return;
        }
        if name.contains('/') {
            return self.alert("The name can't contain “/”.", "Please choose another name.", false);
        }
        let dst = e.path.with_file_name(name);
        if dst.exists() {
            return self.alert(
                &crate::trf("The name “{name}” is already taken.", &[("name", &name)]),
                "Please choose a different name.",
                false,
            );
        }
        match std::fs::rename(&e.path, &dst) {
            Ok(()) => {
                self.meta.moved(&e.path, &dst);
                self.meta.save();
                self.reload_keep(vec![dst]);
            }
            Err(err) => {
                self.alert(&crate::trf("“{name}” couldn't be renamed.", &[("name", &e.name)]), &err.to_string(), false)
            }
        }
    }

    pub(super) fn trash_selection(&mut self) {
        let paths = self.sel_paths();
        if paths.is_empty() {
            return;
        }
        if matches!(self.loc, Loc::Trash) {
            return self.confirm_delete(paths);
        }
        if matches!(self.loc, Loc::Apps) {
            return self.alert(
                "Applications are removed with your package manager.",
                "Finder can't move installed apps to the Trash.",
                false,
            );
        }
        let mut failed = vec![];
        for p in &paths {
            if let Err(e) = fs::trash(p) {
                failed.push((p.clone(), e));
            }
        }
        if let Some((p, e)) = failed.first() {
            if e.contains("another volume") {
                let all: Vec<PathBuf> = failed.iter().map(|x| x.0.clone()).collect();
                self.reload_keep(vec![]);
                return self.confirm_delete(all);
            }
            self.alert(
                &crate::trf(
                    "“{name}” couldn't be moved to the Trash.",
                    &[("name", &p.file_name().unwrap_or_default().to_string_lossy())],
                ),
                e,
                false,
            );
        }
        self.reload_keep(vec![]);
    }

    pub(super) fn confirm_delete(&mut self, paths: Vec<PathBuf>) {
        let what = if paths.len() == 1 {
            format!("“{}”", paths[0].file_name().unwrap_or_default().to_string_lossy())
        } else {
            crate::ntr("these {n} item", "these {n} items", paths.len() as i64)
        };
        self.pending = Pending::DeleteNow(paths);
        self.alert(
            &crate::trf("Are you sure you want to delete {what} immediately?", &[("what", &what)]),
            "You can't undo this action.",
            true,
        );
        self.ui().global::<F>().set_alert_ok(crate::tr("Delete").into());
    }

    pub(super) fn copy(&mut self, cut: bool) {
        let paths = self.sel_paths();
        if paths.is_empty() {
            return;
        }
        let mut s = String::from(if cut { "cut\n" } else { "copy\n" });
        for p in &paths {
            s.push_str(&p.to_string_lossy());
            s.push('\n');
        }
        let _ = std::fs::create_dir_all(clip_file().parent().unwrap());
        let _ = std::fs::write(clip_file(), s);
        let txt: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        let uris: Vec<String> = paths.iter().map(|p| fs::file_uri(p)).collect();
        if !crate::aqua_msg(&format!("clipfiles {} {}", if cut { "cut" } else { "copy" }, uris.join(" "))) {
            self.ui().invoke_copy_text(txt.join("\n").into());
        }
        if cut {
            self.refresh();
        }
    }

    pub(super) fn paste(&mut self) {
        let Some(dir) = self.loc.dir().map(|d| d.to_path_buf()) else { return };
        let (cut, paths) = read_clip();
        if paths.is_empty() {
            return;
        }
        let mut done = vec![];
        for p in paths {
            let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let same = p.parent() == Some(dir.as_path());
            let r = if cut {
                if same {
                    continue;
                }
                let dst = fs::unique(&dir, &name, "");
                fs::move_to(&p, &dst).map(|_| {
                    self.meta.moved(&p, &dst);
                    dst
                })
            } else {
                let dst = if same || dir.join(&name).exists() {
                    fs::unique(&dir, &name, crate::tr("copy"))
                } else {
                    dir.join(&name)
                };
                fs::copy_rec(&p, &dst).map(|_| dst)
            };
            match r {
                Ok(d) => done.push(d),
                Err(e) => {
                    self.alert(
                        &crate::trf(
                            if cut { "“{name}” couldn't be moved." } else { "“{name}” couldn't be copied." },
                            &[("name", &name)],
                        ),
                        &e.to_string(),
                        false,
                    );
                    break;
                }
            }
        }
        if cut {
            let _ = std::fs::remove_file(clip_file());
            self.meta.save();
        }
        self.reload_keep(done);
    }

    pub(super) fn duplicate(&mut self) {
        let mut done = vec![];
        for p in self.sel_paths() {
            let dir = p.parent().unwrap_or(Path::new("/")).to_path_buf();
            let dst = fs::unique(&dir, &p.file_name().unwrap_or_default().to_string_lossy(), crate::tr("copy"));
            match fs::copy_rec(&p, &dst) {
                Ok(()) => done.push(dst),
                Err(e) => {
                    self.alert("The item couldn't be duplicated.", &e.to_string(), false);
                    break;
                }
            }
        }
        self.reload_keep(done);
    }

    pub(super) fn make_alias(&mut self) {
        let mut done = vec![];
        for p in self.sel_paths() {
            let dir = p.parent().unwrap_or(Path::new("/")).to_path_buf();
            let dst = fs::unique(
                &dir,
                &crate::trf("{name} alias", &[("name", &p.file_name().unwrap_or_default().to_string_lossy())]),
                "",
            );
            if std::os::unix::fs::symlink(&p, &dst).is_ok() {
                done.push(dst);
            }
        }
        self.reload_keep(done);
    }

    pub(super) fn compress(&mut self) {
        let paths = self.sel_paths();
        let Some(first) = paths.first() else { return };
        let dir = first.parent().unwrap_or(Path::new("/")).to_path_buf();
        let name = if paths.len() == 1 {
            format!("{}.zip", first.file_name().unwrap_or_default().to_string_lossy())
        } else {
            format!("{}.zip", crate::tr("Archive"))
        };
        let dst = fs::unique(&dir, &name, "");
        let rels: Vec<String> =
            paths.iter().map(|p| fs::sh_quote(Path::new(p.file_name().unwrap_or_default()))).collect();
        let cmd = if aqua_sys::have("zip") {
            format!("cd {} && zip -qry {} {}", fs::sh_quote(&dir), fs::sh_quote(&dst), rels.join(" "))
        } else {
            format!("cd {} && bsdtar -a -cf {} {}", fs::sh_quote(&dir), fs::sh_quote(&dst), rels.join(" "))
        };
        let ok = std::process::Command::new("sh").arg("-c").arg(&cmd).status().map(|s| s.success()).unwrap_or(false);
        if !ok {
            return self.alert("The items couldn't be compressed.", "Install “zip” (or libarchive's bsdtar).", false);
        }
        self.reload_keep(vec![dst]);
    }

    pub(super) fn put_back(&mut self) {
        for e in self.sel_entries().into_iter().cloned().collect::<Vec<_>>() {
            if let Err(err) = fs::put_back(&e) {
                self.alert(&crate::trf("“{name}” couldn't be put back.", &[("name", &e.name)]), &err, false);
                break;
            }
        }
        self.reload_keep(vec![]);
    }

    pub(super) fn drop_on(&mut self, target: &str, copy: bool) {
        let paths = std::mem::take(&mut self.drag);
        if paths.is_empty() {
            return;
        }
        match Loc::parse(target) {
            Loc::Trash => {
                self.sel = paths.iter().filter_map(|p| self.row_of.get(p).copied()).collect();
                self.trash_selection();
            }
            Loc::Tag(t) => {
                for p in &paths {
                    self.meta.toggle_tag(p, &t, true);
                }
                self.meta.save();
                self.refresh();
            }
            Loc::Dir(d) if d.is_dir() => {
                let mut done = vec![];
                for p in &paths {
                    if p == &d || p.parent() == Some(d.as_path()) && !copy {
                        continue;
                    }
                    let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
                    let dst = if d.join(&name).exists() {
                        fs::unique(&d, &name, if copy { crate::tr("copy") } else { "" })
                    } else {
                        d.join(&name)
                    };
                    use std::os::unix::fs::MetadataExt;
                    let other_volume =
                        std::fs::metadata(p).map(|m| m.dev()).ok() != std::fs::metadata(&d).map(|m| m.dev()).ok();
                    let r = if copy || other_volume {
                        fs::copy_rec(p, &dst)
                    } else {
                        fs::move_to(p, &dst).map(|_| self.meta.moved(p, &dst))
                    };
                    match r {
                        Ok(()) => done.push(dst),
                        Err(e) => {
                            self.alert(
                                &crate::trf("“{name}” couldn't be moved.", &[("name", &name)]),
                                &e.to_string(),
                                false,
                            );
                            break;
                        }
                    }
                }
                self.meta.save();
                self.reload_keep(vec![]);
            }
            _ => {}
        }
    }
}
