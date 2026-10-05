//! Get Info windows.
use super::*;
use crate::{FInfo, InfoWindow};

pub(super) struct InfoWin {
    pub win: InfoWindow,
    pub path: PathBuf,
    pub apps: Vec<openwith::DesktopApp>,
    pub size_rx: Option<std::sync::mpsc::Receiver<(u64, u64)>>,
    pub group: Vec<PathBuf>,
}

fn writable(p: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(p.as_os_str().as_bytes())
        .map(|c| unsafe { libc::access(c.as_ptr(), libc::W_OK) } == 0)
        .unwrap_or(false)
}

fn size_text(bytes: u64, items: Option<u64>) -> String {
    let b = crate::ntr("{n} byte", "{n} bytes", bytes as i64);
    match items {
        Some(n) => crate::trf(
            "{size} ({bytes}) for {items}",
            &[("size", &fs::human(bytes)), ("bytes", &b), ("items", &crate::ntr("{n} item", "{n} items", n as i64))],
        ),
        None => format!("{} ({b})", fs::human(bytes)),
    }
}

impl App {
    pub(super) fn open_info(&mut self, paths: &[PathBuf]) {
        for p in paths.iter().take(8) {
            if let Some(w) = self.infos.iter().find(|w| &w.path == p) {
                let _ = w.win.show();
                continue;
            }
            let Ok(win) = InfoWindow::new() else { continue };
            crate::apply_theme!(win);
            let mut iw = InfoWin { win, path: p.clone(), apps: vec![], size_rx: None, group: vec![] };
            self.fill_info(&mut iw);
            self.wire_info(&iw);
            iw.win.window().set_size(slint::LogicalSize::new(300.0, 620.0));
            let _ = iw.win.show();
            self.infos.push(iw);
        }
    }

    /// One window describing several items at once.
    pub(super) fn open_summary(&mut self, paths: &[PathBuf]) {
        if paths.len() < 2 {
            return self.open_info(paths);
        }
        let Ok(win) = InfoWindow::new() else { return };
        crate::apply_theme!(win);
        self.summary_seq += 1;
        let key = PathBuf::from(format!("summary:{}", self.summary_seq));
        let mut iw = InfoWin { win, path: key, apps: vec![], size_rx: None, group: paths.to_vec() };
        self.fill_info(&mut iw);
        self.wire_info(&iw);
        iw.win.window().set_size(slint::LogicalSize::new(300.0, 520.0));
        let _ = iw.win.show();
        self.infos.push(iw);
    }

    fn fill_summary(&mut self, iw: &mut InfoWin) {
        let entries: Vec<Entry> = iw.group.iter().filter_map(|p| fs::entry(p)).collect();
        let w = &iw.win;
        let n = entries.len();
        let what = crate::ntr("{n} item", "{n} items", n as i64);
        w.set_summary(true);
        w.set_heading(crate::trf("{name} Info", &[("name", &what)]).into());
        let mut it = entries.first().map(|e| self.item(e, false, &[])).unwrap_or_default();
        it.name = what.clone().into();
        w.set_item(it);
        let kv = |k: &str, v: String| FKV { k: crate::tr(k).into(), v: v.into() };
        let labels: HashSet<&str> = entries.iter().map(|e| e.label.as_str()).collect();
        let kind =
            if labels.len() == 1 { labels.iter().next().unwrap().to_string() } else { crate::tr("Mixed").into() };
        let parents: HashSet<PathBuf> = entries.iter().filter_map(|e| e.path.parent().map(Path::to_path_buf)).collect();
        let mut general = vec![kv("Kind:", kind), kv("Size:", crate::tr("Calculating…").into())];
        if parents.len() == 1 {
            general.push(kv("Where:", parents.iter().next().unwrap().display().to_string()));
        }
        w.set_general(model(general));
        w.set_more(model(vec![]));
        w.set_sub(what.into());
        let (tx, rx) = std::sync::mpsc::channel();
        let paths = iw.group.clone();
        std::thread::spawn(move || {
            let t0 = std::time::Instant::now();
            let mut total = (0, 0);
            for p in &paths {
                let (b, i) = fs::tree_size(p, &|| t0.elapsed().as_secs() > 120);
                total.0 += b;
                total.1 += i.max(1);
            }
            let _ = tx.send(total);
        });
        iw.size_rx = Some(rx);
        w.set_is_dir(entries.iter().any(|e| e.is_dir));
        w.set_locked(!entries.is_empty() && entries.iter().all(|e| e.mode & 0o222 == 0));
        w.set_can_edit(entries.iter().all(|e| writable(&e.path)) && !matches!(self.loc, Loc::Trash));
        let tags: Vec<Vec<String>> = entries.iter().map(|e| self.meta.tags_of(&e.path)).collect();
        w.set_has_tags(fs::TAGS.iter().any(|(t, _)| !tags.is_empty() && tags.iter().all(|v| v.iter().any(|x| x == t))));
        w.set_tags(model(
            fs::TAGS
                .iter()
                .map(|(t, c)| FTagDef {
                    name: (*t).into(),
                    color: rgb(*c),
                    on: !tags.is_empty() && tags.iter().all(|v| v.iter().any(|x| x == t)),
                })
                .collect(),
        ));
        w.set_can_open_with(false);
        let first = entries.first().map(|e| e.mode).unwrap_or(0);
        let same = |who: u32| entries.iter().all(|e| fs::access_level(e.mode, who) == fs::access_level(first, who));
        let lvl = |who: u32| if same(who) { fs::access_level(first, who) } else { -1 };
        w.set_perms(model(vec![
            FInfo { who: "owner".into(), name: crate::tr("Owner").into(), level: lvl(0) },
            FInfo { who: "group".into(), name: crate::tr("Group").into(), level: lvl(1) },
            FInfo { who: "others".into(), name: crate::tr("everyone").into(), level: lvl(2) },
        ]));
    }

    /// Files an Info window acts on.
    fn info_targets(&self, i: usize) -> Vec<PathBuf> {
        let iw = &self.infos[i];
        if iw.group.is_empty() {
            vec![iw.path.clone()]
        } else {
            iw.group.clone()
        }
    }

    fn fill_info(&mut self, iw: &mut InfoWin) {
        if !iw.group.is_empty() {
            return self.fill_summary(iw);
        }
        let Some(e) = fs::entry(&iw.path) else { return };
        let w = &iw.win;
        let it = self.item(&e, false, &[]);
        w.set_heading(crate::trf("{name} Info", &[("name", &e.name)]).into());
        w.set_item(it);
        let kv = |k: &str, v: String| FKV { k: crate::tr(k).into(), v: v.into() };
        let mut general = vec![kv("Kind:", e.label.clone())];
        if e.is_dir {
            general.push(kv("Size:", crate::tr("Calculating…").into()));
            let (tx, rx) = std::sync::mpsc::channel();
            let p = iw.path.clone();
            std::thread::spawn(move || {
                let t0 = std::time::Instant::now();
                let r = fs::tree_size(&p, &|| t0.elapsed().as_secs() > 120);
                let _ = tx.send(r);
            });
            iw.size_rx = Some(rx);
        } else {
            general.push(kv("Size:", size_text(e.size, None)));
        }
        general.push(kv("Where:", e.path.parent().map(|p| p.display().to_string()).unwrap_or_else(|| "/".into())));
        general.push(kv("Created:", fs::long_date(e.ctime)));
        general.push(kv("Modified:", fs::long_date(e.mtime)));
        w.set_general(model(general));
        let mut more = vec![kv("Last opened:", fs::long_date(e.atime))];
        if e.kind == 2 {
            if let Ok((x, y)) = image::image_dimensions(&e.path) {
                more.push(kv("Dimensions:", format!("{x}×{y}")));
            }
        }
        more.push(kv("Content type:", fs::mime_of(&e.path, e.is_dir)));
        if let Ok(t) = std::fs::read_link(&e.path) {
            more.push(kv("Original:", t.display().to_string()));
        }
        if let Some(o) = &e.orig {
            more.push(kv("Original:", o.display().to_string()));
        }
        w.set_more(model(more));
        w.set_sub(
            format!(
                "{}  ·  {}",
                if e.is_dir { "--".into() } else { fs::human(e.size) },
                crate::trf("Modified: {date}", &[("date", &fs::short_date(e.mtime))])
            )
            .into(),
        );
        w.set_name(e.name.clone().into());
        w.set_is_dir(e.is_dir);
        w.set_comment(fs::comment(&e.path).into());
        w.set_locked(e.mode & 0o222 == 0);
        let editable = writable(&e.path) && e.app.is_none() && !matches!(self.loc, Loc::Trash);
        w.set_can_edit(editable);
        let tags = self.meta.tags_of(&e.path);
        w.set_has_tags(fs::TAGS.iter().any(|(n, _)| tags.iter().any(|t| t == n)));
        w.set_tags(model(
            fs::TAGS
                .iter()
                .map(|(n, c)| FTagDef { name: (*n).into(), color: rgb(*c), on: tags.iter().any(|t| t == n) })
                .collect(),
        ));
        if !e.is_dir && e.app.is_none() {
            let (apps, def) = openwith::for_mime(&fs::mime_of(&e.path, false));
            let chosen = self.st.open_with.get(e.path.to_string_lossy().as_ref());
            let idx = chosen.and_then(|id| apps.iter().position(|a| &a.id == id)).or(def).unwrap_or(0);
            w.set_apps(model(apps.iter().map(|a| SharedString::from(a.name.as_str())).collect()));
            w.set_app_idx(idx as i32);
            w.set_can_open_with(!apps.is_empty());
            iw.apps = apps;
        } else {
            w.set_can_open_with(false);
        }
        let (owner, group, uid, _) = fs::owner_group(&e.path);
        let me = unsafe { libc::getuid() };
        let (owner, group) = share_names(&owner, &group, uid == me);
        w.set_perms(model(vec![
            FInfo { who: "owner".into(), name: owner.into(), level: fs::access_level(e.mode, 0) },
            FInfo { who: "group".into(), name: group.into(), level: fs::access_level(e.mode, 1) },
            FInfo { who: "others".into(), name: crate::tr("everyone").into(), level: fs::access_level(e.mode, 2) },
        ]));
    }

    fn wire_info(&self, iw: &InfoWin) {
        let w = &iw.win;
        let me = self.me.clone();
        let path = iw.path.clone();
        macro_rules! on {
            ($cb:ident, |$a:ident, $p:ident $(, $arg:ident)*| $body:expr) => {{
                let me = me.clone();
                let path = path.clone();
                w.$cb(move |$($arg),*| {
                    if let Some(app) = me.upgrade() {
                        if let Ok(mut g) = app.try_borrow_mut() {
                            let $a = &mut *g;
                            let $p = $a.infos.iter().position(|i| i.path == path).unwrap_or(usize::MAX);
                            if $p != usize::MAX {
                                $body
                            }
                        }
                    }
                });
            }};
        }
        on!(on_renamed, |a, i, name| a.info_rename(i, &name));
        on!(on_comment_done, |a, i, c| {
            for p in a.info_targets(i) {
                fs::set_comment(&p, &c)
            }
        });
        on!(on_locked_changed, |a, i, lock| a.info_lock(i, lock));
        on!(on_app_changed, |a, i, k| {
            let p = a.infos[i].path.to_string_lossy().into_owned();
            if let Some(id) = a.infos[i].apps.get(k.max(0) as usize).map(|x| x.id.clone()) {
                a.st.open_with.insert(p, id);
                a.st.save();
                a.infos[i].win.set_app_idx(k);
            }
        });
        on!(on_change_all, |a, i| {
            let k = a.infos[i].win.get_app_idx().max(0) as usize;
            let mime = fs::mime_of(&a.infos[i].path, false);
            if let Some(app) = a.infos[i].apps.get(k).cloned() {
                if openwith::set_default(&app, &mime) {
                    let key = a.infos[i].path.to_string_lossy().into_owned();
                    a.st.open_with.remove(&key);
                    a.st.save();
                    a.message(
                        &crate::trf("All documents of this kind will now open with “{app}”.", &[("app", &app.name)]),
                        "",
                    );
                } else {
                    a.message("The default application couldn't be changed.", "“xdg-mime” is not available.");
                }
            }
        });
        on!(on_perm_changed, |a, i, who, level| a.info_perm(i, who, level));
        on!(on_tag_toggle, |a, i, t| {
            let ps = a.info_targets(i);
            let on = !ps.iter().all(|p| a.meta.tags_of(p).iter().any(|x| x == t.as_str()));
            for p in &ps {
                a.meta.toggle_tag(p, &t, on);
            }
            a.meta.save();
            a.hist.push(undo::Op::Tag { paths: ps.clone(), tag: t.to_string(), on });
            for p in &ps {
                a.update_row(p);
            }
            let mut iw = a.infos.remove(i);
            a.fill_info(&mut iw);
            a.infos.insert(i, iw);
        });
        on!(on_close_clicked, |a, i| {
            let iw = a.infos.remove(i);
            let _ = iw.win.hide();
        });
        let ww = w.as_weak();
        w.on_drag(move || {
            use slint::winit_030::WinitWindowAccessor;
            if let Some(u) = ww.upgrade() {
                u.window().with_winit_window(|win| {
                    let _ = win.drag_window();
                });
            }
        });
        let ww = w.as_weak();
        w.on_minimize(move || {
            if let Some(u) = ww.upgrade() {
                u.window().set_minimized(true);
            }
        });
    }

    fn info_rename(&mut self, i: usize, name: &str) {
        if !self.infos[i].group.is_empty() {
            return;
        }
        let from = self.infos[i].path.clone();
        let name = name.trim();
        if name.is_empty() || name.contains('/') || Some(name) == from.file_name().and_then(|n| n.to_str()) {
            return;
        }
        let to = from.with_file_name(name);
        if to.symlink_metadata().is_ok() {
            return self.message(
                &crate::trf("The name “{name}” is already taken.", &[("name", &name)]),
                "Please choose a different name.",
            );
        }
        match std::fs::rename(&from, &to) {
            Ok(()) => {
                self.meta.moved(&from, &to);
                self.arr.renamed(&from, &to);
                self.meta.save();
                self.hist.push(undo::Op::Rename { from: from.clone(), to: to.clone() });
                let mut iw = self.infos.remove(i);
                iw.path = to.clone();
                self.fill_info(&mut iw);
                self.infos.insert(i, iw);
                if to.parent() == self.loc.dir() {
                    self.reload_keep(vec![to]);
                }
            }
            Err(e) => {
                self.message(&crate::trf("“{name}” couldn't be renamed.", &[("name", &name_of(&from))]), &e.to_string())
            }
        }
    }

    fn set_modes(&mut self, i: usize, f: &dyn Fn(u32, bool) -> u32) {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        for p in self.info_targets(i) {
            let Ok(m) = std::fs::metadata(&p) else { continue };
            let mode = f(m.mode() & 0o7777, m.is_dir());
            if let Err(e) = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)) {
                self.message("The permissions couldn't be changed.", &e.to_string());
                break;
            }
            self.update_row(&p);
        }
        let mut iw = self.infos.remove(i);
        self.fill_info(&mut iw);
        self.infos.insert(i, iw);
    }

    fn info_lock(&mut self, i: usize, lock: bool) {
        self.set_modes(i, &|mode, _| if lock { mode & !0o222 } else { mode | 0o200 });
    }

    fn info_perm(&mut self, i: usize, who: i32, level: i32) {
        self.set_modes(i, &|mode, dir| fs::with_access(mode, who.clamp(0, 2) as u32, level, dir));
    }

    pub(super) fn poll_infos(&mut self) {
        for iw in &mut self.infos {
            let Some(rx) = &iw.size_rx else { continue };
            if let Ok((bytes, items)) = rx.try_recv() {
                iw.size_rx = None;
                let rows: Vec<FKV> = iw
                    .win
                    .get_general()
                    .iter()
                    .map(|mut kv| {
                        if kv.k == crate::tr("Size:") {
                            kv.v = size_text(bytes, Some(items)).into();
                        }
                        kv
                    })
                    .collect();
                iw.win.set_general(model(rows));
                iw.win.set_sub(
                    format!("{}  ·  {}", fs::human(bytes), crate::ntr("{n} item", "{n} items", items as i64)).into(),
                );
            }
        }
    }
}

/// Names of the owner and group rows of "Sharing & Permissions". On most Linux systems a
/// user's files belong to a private group of the same name; label it as the group so the
/// two rows do not read as the same person twice ("valance (Me)", "valance").
pub fn share_names(owner: &str, group: &str, mine: bool) -> (String, String) {
    let o = if mine { crate::trf("{name} (Me)", &[("name", &owner)]) } else { owner.to_string() };
    let g = if group == owner { crate::trf("{name} (group)", &[("name", &group)]) } else { group.to_string() };
    (o, g)
}

#[cfg(test)]
mod share_tests {
    #[test]
    fn private_group_is_not_shown_as_a_second_user() {
        let (o, g) = super::share_names("valance", "valance", true);
        assert_eq!(o, "valance (Me)");
        assert_eq!(g, "valance (group)");
        assert_ne!(o.replace(" (Me)", ""), g);
        assert_eq!(super::share_names("root", "wheel", false), ("root".into(), "wheel".into()));
    }
}
