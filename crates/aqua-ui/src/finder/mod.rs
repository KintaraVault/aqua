//! Finder: Aqua's file manager, and — in chooser mode — the system open/save panel.
//!
//!   aqua-finder [PATH | recents: | apps: | trash: | tag:NAME]
//!   aqua-filechooser [--title T] [--accept-label L] [--save] [--multiple] [--directory]
//!                    [--name FILE] [--folder DIR] [--filter "Images:*.png;*.jpg"]...
//!
//! The chooser prints the chosen absolute paths (one per line) and exits 0, or exits 1 when
//! cancelled (protocol of the xdg-desktop-portal backend in aqua-notify).
pub mod fs;
mod input;
mod menus;
mod ops;
mod places;
mod run;
mod search;
mod thumbs;

pub use run::run;
use run::{finish, hostname, open_path, open_terminal, record_recent, spawn_finder, trunc};

use crate::{FColumn, FItem, FMenuItem, FPlace, FTagDef, FinderWindow, F, FKV};
use fs::{Entry, Loc};
use slint::{Color, ComponentHandle, Image, Model, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[derive(Default, Clone)]
pub struct Chooser {
    pub save: bool,
    pub multiple: bool,
    pub directory: bool,
    pub title: String,
    pub accept: String,
    pub name: String,
    pub filters: Vec<(String, Vec<String>)>,
}

impl Chooser {
    /// Parse the aqua-filechooser command line; returns the start folder too.
    pub fn from_args(args: impl Iterator<Item = String>) -> (Self, Option<PathBuf>, bool) {
        let mut c = Chooser::default();
        let (mut folder, mut hidden) = (None, false);
        let mut args = args.peekable();
        while let Some(a) = args.next() {
            match a.as_str() {
                "--title" => c.title = args.next().unwrap_or_default(),
                "--accept-label" => c.accept = args.next().unwrap_or_default().replace('_', ""),
                "--save" => c.save = true,
                "--multiple" => c.multiple = true,
                "--directory" => c.directory = true,
                "--name" => c.name = args.next().unwrap_or_default(),
                "--folder" => folder = args.next().map(PathBuf::from),
                "--hidden" => hidden = true,
                "--filter" => {
                    let f = args.next().unwrap_or_default();
                    let (label, pats) = f.split_once(':').unwrap_or((&f, &f));
                    c.filters.push((
                        label.to_string(),
                        pats.split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
                    ));
                }
                _ => {}
            }
        }
        (c, folder, hidden)
    }
}

fn glob_match(pat: &str, name: &str, kind: i32) -> bool {
    let (p, n) = (pat.to_lowercase(), name.to_lowercase());
    if p == "*" || p == "*.*" {
        return true;
    }
    if let Some(mime) = p.strip_suffix("/*") {
        return kind
            == match mime {
                "image" => 2,
                "audio" => 3,
                "video" => 4,
                "text" => 6,
                _ => -1,
            };
    }
    if let Some(ext) = p.strip_prefix("*.") {
        return n.ends_with(&format!(".{ext}"));
    }
    if p.contains('*') {
        let parts: Vec<&str> = p.split('*').collect();
        let mut rest = n.as_str();
        for (i, part) in parts.iter().enumerate() {
            if part.is_empty() {
                continue;
            }
            match rest.find(part) {
                Some(pos) if i > 0 || pos == 0 => rest = &rest[pos + part.len()..],
                _ => return false,
            }
        }
        return parts.last().map(|l| l.is_empty() || n.ends_with(l)).unwrap_or(true);
    }
    p == n
}

fn rgb(c: u32) -> Color {
    Color::from_rgb_u8((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

fn tag_color(name: &str) -> Color {
    rgb(fs::TAGS.iter().find(|t| t.0 == name).map(|t| t.1).unwrap_or(0x98989d))
}

struct Settings {
    view: i32,
    preview: bool,
    sort: (i32, bool),
    hidden: bool,
    size: (f32, f32),
    chooser_dir: Option<PathBuf>,
}

fn settings_path() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| fs::home().join(".config")).join("aqua/finder.json")
}

impl Settings {
    fn load() -> Self {
        let v: serde_json::Value = std::fs::read_to_string(settings_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let g = |k: &str| v.get(k);
        Self {
            view: g("view").and_then(|x| x.as_i64()).unwrap_or(0) as i32,
            preview: g("preview").and_then(|x| x.as_bool()).unwrap_or(false),
            sort: (
                g("sort").and_then(|x| x.as_i64()).unwrap_or(0) as i32,
                g("desc").and_then(|x| x.as_bool()).unwrap_or(false),
            ),
            hidden: g("hidden").and_then(|x| x.as_bool()).unwrap_or(false),
            size: (
                g("w").and_then(|x| x.as_f64()).unwrap_or(1000.0) as f32,
                g("h").and_then(|x| x.as_f64()).unwrap_or(600.0) as f32,
            ),
            chooser_dir: g("chooser_dir").and_then(|x| x.as_str()).map(PathBuf::from),
        }
    }
    fn save(&self) {
        let v = serde_json::json!({
            "view": self.view, "preview": self.preview, "sort": self.sort.0, "desc": self.sort.1, "hidden": self.hidden,
            "w": self.size.0, "h": self.size.1, "chooser_dir": self.chooser_dir.as_ref().map(|p| p.to_string_lossy().into_owned()),
        });
        let p = settings_path();
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let _ = std::fs::write(p, serde_json::to_string_pretty(&v).unwrap_or_default());
    }
}

/// What a confirmation alert does when accepted.
enum Pending {
    None,
    EmptyTrash,
    DeleteNow(Vec<PathBuf>),
    Replace(PathBuf),
}

struct App {
    ui: slint::Weak<FinderWindow>,
    loc: Loc,
    back: Vec<Loc>,
    fwd: Vec<Loc>,
    all: Vec<Entry>,
    shown: Vec<usize>,
    sel: Vec<usize>,
    anchor: Option<usize>,
    focus: Option<usize>,
    st: Settings,
    chooser: Option<Chooser>,
    filter: usize,
    meta: fs::Meta,
    items: Rc<VecModel<FItem>>,
    row_of: HashMap<PathBuf, usize>,
    thumbs: HashMap<PathBuf, (Option<Image>, SharedString)>,
    requested: std::collections::HashSet<PathBuf>,
    worker: thumbs::Worker,
    places: Vec<(f32, f32, String)>,
    dir_stamp: Option<std::time::SystemTime>,
    search_gen: u64,
    search_rx: Option<std::sync::mpsc::Receiver<(u64, Vec<Entry>, bool)>>,
    pending: Pending,
    drag: Vec<PathBuf>,
    /// the current drag left the window and was handed to the compositor
    ext_drag: Option<std::time::Instant>,
    menu_paths: Vec<PathBuf>,
    custom: Option<PathBuf>,
    type_buf: (String, std::time::Instant),
    col_cache: HashMap<PathBuf, (Option<std::time::SystemTime>, Vec<Entry>)>,
}

fn clip_file() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(|| "/tmp".into()).join("aqua/finder-clipboard")
}

fn read_clip() -> (bool, Vec<PathBuf>) {
    let s = std::fs::read_to_string(clip_file()).unwrap_or_default();
    let mut lines = s.lines();
    let cut = lines.next() == Some("cut");
    (cut, lines.map(PathBuf::from).filter(|p| p.exists()).collect())
}

impl App {
    fn ui(&self) -> FinderWindow {
        self.ui.upgrade().expect("finder window")
    }

    fn load(&mut self) {
        self.worker.clear();
        self.requested.clear();
        self.search_rx = None;
        let ui = self.ui();
        let f = ui.global::<F>();
        f.set_search_busy(false);
        let mut empty = String::new();
        self.all = match &self.loc {
            Loc::Dir(p) => match fs::list_dir(p) {
                Ok(v) => v,
                Err(e) => {
                    empty = format!(
                        "“{}” can't be opened: {e}",
                        p.file_name()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_else(|| p.display().to_string())
                    );
                    vec![]
                }
            },
            Loc::Recents => fs::list_recents(),
            Loc::Apps => fs::list_apps(),
            Loc::Trash => fs::list_trash(),
            Loc::Tag(t) => {
                let t = t.clone();
                let mut v: Vec<Entry> = self
                    .meta
                    .tags
                    .iter()
                    .filter(|(_, tags)| tags.contains(&t))
                    .filter_map(|(p, _)| fs::entry(Path::new(p)))
                    .collect();
                v.sort_by(|a, b| fs::natural(&a.name, &b.name));
                v
            }
            Loc::Search(root, q) => {
                self.start_search(root.clone(), q.clone());
                vec![]
            }
        };
        if empty.is_empty() {
            empty = match &self.loc {
                Loc::Trash => crate::tr("Trash is empty").into(),
                Loc::Recents => crate::tr("No recent documents").into(),
                Loc::Tag(t) => crate::trf("No items tagged “{tag}”", &[("tag", &crate::tr(t))]),
                Loc::Search(..) => crate::tr("Searching…").into(),
                _ => String::new(),
            };
        }
        f.set_empty_text(empty.into());
        self.dir_stamp = self.loc.dir().and_then(|p| std::fs::metadata(p).ok()).and_then(|m| m.modified().ok());
        self.sel.clear();
        self.anchor = None;
        self.focus = None;
        self.sort_and_filter();
        self.refresh();
    }

    fn sort_and_filter(&mut self) {
        let (key, desc) = self.st.sort;
        let virt_recents = matches!(self.loc, Loc::Recents);
        let mut idx: Vec<usize> = (0..self.all.len())
            .filter(|&i| {
                let e = &self.all[i];
                self.st.hidden || !e.name.starts_with('.') || matches!(self.loc, Loc::Trash)
            })
            .collect();
        let all = &self.all;
        if !(virt_recents && key == 0) {
            idx.sort_by(|&a, &b| {
                let (x, y) = (&all[a], &all[b]);
                let o = match key {
                    1 => x.label.cmp(&y.label).then(fs::natural(&x.name, &y.name)),
                    2 => y.mtime.cmp(&x.mtime),
                    3 => y.size.cmp(&x.size).then(fs::natural(&x.name, &y.name)),
                    _ => fs::natural(&x.name, &y.name),
                };
                if desc {
                    o.reverse()
                } else {
                    o
                }
            });
        }
        let q = self.ui().global::<F>().get_query().to_lowercase();
        if !q.is_empty() && !matches!(self.loc, Loc::Search(..)) {
            idx.retain(|&i| self.all[i].name.to_lowercase().contains(&q));
        }
        self.shown = idx;
    }

    fn dimmed(&self, e: &Entry) -> bool {
        let Some(c) = &self.chooser else { return false };
        if e.is_dir {
            return false;
        }
        if c.directory || c.save {
            return true;
        }
        match c.filters.get(self.filter) {
            Some((_, pats)) if !pats.is_empty() => !pats.iter().any(|p| glob_match(p, &e.name, e.kind)),
            _ => false,
        }
    }

    fn item(&self, e: &Entry, selected: bool, cut: &[PathBuf]) -> FItem {
        let (thumb, snippet) = self.thumbs.get(&e.path).cloned().unwrap_or((None, SharedString::new()));
        let key = e.path.to_string_lossy();
        let look = if e.is_dir { self.meta.folders.get(&*key).cloned() } else { None };
        let (tint, has_tint, symbol) = match look {
            Some((c, s)) if c != 0 => (rgb(c), true, s),
            Some((_, s)) => (Color::default(), false, s),
            None => (Color::default(), false, String::new()),
        };
        let tags: Vec<Color> = self.meta.tags_of(&e.path).iter().map(|t| tag_color(t)).collect();
        let size = if e.is_dir || e.kind == 5 { "--".to_string() } else { fs::human(e.size) };
        FItem {
            name: e.name.clone().into(),
            path: key.to_string().into(),
            is_dir: e.is_dir,
            kind: e.kind,
            ext: e.ext.clone().into(),
            size: size.into(),
            modified: fs::short_date(e.mtime).into(),
            kind_label: e.label.clone().into(),
            selected,
            has_thumb: thumb.is_some(),
            thumb: thumb.unwrap_or_default(),
            snippet,
            tint,
            has_tint,
            symbol: symbol.into(),
            tags: ModelRc::new(VecModel::from(tags)),
            dim: self.dimmed(e),
            cut: cut.contains(&e.path),
        }
    }

    fn want_preview(&mut self, e: &Entry) {
        if self.thumbs.contains_key(&e.path) || self.requested.contains(&e.path) {
            return;
        }
        let job = if let Some((_, id, icon)) = &e.app {
            Some(thumbs::Job::App(e.path.clone(), id.clone(), e.name.clone(), icon.clone()))
        } else if !e.is_dir && matches!(e.kind, 1 | 2 | 4 | 6 | 8) && e.size > 0 {
            Some(thumbs::Job::File(e.path.clone(), e.kind, e.mtime))
        } else {
            None
        };
        if let Some(j) = job {
            self.requested.insert(e.path.clone());
            self.worker.push(j);
        }
    }

    /// Rebuild the item model (after listing / sorting / filtering).
    fn refresh(&mut self) {
        let (cut_on, clip) = read_clip();
        let cut: Vec<PathBuf> = if cut_on { clip } else { vec![] };
        let v: Vec<FItem> = self
            .shown
            .iter()
            .enumerate()
            .map(|(vi, &i)| self.item(&self.all[i], self.sel.contains(&vi), &cut))
            .collect();
        self.row_of = self.shown.iter().enumerate().map(|(vi, &i)| (self.all[i].path.clone(), vi)).collect();
        self.items.set_vec(v);
        let todo: Vec<Entry> = self.shown.iter().take(800).rev().map(|&i| self.all[i].clone()).collect();
        for e in todo {
            self.want_preview(&e);
        }
        self.chrome();
    }

    fn sel_entries(&self) -> Vec<&Entry> {
        self.sel.iter().filter_map(|&v| self.shown.get(v).map(|&i| &self.all[i])).collect()
    }

    fn sel_paths(&self) -> Vec<PathBuf> {
        self.sel_entries().into_iter().map(|e| e.path.clone()).collect()
    }

    /// Selection changed: patch the affected rows only.
    fn update_selection(&mut self) {
        for vi in 0..self.items.row_count() {
            let want = self.sel.contains(&vi);
            if let Some(mut it) = self.items.row_data(vi) {
                if it.selected != want {
                    it.selected = want;
                    self.items.set_row_data(vi, it);
                }
            }
        }
        self.chrome();
    }

    fn update_row(&mut self, path: &Path) {
        let Some(&vi) = self.row_of.get(path) else { return };
        let Some(&i) = self.shown.get(vi) else { return };
        let (cut_on, clip) = read_clip();
        let cut: Vec<PathBuf> = if cut_on { clip } else { vec![] };
        let it = self.item(&self.all[i], self.sel.contains(&vi), &cut);
        self.items.set_row_data(vi, it);
    }

    fn title(&self) -> String {
        match &self.loc {
            Loc::Dir(p) if p == Path::new("/") => hostname(),
            Loc::Dir(p) => {
                p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
            }
            Loc::Recents => crate::tr("Recents").into(),
            Loc::Apps => crate::tr("Applications").into(),
            Loc::Trash => crate::tr("Trash").into(),
            Loc::Tag(t) => crate::tr(t).to_string(),
            Loc::Search(p, q) => crate::trf(
                "Searching “{place}” for “{query}”",
                &[
                    (
                        "place",
                        &p.file_name()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_else(|| crate::tr("Computer").into()),
                    ),
                    ("query", q),
                ],
            ),
        }
    }

    /// Toolbar, preview pane, chooser button: everything derived from the selection.
    fn chrome(&mut self) {
        let ui = self.ui();
        let f = ui.global::<F>();
        f.set_title(self.title().into());
        f.set_can_back(!self.back.is_empty());
        f.set_can_forward(!self.fwd.is_empty());
        f.set_cur_place(self.loc.key().into());
        f.set_trash_view(matches!(self.loc, Loc::Trash));
        f.set_focus_idx(self.focus.map(|x| x as i32).unwrap_or(-1));
        let sel = self.sel_entries();
        f.set_pv_count(sel.len() as i32);
        if sel.len() == 1 {
            let e = sel[0].clone();
            let (cut_on, clip) = read_clip();
            let it = self.item(&e, true, if cut_on { &clip } else { &[] });
            let mut info: Vec<FKV> = vec![];
            let kv = |k: &str, v: String| FKV { k: crate::tr(k).into(), v: v.into() };
            if e.app.is_none() {
                info.push(kv("Created", fs::long_date(e.ctime)));
                info.push(kv("Modified", fs::long_date(e.mtime)));
                info.push(kv("Last opened", fs::long_date(e.atime)));
            }
            if e.kind == 2 {
                if let Ok((w, h)) = image::image_dimensions(&e.path) {
                    info.insert(0, kv("Dimensions", format!("{w}×{h}")));
                }
            }
            if e.is_dir {
                let n = std::fs::read_dir(&e.path).map(|r| r.count()).unwrap_or(0);
                info.insert(0, kv("Contains", crate::ntr("{n} item", "{n} items", n as i64)));
            }
            if let Some((cmd, id, _)) = &e.app {
                info.push(kv("Identifier", id.clone()));
                info.push(kv("Command", cmd.clone()));
            }
            if let Some(o) = &e.orig {
                info.push(kv("Original", o.display().to_string()));
            }
            info.push(kv("Where", e.path.parent().map(|p| p.display().to_string()).unwrap_or_default()));
            info.push(kv("Permissions", fs::perm_string(e.mode)));
            info.push(kv(
                "Size",
                if e.is_dir {
                    "--".into()
                } else {
                    format!("{} ({})", fs::human(e.size), crate::ntr("{n} byte", "{n} bytes", e.size as i64))
                },
            ));
            let mut pv = it;
            if e.is_dir {
                let n = std::fs::read_dir(&e.path).map(|r| r.count()).unwrap_or(0);
                pv.size = crate::ntr("{n} item", "{n} items", n as i64).into();
            }
            f.set_pv(pv);
            f.set_pv_info(ModelRc::new(VecModel::from(info)));
            f.set_pv_any(true);
        } else {
            f.set_pv_any(false);
        }
        let n = self.shown.len();
        f.set_status(
            if sel.is_empty() {
                crate::ntr("{n} item", "{n} items", n as i64)
            } else {
                crate::trf("{sel} of {n} selected", &[("sel", &sel.len()), ("n", &n)])
            }
            .into(),
        );
        if let Some(c) = &self.chooser {
            let ok = if c.save {
                !f.get_save_name().trim().is_empty() && self.loc.dir().is_some()
            } else if c.directory {
                self.loc.dir().is_some() || sel.iter().any(|e| e.is_dir)
            } else {
                sel.iter().any(|e| !e.is_dir && !self.dimmed(e)) || (sel.len() == 1 && sel[0].is_dir)
            };
            f.set_can_accept(ok);
        }
        let paths = self.sel_paths();
        let defs: Vec<FTagDef> = fs::TAGS
            .iter()
            .map(|(name, c)| FTagDef {
                name: (*name).into(),
                color: rgb(*c),
                on: !paths.is_empty() && paths.iter().all(|p| self.meta.tags_of(p).iter().any(|t| t == name)),
            })
            .collect();
        if self.chooser.as_ref().map(|c| c.save).unwrap_or(false) {
        } else {
            f.set_tag_defs(ModelRc::new(VecModel::from(defs)));
        }
        if f.get_view() == 2 {
            self.columns();
        }
    }

    fn columns(&mut self) {
        let ui = self.ui();
        let f = ui.global::<F>();
        let (cut_on, clip) = read_clip();
        let cut: Vec<PathBuf> = if cut_on { clip } else { vec![] };
        let last = FColumn {
            path: self.loc.key().into(),
            items: ModelRc::from(self.items.clone() as Rc<dyn Model<Data = FItem>>),
            active: true,
        };
        let Some(dir) = self.loc.dir().map(|p| p.to_path_buf()).filter(|_| matches!(self.loc, Loc::Dir(_))) else {
            f.set_columns(ModelRc::new(VecModel::from(vec![last])));
            return;
        };
        let base = self.place_base(&dir);
        let mut chain: Vec<PathBuf> =
            dir.ancestors().take_while(|a| a.starts_with(&base)).map(|p| p.to_path_buf()).collect();
        chain.reverse();
        let skip = chain.len().saturating_sub(5);
        let mut cols = vec![];
        for w in chain.windows(2).skip(skip) {
            let (parent, child) = (&w[0], &w[1]);
            let stamp = std::fs::metadata(parent).ok().and_then(|m| m.modified().ok());
            let fresh = self.col_cache.get(parent).map(|(s, _)| *s == stamp).unwrap_or(false);
            if !fresh {
                let mut v = fs::list_dir(parent).unwrap_or_default();
                v.retain(|e| self.st.hidden || !e.name.starts_with('.'));
                v.sort_by(|a, b| fs::natural(&a.name, &b.name));
                self.col_cache.insert(parent.clone(), (stamp, v));
            }
            let entries = self.col_cache.get(parent).map(|c| c.1.clone()).unwrap_or_default();
            let items: Vec<FItem> = entries.iter().map(|e| self.item(e, &e.path == child, &cut)).collect();
            cols.push(FColumn {
                path: parent.to_string_lossy().to_string().into(),
                items: ModelRc::new(VecModel::from(items)),
                active: false,
            });
        }
        cols.push(last);
        f.set_columns(ModelRc::new(VecModel::from(cols)));
    }

    fn place_base(&self, dir: &Path) -> PathBuf {
        let home = fs::home();
        if dir.starts_with(&home) {
            home
        } else {
            PathBuf::from("/")
        }
    }

    fn go(&mut self, loc: Loc, history: bool) {
        if let Loc::Dir(p) = &loc {
            if !p.is_dir() {
                if p.exists() {
                    let file = p.clone();
                    if let Some(parent) = file.parent() {
                        self.go(Loc::Dir(parent.to_path_buf()), history);
                        self.select_path(&file);
                    }
                    return;
                }
                self.alert(&crate::trf("The folder “{path}” can't be found.", &[("path", &p.display())]), "", false);
                return;
            }
        }
        if history && loc != self.loc {
            let old = std::mem::replace(&mut self.loc, loc);
            self.back.push(old);
            self.fwd.clear();
        } else {
            self.loc = loc;
        }
        let ui = self.ui();
        let f = ui.global::<F>();
        if !matches!(self.loc, Loc::Search(..)) {
            f.set_query("".into());
            f.set_searching(false);
        }
        f.set_renaming(-1);
        f.set_scroll_y(0.0);
        self.load();
    }

    fn back(&mut self) {
        if let Some(p) = self.back.pop() {
            let cur = std::mem::replace(&mut self.loc, p);
            let came_from = cur.dir().map(|d| d.to_path_buf());
            self.fwd.push(cur);
            self.go(self.loc.clone(), false);
            if let Some(d) = came_from {
                self.select_path(&d);
            }
        }
    }

    fn forward(&mut self) {
        if let Some(p) = self.fwd.pop() {
            let cur = std::mem::replace(&mut self.loc, p);
            self.back.push(cur);
            self.go(self.loc.clone(), false);
        }
    }

    fn up(&mut self) {
        let Some(d) = self.loc.dir().map(|d| d.to_path_buf()) else { return };
        if let Some(parent) = d.parent() {
            self.go(Loc::Dir(parent.to_path_buf()), true);
            self.select_path(&d);
        }
    }

    fn select_path(&mut self, p: &Path) {
        if let Some(&vi) = self.row_of.get(p) {
            self.sel = vec![vi];
            self.anchor = Some(vi);
            self.focus = Some(vi);
            self.update_selection();
        }
    }

    /// Content width/height and icon columns (mirrors finder.slint).
    fn geometry(&self) -> (f32, f32, usize) {
        let ui = self.ui();
        let s = ui.window().size().to_logical(ui.window().scale_factor());
        let f = ui.global::<F>();
        let pv = if f.get_show_preview() && s.width > 820.0 { 268.0 } else { 0.0 };
        let cw = s.width - 165.0 - pv;
        let top = if f.get_mode() == 2 { 130.0 } else { 52.0 };
        let bottom = if f.get_mode() != 0 { 52.0 } else { 0.0 };
        let cols = (((cw - 16.0) / 127.0).floor() as usize).max(1);
        (cw, s.height - top - bottom, cols)
    }

    fn select(&mut self, vi: usize, toggle: bool, range: bool) {
        let multi = self.chooser.as_ref().map(|c| c.multiple).unwrap_or(true);
        if vi >= self.shown.len() {
            return;
        }
        if self.dimmed(&self.all[self.shown[vi]]) {
            return;
        }
        if let (true, true, Some(a)) = (multi, range, self.anchor) {
            self.sel = (a.min(vi)..=a.max(vi)).filter(|&i| !self.dimmed(&self.all[self.shown[i]])).collect();
        } else if multi && toggle {
            if let Some(p) = self.sel.iter().position(|&x| x == vi) {
                self.sel.remove(p);
            } else {
                self.sel.push(vi);
            }
            self.anchor = Some(vi);
        } else {
            self.sel = vec![vi];
            self.anchor = Some(vi);
        }
        self.focus = Some(vi);
        let ui = self.ui();
        let f = ui.global::<F>();
        if f.get_renaming() >= 0 && f.get_renaming() as usize != vi {
            f.set_renaming(-1);
        }
        if let Some(c) = &self.chooser {
            if c.save {
                let e = &self.all[self.shown[vi]];
                if !e.is_dir {
                    f.set_save_name(e.name.clone().into());
                }
            }
        }
        self.update_selection();
    }

    fn select_none(&mut self) {
        if !self.sel.is_empty() {
            self.sel.clear();
            self.update_selection();
        }
        self.ui().global::<F>().set_renaming(-1);
    }

    fn open_entry(&mut self, e: Entry) {
        if let Some((cmd, ..)) = &e.app {
            if self.chooser.is_none() {
                aqua_apps::launch(cmd);
            }
            return;
        }
        if e.is_dir {
            self.go(Loc::Dir(e.path.clone()), true);
            return;
        }
        if let Some(ch) = &self.chooser {
            if !ch.save && !self.dimmed(&e) {
                finish(vec![e.path.clone()]);
            }
            return;
        }
        if matches!(self.loc, Loc::Trash) {
            self.alert("To open this item, first drag it out of the Trash.", "", false);
            return;
        }
        open_path(&e.path);
        record_recent(&e.path);
    }

    fn open_selection(&mut self) {
        let sel: Vec<Entry> = self.sel_entries().into_iter().cloned().collect();
        if sel.len() == 1 {
            return self.open_entry(sel[0].clone());
        }
        for e in sel {
            if e.is_dir {
                spawn_finder(&e.path.to_string_lossy());
            } else {
                self.open_entry(e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chooser_filters() {
        assert!(glob_match("*", "anything", 0));
        assert!(glob_match("*.PNG", "photo.png", 2));
        assert!(!glob_match("*.png", "photo.jpg", 2));
        assert!(glob_match("image/*", "x.webp", 2));
        assert!(!glob_match("image/*", "x.mp3", 3));
        assert!(!glob_match("application/*", "x.bin", 0));
        assert!(glob_match("report*.pdf", "Report-2024.pdf", 8));
        assert!(!glob_match("report*.pdf", "my-report.pdf", 8));
        assert!(glob_match("*draft*", "the draft v2", 6));
        assert!(glob_match("Makefile", "makefile", 6));
        assert!(!glob_match("Makefile", "Makefile.am", 6));
    }
}
