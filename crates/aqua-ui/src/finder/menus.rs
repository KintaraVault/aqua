//! Context and toolbar menus, menu actions and tags.
use super::*;

impl App {
    pub(super) fn show_menu(&mut self, items: Vec<FMenuItem>, x: f32, y: f32, tags: bool) {
        let ui_ = self.ui();
        let f = ui_.global::<F>();
        f.set_menu_tags(tags);
        f.set_menu(ModelRc::new(VecModel::from(items)));
        f.set_menu_x(x);
        f.set_menu_y(y);
        ui_.global::<crate::Backdrop>().invoke_capture();
        f.set_menu_open(true);
    }

    pub(super) fn context(&mut self, idx: i32, x: f32, y: f32) {
        if idx >= 0 && !self.sel.contains(&(idx as usize)) {
            self.select(idx as usize, false, false);
        }
        let e = |label: &str, id: &str, sc: &str| FMenuItem {
            label: crate::tr(label).into(),
            id: id.into(),
            shortcut: sc.into(),
            enabled: true,
            separator: false,
            checked: false,
        };
        let sep = || FMenuItem { separator: true, ..Default::default() };
        let mut m = vec![];
        let sel: Vec<Entry> = if idx == -1 { vec![] } else { self.sel_entries().into_iter().cloned().collect() };
        let in_dir = self.loc.dir().is_some() && matches!(self.loc, Loc::Dir(_));
        let (_, clip) = read_clip();
        self.menu_paths = sel.iter().map(|e| e.path.clone()).collect();
        if sel.is_empty() {
            if idx == -3 {
                if let Some(d) = self.loc.dir() {
                    self.menu_paths = vec![d.to_path_buf()];
                }
            }
            if in_dir && self.chooser.is_none() {
                m.push(e("New Folder", "new-folder", "⇧⌘N"));
                m.push(sep());
                m.push(e("Get Info", "info", "⌘I"));
                m.push(sep());
                let mut p = e(if clip.len() > 1 { "Paste Items" } else { "Paste Item" }, "paste", "⌘V");
                p.enabled = !clip.is_empty();
                m.push(p);
                m.push(sep());
                m.push(e("Open in Terminal", "terminal", ""));
                m.push(sep());
            }
            if matches!(self.loc, Loc::Trash) {
                let mut t = e("Empty Trash", "empty-trash", "⇧⌘⌫");
                t.enabled = !self.all.is_empty();
                m.push(t);
                m.push(sep());
            }
            for (i, n) in ["Name", "Kind", "Date Modified", "Size"].iter().enumerate() {
                let mut it = e(&crate::trf("Sort by {key}", &[("key", &crate::tr(n))]), &format!("sort:{i}"), "");
                it.checked = self.st.sort.0 == i as i32;
                m.push(it);
            }
            m.push(sep());
            let mut h = e("Show Hidden Files", "hidden", "⇧⌘.");
            h.checked = self.st.hidden;
            m.push(h);
            let mut pvi = e("Show Preview", "preview", "⇧⌘P");
            pvi.checked = self.st.preview;
            m.push(pvi);
            return self.show_menu(m, x, y, false);
        }
        let one = sel.len() == 1;
        let name = if one {
            format!("“{}”", trunc(&sel[0].name, 24))
        } else {
            crate::ntr("{n} Item", "{n} Items", sel.len() as i64)
        };
        m.push(e("Open", "open", "⌘O"));
        if one && sel[0].is_dir && self.chooser.is_none() {
            m.push(e("Open in New Window", "open-window", ""));
        }
        if self.chooser.is_some() {
            return self.show_menu(m, x, y, false);
        }
        if one && !sel[0].is_dir && sel[0].app.is_none() {
            m.push(e("Open With Other Application…", "open-with", ""));
        }
        m.push(sep());
        if matches!(self.loc, Loc::Trash) {
            m.push(e("Put Back", "put-back", "⌘⌫"));
            m.push(e("Delete Immediately…", "delete-now", "⌥⌘⌫"));
            m.push(sep());
            m.push(e("Empty Trash", "empty-trash", ""));
            return self.show_menu(m, x, y, false);
        }
        if sel[0].app.is_none() {
            m.push(e("Move to Trash", "trash", "⌘⌫"));
            m.push(sep());
        }
        m.push(e("Get Info", "info", "⌘I"));
        if sel[0].app.is_none() {
            if one {
                m.push(e("Rename", "rename", "↩"));
            }
            m.push(e(&crate::trf("Compress {name}", &[("name", &name)]), "compress", ""));
            m.push(e("Duplicate", "duplicate", "⌘D"));
            m.push(e("Make Alias", "alias", "⌃⌘A"));
        }
        m.push(e(&crate::trf("Quick Look {name}", &[("name", &name)]), "quicklook", "Space"));
        m.push(sep());
        m.push(e(&crate::trf("Copy {name}", &[("name", &name)]), "copy", "⌘C"));
        if sel[0].app.is_none() {
            m.push(e("Cut", "cut", "⌘X"));
        }
        m.push(e(if one { "Copy as Pathname" } else { "Copy as Pathnames" }, "copy-path", "⌥⌘C"));
        m.push(sep());
        m.push(FMenuItem { id: "tags".into(), ..Default::default() });
        m.push(e("Tags…", "tags-menu", ""));
        if one && sel[0].is_dir {
            m.push(sep());
            m.push(e("Customize Folder…", "customize", ""));
            m.push(e("Open in Terminal", "terminal", ""));
        }
        if matches!(self.loc, Loc::Recents | Loc::Search(..) | Loc::Tag(_) | Loc::Apps) {
            m.push(sep());
            m.push(e("Show in Enclosing Folder", "reveal", "⌘R"));
        }
        self.show_menu(m, x, y, true);
    }

    pub(super) fn toolbar_menu(&mut self, which: &str, x: f32, y: f32) {
        let e = |label: &str, id: &str, sc: &str| FMenuItem {
            label: crate::tr(label).into(),
            id: id.into(),
            shortcut: sc.into(),
            enabled: true,
            separator: false,
            checked: false,
        };
        let sep = || FMenuItem { separator: true, ..Default::default() };
        let has_sel = !self.sel.is_empty();
        self.menu_paths = self.sel_paths();
        let mut m = vec![];
        match which {
            "sort" => {
                for (i, n) in ["Name", "Kind", "Date Modified", "Size"].iter().enumerate() {
                    let mut it = e(n, &format!("sort:{i}"), &format!("⌃⌥⌘{}", i + 1));
                    it.checked = self.st.sort.0 == i as i32;
                    m.push(it);
                }
                m.push(sep());
                let mut r = e("Reverse Order", "sort-reverse", "");
                r.checked = self.st.sort.1;
                m.push(r);
            }
            "share" => {
                let mut c = e("Copy as Pathname", "copy-path", "⌥⌘C");
                c.enabled = has_sel;
                m.push(c);
                let mut z = e("Compress", "compress", "");
                z.enabled = has_sel && self.loc.dir().is_some();
                m.push(z);
                let mut mail = e("Mail", "mail", "");
                mail.enabled = has_sel && aqua_sys::have("xdg-email");
                m.push(mail);
                m.push(sep());
                m.push(e("Open in Terminal", "terminal", ""));
            }
            "tags" => {
                if !has_sel {
                    for (n, _) in fs::TAGS.iter() {
                        m.push(e(
                            &crate::trf("Show “{tag}” items", &[("tag", &crate::tr(n))]),
                            &format!("go:tag:{n}"),
                            "",
                        ));
                    }
                    return self.show_menu(m, x - 120.0, y, false);
                }
                m.push(FMenuItem { id: "tags".into(), ..Default::default() });
                m.push(sep());
                let paths = self.sel_paths();
                for (n, _) in fs::TAGS.iter() {
                    let mut it = e(n, &format!("tag:{n}"), "");
                    it.checked = paths.iter().all(|p| self.meta.tags_of(p).iter().any(|t| t == n));
                    m.push(it);
                }
                return self.show_menu(m, x - 120.0, y, true);
            }
            _ => {
                let in_dir = matches!(self.loc, Loc::Dir(_));
                let mut nf = e("New Folder", "new-folder", "⇧⌘N");
                nf.enabled = in_dir;
                m.push(nf);
                m.push(e("New Finder Window", "new-window", "⌘N"));
                m.push(sep());
                for (label, id, sc) in [
                    ("Get Info", "info", "⌘I"),
                    ("Rename", "rename", "↩"),
                    ("Duplicate", "duplicate", "⌘D"),
                    ("Move to Trash", "trash", "⌘⌫"),
                ] {
                    let mut it = e(label, id, sc);
                    it.enabled =
                        has_sel && (id == "info" || in_dir || matches!(self.loc, Loc::Recents | Loc::Search(..)));
                    m.push(it);
                }
                m.push(sep());
                let mut pv = e("Show Preview", "preview", "⇧⌘P");
                pv.checked = self.st.preview;
                m.push(pv);
                let mut h = e("Show Hidden Files", "hidden", "⇧⌘.");
                h.checked = self.st.hidden;
                m.push(h);
                m.push(e("Go to Folder…", "goto", "⇧⌘G"));
                m.push(e("Open in Terminal", "terminal", ""));
                if matches!(self.loc, Loc::Trash) {
                    m.push(sep());
                    m.push(e("Empty Trash", "empty-trash", "⇧⌘⌫"));
                }
            }
        }
        self.show_menu(m, x - 120.0, y, false);
    }

    pub(super) fn action(&mut self, id: &str) {
        let ui = self.ui();
        let f = ui.global::<F>();
        if !self.menu_paths.is_empty() {
            let paths = self.menu_paths.clone();
            let sel: Vec<usize> = paths.iter().filter_map(|p| self.row_of.get(p).copied()).collect();
            if !sel.is_empty() && sel != self.sel {
                self.sel = sel;
                self.update_selection();
            }
        }
        match id {
            "open" => self.open_selection(),
            "open-window" => {
                for p in self.sel_paths() {
                    spawn_finder(&p.to_string_lossy());
                }
            }
            "open-with" => {
                if let Some(p) = self.sel_paths().first() {
                    let q = fs::sh_quote(p);
                    aqua_apps::launch(&format!("gio open --ask {q} 2>/dev/null || xdg-open {q}"));
                }
            }
            "new-folder" => self.new_folder(),
            "new-window" => spawn_finder(&self.loc.key()),
            "info" => {
                self.st.preview = true;
                f.set_show_preview(true);
                f.set_pv_more(true);
                self.st.save();
            }
            "rename" => {
                if let Some(&vi) = self.sel.first() {
                    f.set_renaming(vi as i32);
                }
            }
            "trash" => self.trash_selection(),
            "delete-now" => {
                let p = self.sel_paths();
                self.confirm_delete(p);
            }
            "put-back" => self.put_back(),
            "empty-trash" => {
                self.pending = Pending::EmptyTrash;
                self.alert(
                    "Are you sure you want to permanently erase the items in the Trash?",
                    "You can't undo this action.",
                    true,
                );
                f.set_alert_ok(crate::tr("Empty Trash").into());
            }
            "compress" => self.compress(),
            "duplicate" => self.duplicate(),
            "alias" => self.make_alias(),
            "quicklook" => {
                self.st.preview = !self.st.preview || f.get_view() == 3;
                f.set_show_preview(self.st.preview);
            }
            "copy" => self.copy(false),
            "cut" => self.copy(true),
            "paste" => self.paste(),
            "copy-path" => {
                let txt: Vec<String> = self.sel_paths().iter().map(|p| p.to_string_lossy().into_owned()).collect();
                let txt = if txt.is_empty() {
                    self.loc.dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default()
                } else {
                    txt.join("\n")
                };
                ui.invoke_copy_text(txt.into());
            }
            "mail" => {
                let att: Vec<String> = self
                    .sel_paths()
                    .iter()
                    .filter(|p| p.is_file())
                    .map(|p| format!("--attach {}", fs::sh_quote(p)))
                    .collect();
                aqua_apps::launch(&format!("xdg-email {}", att.join(" ")));
            }
            "terminal" => {
                let dir = self
                    .sel_entries()
                    .first()
                    .filter(|e| e.is_dir)
                    .map(|e| e.path.clone())
                    .or_else(|| self.loc.dir().map(|d| d.to_path_buf()))
                    .unwrap_or_else(fs::home);
                open_terminal(&dir);
            }
            "tags-menu" => {
                let (x, y) = (f.get_menu_x(), f.get_menu_y());
                self.toolbar_menu("tags", x + 120.0, y);
            }
            "customize" => {
                if let Some(e) = self.sel_entries().first().filter(|e| e.is_dir).map(|e| (*e).clone()) {
                    self.custom = Some(e.path.clone());
                    let it = self.item(&e, false, &[]);
                    f.set_custom(it);
                    f.set_custom_open(true);
                }
            }
            "reveal" => {
                if let Some(p) = self.sel_paths().first().cloned() {
                    if let Some(parent) = p.parent() {
                        self.go(Loc::Dir(parent.to_path_buf()), true);
                        self.select_path(&p);
                    }
                }
            }
            "preview" => {
                self.st.preview = !self.st.preview;
                f.set_show_preview(self.st.preview);
                self.st.save();
            }
            "hidden" => {
                self.st.hidden = !self.st.hidden;
                self.st.save();
                let sel = self.sel_paths();
                self.col_cache.clear();
                self.reload_keep(sel);
            }
            "goto" => f.set_goto_open(true),
            "sort-reverse" => {
                self.st.sort.1 = !self.st.sort.1;
                self.resort();
            }
            s if s.starts_with("sort:") => {
                self.st.sort = (s[5..].parse().unwrap_or(0), false);
                self.resort();
            }
            s if s.starts_with("tag:") => self.tag_toggle(&s[4..]),
            s if s.starts_with("go:") => self.go(Loc::parse(&s[3..]), true),
            s if s.starts_with("eject:") => {
                let mp = PathBuf::from(&s[6..]);
                let q = fs::sh_quote(&mp);
                let ok = std::process::Command::new("sh").arg("-c").arg(format!("gio mount -u {q} 2>/dev/null || udisksctl unmount -b \"$(findmnt -no SOURCE {q})\" || umount {q}")).status().map(|s| s.success()).unwrap_or(false);
                if !ok {
                    self.alert(
                        &crate::trf(
                            "The disk “{name}” couldn't be ejected.",
                            &[("name", &mp.file_name().unwrap_or_default().to_string_lossy())],
                        ),
                        "One or more programs may be using it.",
                        false,
                    );
                } else if self.loc.dir().map(|d| d.starts_with(&mp)).unwrap_or(false) {
                    self.go(Loc::Dir(fs::home()), true);
                }
                self.places();
            }
            s if s.starts_with("place-menu:") => {
                let p = s["place-menu:".len()..].to_string();
                self.menu_paths.clear();
                let e = |label: &str, id: String| FMenuItem {
                    label: crate::tr(label).into(),
                    id: id.into(),
                    shortcut: "".into(),
                    enabled: true,
                    separator: false,
                    checked: false,
                };
                let mut m = vec![e("Open", format!("go:{p}")), e("Open in New Window", format!("window:{p}"))];
                if p == "trash:" {
                    m.push(FMenuItem { separator: true, ..Default::default() });
                    m.push(e("Empty Trash", "empty-trash".into()));
                }
                let (x, y) = (f.get_drag_x(), f.get_drag_y());
                let _ = (x, y);
                self.show_menu(m, 120.0, self.place_y(&p) + 70.0, false);
            }
            s if s.starts_with("window:") => spawn_finder(&s[7..]),
            _ => {}
        }
        self.menu_paths.clear();
    }

    pub(super) fn resort(&mut self) {
        self.st.save();
        let sel = self.sel_paths();
        self.sort_and_filter();
        self.sel = sel.iter().filter_map(|p| self.shown.iter().position(|&i| &self.all[i].path == p)).collect();
        self.refresh();
        let ui_ = self.ui();
        let f = ui_.global::<F>();
        f.set_sort_key(self.st.sort.0);
        f.set_sort_desc(self.st.sort.1);
    }

    pub(super) fn tag_toggle(&mut self, tag: &str) {
        if let Some(c) = &self.chooser {
            if c.save {
                let ui_ = self.ui();
                let f = ui_.global::<F>();
                let defs: Vec<FTagDef> = f
                    .get_tag_defs()
                    .iter()
                    .map(|mut d| {
                        if d.name == tag {
                            d.on = !d.on;
                        }
                        d
                    })
                    .collect();
                f.set_tag_defs(ModelRc::new(VecModel::from(defs)));
                return;
            }
        }
        let paths = if self.menu_paths.is_empty() { self.sel_paths() } else { self.menu_paths.clone() };
        if paths.is_empty() {
            return;
        }
        let on = !paths.iter().all(|p| self.meta.tags_of(p).iter().any(|t| t == tag));
        for p in &paths {
            self.meta.toggle_tag(p, tag, on);
        }
        self.meta.save();
        for p in &paths {
            self.update_row(p);
        }
        self.chrome();
        let ui_ = self.ui();
        let f = ui_.global::<F>();
        if f.get_menu_open() {
            let menu: Vec<FMenuItem> = f
                .get_menu()
                .iter()
                .map(|mut m| {
                    if let Some(t) = m.id.strip_prefix("tag:") {
                        m.checked = paths.iter().all(|p| self.meta.tags_of(p).iter().any(|x| x == t));
                    }
                    m
                })
                .collect();
            f.set_menu(ModelRc::new(VecModel::from(menu)));
        }
        if matches!(self.loc, Loc::Tag(_)) {
            let sel = self.sel_paths();
            self.reload_keep(sel);
        }
    }

    pub(super) fn custom_set(&mut self, color: Option<usize>, symbol: Option<String>) {
        let Some(p) = self.custom.clone() else { return };
        let k = p.to_string_lossy().into_owned();
        let cur = self.meta.folders.get(&k).cloned().unwrap_or((0, String::new()));
        let c = color.map(|i| fs::FOLDER_COLORS.get(i).copied().unwrap_or(0)).unwrap_or(cur.0);
        let s = symbol.unwrap_or(cur.1);
        self.meta.folders.insert(k, (c, s));
        self.meta.save();
        self.update_row(&p);
        if let Some(e) = fs::entry(&p) {
            let it = self.item(&e, false, &[]);
            self.ui().global::<F>().set_custom(it);
        }
        self.chrome();
    }
}
