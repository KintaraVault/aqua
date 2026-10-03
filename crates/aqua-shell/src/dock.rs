//! The Dock: pinned + running apps, separator, Downloads stack, Trash.
use crate::{hash_of, style, Action, Layer, LayerId, Shell};
use aqua_config::{metrics, Config};
use aqua_gfx::{rgba, Rect, Weight};
use aqua_icons::IconRequest;

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    App,
    Separator,
    Folder,
    Trash,
    Launchpad,
    /// A minimised window (id) shown as a live thumbnail right of the separator.
    Minimized(u64),
}

#[derive(Clone, Debug)]
pub struct DockItem {
    pub name: String,
    pub app: String,
    pub exec: String,
    pub icon: IconRequest,
    pub kind: Kind,
    pub pinned: bool,
    /// Extra app ids / binaries that belong to this item.
    pub aliases: Vec<String>,
}

impl DockItem {
    pub fn matches(&self, app_id: &str) -> bool {
        if self.kind != Kind::App || app_id.is_empty() {
            return false;
        }
        let a = app_id.to_lowercase();
        let bin = self.exec.split_whitespace().next().unwrap_or("").rsplit('/').next().unwrap_or("").to_lowercase();
        a == self.app.to_lowercase()
            || (!bin.is_empty() && (a == bin || a.ends_with(&format!(".{bin}"))))
            || a == self.name.to_lowercase()
            || self.aliases.iter().any(|al| {
                let al = al.to_lowercase();
                a == al || a.ends_with(&format!(".{al}"))
            })
    }
}

/// A press on a Dock icon: becomes a click on release, or a drag once moved.
#[derive(Clone, Debug)]
pub struct Press {
    pub key: String,
    pub x0: f32,
    pub y0: f32,
    pub dragging: bool,
    pub item: DockItem,
    /// Insertion index (in the app area, before the separator) while dragging.
    pub to: usize,
    /// Dragged far enough off the Dock to be removed on release.
    pub outside: bool,
    /// Pointer offset from the icon centre when the drag started.
    pub off: (f32, f32),
}

/// The dragged icon after release: flies into its slot, or puffs away when removed.
#[derive(Clone, Debug)]
pub struct Fly {
    pub item: DockItem,
    pub key: String,
    pub from: (f32, f32),
    pub t: f32,
    pub remove: bool,
}

/// An item that left the Dock: it keeps its place while it shrinks away.
#[derive(Clone, Debug)]
struct Ghost {
    key: String,
    item: DockItem,
    /// Key of the item it followed (where it stays while shrinking).
    after: Option<String>,
    /// Not drawn (the dragged icon is shown by the drag layer instead).
    hidden: bool,
}

/// Appear / disappear / move animation state (survives config reloads).
#[derive(Default)]
pub struct Anim {
    init: bool,
    /// Presence 0…1 of every key (width and icon scale).
    pres: std::collections::HashMap<String, f32>,
    ghosts: Vec<Ghost>,
    /// Horizontal offsets easing to 0 after a reorder (FLIP).
    offs: std::collections::HashMap<String, f32>,
    last: Vec<(String, DockItem)>,
    shown_cx: std::collections::HashMap<String, f32>,
}

/// Appear / disappear duration (seconds).
const APPEAR: f32 = 0.28;
const FLY: f32 = 0.26;

#[derive(Default)]
pub struct Dock {
    pub items: Vec<DockItem>,
    pub hover: Option<usize>,
    /// Bouncing icons: (item key, elapsed seconds).
    pub bounce: Vec<(String, f32)>,
    pub press: Option<Press>,
    pub fly: Option<Fly>,
    pub anim: Anim,
    /// Pointer x while magnifying, and the eased magnification amount.
    pub mag_x: Option<f32>,
    pub mag_t: f32,
    last_mag_x: f32,
    /// App ids of running apps in the order they first opened a window, and minimised
    /// window ids in the order they were minimised: Dock icons keep their place
    /// instead of following the focus (stacking) order.
    pub running_order: Vec<String>,
    pub min_order: Vec<u64>,
}

/// Pick the first program of an `a|b|c` alternatives list that exists in $PATH.
pub fn resolve_exec(spec: &str) -> String {
    for alt in spec.split('|') {
        let alt = alt.trim();
        let bin = alt.split_whitespace().next().unwrap_or("");
        if bin.is_empty() {
            continue;
        }
        if bin.contains('/') && std::path::Path::new(bin).exists() {
            return alt.into();
        }
        if let Some(path) = std::env::var_os("PATH") {
            if std::env::split_paths(&path).any(|p| p.join(bin).is_file()) {
                return alt.into();
            }
        }
        if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) {
            if dir.join(bin).is_file() {
                return alt.replacen(bin, &dir.join(bin).to_string_lossy(), 1);
            }
        }
    }
    String::new()
}

/// A pinned item lists every program it *could* run ("Safari": firefox, chromium, …);
/// once the binary is resolved, only that program's ids may group windows under it —
/// otherwise Zen, Chromium and Firefox windows all landed on the same Dock icon.
fn own_aliases_only(it: &mut DockItem) {
    use aqua_config::apple_icons::canon;
    let bin = canon(it.exec.split_whitespace().next().unwrap_or(""));
    if bin.is_empty() {
        return;
    }
    it.aliases.retain(|a| {
        let a = canon(a);
        a == bin
            || a.ends_with(&format!(".{bin}"))
            || bin.starts_with(&format!("{a}-"))
            || a.starts_with(&format!("{bin}-"))
            || a.rsplit('.').next() == Some(bin.as_str())
            || a.replace('.', "-").ends_with(&format!("-{bin}"))
    });
}

/// Ids identifying the app behind a pinned item (binary, desktop id, icon, WM class).
pub(crate) fn owner_ids(it: &DockItem, apps: &[aqua_apps::App]) -> Vec<String> {
    use aqua_config::apple_icons::canon;
    let bin = it.exec.split_whitespace().next().unwrap_or("").rsplit('/').next().unwrap_or("").to_string();
    let mut v = vec![canon(&bin)];
    v.extend(it.aliases.iter().map(|a| canon(a)));
    if let Some(a) = apps
        .iter()
        .find(|a| a.command().split_whitespace().next().unwrap_or("").rsplit('/').next() == Some(bin.as_str()))
    {
        v.push(canon(&a.id));
        v.push(canon(&a.icon));
        if let Some(w) = &a.wm_class {
            v.push(canon(w));
        }
    }
    v.retain(|s| !s.is_empty());
    v.dedup();
    v
}

/// One owner per counterpart: the Dock's pinned program, else the first
/// installed app that maps to it.
pub fn icon_owners(dock: &Dock, apps: &[aqua_apps::App]) -> Vec<(usize, Vec<String>)> {
    use aqua_config::apple_icons::{canon, lookup_index};
    let mut out: Vec<(usize, Vec<String>)> = vec![];
    for it in dock.items.iter().filter(|i| i.kind == Kind::App && !i.exec.is_empty()) {
        let bin = it.exec.split_whitespace().next().unwrap_or("").rsplit('/').next().unwrap_or("");
        if let Some(g) = lookup_index(bin, bin) {
            if !out.iter().any(|(i, _)| *i == g) {
                out.push((g, owner_ids(it, apps)));
            }
        }
    }
    for a in apps {
        if let Some(g) = lookup_index(&a.id, &a.icon) {
            if !out.iter().any(|(i, _)| *i == g) {
                let mut ids = vec![canon(&a.id), canon(&a.icon)];
                if let Some(w) = &a.wm_class {
                    ids.push(canon(w));
                }
                if let Some(b) = a.exec.split_whitespace().next() {
                    ids.push(canon(b));
                }
                ids.retain(|s| !s.is_empty());
                out.push((g, ids));
            }
        }
    }
    out
}

fn kind_is_app(it: &DockItem) -> bool {
    it.kind == Kind::App && !it.exec.is_empty()
}

/// A pinned app ("Safari") that actually runs a Linux program (firefox): when the
/// user turned the branded replacement off for that program, show its real name and icon.
fn real_app_identity(it: &mut DockItem, cfg: &Config, apps: &[aqua_apps::App]) {
    use aqua_config::apple_icons::{canon, lookup, Policy};
    let bin = it.exec.split_whitespace().next().unwrap_or("").rsplit('/').next().unwrap_or("").to_string();
    if bin.is_empty() || bin.starts_with("aqua-") || lookup(&bin, &bin).is_none() {
        return;
    }
    let desktop = apps.iter().find(|a| {
        let ab = a.command();
        let ab = ab.split_whitespace().next().unwrap_or("").rsplit('/').next().unwrap_or("");
        ab == bin || canon(&a.id) == canon(&bin) || canon(&a.id).ends_with(&format!(".{}", canon(&bin)))
    });
    let (id, icon) = match desktop {
        Some(a) => (a.id.clone(), a.icon.clone()),
        None => (bin.clone(), bin.clone()),
    };
    if Policy::from_config(cfg).allows(&id, &icon) || Policy::from_config(cfg).allows(&bin, &bin) {
        return;
    }
    let name = match desktop {
        Some(a) => a.name.clone(),
        None => {
            let mut s = bin.replace(['-', '_'], " ");
            if let Some(f) = s.get(0..1) {
                s = f.to_uppercase() + &s[1..];
            }
            s
        }
    };
    if !it.aliases.iter().any(|a| a == &it.app) {
        it.aliases.push(it.app.clone());
    }
    it.aliases.push(id.clone());
    it.aliases.push(bin);
    it.name = name.clone();
    it.icon = IconRequest { id, name, icon };
}

impl Dock {
    pub fn new(cfg: &Config, apps: &[aqua_apps::App]) -> Self {
        let mut items = vec![];
        for d in &cfg.dock {
            let kind = if d.app == "launchpad" { Kind::Launchpad } else { Kind::App };
            let mut exec = resolve_exec(&d.exec);
            if exec.is_empty() {
                if let Some(a) = apps.iter().find(|a| a.id == d.app) {
                    exec = a.command();
                }
            }
            let mut item = DockItem {
                name: d.name.clone(),
                app: d.app.clone(),
                exec,
                icon: IconRequest { id: d.app.clone(), name: d.name.clone(), icon: d.icon.clone() },
                kind,
                pinned: true,
                aliases: d.ids.clone(),
            };
            if kind_is_app(&item) {
                own_aliases_only(&mut item);
                real_app_identity(&mut item, cfg, apps);
            }
            items.push(item);
        }
        let sep = |k: Kind, n: &str, icon: &str| DockItem {
            name: n.into(),
            app: String::new(),
            exec: String::new(),
            icon: IconRequest { id: n.to_lowercase(), name: n.into(), icon: icon.into() },
            kind: k,
            pinned: true,
            aliases: vec![],
        };
        items.push(sep(Kind::Separator, "", ""));
        items.push(sep(Kind::Folder, "Downloads", "builtin:downloads"));
        items.push(sep(Kind::Trash, "Trash", "builtin:trash"));
        Self { items, ..Default::default() }
    }

    /// Track launch / minimise order (called whenever the window list changes).
    pub fn track_windows(&mut self, wins: &[crate::WindowInfo]) {
        for w in wins {
            if !w.app_id.is_empty() && !self.running_order.contains(&w.app_id) {
                self.running_order.push(w.app_id.clone());
            }
            if w.minimized && !self.min_order.contains(&w.id) {
                self.min_order.push(w.id);
            }
        }
        self.running_order.retain(|a| wins.iter().any(|w| &w.app_id == a));
        self.min_order.retain(|id| wins.iter().any(|w| w.id == *id && w.minimized));
    }

    /// Keep the state that must survive a rebuild (config reload).
    pub fn carry_over(&mut self, old: &mut Dock) {
        self.running_order = std::mem::take(&mut old.running_order);
        self.min_order = std::mem::take(&mut old.min_order);
        self.anim = std::mem::take(&mut old.anim);
        self.fly = old.fly.take();
        self.bounce = std::mem::take(&mut old.bounce);
        self.mag_x = old.mag_x;
        self.mag_t = old.mag_t;
        self.last_mag_x = old.last_mag_x;
        self.hover = old.hover;
    }

    pub fn dragging(&self) -> bool {
        self.press.as_ref().is_some_and(|p| p.dragging)
    }

    pub fn animate(&mut self, dt: f32) -> bool {
        for b in &mut self.bounce {
            b.1 += dt;
        }
        self.bounce.retain(|b| b.1 < 1.5);
        if let Some(x) = self.mag_x {
            self.last_mag_x = x;
        }
        let target = if self.mag_x.is_some() { 1.0 } else { 0.0 };
        let mut anim = !self.bounce.is_empty();
        if (self.mag_t - target).abs() > 0.001 {
            self.mag_t += (target - self.mag_t) * (1.0 - (-14.0 * dt).exp());
            if (self.mag_t - target).abs() < 0.01 {
                self.mag_t = target;
            }
            anim = true;
        }
        anim
    }
}

/// Visible items: pinned items plus running unpinned apps (inserted before the separator).
fn visible(sh: &Shell) -> Vec<DockItem> {
    let mut v: Vec<DockItem> = sh.dock.items.clone();
    let sep_idx = v.iter().position(|i| i.kind == Kind::Separator).unwrap_or(v.len());
    let mut extra = vec![];
    let mut wins: Vec<&crate::WindowInfo> = sh.windows.iter().collect();
    if sh.cfg.dock_keep_order {
        let pos =
            |w: &crate::WindowInfo| sh.dock.running_order.iter().position(|a| a == &w.app_id).unwrap_or(usize::MAX);
        wins.sort_by_key(|w| pos(w));
    }
    for w in wins {
        if w.app_id.is_empty()
            || v.iter().any(|i| i.matches(&w.app_id))
            || extra.iter().any(|i: &DockItem| i.matches(&w.app_id))
        {
            continue;
        }
        let app = aqua_apps::match_app_id(&sh.apps, &w.app_id);
        extra.push(DockItem {
            name: sh.app_display_name(&w.app_id),
            app: w.app_id.clone(),
            exec: app.map(|a| a.command()).unwrap_or_default(),
            icon: IconRequest {
                id: w.app_id.clone(),
                name: app.map(|a| a.name.clone()).unwrap_or_else(|| sh.app_display_name(&w.app_id)),
                icon: app.map(|a| a.icon.clone()).unwrap_or_else(|| w.app_id.clone()),
            },
            kind: Kind::App,
            pinned: false,
            aliases: vec![],
        });
    }
    for (k, e) in extra.into_iter().enumerate() {
        v.insert(sep_idx + k, e);
    }
    let first = v.iter().position(|i| i.kind == Kind::Separator).map(|p| p + 1).unwrap_or(v.len());
    let mut mins: Vec<&crate::WindowInfo> = sh.windows.iter().filter(|w| w.minimized).collect();
    mins.sort_by_key(|w| sh.dock.min_order.iter().position(|id| *id == w.id).unwrap_or(usize::MAX));
    for (at, w) in (first..).zip(mins) {
        let app = aqua_apps::match_app_id(&sh.apps, &w.app_id);
        v.insert(
            at,
            DockItem {
                name: if w.title.is_empty() { sh.app_display_name(&w.app_id) } else { w.title.clone() },
                app: w.app_id.clone(),
                exec: String::new(),
                icon: IconRequest {
                    id: w.app_id.clone(),
                    name: app.map(|a| a.name.clone()).unwrap_or_else(|| sh.app_display_name(&w.app_id)),
                    icon: app.map(|a| a.icon.clone()).unwrap_or_else(|| w.app_id.clone()),
                },
                kind: Kind::Minimized(w.id),
                pinned: false,
                aliases: vec![],
            },
        );
    }
    v
}

/// Stable identity of a Dock item across rebuilds (animations follow it).
pub fn key_of(it: &DockItem) -> String {
    match &it.kind {
        Kind::App if !it.exec.is_empty() => format!("app:{}", it.exec),
        Kind::App => format!("app:{}", it.app.to_lowercase()),
        Kind::Minimized(id) => format!("min:{id}"),
        Kind::Separator => "sep".into(),
        Kind::Folder => "folder".into(),
        Kind::Trash => "trash".into(),
        Kind::Launchpad => "launchpad".into(),
    }
}

/// Visible items in display order, with the dragged icon moved to its drop place
/// (or taken out while it hovers off the Dock).
fn ordered(sh: &Shell) -> Vec<(String, DockItem)> {
    let mut v: Vec<(String, DockItem)> = vec![];
    for it in visible(sh) {
        let mut k = key_of(&it);
        if v.iter().any(|(o, _)| *o == k) {
            let n = v.iter().filter(|(o, _)| o.starts_with(&k)).count();
            k = format!("{k}#{n}");
        }
        v.push((k, it));
    }
    if let Some(p) = sh.dock.press.as_ref().filter(|p| p.dragging) {
        if let Some(pos) = v.iter().position(|(k, _)| *k == p.key) {
            let it = v.remove(pos);
            if !p.outside {
                let sep = v.iter().position(|(_, i)| i.kind == Kind::Separator).unwrap_or(v.len());
                v.insert(p.to.min(sep), it);
            }
        }
    }
    v
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Advance the appear / disappear / reorder / drop animations (once per frame).
pub fn sync(sh: &mut Shell, dt: f32) -> bool {
    let base = ordered(sh);
    let keys: Vec<String> = base.iter().map(|(k, _)| k.clone()).collect();
    let instant = sh.cfg.reduce_motion;
    let hidden_key = sh
        .dock
        .press
        .as_ref()
        .filter(|p| p.dragging)
        .map(|p| p.key.clone())
        .or_else(|| sh.dock.fly.as_ref().map(|f| f.key.clone()));
    let a = &mut sh.dock.anim;
    if !a.init {
        a.init = true;
        a.pres = keys.iter().map(|k| (k.clone(), 1.0)).collect();
        a.last = base;
        return false;
    }
    for k in &keys {
        a.pres.entry(k.clone()).or_insert(if instant { 1.0 } else { 0.0 });
        a.ghosts.retain(|g| &g.key != k);
    }
    let last = std::mem::take(&mut a.last);
    for (i, (k, it)) in last.iter().enumerate() {
        if !keys.contains(k) && !a.ghosts.iter().any(|g| &g.key == k) {
            if instant {
                a.pres.remove(k);
                continue;
            }
            let after = (i > 0).then(|| last[i - 1].0.clone());
            a.ghosts.push(Ghost {
                key: k.clone(),
                item: it.clone(),
                after,
                hidden: hidden_key.as_deref() == Some(k.as_str()),
            });
        }
    }
    let old_common: Vec<&String> = last.iter().map(|(k, _)| k).filter(|k| keys.contains(k)).collect();
    let new_common: Vec<&String> = keys.iter().filter(|k| last.iter().any(|(o, _)| o == *k)).collect();
    let reordered = old_common != new_common;
    a.last = base;
    let step = dt / APPEAR;
    for k in &keys {
        if let Some(p) = a.pres.get_mut(k) {
            *p = (*p + step).min(1.0);
        }
    }
    for g in &a.ghosts {
        if let Some(p) = a.pres.get_mut(&g.key) {
            *p -= step;
        }
    }
    let pres = &a.pres;
    a.ghosts.retain(|g| pres.get(&g.key).is_some_and(|p| *p > 0.0));
    let ghost_keys: Vec<String> = a.ghosts.iter().map(|g| g.key.clone()).collect();
    a.pres.retain(|k, _| keys.contains(k) || ghost_keys.contains(k));
    let k = 1.0 - (-15.0 * dt).exp();
    for o in a.offs.values_mut() {
        *o -= *o * k;
    }
    a.offs.retain(|_, o| o.abs() > 0.3);
    if let Some(f) = &mut sh.dock.fly {
        f.t += dt / FLY;
        if f.t >= 1.0 || instant {
            sh.dock.fly = None;
        }
    }
    let (_, geo) = geometry(sh);
    let a = &mut sh.dock.anim;
    if reordered && !instant {
        for (i, key) in geo.keys.iter().enumerate() {
            if geo.ghost[i] {
                continue;
            }
            if let Some(old) = a.shown_cx.get(key) {
                let d = old - geo.slots[i].cx();
                if d.abs() > 0.5 {
                    a.offs.insert(key.clone(), d);
                }
            }
        }
    }
    a.shown_cx = geo
        .keys
        .iter()
        .enumerate()
        .map(|(i, k)| (k.clone(), geo.slots[i].cx() + a.offs.get(k).copied().unwrap_or(0.0)))
        .collect();
    let busy = a.pres.values().any(|p| *p < 1.0) || !a.ghosts.is_empty() || !a.offs.is_empty() || sh.dock.fly.is_some();
    busy
}

/// Dock slots of minimised windows: (window id, slot rect).
pub fn minimized_slots(sh: &Shell) -> Vec<(u64, Rect)> {
    let (items, geo) = geometry(sh);
    items
        .iter()
        .enumerate()
        .filter(|(i, _)| !geo.ghost[*i])
        .filter_map(|(i, it)| {
            if let Kind::Minimized(id) = it.kind {
                Some((id, geo.slots[i].translate(geo.dx[i], 0.0)))
            } else {
                None
            }
        })
        .collect()
}

/// Centre of the minimised-window slot for `id` (genie target).
pub fn minimized_center(sh: &Shell, id: u64) -> Option<(f32, f32)> {
    minimized_slots(sh).into_iter().find(|(i, _)| *i == id).map(|(_, r)| (r.cx(), r.cy()))
}

/// App badges drawn over the minimised-window thumbnails (the compositor draws the
/// thumbnails themselves from the client textures, between the Dock and this layer).
pub fn badges_layer(sh: &mut Shell) -> Option<Layer> {
    let (items, geo) = geometry(sh);
    let mins: Vec<(DockItem, Rect)> = items
        .into_iter()
        .enumerate()
        .filter(|(i, it)| matches!(it.kind, Kind::Minimized(_)) && !geo.ghost[*i])
        .map(|(i, it)| (it, geo.slots[i].translate(geo.dx[i], 0.0)))
        .collect();
    if mins.is_empty() {
        return None;
    }
    let rect = geo.rect;
    let key = hash_of(&(
        mins.iter().map(|(i, r)| (i.app.clone(), (r.x * 4.0) as i32, (r.w * 4.0) as i32)).collect::<Vec<_>>(),
        (rect.x * 4.0) as i32,
        rect.w as i32,
    ));
    let (pm, serial) = sh.cached(LayerId::DockBadges, key, rect.w, rect.h, |c, sh| {
        for (it, slot) in &mins {
            let b = slot.w * 0.42;
            let px = (b * 1.2 * sh.scale).round() as u32;
            let icon = sh.icons.get(&it.icon, px);
            c.draw_pixmap(
                &icon,
                Rect::new(slot.right() - b * 0.92 - rect.x, slot.bottom() - b * 0.92 - rect.y, b, b),
                1.0,
            );
        }
    });
    Some(Layer {
        id: LayerId::DockBadges,
        rect,
        glass: None,
        tiles: vec![],
        content: pm,
        serial,
        opacity: 1.0,
        zoom: 1.0,
    })
}

pub struct Geo {
    pub rect: Rect,
    /// Layout slots (hit testing); draw at `slot.translate(dx[i], 0)`.
    pub slots: Vec<Rect>,
    pub keys: Vec<String>,
    /// Eased presence (appearing / disappearing items are narrower and smaller).
    pub pres: Vec<f32>,
    /// Items that already left the Dock and are shrinking away.
    pub ghost: Vec<bool>,
    /// Reorder offsets.
    pub dx: Vec<f32>,
}

pub fn geometry(sh: &Shell) -> (Vec<DockItem>, Geo) {
    let a = &sh.dock.anim;
    let mut list: Vec<(String, DockItem, bool)> = ordered(sh).into_iter().map(|(k, it)| (k, it, false)).collect();
    for g in &a.ghosts {
        let at = g.after.as_ref().and_then(|k| list.iter().position(|(o, _, _)| o == k)).map(|p| p + 1).unwrap_or(0);
        list.insert(at.min(list.len()), (g.key.clone(), g.item.clone(), true));
    }
    let pres: Vec<f32> = list.iter().map(|(k, _, _)| smooth(a.pres.get(k).copied().unwrap_or(1.0))).collect();
    let ghost: Vec<bool> = list.iter().map(|(_, _, g)| *g).collect();
    let dx: Vec<f32> = list.iter().map(|(k, _, _)| a.offs.get(k).copied().unwrap_or(0.0)).collect();
    let keys: Vec<String> = list.iter().map(|(k, _, _)| k.clone()).collect();
    let items: Vec<DockItem> = list.into_iter().map(|(_, it, _)| it).collect();
    let s = sh.cfg.dock_icon_size;
    let pad = metrics::DOCK_PADDING;
    let gap = metrics::DOCK_GAP;
    let sep_w = 14.0;
    let base_w = |i: usize| (if items[i].kind == Kind::Separator { sep_w } else { s }) * pres[i];
    let gap_of = |i: usize| if i == 0 { 0.0 } else { gap * pres[i] };
    let mut w = pad * 2.0;
    for i in 0..items.len() {
        w += base_w(i) + gap_of(i);
    }
    let h = s + pad * 2.0;
    let x0 = (sh.w - w) / 2.0;
    let y0 = sh.h - h - metrics::DOCK_BOTTOM_MARGIN + sh.dock_offset();
    let mut centers = vec![];
    let mut x = x0 + pad;
    for i in 0..items.len() {
        x += gap_of(i);
        let iw = base_w(i);
        centers.push(x + iw / 2.0);
        x += iw;
    }
    let m = (sh.cfg.dock_magnification - 1.0).max(0.0) * sh.dock.mag_t;
    let px = sh.dock.mag_x.unwrap_or(sh.dock.last_mag_x);
    let radius = s * 2.6;
    let scale: Vec<f32> = items
        .iter()
        .zip(&centers)
        .map(|(it, c)| {
            if it.kind == Kind::Separator || m <= 0.0 {
                return 1.0;
            }
            let d = ((c - px) / radius).abs();
            if d >= 1.0 {
                1.0
            } else {
                1.0 + m * (0.5 + 0.5 * (d * std::f32::consts::PI).cos())
            }
        })
        .collect();
    let mut tw = pad * 2.0;
    for i in 0..items.len() {
        tw += base_w(i) * scale[i] + gap_of(i);
    }
    let mx0 = (sh.w - tw) / 2.0;
    let mut slots = vec![];
    let mut x = mx0 + pad;
    for (i, it) in items.iter().enumerate() {
        x += gap_of(i);
        let iw = base_w(i) * scale[i];
        let ih = if it.kind == Kind::Separator { s } else { s * scale[i] };
        slots.push(Rect::new(x, y0 + pad + s - ih, iw, ih));
        x += iw;
    }
    (items, Geo { rect: Rect::new(mx0, y0, tw, h), slots, keys, pres, ghost, dx })
}

pub fn is_running(sh: &Shell, it: &DockItem) -> bool {
    sh.windows.iter().any(|w| it.matches(&w.app_id))
}

pub fn layer(sh: &mut Shell) -> Layer {
    let (items, geo) = geometry(sh);
    let maxm = sh.cfg.dock_magnification.max(1.0);
    let over = sh.cfg.dock_icon_size * (0.5 + (maxm - 1.0));
    let rect = Rect::new(geo.rect.x, geo.rect.y - over, geo.rect.w, geo.rect.h + over);
    let running: Vec<bool> = items.iter().enumerate().map(|(i, it)| !geo.ghost[i] && is_running(sh, it)).collect();
    let bounce: Vec<(String, i32)> = sh.dock.bounce.iter().map(|(k, t)| (k.clone(), (t * 60.0) as i32)).collect();
    let dark = sh.style.is_dark_glass(geo.rect);
    let hidden: Option<String> = sh
        .dock
        .press
        .as_ref()
        .filter(|p| p.dragging)
        .map(|p| p.key.clone())
        .or_else(|| sh.dock.fly.as_ref().filter(|f| !f.remove).map(|f| f.key.clone()));
    let ghost_hidden: Vec<String> = sh.dock.anim.ghosts.iter().filter(|g| g.hidden).map(|g| g.key.clone()).collect();
    let draw: Vec<bool> =
        geo.keys.iter().map(|k| hidden.as_deref() != Some(k.as_str()) && !ghost_hidden.contains(k)).collect();
    let magk: Vec<(i32, i32, i32)> = geo
        .slots
        .iter()
        .enumerate()
        .map(|(i, r)| ((r.w * 4.0) as i32, ((r.x + geo.dx[i] - geo.rect.x) * 4.0) as i32, (geo.pres[i] * 100.0) as i32))
        .collect();
    let key = hash_of(&(
        &geo.keys,
        &running,
        &bounce,
        rect.w as i32,
        (rect.x * 4.0) as i32,
        magk,
        sh.icons_serial(),
        dark,
        &draw,
    ));
    let s = sh.cfg.dock_icon_size;
    let (pm, serial) = sh.cached(LayerId::Dock, key, rect.w, rect.h, |c, sh| {
        let px = (s * 1.16 * maxm * sh.scale).round() as u32;
        for (i, (it, slot)) in items.iter().zip(&geo.slots).enumerate() {
            let slot = slot.translate(geo.dx[i] - rect.x, -rect.y);
            let e = geo.pres[i];
            if matches!(it.kind, Kind::Minimized(_)) || !draw[i] || e <= 0.01 {
                continue;
            }
            if it.kind == Kind::Separator {
                c.fill_rect(Rect::new(slot.cx() - 0.5, slot.y + 4.0, 1.0, slot.h - 8.0), style::separator(dark));
                continue;
            }
            let mut dy = 0.0;
            if let Some((_, t)) = sh.dock.bounce.iter().find(|(b, _)| *b == geo.keys[i]) {
                let ph = (t / 0.5).min(3.0);
                dy = -((ph * std::f32::consts::PI).sin().abs()) * s * 0.4 * (1.0 - ph / 3.0);
            }
            let icon = sh.icons.get(&it.icon, px);
            let d = slot.h * 1.16 * e;
            let cy = slot.bottom() - slot.h / 2.0 * e;
            c.draw_pixmap(&icon, Rect::new(slot.cx() - d / 2.0, cy - d / 2.0 + dy, d, d), e.min(1.0));
            if running[i] {
                let col = if dark { rgba(255, 255, 255, 0.85 * e) } else { rgba(0, 0, 0, 0.72 * e) };
                c.fill_circle(slot.cx(), slot.bottom() + metrics::DOCK_PADDING * 0.55, 2.0, col);
            }
        }
    });
    let glass = style::glass_dock(&sh.cfg.glass);
    let mut tiles = vec![(geo.rect, glass)];
    let clear =
        aqua_icons::look::Style::from_config(&sh.cfg.icon_style, sh.style.dark) == aqua_icons::look::Style::Clear;
    if clear && sh.cfg.icon_glass {
        for (i, (it, slot)) in items.iter().zip(&geo.slots).enumerate() {
            let e = geo.pres[i];
            if it.kind == Kind::Separator || matches!(it.kind, Kind::Minimized(_)) || !draw[i] || e <= 0.05 {
                continue;
            }
            let slot = slot.translate(geo.dx[i], 0.0);
            let mut dy = 0.0;
            if let Some((_, t)) = sh.dock.bounce.iter().find(|(b, _)| *b == geo.keys[i]) {
                let ph = (t / 0.5).min(3.0);
                dy = -((ph * std::f32::consts::PI).sin().abs()) * s * 0.4 * (1.0 - ph / 3.0);
            }
            let d = slot.h * 1.16 * e;
            let cy = slot.bottom() - slot.h / 2.0 * e;
            let pw = d * 824.0 / 1024.0;
            let plate = Rect::new(slot.cx() - pw / 2.0 + 0.5, cy - pw / 2.0 + dy + 0.5, pw - 1.0, pw - 1.0);
            tiles.push((plate, style::glass_icon(&sh.cfg.glass, pw - 1.0)));
        }
    }
    Layer { id: LayerId::Dock, rect, glass: None, tiles, content: pm, serial, opacity: 1.0, zoom: 1.0 }
}

pub fn tooltip_layer(sh: &mut Shell) -> Option<Layer> {
    let i = sh.dock.hover?;
    if sh.has_modal() || sh.dock.press.is_some() || sh.dock.fly.is_some() {
        return None;
    }
    let (items, geo) = geometry(sh);
    let it = items.get(i)?;
    if it.kind == Kind::Separator || geo.ghost[i] {
        return None;
    }
    let slot = geo.slots[i].translate(geo.dx[i], 0.0);
    let tw = sh.fonts.measure(&it.name, 13.0, Weight::Regular) + 24.0;
    let rect = Rect::new(slot.cx() - tw / 2.0, slot.y.min(geo.rect.y) - 38.0, tw, 26.0);
    let name = it.name.clone();
    let dark = sh.style.dark;
    let key = hash_of(&(&name, dark));
    let (pm, serial) = sh.cached(LayerId::DockTooltip, key, rect.w, rect.h, |c, sh| {
        let f = sh.fonts.clone();
        c.text_in(
            &f,
            Rect::new(0.0, 0.0, rect.w, rect.h),
            0.5,
            13.0,
            Weight::Regular,
            style::text_primary(dark),
            &name,
        );
    });
    let g = style::glass_menu(&sh.cfg.glass, dark);
    Some(Layer {
        id: LayerId::DockTooltip,
        rect,
        glass: Some(aqua_config::GlassStyle { radius: 13.0, ..g }),
        tiles: vec![],
        content: pm,
        serial,
        opacity: 1.0,
        zoom: 1.0,
    })
}

pub fn hit(sh: &Shell, x: f32, y: f32) -> bool {
    let (_, geo) = geometry(sh);
    geo.rect.contains(x, y)
}

fn draggable(it: &DockItem) -> bool {
    matches!(it.kind, Kind::App | Kind::Launchpad)
}

/// Pointer moved while a Dock icon is pressed: start / update the drag.
fn drag_motion(sh: &mut Shell, x: f32, y: f32) {
    let s = sh.cfg.dock_icon_size;
    let Some(p) = sh.dock.press.as_ref() else { return };
    if !p.dragging {
        if (x - p.x0).hypot(y - p.y0) < 6.0 || !draggable(&p.item) {
            return;
        }
        let (_, geo) = geometry(sh);
        let Some(i) = geo.keys.iter().position(|k| *k == p.key) else { return };
        let slot = geo.slots[i];
        let sep = ordered(sh).iter().position(|(_, it)| it.kind == Kind::Separator).unwrap_or(0);
        let p = sh.dock.press.as_mut().unwrap();
        p.dragging = true;
        p.off = (p.x0 - slot.cx(), p.y0 - slot.cy());
        p.to = i.min(sep);
        sh.dock.hover = None;
        crate::menu::dismiss(sh);
    }
    let (items, geo) = geometry(sh);
    let removable = sh.dock.press.as_ref().is_some_and(|p| p.item.pinned);
    let off = y < geo.rect.y - s * 1.1 || x < geo.rect.x - s * 1.5 || x > geo.rect.right() + s * 1.5;
    let outside = removable && off;
    let key = sh.dock.press.as_ref().map(|p| p.key.clone()).unwrap_or_default();
    let mut to = 0;
    for (i, k) in geo.keys.iter().enumerate() {
        if items[i].kind == Kind::Separator {
            break;
        }
        if geo.ghost[i] || *k == key {
            continue;
        }
        if geo.slots[i].cx() < x {
            to += 1;
        }
    }
    if let Some(p) = sh.dock.press.as_mut() {
        p.outside = outside;
        if !outside {
            p.to = to;
        }
    }
}

pub fn hover(sh: &mut Shell, x: f32, y: f32) {
    if sh.dock.press.is_some() {
        drag_motion(sh, x, y);
    }
    let (_, geo) = geometry(sh);
    let dragging = sh.dock.dragging();
    let band = Rect::new(
        geo.rect.x - 40.0,
        geo.rect.y - sh.cfg.dock_icon_size * 0.6,
        geo.rect.w + 80.0,
        geo.rect.h + sh.cfg.dock_icon_size,
    );
    let inside = (geo.rect.contains(x, y) || (dragging && band.contains(x, y))) && !sh.has_modal();
    sh.dock.mag_x = if inside && sh.cfg.dock_magnification > 1.0 { Some(x) } else { None };
    let (_, geo) = geometry(sh);
    sh.dock.hover =
        if inside && !dragging { geo.slots.iter().position(|s| x >= s.x - 3.0 && x < s.right() + 3.0) } else { None };
}

/// Press on the Dock: remembered until release (click) or movement (drag).
pub fn press(sh: &mut Shell, x: f32, y: f32) -> Option<Vec<Action>> {
    let (items, geo) = geometry(sh);
    if !geo.rect.contains(x, y) {
        return None;
    }
    let Some(i) =
        geo.slots.iter().enumerate().position(|(i, s)| !geo.ghost[i] && x >= s.x - 3.0 && x < s.right() + 3.0)
    else {
        return Some(vec![]);
    };
    if items[i].kind == Kind::Separator {
        return Some(vec![]);
    }
    sh.dock.press = Some(Press {
        key: geo.keys[i].clone(),
        x0: x,
        y0: y,
        dragging: false,
        item: items[i].clone(),
        to: i,
        outside: false,
        off: (0.0, 0.0),
    });
    Some(vec![Action::Redraw])
}

/// Release after a Dock press: a click, or the end of a drag (reorder / remove).
pub fn release(sh: &mut Shell, x: f32, y: f32) -> Option<Vec<Action>> {
    let p = sh.dock.press.as_ref()?.clone();
    if !p.dragging {
        sh.dock.press = None;
        return Some(click(sh, p.x0, p.y0).unwrap_or_default());
    }
    let from = (x - p.off.0, y - p.off.1);
    if p.outside {
        sh.dock.press = None;
        sh.dock.fly = Some(Fly { item: p.item.clone(), key: p.key.clone(), from, t: 0.0, remove: true });
        sh.cfg.dock.retain(|d| d.app != p.item.app);
        sh.commit_dock();
        return Some(vec![Action::Redraw]);
    }
    let order = ordered(sh);
    sh.dock.press = None;
    let mut new_cfg = vec![];
    for (k, it) in &order {
        if it.kind == Kind::Separator {
            break;
        }
        if let Some(d) = sh.cfg.dock.iter().find(|d| d.app == it.app && it.pinned) {
            new_cfg.push(d.clone());
        } else if *k == p.key && !it.pinned {
            new_cfg.push(sh.dock_entry(&it.app));
        }
    }
    for d in &sh.cfg.dock {
        if !new_cfg.iter().any(|n| n.app == d.app) {
            new_cfg.push(d.clone());
        }
    }
    sh.dock.fly = Some(Fly { item: p.item.clone(), key: p.key.clone(), from, t: 0.0, remove: false });
    let changed = new_cfg.iter().map(|d| &d.app).ne(sh.cfg.dock.iter().map(|d| &d.app));
    if changed {
        sh.cfg.dock = new_cfg;
        sh.commit_dock();
    }
    Some(vec![Action::Redraw])
}

/// The floating icon of a Dock drag (and its flight back / puff after release).
pub fn drag_layer(sh: &mut Shell) -> Option<Layer> {
    let s = sh.cfg.dock_icon_size * sh.cfg.dock_magnification.clamp(1.0, 1.6).max(1.15);
    let (item, cx, cy, scale, alpha, label) = if let Some(p) = sh.dock.press.as_ref().filter(|p| p.dragging) {
        (p.item.clone(), sh.pointer.0 - p.off.0, sh.pointer.1 - p.off.1, 1.0, 1.0, p.outside)
    } else {
        let f = sh.dock.fly.as_ref()?;
        let e = smooth(f.t);
        if f.remove {
            (f.item.clone(), f.from.0, f.from.1, 1.0 + 0.5 * e, 1.0 - e, false)
        } else {
            let (_, geo) = geometry(sh);
            let (tx, ty, ts) = geo.keys.iter().position(|k| *k == f.key).map(|i| {
                let r = geo.slots[i].translate(geo.dx[i], 0.0);
                (r.cx(), r.cy(), r.h * 1.16 / s)
            })?;
            (
                f.item.clone(),
                f.from.0 + (tx - f.from.0) * e,
                f.from.1 + (ty - f.from.1) * e,
                1.0 + (ts - 1.0) * e,
                1.0,
                false,
            )
        }
    };
    let d = s * scale;
    let lw = if label { sh.fonts.measure("Remove", 13.0, Weight::Regular) + 24.0 } else { 0.0 };
    let w = d.max(lw) + 8.0;
    let top = if label { 34.0 } else { 0.0 };
    let rect = Rect::new(cx - w / 2.0, cy - d / 2.0 - top, w, d + top + 4.0);
    let px = (s * 1.6 * sh.scale).round() as u32;
    let icon = sh.icons.get(&item.icon, px);
    let dark = sh.style.dark;
    let key = hash_of(&(key_of(&item), (d * 4.0) as i32, label, dark, (w * 4.0) as i32));
    let (pm, serial) = sh.cached(LayerId::DockDrag, key, rect.w, rect.h, move |c, sh| {
        c.draw_pixmap(&icon, Rect::new(w / 2.0 - d / 2.0, top, d, d), 1.0);
        if label {
            let r = Rect::new(w / 2.0 - lw / 2.0, 0.0, lw, 26.0);
            c.fill_rrect(r, 13.0, if dark { rgba(40, 40, 44, 0.92) } else { rgba(246, 246, 248, 0.95) });
            let f = sh.fonts.clone();
            c.text_in(&f, r, 0.5, 13.0, Weight::Regular, style::text_primary(dark), "Remove");
        }
    });
    Some(Layer {
        id: LayerId::DockDrag,
        rect,
        glass: None,
        tiles: vec![],
        content: pm,
        serial,
        opacity: alpha.clamp(0.0, 1.0),
        zoom: 1.0,
    })
}

pub fn click(sh: &mut Shell, x: f32, y: f32) -> Option<Vec<Action>> {
    let (items, geo) = geometry(sh);
    if !geo.rect.contains(x, y) {
        return None;
    }
    let i = geo.slots.iter().enumerate().position(|(i, s)| !geo.ghost[i] && x >= s.x - 3.0 && x < s.right() + 3.0)?;
    let it = &items[i];
    match it.kind {
        Kind::Launchpad => {
            sh.toggle_launchpad();
            Some(vec![Action::Redraw])
        }
        Kind::App => {
            if let Some(w) = sh.windows.iter().find(|w| it.matches(&w.app_id)) {
                return Some(vec![Action::DockClick(w.app_id.clone(), it.exec.clone())]);
            }
            if it.exec.is_empty() {
                return Some(vec![]);
            }
            if sh.cfg.dock_bounce && !sh.cfg.reduce_motion && !sh.dock.bounce.iter().any(|b| b.0 == geo.keys[i]) {
                sh.dock.bounce.push((geo.keys[i].clone(), 0.0));
            }
            sh.launch_origin = Some(geo.slots[i]);
            Some(vec![Action::Launch(it.exec.clone())])
        }
        Kind::Minimized(id) => Some(vec![Action::Restore(id)]),
        Kind::Folder => Some(vec![Action::Launch(open_cmd(&downloads_dir()))]),
        Kind::Trash => Some(vec![Action::Launch(TRASH_OPEN.into())]),
        Kind::Separator => Some(vec![]),
    }
}

/// Opens the freedesktop trash in the user's file manager.
pub const TRASH_OPEN: &str =
    "aqua-finder trash: || gio open trash:/// || xdg-open trash:/// || xdg-open \"$HOME/.local/share/Trash/files\"";
/// Empties the freedesktop trash (gio when available, else the spec directories).
pub const TRASH_EMPTY: &str =
    "gio trash --empty || rm -rf \"$HOME/.local/share/Trash/files/\"* \"$HOME/.local/share/Trash/info/\"*";

pub fn open_cmd(path: &str) -> String {
    let q = path.replace('\'', "'\\''");
    format!("if [ -d '{q}' ] && command -v aqua-finder >/dev/null; then exec aqua-finder '{q}'; else exec xdg-open '{q}'; fi")
}

/// `XDG_DOWNLOAD_DIR` from user-dirs.dirs, else ~/Downloads.
pub fn downloads_dir() -> String {
    crate::menu::xdg_dir("DOWNLOAD", "Downloads")
}

/// Is the freedesktop trash non-empty?
pub fn trash_full() -> bool {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
    std::fs::read_dir(home.join(".local/share/Trash/files")).map(|mut d| d.next().is_some()).unwrap_or(false)
}

/// Dock item under a logical point (for the context menu), with its slot rect.
pub fn item_at(sh: &Shell, x: f32, y: f32) -> Option<(usize, DockItem, Rect)> {
    let (items, geo) = geometry(sh);
    if !geo.rect.contains(x, y) {
        return None;
    }
    let i = geo.slots.iter().enumerate().position(|(i, s)| !geo.ghost[i] && x >= s.x - 3.0 && x < s.right() + 3.0)?;
    let it = items.get(i)?.clone();
    if it.kind == Kind::Separator {
        return None;
    }
    Some((i, it, geo.slots[i]))
}

/// Top edge of the Dock (context menus open above it).
pub fn top(sh: &Shell) -> f32 {
    geometry(sh).1.rect.y
}

/// Dock items as laid out (indices match `item_at` and `MenuKind::Dock`).
pub fn visible_items(sh: &Shell) -> Vec<DockItem> {
    geometry(sh).0
}

impl Shell {
    pub(crate) fn icons_serial(&self) -> u64 {
        0
    }
}

/// Centre of the dock icon representing `app_id` (minimise / restore animation target).
pub fn icon_center_for(sh: &Shell, app_id: &str) -> Option<(f32, f32)> {
    let (items, geo) = geometry(sh);
    items
        .iter()
        .zip(geo.slots.iter())
        .find(|(it, _)| it.kind == Kind::App && it.matches(app_id))
        .map(|(_, r)| (r.cx(), r.cy()))
        .or_else(|| Some((geo.rect.cx(), geo.rect.cy())))
}
