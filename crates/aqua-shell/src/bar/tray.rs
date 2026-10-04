//! Status items of other applications (StatusNotifierItem) in the menu bar.
use crate::menu::{Entry, Extra, MenuKind};
use crate::{Action, Shell};
use aqua_gfx::{Color, Pixmap, Rect};
use aqua_tray::{MenuNode, MenuToggle, TrayItem};
use std::collections::HashMap;
use std::sync::Arc;

/// Width of one tray slot in the menu bar (logical px).
pub const SLOT: f32 = 30.0;
/// Icon edge (logical px).
pub const ICON: f32 = 17.0;

#[derive(Default)]
pub struct Tray {
    serial: u64,
    /// `aqua_tray::appmenu::serial()` seen last (the focused app's global menu).
    app_serial: u64,
    /// Rendered icons by item key: (fingerprint of the icon data, image).
    icons: HashMap<String, (u64, Option<Arc<Pixmap>>)>,
}

impl Tray {
    /// True when the tray changed since the last call (the menu bar must be redrawn).
    pub fn poll(&mut self) -> bool {
        let app = aqua_tray::appmenu::serial();
        let app_changed = app != self.app_serial;
        self.app_serial = app;
        let snap = aqua_tray::snapshot();
        if snap.serial == self.serial {
            return app_changed;
        }
        self.serial = snap.serial;
        self.icons.retain(|k, _| snap.items.iter().any(|i| &i.key == k));
        true
    }

    pub fn serial(&self) -> u64 {
        self.serial ^ self.app_serial.rotate_left(32)
    }

    /// Image for `item` at `px` physical pixels; symbolic icons are tinted with `fg`.
    pub fn icon(&mut self, item: &TrayItem, px: u32, fg: Color) -> Option<Arc<Pixmap>> {
        let (name, pixmaps) = item.current_icon();
        let data: Vec<(u32, usize)> = pixmaps.iter().map(|p| (p.width, Arc::as_ptr(&p.rgba) as usize)).collect();
        let fp = crate::hash_of(&(name, &item.icon_theme_path, data, px, fg.to_color_u8().red()));
        if let Some((k, pm)) = self.icons.get(&item.key) {
            if *k == fp {
                return pm.clone();
            }
        }
        let pm = render_icon(name, &item.icon_theme_path, pixmaps, px, fg).map(Arc::new);
        self.icons.insert(item.key.clone(), (fp, pm.clone()));
        pm
    }
}

fn render_icon(name: &str, theme_path: &str, pixmaps: &[aqua_tray::Icon], px: u32, fg: Color) -> Option<Pixmap> {
    let themed = || {
        let mut pm = aqua_icons::theme::lookup_status(name, theme_path, px)?;
        if name.ends_with("-symbolic") {
            tint(&mut pm, fg);
        }
        Some(pm)
    };
    let from_data = || {
        let best = pixmaps
            .iter()
            .filter(|p| p.width >= px)
            .min_by_key(|p| p.width)
            .or_else(|| pixmaps.iter().max_by_key(|p| p.width))?;
        Pixmap::from_vec(best.rgba.to_vec(), aqua_gfx::tiny_skia::IntSize::from_wh(best.width, best.height)?)
    };
    if name.is_empty() {
        from_data()
    } else {
        themed().or_else(from_data)
    }
}

/// Recolour a monochrome (symbolic) icon, keeping its alpha.
fn tint(pm: &mut Pixmap, c: Color) {
    let c = c.to_color_u8();
    for px in pm.data_mut().as_chunks_mut::<4>().0 {
        let a = px[3] as u32;
        px[0] = (c.red() as u32 * a / 255) as u8;
        px[1] = (c.green() as u32 * a / 255) as u8;
        px[2] = (c.blue() as u32 * a / 255) as u8;
    }
}

/// Items shown in the menu bar, in registration order.
pub fn visible(sh: &Shell) -> Vec<TrayItem> {
    if sh.cfg.menubar_hidden.iter().any(|h| h == "tray") {
        return vec![];
    }
    aqua_tray::snapshot().items.iter().filter(|i| i.status != aqua_tray::Status::Passive).cloned().collect()
}

pub fn find(key: &str) -> Option<TrayItem> {
    aqua_tray::snapshot().items.iter().find(|i| i.key == key).cloned()
}

/// Where the app should place its own popup: below the item, in physical pixels.
fn anchor(sh: &Shell, r: Rect) -> (i32, i32) {
    ((r.x * sh.scale).round() as i32, (sh.cfg.menubar_height * sh.scale).round() as i32)
}

pub fn open_menu(sh: &mut Shell, key: &str, x: f32) {
    aqua_tray::menu_opened(key);
    sh.menu.open = Some(MenuKind::Tray(key.to_string()));
    sh.menu.anchor = x;
    sh.menu.hover = None;
}

/// Primary click on a tray item.
pub fn click(sh: &mut Shell, item: &TrayItem, r: Rect) -> Vec<Action> {
    if sh.menu.open == Some(MenuKind::Tray(item.key.clone())) {
        crate::menu::dismiss(sh);
        return vec![Action::Redraw];
    }
    crate::menu::dismiss(sh);
    if item.item_is_menu && item.has_menu() {
        open_menu(sh, &item.key, r.x);
    } else {
        let (x, y) = anchor(sh, r);
        aqua_tray::activate(&item.key, x, y);
    }
    vec![Action::Redraw]
}

/// Secondary click: the item's dbusmenu, else the app's own context menu.
pub fn secondary_click(sh: &mut Shell, item: &TrayItem, r: Rect) -> Vec<Action> {
    crate::menu::dismiss(sh);
    if item.has_menu() {
        open_menu(sh, &item.key, r.x);
    } else {
        let (x, y) = anchor(sh, r);
        aqua_tray::context_menu(&item.key, x, y);
    }
    vec![Action::Redraw]
}

/// Events from the tray service (an app that cannot be activated wants its menu).
pub fn handle_events(sh: &mut Shell) -> bool {
    let mut any = false;
    for e in aqua_tray::take_events() {
        let aqua_tray::Event::OpenMenu(key) = e;
        if let Some(r) = crate::menubar::tray_rect(sh, &key) {
            open_menu(sh, &key, r.x);
            any = true;
        }
    }
    any
}

/// Menu rows of a tray item; submenus are flattened under a caption.
pub fn menu_entries(key: &str) -> Vec<Option<Entry>> {
    let mut out = vec![];
    if let Some(root) = find(key).and_then(|i| i.menu) {
        flatten(&root.children, &mut out, &|id| Action::TrayMenu(key.to_string(), id));
    }
    while matches!(out.last(), Some(None)) {
        out.pop();
    }
    out
}

/// Titles of the focused app's global menu, if it exports one.
pub fn app_menu_titles() -> Option<Vec<String>> {
    let m = aqua_tray::appmenu::current()?;
    let t: Vec<String> = m.titles().iter().map(|n| n.label.clone()).collect();
    (!t.is_empty()).then_some(t)
}

/// Rows of the `i`-th global menu.
pub fn app_menu_entries(i: usize) -> Option<Vec<Option<Entry>>> {
    let m = aqua_tray::appmenu::current()?;
    let node = *m.titles().get(i)?;
    Some(global_menu_rows(node))
}

pub fn app_menu_opened(i: usize) {
    if let Some(m) = aqua_tray::appmenu::current() {
        if let Some(n) = m.titles().get(i) {
            aqua_tray::appmenu::opened(n.id);
        }
    }
}

/// Rows of one global (dbusmenu) menu; submenus are flattened under captions.
pub fn global_menu_rows(menu: &MenuNode) -> Vec<Option<Entry>> {
    let mut out = vec![];
    flatten(&menu.children, &mut out, &Action::AppMenu);
    while matches!(out.last(), Some(None)) {
        out.pop();
    }
    if out.is_empty() {
        out.push(Some(Entry { label: crate::tr("No Items").to_string(), shortcut: "", action: None, enabled: false, extra: Extra::None }));
    }
    out
}

/// Shortcut labels live as long as the shell: intern them (the set is small and bounded
/// by what apps declare).
fn intern(s: &str) -> &'static str {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    if s.is_empty() {
        return "";
    }
    static SET: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut set = SET.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = set.get(s) {
        return v;
    }
    let v: &'static str = Box::leak(s.to_string().into_boxed_str());
    set.insert(v);
    v
}

fn flatten(nodes: &[MenuNode], out: &mut Vec<Option<Entry>>, act: &dyn Fn(i32) -> Action) {
    for n in nodes {
        if n.separator {
            if !out.is_empty() && !matches!(out.last(), Some(None)) {
                out.push(None);
            }
            continue;
        }
        if !n.children.is_empty() {
            if !out.is_empty() && !matches!(out.last(), Some(None)) {
                out.push(None);
            }
            out.push(Some(Entry {
                label: n.label.clone(),
                shortcut: "",
                action: None,
                enabled: false,
                extra: Extra::Caption,
            }));
            flatten(&n.children, out, act);
            out.push(None);
            continue;
        }
        let extra = match n.toggle {
            MenuToggle::Check(on) | MenuToggle::Radio(on) => Extra::Check(on),
            MenuToggle::None => Extra::None,
        };
        out.push(Some(Entry {
            label: n.label.clone(),
            shortcut: intern(&n.shortcut),
            action: Some(act(n.id)),
            enabled: n.enabled,
            extra,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(id: i32, label: &str) -> MenuNode {
        MenuNode { id, label: label.into(), enabled: true, ..Default::default() }
    }

    #[test]
    fn global_menu_rows_map_to_app_menu_actions() {
        let mut file = n(1, "File");
        let mut recent = n(5, "Open Recent");
        recent.children = vec![n(6, "a.txt"), n(7, "b.txt")];
        file.children = vec![
            MenuNode { shortcut: "⌘O".into(), ..n(2, "Open…") },
            MenuNode { separator: true, ..n(3, "") },
            MenuNode { separator: true, ..n(4, "") },
            recent,
            MenuNode { enabled: false, ..n(8, "Save") },
            MenuNode { toggle: MenuToggle::Check(true), ..n(9, "Autosave") },
            MenuNode { separator: true, ..n(10, "") },
        ];
        let rows = global_menu_rows(&file);
        let labels: Vec<Option<&str>> = rows.iter().map(|r| r.as_ref().map(|e| e.label.as_str())).collect();
        assert_eq!(
            labels,
            vec![
                Some("Open…"),
                None,
                Some("Open Recent"),
                Some("a.txt"),
                Some("b.txt"),
                None,
                Some("Save"),
                Some("Autosave")
            ],
            "duplicate and trailing separators collapse, submenus flatten under a caption"
        );
        let open = rows[0].as_ref().unwrap();
        assert_eq!(open.action, Some(Action::AppMenu(2)));
        assert_eq!(open.shortcut, "⌘O");
        let caption = rows[2].as_ref().unwrap();
        assert_eq!(caption.extra, Extra::Caption);
        assert!(caption.action.is_none());
        assert_eq!(rows[4].as_ref().unwrap().action, Some(Action::AppMenu(7)));
        assert!(!rows[6].as_ref().unwrap().enabled);
        assert_eq!(rows[7].as_ref().unwrap().extra, Extra::Check(true));
    }

    #[test]
    fn empty_global_menu_shows_placeholder() {
        let rows = global_menu_rows(&n(1, "Help"));
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].as_ref().unwrap().enabled);
    }

    #[test]
    fn shortcut_interning_is_stable() {
        let a = intern("⇧⌘S");
        let b = intern(&String::from("⇧⌘S"));
        assert!(std::ptr::eq(a, b));
        assert_eq!(intern(""), "");
    }
}
