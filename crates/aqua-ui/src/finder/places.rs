//! Sidebar places: sections, favorites (GTK bookmarks), volumes, tags.
use super::*;

/// One sidebar row before it becomes an `FPlace`.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Place {
    pub label: String,
    pub path: String,
    pub icon: String,
    pub section: &'static str,
    pub eject: bool,
    pub tag: Option<u32>,
    pub bookmark: bool,
}

pub(super) const SECTIONS: [(&str, &str); 3] =
    [("favorites", "Favorites"), ("locations", "Locations"), ("tags", "Tags")];

/// Rows shown for `places`: a header per non-empty section, items unless it is collapsed.
pub(super) fn side_layout(places: &[Place], collapsed: &[String]) -> Vec<FPlace> {
    let mut v = vec![];
    for (id, title) in SECTIONS {
        let items: Vec<&Place> = places.iter().filter(|p| p.section == id).collect();
        if items.is_empty() {
            continue;
        }
        let closed = collapsed.iter().any(|c| c == id);
        v.push(FPlace { header: crate::tr(title).into(), section: id.into(), collapsed: closed, ..Default::default() });
        if closed {
            continue;
        }
        for p in items {
            v.push(FPlace {
                label: p.label.clone().into(),
                path: p.path.clone().into(),
                icon: p.icon.clone().into(),
                header: SharedString::new(),
                eject: p.eject,
                dot: p.tag.map(rgb).unwrap_or_default(),
                is_tag: p.tag.is_some(),
                collapsed: false,
                section: id.into(),
            });
        }
    }
    v
}

impl App {
    pub(super) fn place_list(&self) -> Vec<Place> {
        let home = fs::home();
        let hidden = |id: &str| self.st.side_hidden.iter().any(|h| h == id);
        let mut v: Vec<Place> = vec![];
        let put = |v: &mut Vec<Place>,
                   label: &str,
                   path: String,
                   icon: &str,
                   section: &'static str,
                   eject: bool,
                   bookmark: bool| {
            v.push(Place { label: label.into(), path, icon: icon.into(), section, eject, tag: None, bookmark });
        };
        if !hidden("recents") {
            put(&mut v, crate::tr("Recents"), "recents:".into(), "recents", "favorites", false, false);
        }
        if let Some(p) = dirs::public_dir().filter(|p| p.is_dir() && p != &home && !hidden("shared")) {
            put(&mut v, crate::tr("Shared"), p.to_string_lossy().into_owned(), "shared", "favorites", false, false);
        }
        if !hidden("apps") {
            put(&mut v, crate::tr("Applications"), "apps:".into(), "apps", "favorites", false, false);
        }
        for (label, d, icon) in [
            ("Desktop", dirs::desktop_dir(), "desktop"),
            ("Documents", dirs::document_dir(), "docs"),
            ("Downloads", dirs::download_dir(), "downloads"),
            ("Pictures", dirs::picture_dir(), "pictures"),
            ("Music", dirs::audio_dir(), "music"),
            ("Movies", dirs::video_dir().or_else(|| Some(home.join("Videos")).filter(|p| p.is_dir())), "movies"),
        ] {
            if hidden(icon) {
                continue;
            }
            let d = d.or_else(|| Some(home.join(label)));
            if let Some(d) = d.filter(|d| d.is_dir() && d != &home) {
                put(&mut v, crate::tr(label), d.to_string_lossy().into_owned(), icon, "favorites", false, false);
            }
        }
        let user = std::env::var("USER").unwrap_or_else(|_| crate::tr("Home").into());
        if !hidden("home") {
            put(&mut v, &user, home.to_string_lossy().into_owned(), "home", "favorites", false, false);
        }
        for (p, label) in fs::read_bookmarks(&fs::bookmarks_file()).into_iter().take(24) {
            if !p.is_dir() {
                continue;
            }
            let ps = p.to_string_lossy().into_owned();
            if v.iter().any(|x| x.path == ps) {
                continue;
            }
            let name = if label.is_empty() { name_of(&p) } else { label };
            put(&mut v, &name, ps, "folder", "favorites", false, true);
        }
        for s in &self.st.side_smart {
            let p = Path::new(s);
            if p.is_file() {
                put(
                    &mut v,
                    &criteria::saved_name(p),
                    Loc::Smart(p.to_path_buf()).key(),
                    "smart",
                    "favorites",
                    false,
                    false,
                );
            }
        }
        if !hidden("computer") {
            put(&mut v, &hostname(), "/".into(), "computer", "locations", false, false);
        }
        if !hidden("disks") {
            let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
            for l in mounts.lines() {
                let mut it = l.split_whitespace();
                let (dev, mp) = (it.next().unwrap_or(""), it.next().unwrap_or("").replace("\\040", " "));
                if (mp.starts_with("/media/") || mp.starts_with("/run/media/") || mp.starts_with("/mnt/"))
                    && dev.starts_with('/')
                {
                    let name =
                        Path::new(&mp).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or(mp.clone());
                    put(&mut v, &name, mp.clone(), "disk", "locations", true, false);
                }
            }
        }
        if !hidden("trash") {
            put(&mut v, crate::tr("Trash"), "trash:".into(), "trash", "locations", false, false);
        }
        for (name, c) in fs::TAGS {
            if self.st.tags_hidden.iter().any(|t| t == name) {
                continue;
            }
            v.push(Place {
                label: crate::tr(name).into(),
                path: format!("tag:{name}"),
                icon: String::new(),
                section: "tags",
                eject: false,
                tag: Some(c),
                bookmark: false,
            });
        }
        v
    }

    pub(super) fn places(&mut self) {
        let rows = side_layout(&self.place_list(), &self.st.collapsed);
        self.side_rows = rows.iter().map(|p| p.path.to_string()).collect();
        self.ui().global::<F>().set_places(model(rows));
    }

    /// Sidebar rows (as in `side_rows`) where folders can be dropped as new favorites.
    pub(super) fn favorites_range(&self) -> Option<std::ops::Range<usize>> {
        let uif = self.ui();
        let f = uif.global::<F>();
        let places = f.get_places();
        let mut start = None;
        let mut end = 0;
        for i in 0..places.row_count() {
            let p = places.row_data(i)?;
            if p.section == "favorites" && !p.path.is_empty() {
                start.get_or_insert(i);
                end = i + 1;
            }
        }
        start.map(|s| s..end)
    }

    pub(super) fn toggle_section(&mut self, id: &str) {
        if let Some(i) = self.st.collapsed.iter().position(|c| c == id) {
            self.st.collapsed.remove(i);
        } else {
            self.st.collapsed.push(id.to_string());
        }
        self.st.save();
        self.places();
    }

    /// Add folders to the favorites, before sidebar row `at` (None: at the end).
    pub(super) fn add_favorites(&mut self, dirs_: &[PathBuf], at: Option<usize>) {
        let file = fs::bookmarks_file();
        let mut bm = fs::read_bookmarks(&file);
        let list = self.place_list();
        let mut pos = bm.len();
        if let (Some(at), Some(range)) = (at, self.favorites_range()) {
            let favs: Vec<&Place> = list.iter().filter(|p| p.section == "favorites").collect();
            let before = at.saturating_sub(range.start).min(favs.len());
            let marks_before = favs[..before].iter().filter(|p| p.bookmark).count();
            pos = marks_before.min(bm.len());
        }
        for d in dirs_.iter().filter(|d| d.is_dir()) {
            if let Some(i) = bm.iter().position(|b| &b.0 == d) {
                bm.remove(i);
                if i < pos {
                    pos -= 1;
                }
            } else if list.iter().any(|p| p.path == d.to_string_lossy()) {
                continue;
            }
            bm.insert(pos, (d.clone(), String::new()));
            pos += 1;
        }
        let _ = fs::write_bookmarks(&file, &bm);
        self.places();
    }

    pub(super) fn remove_place(&mut self, path: &str) {
        let file = fs::bookmarks_file();
        let mut bm = fs::read_bookmarks(&file);
        let n = bm.len();
        bm.retain(|b| b.0.to_string_lossy() != path);
        if bm.len() != n {
            let _ = fs::write_bookmarks(&file, &bm);
        } else if let Some(s) = path.strip_prefix("smart:") {
            self.st.side_smart.retain(|x| x != s);
            self.st.save();
        } else if let Some(p) = self.place_list().into_iter().find(|p| p.path == path) {
            if let Some(name) = path.strip_prefix("tag:") {
                self.st.tags_hidden.push(name.to_string());
            } else {
                let id = if p.eject { "disks".to_string() } else { p.icon.clone() };
                self.st.side_hidden.push(id);
            }
            self.st.save();
        }
        self.places();
    }

    /// Context menu for a sidebar row, at the pointer (window coordinates).
    pub(super) fn place_context(&mut self, path: &str, x: f32, y: f32) {
        self.menu_paths.clear();
        let e = |label: &str, id: String| FMenuItem {
            label: crate::tr(label).into(),
            id: id.into(),
            enabled: true,
            ..Default::default()
        };
        let sep = || FMenuItem { separator: true, ..Default::default() };
        let mut m = vec![
            e("Open", format!("go:{path}")),
            e("Open in New Tab", format!("tab:{path}")),
            e("Open in New Window", format!("window:{path}")),
        ];
        let is_dir = Path::new(path).is_dir();
        if is_dir && path != "/" {
            m.push(e("Show in Enclosing Folder", format!("enclosing:{path}")));
        }
        m.push(sep());
        if is_dir {
            m.push(e("Get Info", format!("info-of:{path}")));
            m.push(e("Open in Terminal", format!("terminal-at:{path}")));
        }
        if path == "trash:" {
            let mut t = e("Empty Trash", "empty-trash".into());
            t.enabled = fs::trash_count() > 0;
            m.push(t);
        }
        if let Some(p) = self.place_list().into_iter().find(|p| p.path == path) {
            if p.eject {
                m.push(e("Eject", format!("eject:{path}")));
            }
            m.push(sep());
            m.push(e("Remove from Sidebar", format!("remove-place:{path}")));
        }
        self.show_menu(m, x, y, false);
        self.ui().global::<F>().set_ctx_place(path.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(path: &str, section: &'static str) -> Place {
        Place {
            label: path.into(),
            path: path.into(),
            icon: "folder".into(),
            section,
            eject: false,
            tag: None,
            bookmark: false,
        }
    }

    #[test]
    fn sections_and_collapsing() {
        let v = vec![place("recents:", "favorites"), place("/a", "favorites"), place("/", "locations")];
        let rows = side_layout(&v, &[]);
        let paths: Vec<&str> = rows.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(paths, ["", "recents:", "/a", "", "/"]);
        assert_eq!(rows[0].section, "favorites");
        assert!(!rows[0].header.is_empty());
        let rows = side_layout(&v, &["favorites".into()]);
        let paths: Vec<&str> = rows.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(paths, ["", "", "/"]);
        assert!(rows[0].collapsed);
    }

    #[test]
    fn sidebar_rows_follow_the_pointer() {
        assert_eq!(super::super::input::side_row(50.0, 40.0, 157.0, 0.0, 10), None);
        assert_eq!(super::super::input::side_row(50.0, 52.0 + 5.0, 157.0, 0.0, 10), Some((0, 5.0)));
        assert_eq!(super::super::input::side_row(50.0, 52.0 + 3.0 * 32.0 + 1.0, 157.0, 0.0, 10), Some((3, 1.0)));
        assert_eq!(super::super::input::side_row(50.0, 52.0 + 1.0, 157.0, -64.0, 10), Some((2, 1.0)));
        assert_eq!(super::super::input::side_row(200.0, 100.0, 157.0, 0.0, 10), None);
        assert_eq!(super::super::input::side_row(50.0, 52.0 + 10.0 * 32.0, 157.0, 0.0, 10), None);
    }
}
