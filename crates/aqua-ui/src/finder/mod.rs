//! Finder: Aqua's file manager, and — in chooser mode — the system open/save panel.
//!
//!   aqua-finder [PATH | recents: | apps: | trash: | tag:NAME]
//!   aqua-filechooser [--title T] [--accept-label L] [--save] [--multiple] [--directory]
//!                    [--name FILE] [--folder DIR] [--filter "Images:*.png;*.jpg"]...
//!
//! The chooser prints the chosen absolute paths (one per line) and exits 0, or exits 1 when
//! cancelled (protocol of the xdg-desktop-portal backend in aqua-notify).
pub mod archive;
pub mod arrange;
pub mod criteria;
pub mod fs;
mod info;
mod input;
pub mod ipc;
pub mod layout;
mod media;
mod menus;
mod openwith;
mod ops;
mod panels;
mod places;
mod popmenu;
pub mod quick;
pub mod rename;
mod run;
mod search;
pub mod settings;
pub mod shim;
#[cfg(test)]
mod tests_ui;
mod thumbs;
pub mod transfer;
pub mod undo;

pub use run::{run, OUTPUT_SEP};
use run::{finish, hostname, open_path, open_terminal, record_recent, spawn_finder, trunc};

use crate::{
    FColumn, FCrit, FCrumb, FItem, FLCol, FMenuItem, FOpt, FPlace, FRow, FSpot, FTab, FTagDef, FinderWindow, F, FKV,
};
use fs::{Entry, Loc};
use layout::{Grid, Group, Row};
use settings::Settings;
use slint::{Color, ComponentHandle, Image, Model, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
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

fn model<T: Clone + 'static>(v: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(v))
}

/// What a confirmation alert does when answered.
enum Pending {
    None,
    EmptyTrash,
    DeleteNow(Vec<PathBuf>),
    Replace(PathBuf),
    Extension { vi: usize, name: String, keep: String },
    Conflict,
    Message,
}

/// A copy/move waiting for answers about name conflicts.
struct Ask {
    srcs: Vec<PathBuf>,
    dest: PathBuf,
    mode: transfer::Mode,
    queue: Vec<PathBuf>,
    choices: HashMap<PathBuf, transfer::Choice>,
}

#[derive(Clone)]
struct Tab {
    loc: Loc,
    back: Vec<Loc>,
    fwd: Vec<Loc>,
    sel: Vec<PathBuf>,
    scroll: f32,
    expanded: HashSet<PathBuf>,
}

struct Marquee {
    x: f32,
    y: f32,
    base: Vec<usize>,
    additive: bool,
    moved: bool,
}

fn cmp_entries(x: &Entry, y: &Entry, key: i32, folders_first: bool) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    if folders_first && x.is_dir != y.is_dir {
        return if x.is_dir { Ordering::Less } else { Ordering::Greater };
    }
    match key {
        1 => x.label.cmp(&y.label).then(fs::natural(&x.name, &y.name)),
        2 => y.mtime.cmp(&x.mtime).then(fs::natural(&x.name, &y.name)),
        3 => y.size.cmp(&x.size).then(fs::natural(&x.name, &y.name)),
        4 => y.ctime.cmp(&x.ctime).then(fs::natural(&x.name, &y.name)),
        5 => y.atime.cmp(&x.atime).then(fs::natural(&x.name, &y.name)),
        _ => fs::natural(&x.name, &y.name),
    }
}

struct App {
    ui: slint::Weak<FinderWindow>,
    loc: Loc,
    back: Vec<Loc>,
    fwd: Vec<Loc>,
    all: Vec<Entry>,
    base_len: usize,
    shown: Vec<usize>,
    depth: Vec<u16>,
    labels: Vec<String>,
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
    requested: HashSet<PathBuf>,
    worker: thumbs::Worker,
    side_rows: Vec<String>,
    dir_stamp: Option<std::time::SystemTime>,
    search_gen: u64,
    search_rx: Option<std::sync::mpsc::Receiver<(u64, Vec<Entry>, bool)>>,
    search_content: bool,
    pending: Pending,
    drag: Vec<PathBuf>,
    ext_drag: Option<std::time::Instant>,
    menu_paths: Vec<PathBuf>,
    custom: Option<PathBuf>,
    type_buf: (String, std::time::Instant),
    col_cache: HashMap<PathBuf, (Option<std::time::SystemTime>, Vec<Entry>)>,
    tabs: Vec<Tab>,
    tab: usize,
    hist: undo::History,
    xfer: Option<transfer::Handle>,
    ask: Option<Ask>,
    expanded: HashSet<PathBuf>,
    rows: Vec<Row>,
    offs: Vec<f32>,
    total: f32,
    grid: Grid,
    geo: (f32, f32, f32, f32),
    mq: Option<Marquee>,
    label_click: Option<(usize, std::time::Instant)>,
    spring: Option<(String, std::time::Instant)>,
    infos: Vec<info::InfoWin>,
    summary_seq: u64,
    connect_rx: Option<std::sync::mpsc::Receiver<Result<PathBuf, String>>>,
    player: Option<media::Player>,
    ql_pdf: Option<(PathBuf, usize, usize)>,
    ipc: Option<ipc::Ipc>,
    criteria: Vec<criteria::Criterion>,
    smart: Option<(PathBuf, criteria::Saved)>,
    /// The current search is an unsaved “New Smart Folder”.
    new_smart: bool,
    /// Query typed into the search field, applied once typing pauses.
    pending_q: Option<(std::time::Instant, String)>,
    arr: arrange::Arrangements,
    spots: Option<Vec<(f32, f32)>>,
    drag_origin: Option<(f32, f32)>,
    sizes: HashMap<PathBuf, u64>,
    size_rx: Option<std::sync::mpsc::Receiver<(PathBuf, u64)>>,
    free: (Option<PathBuf>, String, Option<std::time::Instant>),
    apps_menu: Vec<openwith::DesktopApp>,
    rn_paths: Vec<PathBuf>,
    item_info: HashMap<PathBuf, String>,
    me: std::rc::Weak<RefCell<App>>,
    pop: popmenu::NativeMenus,
}

/// Finder entry for archive member `a` of `arc` (virtual path `arc/member`).
fn archive_entry(arc: &Path, a: &archive::ArcEntry) -> Entry {
    let name = a.path.rsplit('/').next().unwrap_or(&a.path).to_string();
    let (kind, ext, label) = fs::classify(&name, a.is_dir);
    Entry {
        name,
        path: arc.join(&a.path),
        is_dir: a.is_dir,
        size: a.size,
        mtime: a.mtime,
        ctime: a.mtime,
        atime: a.mtime,
        kind,
        ext,
        label,
        app: None,
        orig: None,
        mode: if a.is_dir { 0o40755 } else { 0o100644 },
    }
}

/// Member path of virtual entry path `p` inside archive `arc`.
fn archive_inner(arc: &Path, p: &Path) -> String {
    p.strip_prefix(arc).map(|r| r.to_string_lossy().into_owned()).unwrap_or_default()
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

fn name_of(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

impl App {
    fn new(ui: &FinderWindow, st: Settings, chooser: Option<Chooser>, loc: Loc, items: Rc<VecModel<FItem>>) -> App {
        App {
            ui: ui.as_weak(),
            loc: loc.clone(),
            back: vec![],
            fwd: vec![],
            all: vec![],
            base_len: 0,
            shown: vec![],
            depth: vec![],
            labels: vec![],
            sel: vec![],
            anchor: None,
            focus: None,
            st,
            chooser,
            filter: 0,
            meta: fs::Meta::load(),
            items,
            row_of: HashMap::new(),
            thumbs: HashMap::new(),
            requested: Default::default(),
            worker: thumbs::Worker::spawn(2),
            side_rows: vec![],
            dir_stamp: None,
            search_gen: 0,
            search_rx: None,
            search_content: false,
            pending: Pending::None,
            drag: vec![],
            ext_drag: None,
            menu_paths: vec![],
            custom: None,
            type_buf: (String::new(), std::time::Instant::now()),
            col_cache: HashMap::new(),
            tabs: vec![Tab { loc, back: vec![], fwd: vec![], sel: vec![], scroll: 0.0, expanded: HashSet::new() }],
            tab: 0,
            hist: undo::History::default(),
            xfer: None,
            ask: None,
            expanded: HashSet::new(),
            rows: vec![],
            offs: vec![],
            total: 0.0,
            grid: Grid::new(800.0, 64.0, 0.0, 12.0, false),
            geo: (165.0, 52.0, 835.0, 548.0),
            mq: None,
            label_click: None,
            spring: None,
            infos: vec![],
            summary_seq: 0,
            connect_rx: None,
            player: None,
            ql_pdf: None,
            ipc: None,
            criteria: vec![],
            smart: None,
            new_smart: false,
            pending_q: None,
            arr: arrange::Arrangements::load(arrange::default_file()),
            spots: None,
            drag_origin: None,
            sizes: HashMap::new(),
            size_rx: None,
            free: (None, String::new(), None),
            apps_menu: vec![],
            rn_paths: vec![],
            item_info: HashMap::new(),
            me: std::rc::Weak::new(),
            pop: Default::default(),
        }
    }

    fn ui(&self) -> FinderWindow {
        self.ui.upgrade().expect("finder window")
    }

    fn view(&self) -> i32 {
        self.ui().global::<F>().get_view()
    }

    fn group(&self) -> Group {
        if self.chooser.is_some()
            || matches!(self.loc, Loc::Search(..) | Loc::Smart(_) | Loc::Recents) && self.st.group == 0
        {
            return Group::None;
        }
        match self.view() {
            0 | 1 => Group::from_i32(self.st.group),
            _ => Group::None,
        }
    }

    fn tree(&self) -> bool {
        self.view() == 1 && self.group() == Group::None && matches!(self.loc, Loc::Dir(_))
    }

    fn load(&mut self) {
        self.worker.clear();
        self.requested.clear();
        self.search_rx = None;
        self.item_info.clear();
        let ui = self.ui();
        let f = ui.global::<F>();
        f.set_search_busy(false);
        let mut empty = String::new();
        self.all = match &self.loc {
            Loc::Dir(p) => match fs::list_dir(p) {
                Ok(v) => v,
                Err(e) => {
                    empty = crate::trf("“{name}” can't be opened: {error}", &[("name", &name_of(p)), ("error", &e)]);
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
                self.smart = None;
                if q.is_empty() && self.criteria.is_empty() {
                    f.set_search_busy(false);
                } else {
                    self.start_search(root.clone(), q.clone());
                }
                vec![]
            }
            Loc::Archive(p, inner) => match archive::list(p) {
                Ok(all) => {
                    let mut v: Vec<Entry> =
                        archive::children(&all, inner).into_iter().map(|a| archive_entry(p, &a)).collect();
                    v.sort_by(|a, b| fs::natural(&a.name, &b.name));
                    v
                }
                Err(e) => {
                    empty = crate::trf("“{name}” can't be opened: {error}", &[("name", &name_of(p)), ("error", &e)]);
                    vec![]
                }
            },
            Loc::Smart(p) => {
                if self.smart.as_ref().map(|s| &s.0) != Some(p) {
                    let saved = criteria::Saved::load(p).unwrap_or_default();
                    self.criteria = saved.criteria.clone();
                    self.search_content = saved.content;
                    f.set_query(saved.query.clone().into());
                    self.smart = Some((p.clone(), saved));
                }
                let (root, q) = self.smart.as_ref().map(|s| (s.1.root.clone(), s.1.query.clone())).unwrap_or_default();
                let root = if root.as_os_str().is_empty() { PathBuf::from("/") } else { root };
                self.start_search(root, q);
                vec![]
            }
        };
        if !matches!(self.loc, Loc::Search(..) | Loc::Smart(_)) {
            self.smart = None;
            self.criteria.clear();
        }
        if !matches!(self.loc, Loc::Search(..)) {
            self.new_smart = false;
            self.pending_q = None;
        }
        f.set_crit(model(self.crit_rows()));
        f.set_smart(self.smart.is_some());
        self.base_len = self.all.len();
        if empty.is_empty() {
            empty = match &self.loc {
                Loc::Trash => crate::tr("Trash is empty").into(),
                Loc::Recents => crate::tr("No recent documents").into(),
                Loc::Tag(t) => crate::trf("No items tagged “{tag}”", &[("tag", &crate::tr(t))]),
                Loc::Search(_, q) if q.is_empty() && self.criteria.is_empty() => String::new(),
                Loc::Search(..) | Loc::Smart(_) => crate::tr("Searching…").into(),
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
        if f.get_view() == 3 && !self.shown.is_empty() {
            self.select(0, false, false);
        }
        self.want_sizes();
    }

    fn visible(&self, e: &Entry) -> bool {
        self.st.hidden || !e.name.starts_with('.') || matches!(self.loc, Loc::Trash)
    }

    fn sorted(&self, mut idx: Vec<usize>) -> Vec<usize> {
        let (key, desc) = self.st.sort;
        let ff = self.st.folders_first && key == 0;
        let all = &self.all;
        if matches!(self.loc, Loc::Recents) && key == 0 && self.st.group == 0 {
            return idx;
        }
        idx.sort_by(|&a, &b| {
            let o = cmp_entries(&all[a], &all[b], key, ff);
            if desc {
                o.reverse()
            } else {
                o
            }
        });
        idx
    }

    fn children(&mut self, dir: &Path) -> Vec<Entry> {
        let stamp = std::fs::metadata(dir).ok().and_then(|m| m.modified().ok());
        if let Some((s, v)) = self.col_cache.get(dir) {
            if *s == stamp {
                return v.clone();
            }
        }
        let v = fs::list_dir(dir).unwrap_or_default();
        self.col_cache.insert(dir.to_path_buf(), (stamp, v.clone()));
        v
    }

    fn push_tree(&mut self, list: Vec<usize>, depth: u16, out: &mut Vec<(usize, u16)>) {
        for i in list {
            out.push((i, depth));
            let (is_dir, path) = (self.all[i].is_dir, self.all[i].path.clone());
            if is_dir && depth < 16 && self.expanded.contains(&path) {
                let kids = self.children(&path);
                let start = self.all.len();
                self.all.extend(kids);
                let ids: Vec<usize> = (start..self.all.len()).filter(|&k| self.visible(&self.all[k])).collect();
                let ids = self.sorted(ids);
                self.push_tree(ids, depth + 1, out);
            }
        }
    }

    fn sort_and_filter(&mut self) {
        self.all.truncate(self.base_len);
        let q = self.ui().global::<F>().get_query().to_lowercase();
        let mut idx: Vec<usize> = (0..self.all.len()).filter(|&i| self.visible(&self.all[i])).collect();
        if !q.is_empty() && !matches!(self.loc, Loc::Search(..) | Loc::Smart(_)) {
            idx.retain(|&i| self.all[i].name.to_lowercase().contains(&q));
        }
        let mut idx = self.sorted(idx);
        let by = self.group();
        if by != Group::None {
            let now = fs::now_secs();
            let keyed: HashMap<usize, (i64, String)> = idx
                .iter()
                .map(|&i| {
                    let e = &self.all[i];
                    (i, layout::group_of(e, by, &self.meta.tags_of(&e.path), now))
                })
                .collect();
            idx.sort_by_key(|i| keyed[i].0);
            self.labels = idx.iter().map(|i| keyed[i].1.clone()).collect();
            self.depth = vec![0; idx.len()];
            self.shown = idx;
            return;
        }
        if self.tree() && !self.expanded.is_empty() {
            let mut out = vec![];
            self.push_tree(idx, 0, &mut out);
            self.shown = out.iter().map(|x| x.0).collect();
            self.depth = out.iter().map(|x| x.1).collect();
        } else {
            self.depth = vec![0; idx.len()];
            self.shown = idx;
        }
        self.labels = vec![String::new(); self.shown.len()];
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

    fn info_line(&mut self, e: &Entry) -> String {
        if let Some(s) = self.item_info.get(&e.path) {
            return s.clone();
        }
        let s = if e.is_dir {
            let n = std::fs::read_dir(&e.path).map(|r| r.count()).unwrap_or(0);
            crate::ntr("{n} item", "{n} items", n as i64)
        } else if e.kind == 2 {
            image::image_dimensions(&e.path).map(|(w, h)| format!("{w}×{h}")).unwrap_or_else(|_| fs::human(e.size))
        } else if e.app.is_some() {
            String::new()
        } else {
            fs::human(e.size)
        };
        self.item_info.insert(e.path.clone(), s.clone());
        s
    }

    fn date(&self, t: i64) -> String {
        if self.st.rel_dates {
            fs::short_date(t)
        } else {
            fs::long_date(t)
        }
    }

    fn item(&self, e: &Entry, selected: bool, cut: &[PathBuf]) -> FItem {
        let (mut thumb, snippet) = self.thumbs.get(&e.path).cloned().unwrap_or((None, SharedString::new()));
        if !self.st.icon_preview && e.app.is_none() {
            thumb = None;
        }
        let key = e.path.to_string_lossy();
        let look = if e.is_dir { self.meta.folders.get(&*key).cloned() } else { None };
        let (tint, has_tint, symbol) = match look {
            Some((c, s)) if c != 0 => (rgb(c), true, s),
            Some((_, s)) => (Color::default(), false, s),
            None => (Color::default(), false, String::new()),
        };
        let tags: Vec<Color> = self.meta.tags_of(&e.path).iter().map(|t| tag_color(t)).collect();
        let size = if e.is_dir {
            self.sizes.get(&e.path).map(|&n| fs::human(n)).unwrap_or_else(|| "--".into())
        } else if e.kind == 5 {
            "--".to_string()
        } else {
            fs::human(e.size)
        };
        let alias = e.path.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false);
        FItem {
            name: fs::display_name(&e.name, e.is_dir, self.st.show_ext).into(),
            path: key.to_string().into(),
            is_dir: e.is_dir,
            kind: e.kind,
            ext: e.ext.clone().into(),
            size: size.into(),
            modified: self.date(e.mtime).into(),
            created: self.date(e.ctime).into(),
            opened: self.date(e.atime).into(),
            kind_label: e.label.clone().into(),
            selected,
            has_thumb: thumb.is_some(),
            thumb: thumb.unwrap_or_default(),
            snippet,
            tint,
            has_tint,
            symbol: symbol.into(),
            tags: model(tags),
            dim: self.dimmed(e),
            cut: cut.contains(&e.path),
            depth: 0,
            can_expand: false,
            expanded: false,
            info: SharedString::new(),
            alias,
        }
    }

    fn row_item(&mut self, vi: usize, cut: &[PathBuf]) -> FItem {
        let e = self.all[self.shown[vi]].clone();
        let mut it = self.item(&e, self.sel.contains(&vi), cut);
        it.depth = self.depth.get(vi).copied().unwrap_or(0) as i32;
        if self.tree() && e.is_dir {
            it.can_expand = true;
            it.expanded = self.expanded.contains(&e.path);
        }
        if self.st.item_info && self.view() == 0 {
            it.info = self.info_line(&e).into();
        }
        it
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

    fn cut_list() -> Vec<PathBuf> {
        let (cut_on, clip) = read_clip();
        if cut_on {
            clip
        } else {
            vec![]
        }
    }

    /// Rebuild the item model (after listing / sorting / filtering).
    fn refresh(&mut self) {
        let cut = Self::cut_list();
        let v: Vec<FItem> = (0..self.shown.len()).map(|vi| self.row_item(vi, &cut)).collect();
        self.row_of = self.shown.iter().enumerate().map(|(vi, &i)| (self.all[i].path.clone(), vi)).collect();
        self.items.set_vec(v);
        let todo: Vec<Entry> = self.shown.iter().take(800).rev().map(|&i| self.all[i].clone()).collect();
        for e in todo {
            self.want_preview(&e);
        }
        self.layout_rows();
        self.chrome();
    }

    /// Rows for the icon and list views (grouping, wrapping) and their metrics.
    fn layout_rows(&mut self) {
        let ui = self.ui();
        let f = ui.global::<F>();
        let (_, _, cw, _) = self.geo;
        self.grid = Grid::new(cw, self.st.icon, self.st.spacing(), self.st.icon_text, self.st.item_info);
        let view = f.get_view();
        let per_row = if view == 0 { self.grid.cols } else { 1 };
        self.rows = layout::rows(&self.labels, per_row);
        let (offs, total) = if view == 0 {
            layout::offsets(&self.rows, layout::HEADER_ICON, self.grid.cell_h)
        } else {
            layout::offsets(&self.rows, layout::HEADER_LIST, layout::ROW_LIST)
        };
        self.offs = offs;
        self.total = total;
        self.spots = self.free_folder().map(|fo| {
            let names: Vec<String> = self.shown.iter().map(|&i| self.all[i].name.clone()).collect();
            arrange::place(&self.grid, &names, &fo)
        });
        let arrange_mode = match self.free_folder() {
            Some(fo) if fo.snap => 2,
            Some(_) => 1,
            None => 0,
        };
        f.set_arrange(arrange_mode);
        f.set_can_arrange(matches!(self.loc, Loc::Dir(_)) && self.chooser.is_none());
        f.set_free_mode(self.spots.is_some());
        if let Some(sp) = &self.spots {
            f.set_free_h(arrange::extent(&self.grid, sp));
            f.set_spots(model(sp.iter().map(|&(x, y)| FSpot { x, y }).collect()));
        }
        f.set_cell_w(self.grid.cell_w);
        f.set_cell_h(self.grid.cell_h);
        f.set_grid_pad(self.grid.pad);
        f.set_icon_s(self.grid.icon);
        f.set_text_s(self.st.icon_text);
        f.set_list_text(self.st.list_text);
        f.set_item_info(self.st.item_info);
        f.set_icon_preview(self.st.icon_preview);
        f.set_zoom(self.st.zoom());
        f.set_grid_gap(self.st.gap);
        f.set_rows(model(
            self.rows
                .iter()
                .map(|r| FRow { header: crate::tr(&r.header).into(), start: r.start as i32, count: r.count as i32 })
                .collect(),
        ));
        let widths = |k: i32| self.st.col_w.get(&k).copied().unwrap_or_else(|| layout::default_width(k));
        let cols = layout::list_columns(cw - 20.0, &self.st.list_cols, &widths);
        let title = |k: i32| match k {
            1 => "Kind",
            2 => "Date Modified",
            3 => "Size",
            4 => "Date Created",
            5 => "Date Last Opened",
            6 => "Tags",
            _ => "Name",
        };
        f.set_lcols(model(
            cols.iter()
                .map(|&(key, x, w)| FLCol { key, title: crate::tr(title(key)).into(), x, w, right: key == 3 })
                .collect(),
        ));
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
        if self.ui().global::<F>().get_ql_open() {
            self.ql_update();
        }
    }

    fn update_row(&mut self, path: &Path) {
        let Some(&vi) = self.row_of.get(path) else { return };
        if vi >= self.shown.len() {
            return;
        }
        let cut = Self::cut_list();
        let it = self.row_item(vi, &cut);
        self.items.set_row_data(vi, it);
    }

    fn loc_title(&self, loc: &Loc) -> String {
        match loc {
            Loc::Dir(p) if p == Path::new("/") => hostname(),
            Loc::Dir(p) => name_of(p),
            Loc::Recents => crate::tr("Recents").into(),
            Loc::Apps => crate::tr("Applications").into(),
            Loc::Trash => crate::tr("Trash").into(),
            Loc::Tag(t) => crate::tr(t).to_string(),
            Loc::Smart(p) => criteria::saved_name(p),
            Loc::Archive(p, inner) if inner.is_empty() => name_of(p),
            Loc::Archive(_, inner) => inner.rsplit('/').next().unwrap_or(inner).to_string(),
            Loc::Search(p, q) if q.is_empty() => crate::trf(
                "Searching “{place}”",
                &[("place", &if p == Path::new("/") { crate::tr("This Computer").to_string() } else { name_of(p) })],
            ),
            Loc::Search(p, q) => crate::trf(
                "Searching “{place}” for “{query}”",
                &[
                    ("place", &if p == Path::new("/") { crate::tr("This Computer").to_string() } else { name_of(p) }),
                    ("query", q),
                ],
            ),
        }
    }

    fn title(&self) -> String {
        if self.new_smart && matches!(self.loc, Loc::Search(..)) {
            return crate::tr("New Smart Folder").into();
        }
        self.loc_title(&self.loc)
    }

    fn crumbs(&self) -> Vec<FCrumb> {
        if let Loc::Archive(arc, inner) = &self.loc {
            // the folders up to the archive, the archive, then the folders inside it
            let mut v = self.dir_crumbs(arc.parent().unwrap_or(Path::new("/")));
            v.push(FCrumb {
                label: name_of(arc).into(),
                path: Loc::Archive(arc.clone(), String::new()).key().into(),
                icon: "docs".into(),
            });
            let mut acc = String::new();
            for part in inner.split('/').filter(|s| !s.is_empty()) {
                if !acc.is_empty() {
                    acc.push('/');
                }
                acc.push_str(part);
                v.push(FCrumb {
                    label: part.into(),
                    path: Loc::Archive(arc.clone(), acc.clone()).key().into(),
                    icon: "folder".into(),
                });
            }
            return v;
        }
        let Some(dir) = self.loc.dir().filter(|_| matches!(self.loc, Loc::Dir(_))) else {
            return vec![FCrumb { label: self.title().into(), path: self.loc.key().into(), icon: "folder".into() }];
        };
        let home = fs::home();
        let mut v: Vec<FCrumb> = dir
            .ancestors()
            .map(|a| {
                let icon = if a == Path::new("/") {
                    "computer"
                } else if a == home {
                    "home"
                } else {
                    "folder"
                };
                let label = if a == Path::new("/") { hostname() } else { name_of(a) };
                FCrumb { label: label.into(), path: a.to_string_lossy().to_string().into(), icon: icon.into() }
            })
            .collect();
        v.reverse();
        let mut sel = self.sel_entries();
        if sel.len() == 1 {
            let e = sel.remove(0);
            let icon = if e.is_dir { "folder" } else { "docs" };
            v.push(FCrumb {
                label: e.name.clone().into(),
                path: e.path.to_string_lossy().to_string().into(),
                icon: icon.into(),
            });
        }
        v
    }

    /// Breadcrumbs for a real folder and its ancestors.
    fn dir_crumbs(&self, dir: &Path) -> Vec<FCrumb> {
        let home = fs::home();
        let mut v: Vec<FCrumb> = dir
            .ancestors()
            .map(|a| {
                let icon = if a == Path::new("/") {
                    "computer"
                } else if a == home {
                    "home"
                } else {
                    "folder"
                };
                let label = if a == Path::new("/") { hostname() } else { name_of(a) };
                FCrumb { label: label.into(), path: a.to_string_lossy().to_string().into(), icon: icon.into() }
            })
            .collect();
        v.reverse();
        v
    }

    fn free_space(&mut self) -> String {
        let dir = self.loc.dir().map(|d| d.to_path_buf());
        let fresh = self.free.0 == dir && self.free.2.is_some_and(|t| t.elapsed().as_secs() < 5);
        if !fresh {
            let s = dir
                .as_deref()
                .and_then(fs::free_space)
                .map(|n| crate::trf("{size} available", &[("size", &fs::human(n))]))
                .unwrap_or_default();
            self.free = (dir, s, Some(std::time::Instant::now()));
        }
        self.free.1.clone()
    }

    /// Toolbar, preview pane, bars, chooser button: everything derived from state and selection.
    fn chrome(&mut self) {
        let ui = self.ui();
        let f = ui.global::<F>();
        f.set_title(self.title().into());
        f.set_can_back(!self.back.is_empty());
        f.set_can_forward(!self.fwd.is_empty());
        f.set_cur_place(self.loc.key().into());
        f.set_trash_view(matches!(self.loc, Loc::Trash));
        f.set_focus_idx(self.focus.map(|x| x as i32).unwrap_or(-1));
        f.set_group_by(self.st.group);
        f.set_in_search(matches!(self.loc, Loc::Search(..) | Loc::Smart(_)) && self.chooser.is_none());
        let smart_root = self.smart.as_ref().map(|s| Loc::Search(s.1.root.clone(), String::new()));
        if let Some(Loc::Search(root, _)) = smart_root.as_ref().or(Some(&self.loc)) {
            f.set_search_scope(if root == Path::new("/") { 0 } else { 1 });
            let folder = if root == Path::new("/") { self.scope_root(1) } else { root.clone() };
            f.set_scope_folder(if folder == Path::new("/") { SharedString::new() } else { name_of(&folder).into() });
        } else if let Some(d) = self.loc.dir() {
            f.set_scope_folder(name_of(d).into());
        }
        f.set_search_content(self.search_content);
        f.set_can_undo(self.hist.can_undo().is_some());
        let free = self.free_space();
        f.set_free(free.into());
        f.set_crumbs(model(self.crumbs()));
        if let Some(t) = self.tabs.get_mut(self.tab) {
            t.loc = self.loc.clone();
        }
        let titles: Vec<FTab> =
            self.tabs
                .iter()
                .enumerate()
                .map(|(i, t)| FTab {
                    title: if i == self.tab { self.title() } else { self.loc_title(&t.loc) }.into(),
                    path: t.loc.key().into(),
                })
                .collect();
        f.set_show_tabbar(self.st.tab_bar || titles.len() > 1);
        f.set_tabs(model(titles));
        f.set_tab_idx(self.tab as i32);
        f.set_show_pathbar(self.st.path_bar);
        f.set_show_statusbar(self.st.status_bar);
        f.set_show_sidebar(self.st.sidebar);
        f.set_col_preview(self.st.col_preview);
        f.set_col_icons(self.st.col_icons);
        f.set_rel_dates(self.st.rel_dates);
        f.set_calc_sizes(self.st.calc_sizes);
        let sel: Vec<Entry> = self.sel_entries().into_iter().cloned().collect();
        f.set_pv_count(sel.len() as i32);
        if sel.len() == 1 {
            let e = sel[0].clone();
            let cut = Self::cut_list();
            let it = self.item(&e, true, &cut);
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
            f.set_pv_info(model(info));
            f.set_pv_quick(e.app.is_none() && quick::editable_image(&e.path));
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
        if !self.chooser.as_ref().map(|c| c.save).unwrap_or(false) {
            let paths: Vec<PathBuf> = sel.iter().map(|e| e.path.clone()).collect();
            let defs: Vec<FTagDef> = fs::TAGS
                .iter()
                .map(|(name, c)| FTagDef {
                    name: (*name).into(),
                    color: rgb(*c),
                    on: !paths.is_empty() && paths.iter().all(|p| self.meta.tags_of(p).iter().any(|t| t == name)),
                })
                .collect();
            f.set_tag_defs(model(defs));
        }
        if f.get_view() == 2 {
            self.columns();
        }
    }

    fn columns(&mut self) {
        let ui = self.ui();
        let f = ui.global::<F>();
        let cut = Self::cut_list();
        let last = FColumn {
            path: self.loc.key().into(),
            items: ModelRc::from(self.items.clone() as Rc<dyn Model<Data = FItem>>),
            active: true,
        };
        let Some(dir) = self.loc.dir().map(|p| p.to_path_buf()).filter(|_| matches!(self.loc, Loc::Dir(_))) else {
            f.set_columns(model(vec![last]));
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
            let mut entries = self.children(parent);
            entries.retain(|e| self.st.hidden || !e.name.starts_with('.'));
            entries.sort_by(|a, b| fs::natural(&a.name, &b.name));
            let items: Vec<FItem> = entries.iter().map(|e| self.item(e, &e.path == child, &cut)).collect();
            cols.push(FColumn {
                path: parent.to_string_lossy().to_string().into(),
                items: model(items),
                active: false,
            });
        }
        cols.push(last);
        f.set_columns(model(cols));
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
                self.message(&crate::trf("The folder “{path}” can't be found.", &[("path", &p.display())]), "");
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
        self.expanded.clear();
        let ui = self.ui();
        let f = ui.global::<F>();
        if !matches!(self.loc, Loc::Search(..) | Loc::Smart(_)) {
            f.set_query("".into());
            f.set_searching(false);
        }
        f.set_renaming(-1);
        f.set_scroll_y(0.0);
        self.label_click = None;
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

    /// Jump `n` steps through the history (negative: back).
    fn history_jump(&mut self, n: i32) {
        for _ in 0..n.unsigned_abs() {
            let (from, to) = if n < 0 { (&mut self.back, &mut self.fwd) } else { (&mut self.fwd, &mut self.back) };
            let Some(p) = from.pop() else { break };
            let cur = std::mem::replace(&mut self.loc, p);
            to.push(cur);
        }
        self.go(self.loc.clone(), false);
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
            self.reveal(vi);
        }
    }

    fn select_paths(&mut self, paths: &[PathBuf]) {
        let sel: Vec<usize> = paths.iter().filter_map(|p| self.row_of.get(p).copied()).collect();
        if !sel.is_empty() {
            self.focus = sel.last().copied();
            self.anchor = sel.first().copied();
            self.sel = sel;
            self.update_selection();
            if let Some(f) = self.focus {
                self.reveal(f);
            }
        }
    }

    /// Scroll so that item `vi` is visible.
    /// Hand arrangement of the current folder, when the icon view uses one.
    fn free_folder(&self) -> Option<arrange::Folder> {
        match &self.loc {
            Loc::Dir(d) if self.view() == 0 && self.st.group == 0 && self.chooser.is_none() => self.arr.get(d).cloned(),
            _ => None,
        }
    }

    fn reveal(&mut self, vi: usize) {
        let ui = self.ui();
        let f = ui.global::<F>();
        let view = f.get_view();
        if view > 1 {
            return;
        }
        if let Some(&(_, y)) = self.spots.as_ref().and_then(|s| s.get(vi)) {
            let view_h = self.geo.3;
            let sy = -f.get_scroll_y();
            let want = if y < sy {
                y
            } else if y + self.grid.cell_h > sy + view_h {
                y + self.grid.cell_h - view_h
            } else {
                return;
            };
            f.set_scroll_y(-want.max(0.0));
            return;
        }
        let Some(r) = self.rows.iter().position(|r| r.count > 0 && vi >= r.start && vi < r.start + r.count) else {
            return;
        };
        let top = self.offs[r];
        let h = if view == 0 { self.grid.cell_h } else { layout::ROW_LIST };
        let header = if r > 0 && self.rows[r - 1].count == 0 { self.offs[r] - self.offs[r - 1] } else { 0.0 };
        let view_h = self.geo.3 - if view == 1 { layout::LIST_TOP } else { 0.0 };
        let sy = -f.get_scroll_y();
        let max_scroll = (self.total + if view == 0 { 24.0 } else { 0.0 } - view_h).max(0.0);
        let want = if top - header < sy {
            top - header
        } else if top + h > sy + view_h {
            top + h - view_h
        } else {
            return;
        };
        f.set_scroll_y(-want.clamp(0.0, max_scroll));
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
        self.label_click = None;
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
        self.label_click = None;
        self.ui().global::<F>().set_renaming(-1);
    }

    fn open_entry(&mut self, e: Entry) {
        self.label_click = None;
        if let Some((cmd, ..)) = &e.app {
            if self.chooser.is_none() {
                aqua_apps::launch(cmd);
            }
            return;
        }
        if let Loc::Archive(arc, _) = &self.loc {
            let arc = arc.clone();
            let inner = archive_inner(&arc, &e.path);
            if e.is_dir {
                self.go(Loc::Archive(arc, inner), true);
                return;
            }
            // a file inside an archive: extract it to a temporary folder, then open (or choose) that
            match archive::extract_to_temp(&arc, &inner) {
                Ok(tmp) => {
                    let mut x = fs::entry(&tmp).unwrap_or(e);
                    x.path = tmp;
                    if self.chooser.is_some() || !x.is_dir {
                        if let Some(ch) = &self.chooser {
                            if !ch.save {
                                finish(vec![x.path.clone()]);
                            }
                            return;
                        }
                        open_path(&x.path);
                    }
                }
                Err(err) => self.message(&crate::trf("“{name}” couldn't be extracted.", &[("name", &e.name)]), &err),
            }
            return;
        }
        if e.is_dir {
            self.go(Loc::Dir(e.path.clone()), true);
            return;
        }
        if archive::is_archive(&e.path) && !matches!(self.loc, Loc::Trash) {
            // Archives open like folders (⌥-double-click / "Extract Here" unpacks them).
            self.go(Loc::Archive(e.path.clone(), String::new()), true);
            return;
        }
        if let Some(ch) = &self.chooser {
            if !ch.save && !self.dimmed(&e) {
                finish(vec![e.path.clone()]);
            }
            return;
        }
        if matches!(self.loc, Loc::Trash) {
            self.message("To open this item, first drag it out of the Trash.", "");
            return;
        }
        let chosen = self.st.open_with.get(e.path.to_string_lossy().as_ref()).cloned();
        match chosen.and_then(|id| openwith::by_id(&id)) {
            Some(app) => openwith::launch(&app, std::slice::from_ref(&e.path)),
            None => open_path(&e.path),
        }
        record_recent(&e.path);
    }

    fn open_selection(&mut self) {
        let sel: Vec<Entry> = self.sel_entries().into_iter().cloned().collect();
        if sel.len() == 1 || matches!(self.loc, Loc::Archive(..)) {
            for e in sel.into_iter().take(if matches!(self.loc, Loc::Archive(..)) { 16 } else { 1 }) {
                self.open_entry(e);
            }
            return;
        }
        if sel.len() == 1 {
            return self.open_entry(sel[0].clone());
        }
        let mut dirs = 0;
        for e in sel {
            if e.is_dir && self.chooser.is_none() {
                if self.st.open_tabs {
                    self.new_tab(Loc::Dir(e.path.clone()), dirs > 0);
                } else {
                    spawn_finder(&e.path.to_string_lossy());
                }
                dirs += 1;
            } else {
                self.open_entry(e);
            }
        }
    }

    fn save_tab(&mut self) {
        let scroll = self.ui().global::<F>().get_scroll_y();
        let sel = self.sel_paths();
        if let Some(t) = self.tabs.get_mut(self.tab) {
            t.loc = self.loc.clone();
            t.back = self.back.clone();
            t.fwd = self.fwd.clone();
            t.sel = sel;
            t.scroll = scroll;
            t.expanded = self.expanded.clone();
        }
    }

    fn restore_tab(&mut self) {
        let Some(t) = self.tabs.get(self.tab).cloned() else { return };
        self.loc = t.loc;
        self.back = t.back;
        self.fwd = t.fwd;
        self.expanded = t.expanded;
        let ui = self.ui();
        let f = ui.global::<F>();
        f.set_renaming(-1);
        f.set_query("".into());
        f.set_searching(false);
        self.load();
        self.select_paths(&t.sel);
        f.set_scroll_y(t.scroll);
    }

    /// Open `loc` in a new tab; `background` keeps the current tab in front.
    fn new_tab(&mut self, loc: Loc, background: bool) {
        self.save_tab();
        self.tabs.push(Tab { loc, back: vec![], fwd: vec![], sel: vec![], scroll: 0.0, expanded: HashSet::new() });
        if !background {
            self.tab = self.tabs.len() - 1;
            self.restore_tab();
        } else {
            self.chrome();
        }
    }

    fn select_tab(&mut self, i: usize) {
        if i >= self.tabs.len() || i == self.tab {
            return;
        }
        self.save_tab();
        self.tab = i;
        self.restore_tab();
    }

    fn close_tab(&mut self, i: usize) {
        if self.tabs.len() <= 1 {
            return self.close_window();
        }
        self.save_tab();
        self.tabs.remove(i.min(self.tabs.len() - 1));
        if self.tab >= self.tabs.len() || i < self.tab {
            self.tab = self.tab.saturating_sub(1);
        }
        self.restore_tab();
    }

    /// Drag a tab to another place in the tab bar.
    fn move_tab(&mut self, from: usize, to: usize) {
        if from >= self.tabs.len() || from == to {
            return;
        }
        self.save_tab();
        let to = to.min(self.tabs.len() - 1);
        let cur = self.tab;
        let t = self.tabs.remove(from);
        self.tabs.insert(to, t);
        self.tab = if cur == from {
            to
        } else if from < cur && to >= cur {
            cur - 1
        } else if from > cur && to <= cur {
            cur + 1
        } else {
            cur
        };
        self.chrome();
    }

    /// Open the tab in its own window and remove it here.
    fn detach_tab(&mut self, i: usize) {
        if i >= self.tabs.len() || self.tabs.len() < 2 {
            return;
        }
        self.save_tab();
        spawn_finder(&self.tabs[i].loc.key());
        self.close_tab(i);
    }

    fn close_other_tabs(&mut self, keep: usize) {
        if keep >= self.tabs.len() {
            return;
        }
        self.save_tab();
        let t = self.tabs.remove(keep);
        self.tabs = vec![t];
        self.tab = 0;
        self.restore_tab();
    }

    /// Bring the tabs of every other Finder window into this one.
    fn merge_windows(&mut self) {
        let own = self.ipc.as_ref().map(|i| i.path().to_path_buf());
        for p in ipc::peers(&ipc::dir(), own.as_deref()) {
            let Some(answer) = ipc::request(&p, "merge") else { continue };
            for key in answer.lines().filter(|l| !l.trim().is_empty()) {
                self.new_tab(Loc::parse(key), true);
            }
        }
        self.chrome();
    }

    /// Requests from other windows.
    fn serve_ipc(&mut self) {
        let Some(reqs) = self.ipc.as_ref().map(|i| i.poll()) else { return };
        for (s, line) in reqs {
            match line.as_str() {
                "merge" if self.chooser.is_none() => {
                    self.save_tab();
                    let keys: Vec<String> = self.tabs.iter().map(|t| t.loc.key()).collect();
                    ipc::reply(s, &(keys.join("\n") + "\n"));
                    self.ipc = None;
                    self.close_window();
                }
                _ => ipc::reply(s, ""),
            }
        }
    }

    fn close_window(&mut self) {
        if self.chooser.is_some() {
            std::process::exit(1)
        }
        let _ = slint::quit_event_loop();
    }

    fn start_loc(&self) -> Loc {
        let d = |p: Option<PathBuf>| Loc::Dir(p.filter(|p| p.is_dir()).unwrap_or_else(fs::home));
        match self.st.new_window {
            0 => Loc::Recents,
            2 => d(dirs::desktop_dir()),
            3 => d(dirs::document_dir()),
            4 => d(dirs::download_dir()),
            _ => Loc::Dir(fs::home()),
        }
    }

    /// Folder sizes for the list view, computed in the background.
    fn want_sizes(&mut self) {
        self.size_rx = None;
        if !self.st.calc_sizes || self.view() != 1 {
            return;
        }
        let dirs: Vec<PathBuf> = self
            .shown
            .iter()
            .map(|&i| &self.all[i])
            .filter(|e| e.is_dir && !self.sizes.contains_key(&e.path))
            .map(|e| e.path.clone())
            .take(400)
            .collect();
        if dirs.is_empty() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.size_rx = Some(rx);
        std::thread::spawn(move || {
            let t0 = std::time::Instant::now();
            for d in dirs {
                let (n, _) = fs::tree_size(&d, &|| t0.elapsed().as_secs() > 60);
                if tx.send((d, n)).is_err() {
                    return;
                }
            }
        });
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
