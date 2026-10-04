//! File operations: rename, trash, copy/paste with conflicts, duplicate, alias, compress, drop, undo.
use super::transfer::{Choice, Mode};
use super::*;

impl App {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn ask_user(
        &mut self,
        msg: &str,
        detail: &str,
        ok: &str,
        cancel: bool,
        other: &str,
        check: &str,
        icon: &str,
        pending: Pending,
    ) {
        let ui = self.ui();
        let f = ui.global::<F>();
        self.pending = pending;
        f.set_alert_detail(crate::tr(detail).into());
        f.set_alert_cancel(cancel);
        f.set_alert_ok(crate::tr(ok).into());
        f.set_alert_other(crate::tr(other).into());
        f.set_alert_check(crate::tr(check).into());
        f.set_alert_checked(false);
        f.set_alert_icon(icon.into());
        f.set_alert(crate::tr(msg).into());
    }

    pub(super) fn alert(&mut self, msg: &str, detail: &str, cancel: bool) {
        let p = std::mem::replace(&mut self.pending, Pending::None);
        self.ask_user(msg, detail, "OK", cancel, "", "", "", p);
    }

    pub(super) fn message(&mut self, msg: &str, detail: &str) {
        self.ask_user(msg, detail, "OK", false, "", "", "", Pending::Message);
    }

    /// The alert was answered: 0 cancel, 1 default button, 2 the other button.
    pub(super) fn alert_choose(&mut self, choice: i32) {
        let ui = self.ui();
        let f = ui.global::<F>();
        let checked = f.get_alert_checked();
        f.set_alert("".into());
        match std::mem::replace(&mut self.pending, Pending::None) {
            Pending::EmptyTrash if choice == 1 => self.empty_trash_now(),
            Pending::DeleteNow(paths) if choice == 1 => {
                for p in paths {
                    let _ = fs::remove_rec(&p);
                    let _ = std::fs::remove_file(fs::info_of(&p));
                }
                self.reload_keep(vec![]);
            }
            Pending::Replace(t) if choice == 1 => {
                self.pending = Pending::Replace(t);
                self.accept();
            }
            Pending::Extension { vi, name, keep } => match choice {
                1 => self.do_rename(vi, &name),
                2 => self.do_rename(vi, &keep),
                _ => {}
            },
            Pending::Conflict => {
                let answer = match choice {
                    1 => Some(Choice::Replace),
                    2 => Some(Choice::KeepBoth),
                    _ => None,
                };
                match answer {
                    None => self.ask = None,
                    Some(c) => self.conflict_answer(c, checked),
                }
            }
            _ => {}
        }
    }

    pub(super) fn reload_keep(&mut self, select: Vec<PathBuf>) {
        let q = self.ui().global::<F>().get_query();
        self.load();
        if !q.is_empty() {
            self.ui().global::<F>().set_query(q);
            self.sort_and_filter();
            self.refresh();
        }
        self.select_paths(&select);
    }

    pub(super) fn new_folder(&mut self) {
        let Some(dir) = self.loc.dir().filter(|_| matches!(self.loc, Loc::Dir(_))).map(|d| d.to_path_buf()) else {
            return;
        };
        let p = fs::unique(&dir, crate::tr("untitled folder"), "");
        if let Err(e) = std::fs::create_dir(&p) {
            return self.message("The folder couldn't be created.", &e.to_string());
        }
        self.hist.push(undo::Op::NewFolder { path: p.clone() });
        self.reload_keep(vec![p.clone()]);
        if let Some(&vi) = self.row_of.get(&p) {
            self.start_rename(vi);
        }
    }

    pub(super) fn new_folder_with_selection(&mut self) {
        let Some(dir) = self.loc.dir().filter(|_| matches!(self.loc, Loc::Dir(_))).map(|d| d.to_path_buf()) else {
            return;
        };
        let paths: Vec<PathBuf> = self.sel_paths().into_iter().filter(|p| p.parent() == Some(dir.as_path())).collect();
        if paths.is_empty() {
            return self.new_folder();
        }
        let folder = fs::unique(&dir, crate::tr("New Folder With Items"), "");
        if let Err(e) = std::fs::create_dir(&folder) {
            return self.message("The folder couldn't be created.", &e.to_string());
        }
        self.hist.push(undo::Op::NewFolder { path: folder.clone() });
        let mut pairs = vec![];
        for p in paths {
            let dst = folder.join(p.file_name().unwrap_or_default());
            if fs::move_to(&p, &dst).is_ok() {
                self.meta.moved(&p, &dst);
                self.arr.renamed(&p, &dst);
                pairs.push((p, dst));
            }
        }
        self.meta.save();
        self.hist.push(undo::Op::Move { pairs });
        self.reload_keep(vec![folder.clone()]);
        if let Some(&vi) = self.row_of.get(&folder) {
            self.start_rename(vi);
        }
    }

    pub(super) fn rename(&mut self, vi: usize, name: &str) {
        let ui_ = self.ui();
        let f = ui_.global::<F>();
        f.set_renaming(-1);
        let Some(&i) = self.shown.get(vi) else { return };
        let e = self.all[i].clone();
        let mut name = name.trim().to_string();
        let shown = fs::display_name(&e.name, e.is_dir, self.st.show_ext);
        if name.is_empty() || name == shown {
            return;
        }
        if shown != e.name && !e.is_dir && fs::split_ext(&name).1.is_empty() {
            name.push_str(fs::split_ext(&e.name).1);
        }
        if name.contains('/') {
            return self.message("The name can't contain “/”.", "Please choose another name.");
        }
        let (old_ext, new_ext) = (fs::split_ext(&e.name).1.to_string(), fs::split_ext(&name).1.to_string());
        if self.st.warn_ext && !e.is_dir && old_ext != new_ext && !old_ext.is_empty() {
            let keep = format!("{}{}", fs::split_ext(&name).0, old_ext);
            let msg = if new_ext.is_empty() {
                crate::trf("Are you sure you want to remove the extension “{old}”?", &[("old", &old_ext)])
            } else {
                crate::trf(
                    "Are you sure you want to change the extension from “{old}” to “{new}”?",
                    &[("old", &old_ext), ("new", &new_ext)],
                )
            };
            let ok = if new_ext.is_empty() {
                crate::tr("Remove").to_string()
            } else {
                crate::trf("Use {ext}", &[("ext", &new_ext)])
            };
            let other = crate::trf("Keep {ext}", &[("ext", &old_ext)]);
            return self.ask_user(
                &msg,
                "If you make this change, your document may open in a different application.",
                &ok,
                false,
                &other,
                "",
                "warn",
                Pending::Extension { vi, name, keep },
            );
        }
        self.do_rename(vi, &name);
    }

    fn do_rename(&mut self, vi: usize, name: &str) {
        let Some(&i) = self.shown.get(vi) else { return };
        let e = self.all[i].clone();
        if name == e.name {
            return;
        }
        let dst = e.path.with_file_name(name);
        let case_only = dst.to_string_lossy().to_lowercase() == e.path.to_string_lossy().to_lowercase();
        if dst.symlink_metadata().is_ok() && !case_only {
            return self.message(
                &crate::trf("The name “{name}” is already taken.", &[("name", &name)]),
                "Please choose a different name.",
            );
        }
        match std::fs::rename(&e.path, &dst) {
            Ok(()) => {
                self.meta.moved(&e.path, &dst);
                self.arr.renamed(&e.path, &dst);
                self.meta.save();
                self.hist.push(undo::Op::Rename { from: e.path.clone(), to: dst.clone() });
                self.reload_keep(vec![dst]);
            }
            Err(err) => {
                self.message(&crate::trf("“{name}” couldn't be renamed.", &[("name", &e.name)]), &err.to_string())
            }
        }
    }

    pub(super) fn rename_multi(&mut self) {
        let paths = self.sel_paths();
        if paths.len() < 2
            || !matches!(self.loc, Loc::Dir(_) | Loc::Search(..) | Loc::Smart(_) | Loc::Recents | Loc::Tag(_))
        {
            if let Some(&vi) = self.sel.first() {
                self.start_rename(vi);
            }
            return;
        }
        self.rn_paths = paths;
        let ui_ = self.ui();
        let f = ui_.global::<F>();
        f.set_rn_count(self.rn_paths.len() as i32);
        f.set_rn_a("".into());
        f.set_rn_b("".into());
        self.rn_changed();
        f.set_rn_open(true);
    }

    fn rn_rule(&self) -> rename::Rule {
        let uif = self.ui();
        let f = uif.global::<F>();
        let (a, b) = (f.get_rn_a().to_string(), f.get_rn_b().to_string());
        let after = f.get_rn_where() == 0;
        match f.get_rn_mode() {
            1 => rename::Rule::Add { text: a, after },
            2 => rename::Rule::Format {
                style: match f.get_rn_style() {
                    1 => rename::Style::Counter,
                    2 => rename::Style::Date,
                    _ => rename::Style::Index,
                },
                base: a,
                start: f.get_rn_start().trim().parse().unwrap_or(1),
                after,
            },
            _ => rename::Rule::Replace { find: a, with: b },
        }
    }

    fn rn_names(&self) -> (Vec<String>, Vec<String>) {
        let old: Vec<String> = self.rn_paths.iter().map(|p| name_of(p)).collect();
        let stamp = fs::long_date(fs::now_secs()).replace(':', ".");
        let new = rename::apply(&old, &self.rn_rule(), &|_| stamp.clone());
        (old, new)
    }

    pub(super) fn rn_changed(&mut self) {
        let (_, new) = self.rn_names();
        let ex = new.first().cloned().unwrap_or_default();
        self.ui().global::<F>().set_rn_example(ex.into());
    }

    pub(super) fn rn_apply(&mut self) {
        let (old, new) = self.rn_names();
        let ui_ = self.ui();
        let f = ui_.global::<F>();
        let dirs_: Vec<PathBuf> =
            self.rn_paths.iter().map(|p| p.parent().unwrap_or(Path::new("/")).to_path_buf()).collect();
        let exists = |n: &str| dirs_.iter().any(|d| d.join(n).symlink_metadata().is_ok());
        if let Some(problem) = rename::problem(&old, &new, &exists) {
            return self.message("The items couldn't be renamed.", &problem);
        }
        f.set_rn_open(false);
        let mut pairs = vec![];
        let mut err = None;
        let mut moves: Vec<(PathBuf, PathBuf)> = vec![];
        for (k, p) in self.rn_paths.iter().enumerate() {
            if old[k] != new[k] {
                moves.push((p.clone(), p.with_file_name(&new[k])));
            }
        }
        let plain = rename::order(&old.iter().cloned().zip(new.iter().cloned()).collect::<Vec<_>>()).is_some();
        let mut temps = vec![];
        for (from, to) in &moves {
            let target = if plain {
                to.clone()
            } else {
                let t = from.with_file_name(format!(".aqua-rename-{}-{}", std::process::id(), temps.len()));
                temps.push((t.clone(), to.clone()));
                t
            };
            if let Err(e) = std::fs::rename(from, &target) {
                err = Some(e.to_string());
                break;
            }
            if plain {
                self.meta.moved(from, to);
                self.arr.renamed(from, to);
                pairs.push((from.clone(), to.clone()));
            }
        }
        if !plain {
            for ((t, to), (from, _)) in temps.iter().zip(&moves) {
                if std::fs::rename(t, to).is_ok() {
                    self.meta.moved(from, to);
                    self.arr.renamed(from, to);
                    pairs.push((from.clone(), to.clone()));
                }
            }
        }
        self.meta.save();
        let sel: Vec<PathBuf> = pairs.iter().map(|p| p.1.clone()).collect();
        self.hist.push(undo::Op::Move { pairs });
        self.reload_keep(sel);
        if let Some(e) = err {
            self.message("The items couldn't be renamed.", &e);
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
            return self.message(
                "Applications are removed with your package manager.",
                "Finder can't move installed apps to the Trash.",
            );
        }
        let next = self.sel.iter().max().map(|&m| m + 1 - self.sel.len());
        let mut failed = vec![];
        let mut items = vec![];
        for p in &paths {
            match self.hist_trash(p) {
                Ok(t) if !t.as_os_str().is_empty() => items.push((p.clone(), t)),
                Ok(_) => {}
                Err(e) => failed.push((p.clone(), e)),
            }
        }
        self.hist.push(undo::Op::Trash { items });
        if let Some((p, e)) = failed.first().cloned() {
            if e.contains("another volume") {
                let all: Vec<PathBuf> = failed.iter().map(|x| x.0.clone()).collect();
                self.reload_keep(vec![]);
                return self.confirm_delete(all);
            }
            self.reload_keep(vec![]);
            return self
                .message(&crate::trf("“{name}” couldn't be moved to the Trash.", &[("name", &name_of(&p))]), &e);
        }
        self.reload_keep(vec![]);
        if let Some(n) = next.filter(|_| self.view() == 3 && !self.shown.is_empty()) {
            self.select(n.min(self.shown.len() - 1), false, false);
        }
    }

    fn hist_trash(&self, p: &Path) -> Result<PathBuf, String> {
        match &self.hist.trash {
            Some(t) => fs::trash_in(t, p),
            None => fs::trash(p),
        }
    }

    pub(super) fn confirm_delete(&mut self, paths: Vec<PathBuf>) {
        let what = if paths.len() == 1 {
            format!("“{}”", name_of(&paths[0]))
        } else {
            crate::ntr("these {n} item", "these {n} items", paths.len() as i64)
        };
        self.ask_user(
            &crate::trf("Are you sure you want to delete {what} immediately?", &[("what", &what)]),
            "You can't undo this action.",
            "Delete",
            true,
            "",
            "",
            "warn",
            Pending::DeleteNow(paths),
        );
    }

    pub(super) fn empty_trash(&mut self) {
        if !self.st.warn_trash {
            return self.empty_trash_now();
        }
        self.ask_user(
            "Are you sure you want to permanently erase the items in the Trash?",
            "You can't undo this action.",
            "Empty Trash",
            true,
            "",
            "",
            "trash",
            Pending::EmptyTrash,
        );
    }

    pub(super) fn empty_trash_now(&mut self) {
        fs::empty_trash();
        if matches!(self.loc, Loc::Trash) {
            self.reload_keep(vec![]);
        }
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
            let sel = self.sel_paths();
            self.refresh();
            self.select_paths(&sel);
        }
    }

    pub(super) fn paste(&mut self) {
        let (cut, paths) = read_clip();
        self.paste_with(paths, cut);
    }

    pub(super) fn paste_move(&mut self) {
        let (_, paths) = read_clip();
        self.paste_with(paths, true);
    }

    fn paste_with(&mut self, paths: Vec<PathBuf>, mv: bool) {
        let Some(dir) = self.loc.dir().filter(|_| matches!(self.loc, Loc::Dir(_))).map(|d| d.to_path_buf()) else {
            return;
        };
        if paths.is_empty() {
            return;
        }
        if mv {
            let _ = std::fs::remove_file(clip_file());
        }
        self.start_transfer(paths, dir, if mv { Mode::Move } else { Mode::Copy });
    }

    /// Copy or move `srcs` into `dest`, asking about name conflicts first.
    pub(super) fn start_transfer(&mut self, srcs: Vec<PathBuf>, dest: PathBuf, mode: Mode) {
        if self.xfer.is_some() {
            return self.message("Another copy or move is in progress.", "Wait for it to finish, or stop it.");
        }
        let srcs: Vec<PathBuf> = srcs.into_iter().filter(|s| !dest.starts_with(s)).collect();
        if srcs.is_empty() {
            return;
        }
        let queue = transfer::conflicts(&srcs, &dest, mode);
        self.ask = Some(Ask { srcs, dest, mode, queue, choices: HashMap::new() });
        self.next_conflict();
    }

    fn next_conflict(&mut self) {
        let Some(ask) = &self.ask else { return };
        let Some(first) = ask.queue.first().cloned() else {
            let ask = self.ask.take().unwrap();
            let steps = transfer::plan(&ask.srcs, &ask.dest, ask.mode, &|p| {
                ask.choices.get(p).copied().unwrap_or(Choice::KeepBoth)
            });
            if steps.is_empty() {
                return;
            }
            let trash = Some(self.hist.trash.clone().unwrap_or_else(fs::trash_dir));
            self.xfer = Some(transfer::start(steps, trash));
            return;
        };
        let name = name_of(&first);
        let msg = crate::trf(
            if ask.mode == Mode::Move {
                "An item named “{name}” already exists in this location. Do you want to replace it with the one you're moving?"
            } else {
                "An item named “{name}” already exists in this location. Do you want to replace it with the one you're copying?"
            },
            &[("name", &name)],
        );
        let check = if ask.queue.len() > 1 { "Apply to All" } else { "" };
        self.ask_user(&msg, "", "Replace", true, "Keep Both", check, "", Pending::Conflict);
        let uif = self.ui();
        let f = uif.global::<F>();
        f.set_alert_cancel(true);
    }

    fn conflict_answer(&mut self, c: Choice, all: bool) {
        let Some(ask) = &mut self.ask else { return };
        let take = if all { ask.queue.len() } else { 1 };
        for p in ask.queue.drain(..take.min(ask.queue.len())) {
            ask.choices.insert(p, c);
        }
        self.next_conflict();
    }

    /// Progress and completion of the running copy/move.
    pub(super) fn poll_transfer(&mut self) {
        let ui_ = self.ui();
        let f = ui_.global::<F>();
        let Some(h) = &mut self.xfer else { return };
        let mut done = None;
        while let Ok(m) = h.rx.try_recv() {
            match m {
                transfer::Msg::Progress(p) => h.last = p,
                d @ transfer::Msg::Done { .. } => done = Some(d),
            }
        }
        let Some(transfer::Msg::Done { done, error, cancelled, .. }) = done else {
            if h.started.elapsed().as_millis() > 400 {
                let what = crate::ntr("{n} item", "{n} items", h.count as i64);
                let dest = name_of(&h.dest);
                let title = if h.mode == Mode::Move {
                    crate::trf("Moving {what} to “{dest}”", &[("what", &what), ("dest", &dest)])
                } else {
                    crate::trf("Copying {what} to “{dest}”", &[("what", &what), ("dest", &dest)])
                };
                let p = &h.last;
                let frac =
                    if p.total > 0 { p.bytes as f32 / p.total as f32 } else { p.items as f32 / h.count.max(1) as f32 };
                let mut detail =
                    crate::trf("{done} of {total}", &[("done", &fs::human(p.bytes)), ("total", &fs::human(p.total))]);
                let secs = h.started.elapsed().as_secs_f32();
                if p.bytes > 0 && frac > 0.02 && frac < 1.0 {
                    let left = (secs / frac - secs).ceil() as i64;
                    detail = format!("{detail} — {}", crate::ntr("about {n} second", "about {n} seconds", left));
                }
                f.set_prog_title(title.into());
                f.set_prog_detail(detail.into());
                f.set_prog_frac(frac);
                f.set_prog_on(true);
            }
            return;
        };
        let mode = h.mode;
        let dest = h.dest.clone();
        self.xfer = None;
        f.set_prog_on(false);
        let pairs: Vec<(PathBuf, PathBuf)> = done.iter().map(|s| (s.src.clone(), s.dst.clone())).collect();
        if mode == Mode::Move {
            for (a, b) in &pairs {
                self.meta.moved(a, b);
                self.arr.renamed(a, b);
            }
            self.meta.save();
            self.hist.push(undo::Op::Move { pairs: pairs.clone() });
        } else {
            self.hist.push(undo::Op::Copy { pairs: pairs.clone() });
        }
        let here = self.loc.dir().map(|d| d == dest).unwrap_or(false);
        let sel = if here { pairs.iter().map(|p| p.1.clone()).collect() } else { self.sel_paths() };
        self.col_cache.clear();
        self.reload_keep(sel);
        if let Some(e) = error.filter(|_| !cancelled) {
            self.message("The operation can't be completed.", &e);
        }
    }

    pub(super) fn cancel_transfer(&mut self) {
        if let Some(h) = &self.xfer {
            h.cancel();
        }
    }

    pub(super) fn duplicate(&mut self) {
        let paths = self.sel_paths();
        let Some(dir) = paths.first().and_then(|p| p.parent()).map(|p| p.to_path_buf()) else { return };
        if matches!(self.loc, Loc::Apps | Loc::Trash) {
            return;
        }
        let same: Vec<PathBuf> = paths.into_iter().filter(|p| p.parent() == Some(dir.as_path())).collect();
        self.start_transfer(same, dir, Mode::Copy);
    }

    pub(super) fn make_alias(&mut self) {
        let mut pairs = vec![];
        for p in self.sel_paths() {
            let dir = p.parent().unwrap_or(Path::new("/")).to_path_buf();
            let dst = fs::unique(&dir, &crate::trf("{name} alias", &[("name", &name_of(&p))]), "");
            if std::os::unix::fs::symlink(&p, &dst).is_ok() {
                pairs.push((p, dst));
            }
        }
        let sel = pairs.iter().map(|p| p.1.clone()).collect();
        self.hist.push(undo::Op::Alias { pairs });
        self.reload_keep(sel);
    }

    pub(super) fn compress(&mut self) {
        let paths = self.sel_paths();
        let Some(first) = paths.first() else { return };
        let dir = first.parent().unwrap_or(Path::new("/")).to_path_buf();
        let name =
            if paths.len() == 1 { format!("{}.zip", name_of(first)) } else { format!("{}.zip", crate::tr("Archive")) };
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
            return self.message("The items couldn't be compressed.", "Install “zip” (or libarchive's bsdtar).");
        }
        self.hist.push(undo::Op::Copy { pairs: vec![(first.clone(), dst.clone())] });
        self.reload_keep(vec![dst]);
    }

    /// "Extract Here": unpack the selected archives next to themselves.
    pub(super) fn extract_here(&mut self) {
        let mut made = vec![];
        for p in self.sel_paths() {
            match super::archive::extract_here(&p) {
                Ok(d) => made.push(d),
                Err(e) => {
                    self.message(&crate::trf("“{name}” couldn't be extracted.", &[("name", &name_of(&p))]), &e);
                    break;
                }
            }
        }
        if !made.is_empty() {
            self.reload_keep(made);
        }
    }

    /// Inside an archive: extract the selected items into the archive's folder and show them.
    pub(super) fn extract_selection(&mut self) {
        let Loc::Archive(arc, _) = self.loc.clone() else { return };
        let inner: Vec<String> = self.sel_paths().iter().map(|p| super::archive_inner(&arc, p)).collect();
        let dest = arc.parent().unwrap_or(Path::new("/")).to_path_buf();
        match super::archive::extract_items(&arc, &inner, &dest) {
            Ok(made) => {
                self.go(Loc::Dir(dest), true);
                self.select_paths(&made);
            }
            Err(e) => self.message(&crate::trf("“{name}” couldn't be extracted.", &[("name", &name_of(&arc))]), &e),
        }
    }

    /// Inside an archive: "Extract All" next to the archive and show the result.
    pub(super) fn extract_all(&mut self) {
        let Loc::Archive(arc, _) = self.loc.clone() else { return };
        match super::archive::extract_here(&arc) {
            Ok(d) => {
                self.go(Loc::Dir(arc.parent().unwrap_or(Path::new("/")).to_path_buf()), true);
                self.select_paths(&[d]);
            }
            Err(e) => self.message(&crate::trf("“{name}” couldn't be extracted.", &[("name", &name_of(&arc))]), &e),
        }
    }

    pub(super) fn put_back(&mut self) {
        let mut items = vec![];
        for e in self.sel_entries().into_iter().cloned().collect::<Vec<_>>() {
            match fs::put_back(&e) {
                Ok(dst) => items.push((e.path.clone(), dst)),
                Err(err) => {
                    self.message(&crate::trf("“{name}” couldn't be put back.", &[("name", &e.name)]), &err);
                    break;
                }
            }
        }
        self.hist.push(undo::Op::PutBack { items });
        self.reload_keep(vec![]);
    }

    pub(super) fn drop_on(&mut self, target: &str, copy: bool) {
        let paths = std::mem::take(&mut self.drag);
        if paths.is_empty() {
            return;
        }
        if let Some(at) = target.strip_prefix("insert:") {
            let dirs_: Vec<PathBuf> = paths.into_iter().filter(|p| p.is_dir()).collect();
            return self.add_favorites(&dirs_, at.parse().ok());
        }
        match Loc::parse(target) {
            Loc::Trash => {
                self.sel = paths.iter().filter_map(|p| self.row_of.get(p).copied()).collect();
                if self.sel.len() == paths.len() {
                    self.trash_selection();
                } else {
                    let mut items = vec![];
                    for p in &paths {
                        if let Ok(t) = self.hist_trash(p) {
                            if !t.as_os_str().is_empty() {
                                items.push((p.clone(), t));
                            }
                        }
                    }
                    self.hist.push(undo::Op::Trash { items });
                    self.reload_keep(vec![]);
                }
            }
            Loc::Tag(t) => {
                for p in &paths {
                    self.meta.toggle_tag(p, &t, true);
                }
                self.meta.save();
                self.hist.push(undo::Op::Tag { paths: paths.clone(), tag: t, on: true });
                let sel = self.sel_paths();
                self.refresh();
                self.select_paths(&sel);
            }
            Loc::Dir(d) if d.is_dir() => {
                use std::os::unix::fs::MetadataExt;
                let dev = |p: &Path| std::fs::metadata(p).map(|m| m.dev()).ok();
                let other_volume = paths.first().map(|p| dev(p) != dev(&d)).unwrap_or(false);
                let mode = if copy || other_volume { Mode::Copy } else { Mode::Move };
                let srcs: Vec<PathBuf> = paths
                    .into_iter()
                    .filter(|p| p != &d && !(mode == Mode::Move && p.parent() == Some(d.as_path())))
                    .collect();
                self.start_transfer(srcs, d, mode);
            }
            _ => {}
        }
    }

    fn apply_outcome(&mut self, out: undo::Outcome) {
        for (a, b) in &out.moved {
            self.meta.moved(a, b);
            self.arr.renamed(a, b);
        }
        for (p, t, on) in &out.tags {
            self.meta.toggle_tag(p, t, *on);
        }
        self.meta.save();
        self.col_cache.clear();
        let here: Vec<PathBuf> =
            out.select.iter().filter(|p| p.parent().is_some() && p.parent() == self.loc.dir()).cloned().collect();
        self.reload_keep(here);
        if let Some(e) = out.error {
            self.message("The operation can't be completed.", &e);
        }
    }

    pub(super) fn undo(&mut self) {
        if let Some(out) = self.hist.undo() {
            self.apply_outcome(out);
        }
    }

    pub(super) fn redo(&mut self) {
        if let Some(out) = self.hist.redo() {
            self.apply_outcome(out);
        }
    }
}
