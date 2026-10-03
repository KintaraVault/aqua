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
    /// Rendered icons by item key: (fingerprint of the icon data, image).
    icons: HashMap<String, (u64, Option<Arc<Pixmap>>)>,
}

impl Tray {
    /// True when the tray changed since the last call (the menu bar must be redrawn).
    pub fn poll(&mut self) -> bool {
        let snap = aqua_tray::snapshot();
        if snap.serial == self.serial {
            return false;
        }
        self.serial = snap.serial;
        self.icons.retain(|k, _| snap.items.iter().any(|i| &i.key == k));
        true
    }

    pub fn serial(&self) -> u64 {
        self.serial
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
        flatten(key, &root.children, &mut out);
    }
    while matches!(out.last(), Some(None)) {
        out.pop();
    }
    out
}

fn flatten(key: &str, nodes: &[MenuNode], out: &mut Vec<Option<Entry>>) {
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
            flatten(key, &n.children, out);
            out.push(None);
            continue;
        }
        let extra = match n.toggle {
            MenuToggle::Check(on) | MenuToggle::Radio(on) => Extra::Check(on),
            MenuToggle::None => Extra::None,
        };
        out.push(Some(Entry {
            label: n.label.clone(),
            shortcut: "",
            action: Some(Action::TrayMenu(key.to_string(), n.id)),
            enabled: n.enabled,
            extra,
        }));
    }
}
