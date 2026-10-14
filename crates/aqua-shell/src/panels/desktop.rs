//! Desktop items: the files of the Desktop folder as icons on the wallpaper. Icons sit on a
//! grid filled from the top-right corner (like Finder) or anywhere when grid snapping is
//! off; they can be selected (click, rubber band), dragged, dropped onto folders and opened.
use crate::{hash_of, style, Action, Layer, LayerId, Shell};
use aqua_gfx::{rgba, Canvas, Pixmap, Rect, Weight};
use aqua_icons::IconRequest;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

pub const CELL_W: f32 = 96.0;
pub const CELL_H: f32 = 102.0;
const ICON: f32 = 60.0;
const MARGIN: f32 = 10.0;
const LABEL: f32 = 12.0;
const MAX_ITEMS: usize = 400;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Dir,
    App,
    Image,
    File,
}

pub struct Item {
    pub path: PathBuf,
    pub label: String,
    kind: Kind,
    mtime: Option<SystemTime>,
    app: Option<aqua_apps::App>,
    /// Top-left corner of the item's cell (logical).
    pub pos: (f32, f32),
}

struct Press {
    start: (f32, f32),
    /// Item under the press; `None` starts a rubber-band selection.
    item: Option<usize>,
    origin: Vec<(usize, (f32, f32))>,
    moved: bool,
}

type Thumbs = Arc<Mutex<HashMap<PathBuf, Option<Arc<Pixmap>>>>>;

#[derive(Default)]
pub struct Desktop {
    pub items: Vec<Item>,
    pub selected: HashSet<PathBuf>,
    press: Option<Press>,
    band: Option<Rect>,
    dir_mtime: Option<SystemTime>,
    last_check: Option<Instant>,
    /// Saved placements by file name: (distance from the right screen edge, y).
    places: HashMap<String, (f32, f32)>,
    places_loaded: bool,
    last_click: Option<(PathBuf, Instant)>,
    icons: HashMap<(PathBuf, u32), Arc<Pixmap>>,
    thumbs: Thumbs,
    thumbs_ready: Arc<AtomicBool>,
    layout: (f32, f32, bool),
}

impl Desktop {
    pub fn pressing(&self) -> bool {
        self.press.is_some()
    }
}

pub fn dir() -> PathBuf {
    aqua_config::paths::user_dir("DESKTOP", "Desktop")
}

fn shown(sh: &Shell) -> bool {
    sh.cfg.desktop_icons && !sh.locked && dir() != aqua_config::paths::home()
}

fn places_file() -> PathBuf {
    aqua_config::Config::path().with_file_name("desktop-places")
}

fn load_places(d: &mut Desktop) {
    d.places_loaded = true;
    let Ok(text) = std::fs::read_to_string(places_file()) else { return };
    for line in text.lines() {
        let mut it = line.splitn(3, '\t');
        let (Some(dx), Some(y), Some(name)) = (it.next(), it.next(), it.next()) else { continue };
        if let (Ok(dx), Ok(y)) = (dx.parse(), y.parse()) {
            d.places.insert(name.to_string(), (dx, y));
        }
    }
}

fn save_places(sh: &mut Shell) {
    let w = sh.w;
    let d = &mut sh.desktop;
    d.places = d.items.iter().map(|it| (file_name(&it.path), (w - it.pos.0, it.pos.1))).collect();
    let mut lines: Vec<String> = d.places.iter().map(|(n, (dx, y))| format!("{dx:.1}\t{y:.1}\t{n}")).collect();
    lines.sort();
    let p = places_file();
    let tmp = p.with_extension("tmp");
    if std::fs::write(&tmp, lines.join("\n") + "\n").is_ok() {
        let _ = std::fs::rename(tmp, p);
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn kind_of(p: &Path, is_dir: bool) -> Kind {
    if is_dir {
        return Kind::Dir;
    }
    match ext(p).as_str() {
        "desktop" => Kind::App,
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "avif" | "heic" | "svg" => Kind::Image,
        _ => Kind::File,
    }
}

fn ext(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// Top of the grid and the number of rows that fit above the Dock.
fn grid(sh: &Shell) -> (f32, usize, usize) {
    let top = sh.cfg.menubar_height + 6.0;
    let bottom = if sh.cfg.dock_autohide { sh.h - 6.0 } else { crate::dock::top(sh) - 4.0 };
    let rows = (((bottom - top) / CELL_H).floor() as usize).max(1);
    let cols = (((sh.w - 2.0 * MARGIN) / CELL_W).floor() as usize).max(1);
    (top, rows, cols)
}

fn cell(sh: &Shell, col: usize, row: usize) -> (f32, f32) {
    let (top, _, _) = grid(sh);
    (sh.w - MARGIN - (col + 1) as f32 * CELL_W, top + row as f32 * CELL_H)
}

fn occupied(at: (f32, f32), taken: &[(f32, f32)]) -> bool {
    taken.iter().any(|t| (t.0 - at.0).abs() < CELL_W * 0.5 && (t.1 - at.1).abs() < CELL_H * 0.5)
}

/// First free grid cell in reading order (columns from the right, top to bottom).
fn first_free(sh: &Shell, taken: &[(f32, f32)]) -> (f32, f32) {
    let (_, rows, cols) = grid(sh);
    for c in 0..cols {
        for r in 0..rows {
            let p = cell(sh, c, r);
            if !occupied(p, taken) {
                return p;
            }
        }
    }
    cell(sh, 0, 0)
}

fn nearest_free(sh: &Shell, want: (f32, f32), taken: &[(f32, f32)]) -> (f32, f32) {
    let (_, rows, cols) = grid(sh);
    let mut best: Option<(f32, (f32, f32))> = None;
    for c in 0..cols {
        for r in 0..rows {
            let p = cell(sh, c, r);
            if occupied(p, taken) {
                continue;
            }
            let d = (p.0 - want.0).powi(2) + (p.1 - want.1).powi(2);
            if best.is_none_or(|b| d < b.0) {
                best = Some((d, p));
            }
        }
    }
    best.map(|b| b.1).unwrap_or(want)
}

fn clamp_pos(sh: &Shell, p: (f32, f32)) -> (f32, f32) {
    let top = sh.cfg.menubar_height;
    (p.0.clamp(0.0, (sh.w - CELL_W).max(0.0)), p.1.clamp(top, (sh.h - CELL_H).max(top)))
}

/// Restored placement of a saved item, adapted to the current screen and grid mode.
fn restore(sh: &Shell, saved: (f32, f32), taken: &[(f32, f32)]) -> (f32, f32) {
    let p = clamp_pos(sh, (sh.w - saved.0, saved.1));
    if sh.cfg.desktop_snap || occupied(p, taken) {
        nearest_free(sh, p, taken)
    } else {
        p
    }
}

/// Re-read the Desktop folder when it changed; returns true when the items changed.
fn rescan(sh: &mut Shell) {
    if !sh.desktop.places_loaded {
        load_places(&mut sh.desktop);
    }
    let mut found: Vec<(PathBuf, bool, Option<SystemTime>)> = std::fs::read_dir(dir())
        .map(|rd| {
            rd.flatten()
                .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                .map(|e| {
                    let md = std::fs::metadata(e.path()).ok();
                    let is_dir = md.as_ref().is_some_and(|m| m.is_dir());
                    (e.path(), is_dir, md.and_then(|m| m.modified().ok()))
                })
                .collect()
        })
        .unwrap_or_default();
    found.sort_by_key(|(p, _, _)| file_name(p).to_lowercase());
    found.truncate(MAX_ITEMS);
    let mut old: HashMap<PathBuf, Item> = sh.desktop.items.drain(..).map(|it| (it.path.clone(), it)).collect();
    let mut items: Vec<Item> = vec![];
    let mut fresh = vec![];
    for (path, is_dir, mtime) in found {
        if let Some(mut it) = old.remove(&path) {
            if it.mtime != mtime {
                it.mtime = mtime;
                sh.desktop.icons.retain(|(p, _), _| *p != path);
                sh.desktop.thumbs.lock().unwrap_or_else(|e| e.into_inner()).remove(&path);
                if it.kind == Kind::App {
                    it.app = aqua_apps::parse_desktop(&path);
                }
            }
            items.push(it);
            continue;
        }
        let kind = kind_of(&path, is_dir);
        let app = (kind == Kind::App).then(|| aqua_apps::parse_desktop(&path)).flatten();
        let label = match &app {
            Some(a) if !a.name.is_empty() => a.name.clone(),
            _ => file_name(&path),
        };
        fresh.push(items.len());
        items.push(Item { path, label, kind, mtime, app, pos: (0.0, 0.0) });
    }
    let mut taken: Vec<(f32, f32)> =
        items.iter().enumerate().filter(|(i, _)| !fresh.contains(i)).map(|(_, it)| it.pos).collect();
    for &i in &fresh {
        let saved = sh.desktop.places.get(&file_name(&items[i].path)).copied();
        let p = match saved {
            Some(s) => restore(sh, s, &taken),
            None => first_free(sh, &taken),
        };
        items[i].pos = p;
        taken.push(p);
    }
    let keep: HashSet<PathBuf> = items.iter().map(|it| it.path.clone()).collect();
    sh.desktop.selected.retain(|p| keep.contains(p));
    sh.desktop.icons.retain(|(p, _), _| keep.contains(p));
    sh.desktop.items = items;
}

/// Re-place every item after the screen or the Dock changed size.
fn relayout(sh: &mut Shell) {
    let mut taken = vec![];
    for i in 0..sh.desktop.items.len() {
        let name = file_name(&sh.desktop.items[i].path);
        let p = match sh.desktop.places.get(&name).copied() {
            Some(s) => restore(sh, s, &taken),
            None => first_free(sh, &taken),
        };
        sh.desktop.items[i].pos = p;
        taken.push(p);
    }
}

/// Poll the Desktop folder; returns true when something has to be redrawn.
pub fn tick(sh: &mut Shell) -> bool {
    if !shown(sh) {
        let had = !sh.desktop.items.is_empty();
        if had {
            sh.desktop.items.clear();
            sh.desktop.dir_mtime = None;
            sh.desktop.last_check = None;
        }
        return had;
    }
    let mut changed = sh.desktop.thumbs_ready.swap(false, Ordering::AcqRel);
    let layout = (sh.w, grid(sh).0 + grid(sh).1 as f32 * CELL_H, sh.cfg.desktop_snap);
    if sh.desktop.last_check.is_none_or(|t| t.elapsed() > Duration::from_millis(1500)) {
        sh.desktop.last_check = Some(Instant::now());
        let m = std::fs::metadata(dir()).and_then(|m| m.modified()).ok();
        if m != sh.desktop.dir_mtime || sh.desktop.items.is_empty() && m.is_some() {
            sh.desktop.dir_mtime = m;
            rescan(sh);
            changed = true;
        }
    }
    if sh.desktop.layout != layout && !sh.desktop.pressing() {
        if sh.desktop.layout.0 > 0.0 {
            relayout(sh);
        }
        sh.desktop.layout = layout;
        changed = true;
    }
    changed
}

fn icon_rect(it: &Item) -> Rect {
    Rect::new(it.pos.0 + (CELL_W - ICON) / 2.0, it.pos.1 + 4.0, ICON, ICON)
}

fn label_rect(it: &Item) -> Rect {
    Rect::new(it.pos.0 + 2.0, it.pos.1 + ICON + 8.0, CELL_W - 4.0, CELL_H - ICON - 8.0)
}

pub fn item_at(sh: &Shell, x: f32, y: f32) -> Option<usize> {
    if !shown(sh) {
        return None;
    }
    sh.desktop.items.iter().rposition(|it| icon_rect(it).contains(x, y) || label_rect(it).inset(4.0).contains(x, y))
}

fn hit_rect(it: &Item) -> Rect {
    let a = icon_rect(it);
    let b = label_rect(it);
    Rect::new(b.x, a.y, b.w, b.bottom() - a.y)
}

fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
}

fn open_action(it: &Item) -> Action {
    match &it.app {
        Some(a) => Action::Launch(a.launch_command()),
        None => Action::Launch(crate::dock::open_cmd(&it.path.to_string_lossy())),
    }
}

/// Left press on the bare desktop.
pub fn press(sh: &mut Shell, x: f32, y: f32) -> Vec<Action> {
    if !shown(sh) {
        return vec![];
    }
    let hit = item_at(sh, x, y);
    let d = &mut sh.desktop;
    let Some(i) = hit else {
        d.selected.clear();
        d.press = Some(Press { start: (x, y), item: None, origin: vec![], moved: false });
        d.band = None;
        d.last_click = None;
        return vec![Action::Redraw];
    };
    let path = d.items[i].path.clone();
    let double = d.last_click.as_ref().is_some_and(|(p, t)| *p == path && t.elapsed() < Duration::from_millis(450));
    if double {
        d.last_click = None;
        d.press = None;
        let acts: Vec<Action> = d.items.iter().filter(|it| d.selected.contains(&it.path)).map(open_action).collect();
        return if acts.is_empty() { vec![open_action(&d.items[i])] } else { acts };
    }
    d.last_click = Some((path.clone(), Instant::now()));
    if !d.selected.contains(&path) {
        d.selected.clear();
        d.selected.insert(path);
    }
    let origin =
        d.items.iter().enumerate().filter(|(_, it)| d.selected.contains(&it.path)).map(|(j, it)| (j, it.pos)).collect();
    d.press = Some(Press { start: (x, y), item: Some(i), origin, moved: false });
    vec![Action::Redraw]
}

pub fn motion(sh: &mut Shell, x: f32, y: f32) {
    let Some(p) = sh.desktop.press.as_mut() else { return };
    let (dx, dy) = (x - p.start.0, y - p.start.1);
    if !p.moved && dx.hypot(dy) < 4.0 {
        return;
    }
    p.moved = true;
    if p.item.is_some() {
        let origin = p.origin.clone();
        for (j, o) in origin {
            let np = clamp_pos(sh, (o.0 + dx, o.1 + dy));
            sh.desktop.items[j].pos = np;
        }
    } else {
        let (sx, sy) = p.start;
        let band = Rect::new(sx.min(x), sy.min(y), (x - sx).abs(), (y - sy).abs());
        sh.desktop.band = Some(band);
        let sel: HashSet<PathBuf> =
            sh.desktop.items.iter().filter(|it| overlaps(&hit_rect(it), &band)).map(|it| it.path.clone()).collect();
        sh.desktop.selected = sel;
    }
}

pub fn release(sh: &mut Shell, x: f32, y: f32) -> Vec<Action> {
    let Some(p) = sh.desktop.press.take() else { return vec![] };
    sh.desktop.band = None;
    if p.item.is_none() || !p.moved {
        return vec![Action::Redraw];
    }
    let moving: HashSet<usize> = p.origin.iter().map(|(j, _)| *j).collect();
    let target = sh
        .desktop
        .items
        .iter()
        .enumerate()
        .rposition(|(j, it)| !moving.contains(&j) && it.kind == Kind::Dir && icon_rect(it).contains(x, y));
    if let Some(t) = target {
        for (j, o) in &p.origin {
            sh.desktop.items[*j].pos = *o;
        }
        let dest = sh.desktop.items[t].path.to_string_lossy().into_owned();
        let srcs: Vec<String> = p
            .origin
            .iter()
            .map(|(j, _)| aqua_apps::shell_quote(&sh.desktop.items[*j].path.to_string_lossy()))
            .collect();
        sh.desktop.last_check = None;
        return vec![
            Action::Launch(format!("mv -n -- {} {}", srcs.join(" "), aqua_apps::shell_quote(&dest))),
            Action::Redraw,
        ];
    }
    if sh.cfg.desktop_snap {
        let mut taken: Vec<(f32, f32)> =
            sh.desktop.items.iter().enumerate().filter(|(j, _)| !moving.contains(j)).map(|(_, it)| it.pos).collect();
        let mut order: Vec<usize> = moving.into_iter().collect();
        order.sort_by(|a, b| {
            let (pa, pb) = (sh.desktop.items[*a].pos, sh.desktop.items[*b].pos);
            (pb.0, pa.1).partial_cmp(&(pa.0, pb.1)).unwrap_or(std::cmp::Ordering::Equal)
        });
        for j in order {
            let np = nearest_free(sh, sh.desktop.items[j].pos, &taken);
            sh.desktop.items[j].pos = np;
            taken.push(np);
        }
    }
    save_places(sh);
    vec![Action::Redraw]
}

/// Desktop ▸ Clean Up: move every item to the nearest free grid cell.
pub fn clean_up(sh: &mut Shell) {
    let mut order: Vec<usize> = (0..sh.desktop.items.len()).collect();
    order.sort_by(|a, b| {
        let (pa, pb) = (sh.desktop.items[*a].pos, sh.desktop.items[*b].pos);
        (pb.0, pa.1).partial_cmp(&(pa.0, pb.1)).unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut taken = vec![];
    for j in order {
        let np = nearest_free(sh, sh.desktop.items[j].pos, &taken);
        sh.desktop.items[j].pos = np;
        taken.push(np);
    }
    save_places(sh);
}

/// Desktop ▸ Sort by Name: refill the grid in name order.
pub fn sort_by_name(sh: &mut Shell) {
    let mut taken = vec![];
    for j in 0..sh.desktop.items.len() {
        let np = first_free(sh, &taken);
        sh.desktop.items[j].pos = np;
        taken.push(np);
    }
    save_places(sh);
}

pub fn selected_paths(sh: &Shell) -> Vec<String> {
    sh.desktop
        .items
        .iter()
        .filter(|it| sh.desktop.selected.contains(&it.path))
        .map(|it| it.path.to_string_lossy().into_owned())
        .collect()
}

pub fn open_selected(sh: &Shell) -> Vec<Action> {
    sh.desktop.items.iter().filter(|it| sh.desktop.selected.contains(&it.path)).map(open_action).collect()
}

/// Right click: selects the item under the pointer; true when there is one.
pub fn secondary(sh: &mut Shell, x: f32, y: f32) -> bool {
    match item_at(sh, x, y) {
        Some(i) => {
            let p = sh.desktop.items[i].path.clone();
            if !sh.desktop.selected.contains(&p) {
                sh.desktop.selected.clear();
                sh.desktop.selected.insert(p);
            }
            true
        }
        None => {
            sh.desktop.selected.clear();
            false
        }
    }
}

fn mime_icons(p: &Path) -> &'static [&'static str] {
    match ext(p).as_str() {
        "pdf" => &["application-pdf"],
        "txt" | "md" | "log" | "rst" | "org" => &["text-x-generic"],
        "rs" | "py" | "sh" | "js" | "ts" | "c" | "h" | "cpp" | "go" | "java" | "rb" | "lua" | "toml" | "json"
        | "yaml" | "yml" | "xml" => &["text-x-script", "text-x-generic"],
        "html" | "htm" => &["text-html", "text-x-generic"],
        "zip" | "tar" | "gz" | "xz" | "bz2" | "7z" | "rar" | "zst" | "tgz" | "deb" | "rpm" => {
            &["package-x-generic", "application-x-archive"]
        }
        "mp3" | "flac" | "ogg" | "opus" | "wav" | "m4a" | "aac" => &["audio-x-generic"],
        "mp4" | "mkv" | "webm" | "avi" | "mov" | "m4v" => &["video-x-generic"],
        "doc" | "docx" | "odt" | "rtf" => &["x-office-document"],
        "xls" | "xlsx" | "ods" | "csv" => &["x-office-spreadsheet"],
        "ppt" | "pptx" | "odp" => &["x-office-presentation"],
        "iso" | "img" => &["application-x-cd-image", "media-optical"],
        "appimage" | "run" | "bin" => &["application-x-executable"],
        "ttf" | "otf" | "woff" | "woff2" => &["font-x-generic"],
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "avif" | "heic" | "svg" => {
            &["image-x-generic"]
        }
        _ => &["text-x-generic"],
    }
}

fn request_thumb(d: &Desktop, path: &Path, px: u32) {
    let mut t = d.thumbs.lock().unwrap_or_else(|e| e.into_inner());
    if t.contains_key(path) {
        return;
    }
    t.insert(path.to_path_buf(), None);
    let (thumbs, ready, path) = (d.thumbs.clone(), d.thumbs_ready.clone(), path.to_path_buf());
    std::thread::Builder::new()
        .name("aqua-thumb".into())
        .spawn(move || {
            let pm = if ext(&path) == "svg" {
                aqua_icons::theme::load_svg(&path, px)
            } else {
                aqua_gfx::read_regular(&path, 48 << 20).and_then(|b| aqua_gfx::load_image_bytes(&b))
            };
            let Some(pm) = pm else { return };
            let k = (px as f32 / pm.width().max(pm.height()) as f32).min(1.0);
            let small = if k < 1.0 {
                let (w, h) =
                    ((pm.width() as f32 * k).round().max(1.0) as u32, (pm.height() as f32 * k).round().max(1.0) as u32);
                aqua_gfx::resample(&pm, k, k, 0.0, 0.0, w, h)
            } else {
                pm
            };
            thumbs.lock().unwrap_or_else(|e| e.into_inner()).insert(path, Some(Arc::new(small)));
            ready.store(true, Ordering::Release);
        })
        .ok();
}

/// Icon of item `i` at `px` physical pixels; `bool` = final (not a placeholder).
fn icon_for(sh: &mut Shell, i: usize, px: u32) -> (Arc<Pixmap>, bool) {
    let it = &sh.desktop.items[i];
    let key = (it.path.clone(), px);
    if let Some(pm) = sh.desktop.icons.get(&key) {
        return (pm.clone(), true);
    }
    if it.kind == Kind::Image {
        request_thumb(&sh.desktop, &it.path, px);
        let t = sh.desktop.thumbs.lock().unwrap_or_else(|e| e.into_inner()).get(&it.path).cloned().flatten();
        if let Some(t) = t {
            sh.desktop.icons.insert(key, t.clone());
            return (t, true);
        }
    }
    let pm = match (it.kind, &it.app) {
        (Kind::Dir, _) => {
            sh.icons.get(&IconRequest { id: "folder".into(), name: "Folder".into(), icon: "builtin:folder".into() }, px)
        }
        (Kind::App, Some(a)) => {
            let req = IconRequest { id: a.id.clone(), name: a.name.clone(), icon: a.icon.clone() };
            sh.icons.get(&req, px)
        }
        _ => mime_icons(&it.path).iter().find_map(|n| aqua_icons::theme::lookup(n, px)).unwrap_or_else(|| {
            let mut c = Canvas::new(px as f32, px as f32, 1.0);
            let r = Rect::new(0.0, 0.0, px as f32, px as f32);
            aqua_gfx::symbols::document(&mut c, r, rgba(255, 255, 255, 0.97), rgba(120, 120, 128, 0.7));
            c.pm
        }),
    };
    let pm = Arc::new(pm);
    let thumb_pending = it.kind == Kind::Image;
    if !thumb_pending {
        sh.desktop.icons.insert(key, pm.clone());
    }
    (pm, !thumb_pending)
}

/// Label split like Finder: up to two centred lines, the second one shortened with "…".
fn label_lines(f: &aqua_gfx::Fonts, s: &str, max: f32) -> Vec<String> {
    if f.measure(s, LABEL, Weight::Medium) <= max {
        return vec![s.to_string()];
    }
    let chars: Vec<char> = s.chars().collect();
    let mut cut = 0;
    for n in 1..chars.len() {
        let head: String = chars[..n].iter().collect();
        if f.measure(&head, LABEL, Weight::Medium) > max {
            break;
        }
        cut = n;
    }
    if let Some(sp) = chars[..cut.min(chars.len())].iter().rposition(|c| c.is_whitespace() || *c == '-' || *c == '_') {
        if sp > 0 {
            cut = sp + 1;
        }
    }
    let cut = cut.max(1);
    let first: String = chars[..cut].iter().collect::<String>().trim_end().to_string();
    let rest: String = chars[cut..].iter().collect::<String>().trim_start().to_string();
    vec![first, f.ellipsize(&rest, LABEL, Weight::Medium, max)]
}

pub fn layers(sh: &mut Shell) -> Vec<Layer> {
    if !shown(sh) || sh.desktop.items.is_empty() {
        return vec![];
    }
    let px = (ICON * sh.scale).round() as u32;
    let dragging: HashSet<usize> = match &sh.desktop.press {
        Some(Press { item: Some(_), moved: true, origin, .. }) => origin.iter().map(|(j, _)| *j).collect(),
        _ => HashSet::new(),
    };
    let mut out = vec![];
    for i in 0..sh.desktop.items.len().min(u16::MAX as usize) {
        let (icon, done) = icon_for(sh, i, px);
        let it = &sh.desktop.items[i];
        let sel = sh.desktop.selected.contains(&it.path);
        let rect = Rect::new(it.pos.0, it.pos.1, CELL_W, CELL_H);
        let key = hash_of(&(&it.label, sel, it.kind, it.mtime, done, Arc::as_ptr(&icon) as usize, sh.scale.to_bits()));
        let label = it.label.clone();
        let kind = it.kind;
        let (pm, serial) = sh.cached(LayerId::Desktop(i as u16), key, CELL_W, CELL_H, |c, sh| {
            draw_item(c, sh, &label, kind, &icon, sel);
        });
        out.push(Layer {
            id: LayerId::Desktop(i as u16),
            rect,
            glass: None,
            tiles: vec![],
            content: pm,
            serial,
            opacity: if dragging.contains(&i) { 0.8 } else { 1.0 },
            zoom: 1.0,
        });
    }
    if let Some(b) = sh.desktop.band.filter(|b| b.w >= 1.0 && b.h >= 1.0) {
        let key = hash_of(&((b.w * 2.0) as i32, (b.h * 2.0) as i32));
        let (pm, serial) = sh.cached(LayerId::DesktopBand, key, b.w, b.h, |c, _| {
            let r = Rect::new(0.0, 0.0, b.w, b.h);
            c.fill_rect(r, rgba(200, 200, 210, 0.18));
            c.stroke_rrect(r.inset(0.5), 0.0, rgba(235, 235, 245, 0.55), 1.0);
        });
        out.push(Layer {
            id: LayerId::DesktopBand,
            rect: b,
            glass: None,
            tiles: vec![],
            content: pm,
            serial,
            opacity: 1.0,
            zoom: 1.0,
        });
    }
    out
}

fn draw_item(c: &mut Canvas, sh: &Shell, label: &str, kind: Kind, icon: &Pixmap, sel: bool) {
    let f = sh.fonts.clone();
    let ir = Rect::new((CELL_W - ICON) / 2.0, 4.0, ICON, ICON);
    if sel {
        c.fill_continuous(ir.inset(-4.0), 8.0, rgba(0, 0, 0, 0.28));
        c.stroke_continuous(ir.inset(-4.0), 8.0, rgba(255, 255, 255, 0.28), 1.0);
    }
    let (iw, ih) = (icon.width() as f32 / c.scale, icon.height() as f32 / c.scale);
    let k = (ICON / iw.max(ih)).min(1.0);
    let (w, h) = (iw * k, ih * k);
    let dst = Rect::new(ir.x + (ICON - w) / 2.0, ir.y + (ICON - h) / 2.0, w, h);
    if kind == Kind::Image {
        let frame = dst.inset(-2.0);
        c.fill_rect(frame.translate(0.0, 1.0), rgba(0, 0, 0, 0.25));
        c.fill_rect(frame, rgba(255, 255, 255, 1.0));
    }
    c.draw_pixmap(icon, dst, 1.0);
    let max = CELL_W - 10.0;
    let lines = label_lines(&f, label, max);
    let lh = 15.0;
    let y0 = ICON + 10.0;
    if sel {
        for (n, l) in lines.iter().enumerate() {
            let tw = f.measure(l, LABEL, Weight::Medium);
            let r = Rect::new((CELL_W - tw) / 2.0 - 4.0, y0 + n as f32 * lh, tw + 8.0, lh);
            c.fill_rrect(r, 4.0, style::accent(1.0));
        }
    } else {
        let mut sc = Canvas::new(CELL_W, CELL_H, c.scale);
        for (n, l) in lines.iter().enumerate() {
            let r = Rect::new(0.0, y0 + n as f32 * lh + 1.0, CELL_W, lh);
            sc.text_in(&f, r, 0.5, LABEL, Weight::Medium, rgba(0, 0, 0, 0.95), l);
        }
        aqua_gfx::blur::blur(&mut sc.pm, 1.6 * c.scale);
        c.blit(&sc.pm, 0.0, 0.0);
        c.blit(&sc.pm, 0.0, 0.0);
    }
    for (n, l) in lines.iter().enumerate() {
        let r = Rect::new(0.0, y0 + n as f32 * lh, CELL_W, lh);
        c.text_in(&f, r, 0.5, LABEL, Weight::Medium, rgba(255, 255, 255, 1.0), l);
    }
}
