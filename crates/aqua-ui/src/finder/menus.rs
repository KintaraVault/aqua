//! Context and toolbar menus, menu actions and tags.
use super::*;

pub(super) fn mi(label: &str, id: &str, sc: &str) -> FMenuItem {
    FMenuItem {
        label: crate::tr(label).into(),
        id: id.into(),
        shortcut: sc.into(),
        enabled: true,
        ..Default::default()
    }
}

pub(super) fn raw(label: String, id: String) -> FMenuItem {
    FMenuItem { label: label.into(), id: id.into(), enabled: true, ..Default::default() }
}

pub(super) fn sub(label: &str, id: &str) -> FMenuItem {
    FMenuItem { sub: true, ..mi(label, id, "") }
}

pub(super) fn sep() -> FMenuItem {
    FMenuItem { separator: true, ..Default::default() }
}

pub(super) fn on(mut m: FMenuItem, enabled: bool) -> FMenuItem {
    m.enabled = enabled;
    m
}

pub(super) fn checked(mut m: FMenuItem, c: bool) -> FMenuItem {
    m.checked = c;
    m
}

pub(super) const SORT_KEYS: [&str; 6] = ["Name", "Kind", "Date Modified", "Size", "Date Created", "Date Last Opened"];

impl App {
    pub(super) fn show_menu(&mut self, items: Vec<FMenuItem>, x: f32, y: f32, tags: bool) {
        let ui_ = self.ui();
        let f = ui_.global::<F>();
        f.set_menu_tags(tags);
        f.set_ctx_place("".into());
        f.set_menu(model(items));
        f.set_menu_x(x);
        f.set_menu_y(y);
        f.set_sub_open(false);
        if crate::native_menus() {
            f.set_menu_open(true);
            self.show_native_menu(x, y);
            if self.pop.is_open() {
                return;
            }
        }
        f.set_menu_native(false);
        ui_.global::<crate::Backdrop>().invoke_capture();
        f.set_menu_open(true);
    }

    fn sort_items(&self) -> Vec<FMenuItem> {
        let free = self.free_folder();
        let mut m: Vec<FMenuItem> = vec![];
        if self.view() == 0 && self.st.group == 0 && matches!(self.loc, Loc::Dir(_)) && self.chooser.is_none() {
            m.push(checked(mi("None", "arrange:0", "⌃⌥⌘0"), free.as_ref().is_some_and(|f| !f.snap)));
            m.push(checked(mi("Snap to Grid", "arrange:1", ""), free.as_ref().is_some_and(|f| f.snap)));
            m.push(sep());
        }
        let mut keys: Vec<FMenuItem> = SORT_KEYS
            .iter()
            .enumerate()
            .map(|(i, n)| {
                checked(
                    mi(n, &format!("sort:{i}"), &format!("⌃⌥⌘{}", i + 1)),
                    free.is_none() && self.st.sort.0 == i as i32,
                )
            })
            .collect();
        m.append(&mut keys);
        m.push(sep());
        m.push(checked(mi("Reverse Order", "sort-reverse", ""), self.st.sort.1));
        m
    }

    fn group_items(&self) -> Vec<FMenuItem> {
        Group::ALL
            .iter()
            .map(|g| checked(mi(g.label(), &format!("group:{}", g.to_i32()), ""), self.st.group == g.to_i32()))
            .collect()
    }

    fn open_with_items(&mut self) -> Vec<FMenuItem> {
        let paths = self.sel_paths();
        let Some(p) = paths.first() else { return vec![] };
        let mime = fs::mime_of(p, p.is_dir());
        let (apps, default) = openwith::for_mime(&mime);
        let mut m = vec![];
        for (i, a) in apps.iter().enumerate() {
            let label =
                if Some(i) == default { crate::trf("{app} (default)", &[("app", &a.name)]) } else { a.name.clone() };
            m.push(raw(label, format!("openwith:{i}")));
            if Some(i) == default && apps.len() > 1 {
                m.push(sep());
            }
        }
        if !apps.is_empty() {
            m.push(sep());
        }
        m.push(mi("Other…", "openwith-other", ""));
        self.apps_menu = apps;
        m
    }

    pub(super) fn tab_context(&mut self, i: usize, x: f32, y: f32) {
        self.menu_paths.clear();
        let many = self.tabs.len() > 1;
        let m = vec![
            raw(crate::tr("Close Tab").into(), format!("close-tab:{i}")),
            on(raw(crate::tr("Close Other Tabs").into(), format!("close-others:{i}")), many),
            on(raw(crate::tr("Move Tab to New Window").into(), format!("detach-tab:{i}")), many),
            sep(),
            mi("Merge All Windows", "merge-windows", ""),
        ];
        self.show_menu(m, x, y, false);
    }

    /// Show the toolbar items chosen in settings.
    pub(super) fn toolbar_apply(&self) {
        let ui_ = self.ui();
        let f = ui_.global::<F>();
        let has = |id: &str| self.st.toolbar.iter().any(|t| t == id);
        f.set_tb_back(has("back"));
        f.set_tb_view(has("view"));
        f.set_tb_group(has("group"));
        f.set_tb_action(has("action"));
        f.set_tb_share(has("share"));
        f.set_tb_tags(has("tags"));
        f.set_tb_newfolder(has("newfolder"));
        f.set_tb_delete(has("delete"));
        f.set_tb_info(has("info"));
        f.set_tb_ql(has("ql"));
        f.set_tb_connect(has("connect"));
        f.set_tb_eject(has("eject"));
        f.set_tb_search(has("search"));
        let labels = [
            "Back/Forward",
            "View",
            "Group",
            "Action",
            "Share",
            "Edit Tags",
            "New Folder",
            "Delete",
            "Get Info",
            "Quick Look",
            "Connect",
            "Eject",
            "Search",
        ];
        let opts: Vec<FOpt> = settings::TOOLBAR_ITEMS
            .iter()
            .zip(labels)
            .map(|(id, l)| FOpt { id: (*id).into(), label: crate::tr(l).into(), on: has(id) })
            .collect();
        f.set_tb_opts(model(opts));
    }

    pub(super) fn toolbar_toggle(&mut self, id: &str) {
        if self.st.toolbar.iter().any(|t| t == id) {
            self.st.toolbar.retain(|t| t != id);
        } else {
            self.st.toolbar = settings::TOOLBAR_ITEMS
                .iter()
                .filter(|t| **t == id || self.st.toolbar.iter().any(|x| x == *t))
                .map(|t| t.to_string())
                .collect();
        }
        self.st.save();
        self.toolbar_apply();
    }

    pub(super) fn toolbar_reset(&mut self) {
        self.st.toolbar = settings::TOOLBAR_DEFAULT.iter().map(|t| t.to_string()).collect();
        self.st.save();
        self.toolbar_apply();
    }

    /// What Share acts on: the menu's items, or the selection.
    pub(super) fn share_paths(&self) -> Vec<PathBuf> {
        if self.menu_paths.is_empty() {
            self.sel_paths()
        } else {
            self.menu_paths.clone()
        }
    }

    pub(super) fn share_items(&self) -> Vec<FMenuItem> {
        let paths = self.share_paths();
        let files = paths.iter().any(|p| p.is_file());
        let has = !paths.is_empty();
        let mut m = vec![on(mi("Mail", "mail", ""), files && aqua_sys::have("xdg-email"))];
        if aqua_sys::have("bluetooth-sendto") {
            m.push(on(mi("Bluetooth", "share-bt", ""), files));
        }
        if aqua_sys::have("kdeconnect-cli") {
            m.push(on(mi("KDE Connect", "share-kde", ""), files));
        }
        m.push(sep());
        m.push(on(mi("Copy", "copy", "⌘C"), has));
        m.push(on(mi("Copy as Pathname", "copy-path", "⌥⌘C"), true));
        m.push(on(mi("Compress", "compress", ""), has && self.loc.dir().is_some()));
        m
    }

    /// Hovering a submenu row: fill and show the submenu at window y.
    pub(super) fn menu_sub(&mut self, id: &str, y: f32) {
        let items = match id {
            "open-with-menu" => self.open_with_items(),
            "sort-menu" => self.sort_items(),
            "cleanup-menu" => {
                SORT_KEYS.iter().enumerate().map(|(i, n)| mi(n, &format!("cleanup-by:{i}"), "")).collect()
            }
            "group-menu" => self.group_items(),
            "quick-menu" => self.quick_items(),
            "share-menu" => self.share_items(),
            _ => return,
        };
        let ui_ = self.ui();
        let f = ui_.global::<F>();
        f.set_sub_menu(model(items));
        if self.pop.is_open() {
            self.show_native_sub(id, y);
            return;
        }
        f.set_sub_id(id.into());
        f.set_sub_y(y);
        f.set_sub_open(true);
    }

    pub(super) fn context(&mut self, idx: i32, x: f32, y: f32) {
        if idx >= 0 && !self.sel.contains(&(idx as usize)) {
            self.select(idx as usize, false, false);
        }
        let mut m = vec![];
        let sel: Vec<Entry> = if idx == -1 { vec![] } else { self.sel_entries().into_iter().cloned().collect() };
        let in_dir = matches!(self.loc, Loc::Dir(_));
        let (_, clip) = read_clip();
        self.menu_paths = sel.iter().map(|e| e.path.clone()).collect();
        if let Loc::Archive(arc, _) = &self.loc {
            if self.chooser.is_none() {
                if !sel.is_empty() {
                    m.push(mi("Open", "open", "⌘O"));
                    m.push(sep());
                    let label = if sel.len() == 1 {
                        crate::trf("Extract “{name}”", &[("name", &sel[0].name)])
                    } else {
                        crate::trf("Extract {n} Items", &[("n", &sel.len())])
                    };
                    m.push(raw(label, "extract-sel".into()));
                }
                m.push(raw(crate::trf("Extract All of “{name}”", &[("name", &name_of(arc))]), "extract-all".into()));
                return self.show_menu(m, x, y, false);
            }
        }
        if sel.is_empty() {
            if idx == -3 {
                if let Some(d) = self.loc.dir() {
                    self.menu_paths = vec![d.to_path_buf()];
                }
            }
            if in_dir && self.chooser.is_none() {
                m.push(mi("New Folder", "new-folder", "⇧⌘N"));
                m.push(sep());
                m.push(mi("Get Info", "info-window", "⌘I"));
                m.push(sep());
                m.push(on(
                    mi(if clip.len() > 1 { "Paste Items" } else { "Paste Item" }, "paste", "⌘V"),
                    !clip.is_empty(),
                ));
                m.push(sep());
            }
            if matches!(self.loc, Loc::Trash) {
                m.push(on(mi("Empty Trash", "empty-trash", "⇧⌘⌫"), !self.all.is_empty()));
                m.push(sep());
            }
            if self.chooser.is_none() {
                m.push(mi("Show View Options", "view-options", "⌘J"));
            }
            if self.free_folder().is_some() {
                m.push(mi("Clean Up", "clean-up", ""));
                m.push(sub("Clean Up By", "cleanup-menu"));
            }
            if self.view() <= 1 {
                m.push(checked(mi("Use Groups", "groups-toggle", "⌃⌘0"), self.st.group != 0));
                m.push(sub("Group By", "group-menu"));
            }
            m.push(sub("Sort By", "sort-menu"));
            m.push(sep());
            m.push(checked(mi("Show Hidden Files", "hidden", "⇧⌘."), self.st.hidden));
            if self.chooser.is_none() {
                m.push(checked(mi("Show Preview", "preview", "⇧⌘P"), self.st.preview));
                if in_dir {
                    m.push(mi("Open in Terminal", "terminal", ""));
                }
            }
            return self.show_menu(m, x, y, false);
        }
        let one = sel.len() == 1;
        let name = if one {
            format!("“{}”", trunc(&sel[0].name, 24))
        } else {
            crate::ntr("{n} Item", "{n} Items", sel.len() as i64)
        };
        m.push(mi("Open", "open", "⌘O"));
        if self.chooser.is_some() {
            return self.show_menu(m, x, y, false);
        }
        if one && !sel[0].is_dir && sel[0].app.is_none() {
            m.push(sub("Open With", "open-with-menu"));
        }
        if sel.iter().all(|e| e.is_dir) {
            m.push(mi("Open in New Tab", "open-tab", ""));
            if one {
                m.push(mi("Open in New Window", "open-window", ""));
            }
        }
        m.push(sep());
        if matches!(self.loc, Loc::Trash) {
            m.push(mi("Put Back", "put-back", "⌘⌫"));
            m.push(mi("Delete Immediately…", "delete-now", "⌥⌘⌫"));
            m.push(sep());
            m.push(mi("Get Info", "info-window", "⌘I"));
            m.push(mi("Empty Trash", "empty-trash", ""));
            return self.show_menu(m, x, y, false);
        }
        let real = sel[0].app.is_none();
        if real {
            m.push(mi("Move to Trash", "trash", "⌘⌫"));
            m.push(sep());
        }
        m.push(mi("Get Info", "info-window", "⌘I"));
        if !one {
            m.push(mi("Get Summary Info", "summary-info", "⌃⌘I"));
        }
        if real {
            if one {
                m.push(mi("Rename", "rename", "↩"));
            } else {
                m.push(raw(crate::trf("Rename {n} Items…", &[("n", &sel.len())]), "rename-multi".into()));
            }
            m.push(raw(crate::trf("Compress {name}", &[("name", &name)]), "compress".into()));
            if sel.iter().all(|e| !e.is_dir && super::archive::is_archive(&e.path)) {
                m.push(mi("Extract Here", "extract-here", ""));
            }
            m.push(mi("Duplicate", "duplicate", "⌘D"));
            m.push(mi("Make Alias", "alias", "⌃⌘A"));
        }
        if one && sel[0].path.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false) {
            m.push(mi("Show Original", "show-original", "⌘R"));
        }
        m.push(raw(crate::trf("Quick Look {name}", &[("name", &name)]), "quicklook".into()));
        m.push(sep());
        m.push(raw(crate::trf("Copy {name}", &[("name", &name)]), "copy".into()));
        if real {
            m.push(mi("Cut", "cut", "⌘X"));
        }
        m.push(sub("Share", "share-menu"));
        m.push(sep());
        m.push(FMenuItem { id: "tags".into(), ..Default::default() });
        m.push(mi("Tags…", "tags-menu", ""));
        if real && in_dir && sel.len() > 1 {
            m.push(sep());
            m.push(raw(
                crate::trf("New Folder with Selection ({n} Items)", &[("n", &sel.len())]),
                "new-folder-sel".into(),
            ));
        }
        if real && !self.quick_targets().is_empty() {
            m.push(sep());
            m.push(sub("Quick Actions", "quick-menu"));
        }
        if one && sel[0].is_dir {
            m.push(sep());
            m.push(mi("Customize Folder…", "customize", ""));
            m.push(mi("Add to Sidebar", "add-sidebar", "⌃⌘T"));
            m.push(mi("Open in Terminal", "terminal", ""));
        }
        if matches!(self.loc, Loc::Recents | Loc::Search(..) | Loc::Smart(_) | Loc::Tag(_) | Loc::Apps) {
            m.push(sep());
            m.push(mi("Show in Enclosing Folder", "reveal", "⌘R"));
        }
        self.show_menu(m, x, y, true);
    }

    /// List view header: choose the visible columns.
    pub(super) fn header_context(&mut self, x: f32, y: f32) {
        let mut m = vec![];
        for (k, n) in
            [(2, "Date Modified"), (4, "Date Created"), (5, "Date Last Opened"), (3, "Size"), (1, "Kind"), (6, "Tags")]
        {
            m.push(checked(mi(n, &format!("col:{k}"), ""), self.st.list_cols.contains(&k)));
        }
        self.menu_paths.clear();
        self.show_menu(m, x, y, false);
    }

    pub(super) fn crumb_context(&mut self, path: &str, x: f32, y: f32) {
        self.menu_paths.clear();
        let m = vec![
            raw(crate::tr("Open").into(), format!("go:{path}")),
            raw(crate::tr("Open in New Tab").into(), format!("tab:{path}")),
            raw(crate::tr("Open in New Window").into(), format!("window:{path}")),
            sep(),
            raw(crate::tr("Get Info").into(), format!("info-of:{path}")),
            raw(crate::tr("Copy as Pathname").into(), format!("copy-path-of:{path}")),
        ];
        self.show_menu(m, x, y - 120.0, false);
    }

    pub(super) fn toolbar_menu(&mut self, which: &str, x: f32, y: f32) {
        let has_sel = !self.sel.is_empty();
        self.menu_paths = self.sel_paths();
        let mut m = vec![];
        match which {
            "view" => {
                let v = self.view();
                for (i, (l, sc)) in [("as Icons", "⌘1"), ("as List", "⌘2"), ("as Columns", "⌘3"), ("as Gallery", "⌘4")]
                    .iter()
                    .enumerate()
                {
                    m.push(checked(mi(l, &format!("view:{i}"), sc), v == i as i32));
                }
            }
            "group" | "sort" => {
                if self.view() <= 1 {
                    m.extend(self.group_items());
                    m.push(sep());
                }
                m.extend(self.sort_items());
            }
            "share" => {
                let _ = has_sel;
                m.extend(self.share_items());
                m.push(sep());
                m.push(mi("Open in Terminal", "terminal", ""));
            }
            "tags" => {
                if !has_sel {
                    for (n, _) in fs::TAGS.iter() {
                        m.push(raw(crate::trf("Show “{tag}” items", &[("tag", &crate::tr(n))]), format!("go:tag:{n}")));
                    }
                    return self.show_menu(m, x - 120.0, y, false);
                }
                m.push(FMenuItem { id: "tags".into(), ..Default::default() });
                m.push(sep());
                let paths = self.sel_paths();
                for (n, _) in fs::TAGS.iter() {
                    let mut it = raw(crate::tr(n).into(), format!("tag:{n}"));
                    it.checked = paths.iter().all(|p| self.meta.tags_of(p).iter().any(|t| t == n));
                    m.push(it);
                }
                return self.show_menu(m, x - 120.0, y, true);
            }
            "history-back" | "history-forward" => {
                let back = which == "history-back";
                let list = if back { &self.back } else { &self.fwd };
                for (k, l) in list.iter().rev().enumerate().take(16) {
                    let n = (k + 1) as i32 * if back { -1 } else { 1 };
                    m.push(raw(self.loc_title(l), format!("hist:{n}")));
                }
                if m.is_empty() {
                    return;
                }
                return self.show_menu(m, x - 120.0, y, false);
            }
            "toolbar" => {
                self.menu_paths.clear();
                return self.show_menu(vec![mi("Customize Toolbar…", "tb-customize", "")], x, y, false);
            }
            "path" => {
                let Some(d) = self.loc.dir().map(|d| d.to_path_buf()) else { return };
                for a in d.ancestors().skip(1) {
                    let label = if a == Path::new("/") { hostname() } else { name_of(a) };
                    m.push(raw(label, format!("go:{}", a.display())));
                }
                if m.is_empty() {
                    return;
                }
                self.menu_paths.clear();
                return self.show_menu(m, x - 120.0, y, false);
            }
            _ => {
                let in_dir = matches!(self.loc, Loc::Dir(_));
                let n = self.sel.len();
                m.push(on(mi("New Folder", "new-folder", "⇧⌘N"), in_dir));
                if n > 1 && in_dir {
                    m.push(raw(
                        crate::trf("New Folder with Selection ({n} Items)", &[("n", &n)]),
                        "new-folder-sel".into(),
                    ));
                }
                m.push(mi("New Tab", "new-tab", "⌘T"));
                m.push(mi("New Finder Window", "new-window", "⌘N"));
                m.push(on(mi("New Smart Folder", "new-smart", "⌥⌘N"), self.chooser.is_none()));
                m.push(sep());
                let editable =
                    in_dir || matches!(self.loc, Loc::Recents | Loc::Search(..) | Loc::Smart(_) | Loc::Tag(_));
                m.push(on(mi("Get Info", "info-window", "⌘I"), true));
                if n > 1 {
                    m.push(on(raw(crate::trf("Rename {n} Items…", &[("n", &n)]), "rename-multi".into()), editable));
                } else {
                    m.push(on(mi("Rename", "rename", "↩"), has_sel && editable));
                }
                m.push(on(mi("Duplicate", "duplicate", "⌘D"), has_sel && editable));
                m.push(on(mi("Quick Look", "quicklook", "Space"), has_sel));
                m.push(on(mi("Move to Trash", "trash", "⌘⌫"), has_sel && editable));
                if n > 1 {
                    m.push(mi("Get Summary Info", "summary-info", "⌃⌘I"));
                }
                if has_sel && !self.quick_targets().is_empty() {
                    m.push(sub("Quick Actions", "quick-menu"));
                }
                m.push(sep());
                let undo = self.hist.can_undo();
                m.push(on(
                    raw(
                        undo.map(|l| crate::trf("Undo {action}", &[("action", &l)]))
                            .unwrap_or_else(|| crate::tr("Undo").into()),
                        "undo".into(),
                    ),
                    self.hist.can_undo().is_some(),
                ));
                m.push(sep());
                m.push(checked(mi("Show Path Bar", "path-bar", "⌥⌘P"), self.st.path_bar));
                m.push(checked(mi("Show Status Bar", "status-bar", "⌘/"), self.st.status_bar));
                m.push(checked(mi("Show Tab Bar", "tab-bar", "⇧⌘T"), self.st.tab_bar));
                m.push(checked(mi("Show Sidebar", "sidebar", "⌥⌘S"), self.st.sidebar));
                m.push(checked(mi("Show Preview", "preview", "⇧⌘P"), self.st.preview));
                m.push(checked(mi("Show Hidden Files", "hidden", "⇧⌘."), self.st.hidden));
                m.push(mi("Show View Options", "view-options", "⌘J"));
                m.push(mi("Customize Toolbar…", "tb-customize", ""));
                if self.free_folder().is_some() {
                    m.push(mi("Clean Up", "clean-up", ""));
                }
                m.push(sep());
                m.push(mi("Go to Folder…", "goto", "⇧⌘G"));
                m.push(mi("Connect to Server…", "connect", "⌘K"));
                m.push(mi("Open in Terminal", "terminal", ""));
                m.push(mi("Settings…", "settings", "⌘,"));
                m.push(sep());
                m.push(mi("Merge All Windows", "merge-windows", ""));
                if self.tabs.len() > 1 {
                    m.push(mi("Move Tab to New Window", "detach-tab", ""));
                }
                if matches!(self.loc, Loc::Trash) {
                    m.push(sep());
                    m.push(mi("Empty Trash", "empty-trash", "⇧⌘⌫"));
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
        let toggle = |s: &mut Self, get: fn(&mut Settings) -> &mut bool| {
            let v = get(&mut s.st);
            *v = !*v;
            s.st.save();
            s.chrome();
        };
        match id {
            "open" => self.open_selection(),
            "open-tab" => {
                let dirs_: Vec<PathBuf> =
                    self.sel_entries().iter().filter(|e| e.is_dir).map(|e| e.path.clone()).collect();
                for (i, d) in dirs_.into_iter().enumerate() {
                    self.new_tab(Loc::Dir(d), i > 0);
                }
            }
            "open-window" => {
                for p in self.sel_paths() {
                    spawn_finder(&p.to_string_lossy());
                }
            }
            "openwith-other" => {
                if let Some(p) = self.sel_paths().first() {
                    let q = fs::sh_quote(p);
                    aqua_apps::launch(&format!("gio open --ask {q} 2>/dev/null || xdg-open {q}"));
                }
            }
            s if s.starts_with("openwith:") => {
                if let Some(app) = s[9..].parse::<usize>().ok().and_then(|i| self.apps_menu.get(i)).cloned() {
                    openwith::launch(&app, &self.sel_paths());
                }
            }
            "new-folder" => self.new_folder(),
            "new-smart" => self.new_smart_folder(),
            s if s.starts_with("search-pick:") => self.search_pick(s[12..].parse().unwrap_or(0)),
            "crit-clear" => {
                while !self.criteria.is_empty() {
                    self.crit_remove(self.criteria.len() as i32 - 1);
                }
            }
            "eject" => self.eject_selection(),
            "merge-windows" => self.merge_windows(),
            "detach-tab" => self.detach_tab(self.tab),
            s if s.starts_with("detach-tab:") => self.detach_tab(s[11..].parse().unwrap_or(usize::MAX)),
            s if s.starts_with("close-tab:") => self.close_tab(s[10..].parse().unwrap_or(self.tab)),
            s if s.starts_with("close-others:") => self.close_other_tabs(s[13..].parse().unwrap_or(usize::MAX)),
            "tb-customize" => {
                self.toolbar_apply();
                f.set_tb_open(true);
            }
            "new-folder-sel" => self.new_folder_with_selection(),
            "new-window" => spawn_finder(&self.start_loc().key()),
            "new-tab" => {
                let l = self.start_loc();
                self.new_tab(l, false)
            }
            "info" => {
                self.st.preview = true;
                f.set_show_preview(true);
                f.set_pv_more(true);
                self.st.save();
            }
            "info-window" => {
                let mut paths = self.sel_paths();
                if paths.is_empty() {
                    if let Some(d) = self.loc.dir() {
                        paths.push(d.to_path_buf());
                    }
                }
                self.open_info(&paths);
            }
            "summary-info" => {
                let paths = self.sel_paths();
                if paths.len() > 1 {
                    self.open_summary(&paths);
                } else {
                    self.action("info-window");
                }
            }
            "rename" => {
                if let Some(&vi) = self.sel.first() {
                    if self.sel.len() > 1 {
                        self.rename_multi();
                    } else {
                        self.start_rename(vi);
                    }
                }
            }
            "rename-multi" => self.rename_multi(),
            "trash" => self.trash_selection(),
            "delete-now" => {
                let p = self.sel_paths();
                self.confirm_delete(p);
            }
            "put-back" => self.put_back(),
            "empty-trash" => self.empty_trash(),
            "compress" => self.compress(),
            "extract-here" => self.extract_here(),
            "extract-sel" => self.extract_selection(),
            "extract-all" => self.extract_all(),
            "duplicate" => self.duplicate(),
            "alias" => self.make_alias(),
            "show-original" => {
                if let Some(p) = self.sel_paths().first() {
                    if let Ok(t) = std::fs::canonicalize(p) {
                        if let Some(parent) = t.parent() {
                            self.go(Loc::Dir(parent.to_path_buf()), true);
                            self.select_path(&t);
                        }
                    }
                }
            }
            "quicklook" => {
                if f.get_ql_open() {
                    self.ql_close();
                } else {
                    self.ql_open();
                }
            }
            "copy" => self.copy(false),
            "cut" => self.copy(true),
            "paste" => self.paste(),
            "move-here" => self.paste_move(),
            "copy-path" => {
                let txt: Vec<String> = self.sel_paths().iter().map(|p| p.to_string_lossy().into_owned()).collect();
                let txt = if txt.is_empty() {
                    self.loc.dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default()
                } else {
                    txt.join("\n")
                };
                ui.invoke_copy_text(txt.into());
            }
            s if s.starts_with("copy-path-of:") => ui.invoke_copy_text(s[13..].into()),
            s if s.starts_with("quick:") => self.quick_action(&s[6..]),
            "share-bt" => {
                let files: Vec<String> =
                    self.share_paths().iter().filter(|p| p.is_file()).map(|p| fs::sh_quote(p)).collect();
                aqua_apps::launch(&format!("bluetooth-sendto {}", files.join(" ")));
            }
            "share-kde" => {
                let cmds: Vec<String> = self
                    .share_paths()
                    .iter()
                    .filter(|p| p.is_file())
                    .map(|p| format!("kdeconnect-cli -d \"$d\" --share {}", fs::sh_quote(p)))
                    .collect();
                aqua_apps::launch(&format!("d=$(kdeconnect-cli -a --id-only | head -n1); {}", cmds.join("; ")));
            }
            "mail" => {
                let att: Vec<String> = self
                    .share_paths()
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
            s if s.starts_with("terminal-at:") => open_terminal(Path::new(&s[12..])),
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
            s if s.starts_with("enclosing:") => {
                let p = PathBuf::from(&s[10..]);
                if let Some(parent) = p.parent() {
                    self.go(Loc::Dir(parent.to_path_buf()), true);
                    self.select_path(&p);
                }
            }
            "preview" => {
                self.st.preview = !self.st.preview;
                f.set_show_preview(self.st.preview);
                self.st.save();
            }
            "path-bar" => toggle(self, |s| &mut s.path_bar),
            "status-bar" => toggle(self, |s| &mut s.status_bar),
            "tab-bar" => toggle(self, |s| &mut s.tab_bar),
            "sidebar" => toggle(self, |s| &mut s.sidebar),
            "view-options" => {
                self.fill_view_options();
                f.set_vo_open(!f.get_vo_open());
            }
            "settings" => {
                self.fill_prefs();
                f.set_prefs_open(true);
            }
            "hidden" => {
                self.st.hidden = !self.st.hidden;
                self.st.save();
                let sel = self.sel_paths();
                self.col_cache.clear();
                self.reload_keep(sel);
            }
            "goto" => {
                f.set_goto_kind(0);
                f.set_goto_text("".into());
                f.set_goto_sugg(model(vec![]));
                f.set_goto_open(true);
            }
            "connect" => self.connect_open(),
            "add-sidebar" => {
                let mut d: Vec<PathBuf> =
                    self.sel_entries().iter().filter(|e| e.is_dir).map(|e| e.path.clone()).collect();
                if d.is_empty() {
                    if let Some(cur) = self.loc.dir().filter(|_| matches!(self.loc, Loc::Dir(_))) {
                        d.push(cur.to_path_buf());
                    }
                }
                self.add_favorites(&d, None);
            }
            "undo" => self.undo(),
            "redo" => self.redo(),
            "select-all" => {
                self.sel = (0..self.shown.len()).collect();
                self.update_selection();
            }
            "sort-reverse" => {
                self.st.sort.1 = !self.st.sort.1;
                self.resort();
            }
            "groups-toggle" => {
                self.st.group = if self.st.group == 0 { 2 } else { 0 };
                self.resort();
            }
            s if s.starts_with("sort:") => {
                self.opt("icon-sort", s[5..].parse::<i32>().unwrap_or(0) + 2);
            }
            s if s.starts_with("arrange:") => self.opt("icon-sort", s[8..].parse().unwrap_or(0)),
            "clean-up" => self.clean_up(None),
            s if s.starts_with("cleanup-by:") => self.clean_up(s[11..].parse().ok()),
            s if s.starts_with("group:") => {
                self.st.group = s[6..].parse().unwrap_or(0);
                self.resort();
            }
            s if s.starts_with("view:") => self.set_view(s[5..].parse().unwrap_or(0)),
            s if s.starts_with("col:") => {
                let k: i32 = s[4..].parse().unwrap_or(0);
                if let Some(i) = self.st.list_cols.iter().position(|&c| c == k) {
                    self.st.list_cols.remove(i);
                } else {
                    self.st.list_cols.push(k);
                }
                self.st.save();
                self.layout_rows();
                self.fill_view_options();
            }
            s if s.starts_with("hist:") => self.history_jump(s[5..].parse().unwrap_or(0)),
            s if s.starts_with("tag:") => self.tag_toggle(&s[4..]),
            s if s.starts_with("go:") => self.go(Loc::parse(&s[3..]), true),
            s if s.starts_with("tab:") => self.new_tab(Loc::parse(&s[4..]), false),
            s if s.starts_with("window:") => spawn_finder(&s[7..]),
            s if s.starts_with("info-of:") => self.open_info(&[PathBuf::from(&s[8..])]),
            s if s.starts_with("remove-place:") => self.remove_place(&s[13..]),
            s if s.starts_with("eject:") => self.eject(PathBuf::from(&s[6..])),
            _ => {}
        }
        self.menu_paths.clear();
    }

    pub(super) fn eject(&mut self, mp: PathBuf) {
        let q = fs::sh_quote(&mp);
        let ok = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!(
                "gio mount -u {q} 2>/dev/null || udisksctl unmount -b \"$(findmnt -no SOURCE {q})\" || umount {q}"
            ))
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            self.message(
                &crate::trf("The disk “{name}” couldn't be ejected.", &[("name", &name_of(&mp))]),
                "One or more programs may be using it.",
            );
        } else if self.loc.dir().map(|d| d.starts_with(&mp)).unwrap_or(false) {
            self.go(Loc::Dir(fs::home()), true);
        }
        self.places();
    }

    pub(super) fn eject_selection(&mut self) {
        let disks: Vec<PathBuf> = self
            .place_list()
            .into_iter()
            .filter(|p| p.eject)
            .map(|p| PathBuf::from(p.path))
            .filter(|mp| self.sel_paths().contains(mp) || self.loc.dir().is_some_and(|d| d.starts_with(mp)))
            .collect();
        for d in disks {
            self.eject(d);
        }
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
        self.hist.push(undo::Op::Tag { paths: paths.clone(), tag: tag.to_string(), on });
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
