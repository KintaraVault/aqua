//! aqua-shell: the desktop shell UI, independent of the display server.
//!
//! The shell produces an ordered list of [`Layer`]s (CPU-rendered content + optional
//! glass material descriptor). A backend (the compositor, or the software [`soft`]
//! compositor used for previews) draws the glass behind each layer and the content on top.
//!
//! * `bar`: menu bar, its menus, clock, status and tray items
//! * `panels`: overlays (Control Centre, Launchpad, Spotlight, notifications, alerts…)
//! * `kit`: shared styling, scrolling and the software compositor
mod bar;
mod kit;
mod panels;

mod actions;
mod input;
mod types;
mod visibility;

pub mod decor;
pub mod dock;

pub use aqua_i18n::{tr, trf};
pub use types::*;

pub use bar::{clock, menu, menubar, sysinfo, tray};
pub use kit::{scroll, soft, style};
pub use panels::{
    alert, charviewer, clipboard, control, launchpad, lockscreen, mission, notifications, screenshot, spotlight,
    switcher, widgets,
};

use aqua_config::{Config, GlassStyle};
use aqua_gfx::{Fonts, Pixmap, Rect};
use aqua_icons::IconProvider;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

pub(crate) struct Cached {
    key: u64,
    pm: Arc<Pixmap>,
    serial: u64,
    /// Last time the layer was produced (closed panels are forgotten).
    used: std::time::Instant,
}

pub(crate) fn hash_of<T: Hash>(t: &T) -> u64 {
    let mut h = DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

pub struct Shell {
    pub cfg: Config,
    pub fonts: Arc<Fonts>,
    pub icons: IconProvider,
    pub apps: Vec<aqua_apps::App>,
    pub w: f32,
    pub h: f32,
    pub scale: f32,
    pub windows: Vec<WindowInfo>,
    pub pointer: (f32, f32),
    pub style: style::Adaptive,
    pub menubar: menubar::MenuBar,
    pub dock: dock::Dock,
    pub launchpad: launchpad::Launchpad,
    pub menu: menu::MenuState,
    pub control: control::ControlCenter,
    pub widgets: widgets::Widgets,
    pub spotlight: spotlight::Spotlight,
    pub switcher: switcher::Switcher,
    pub notes: notifications::Notifications,
    pub mission: mission::Mission,
    pub lockscreen: lockscreen::LockScreen,
    pub clipboard: clipboard::ClipboardPanel,
    pub alert: Option<alert::Alert>,
    pub hud: Option<alert::Hud>,
    pub chars: charviewer::CharViewer,
    pub shot: screenshot::ShotUi,
    pub tray: tray::Tray,
    /// Configured XKB layouts and the active index (set by the compositor).
    pub layouts: Vec<String>,
    pub layout_idx: usize,
    /// A full-screen window covers the output: menu bar and Dock hide until the
    /// pointer touches the top / bottom edge (set by the compositor every frame).
    pub fullscreen: bool,
    pub fs_reveal_bar: bool,
    pub fs_reveal_dock: bool,
    /// Slide-away progress of the menu bar / Dock (0 = shown, 1 = hidden).
    pub bar_hide: f32,
    pub dock_hide: f32,
    /// Labels for evdev key codes in the active layout (Keyboard Viewer).
    pub key_labels: std::collections::HashMap<u32, String>,
    /// Session is locked: only the lock screen is interactive.
    pub locked: bool,
    cache: std::collections::HashMap<LayerId, Cached>,
    serial: u64,
    pub last_tick: std::time::Instant,
    last_icon_trim: std::time::Instant,
    /// Bytes released by cache trimming (the compositor returns them to the OS).
    pub freed: usize,
    /// Icon rectangle (logical) of the last app launched from the Dock, Launchpad or
    /// Spotlight: the new window grows out of it.
    pub launch_origin: Option<Rect>,
    /// Fingerprint of the application directories at the last scan.
    apps_fp: Option<u64>,
    /// `aqua_sys` state drawn last (menu-bar status icons, Control Centre).
    sys_serial: u64,
}

impl Shell {
    /// `w`,`h` are the logical output size.
    pub fn new(cfg: Config, w: f32, h: f32, scale: f32, wallpaper: &Pixmap) -> Self {
        aqua_gfx::text::set_translator(aqua_i18n::tr);
        let fonts = Arc::new(Fonts::load(&cfg.font_dir()));
        let mut icons = IconProvider::new(cfg.icon_cache(), fonts.clone(), cfg.fetch_icons);
        let apps_fp = Some(aqua_apps::fingerprint());
        let apps = aqua_apps::scan();
        let mut style = style::Adaptive::from_wallpaper(wallpaper, w, h);
        style.dark = cfg.dark;
        let dock = dock::Dock::new(&cfg, &apps);
        icons.set_policy(aqua_config::apple_icons::Policy {
            owners: dock::icon_owners(&dock, &apps),
            ..aqua_config::apple_icons::Policy::from_config(&cfg)
        });
        icons.set_look(aqua_icons::look::Look {
            style: aqua_icons::look::Style::from_config(&cfg.icon_style, cfg.dark),
            dark: cfg.dark,
            tint: cfg.accent_rgb(),
            glass: cfg.icon_glass,
        });
        let control = control::ControlCenter { dark: cfg.dark, ..Default::default() };
        Self {
            fonts,
            icons,
            apps,
            w,
            h,
            scale,
            windows: vec![],
            pointer: (-1.0, -1.0),
            style,
            menubar: menubar::MenuBar::default(),
            dock,
            launchpad: launchpad::Launchpad::default(),
            menu: menu::MenuState::default(),
            control,
            widgets: widgets::Widgets::default(),
            spotlight: Default::default(),
            switcher: Default::default(),
            notes: Default::default(),
            mission: Default::default(),
            lockscreen: Default::default(),
            clipboard: Default::default(),
            alert: None,
            hud: None,
            chars: Default::default(),
            shot: screenshot::ShotUi {
                save_to: cfg.screenshot_save.clone(),
                timer: cfg.screenshot_timer,
                show_thumb: cfg.screenshot_thumbnail,
                show_pointer: cfg.screenshot_pointer,
                rec_save: cfg.record_save.clone(),
                rec_mic: cfg.record_mic,
                rec_clicks: cfg.record_clicks,
                rec_pointer: cfg.record_pointer,
                ..Default::default()
            },
            layouts: cfg.keyboard.layouts.clone(),
            layout_idx: 0,
            fullscreen: false,
            fs_reveal_bar: false,
            fs_reveal_dock: false,
            bar_hide: 0.0,
            dock_hide: 0.0,
            key_labels: Default::default(),
            locked: false,
            cache: Default::default(),
            serial: 1,
            last_tick: std::time::Instant::now(),
            last_icon_trim: std::time::Instant::now(),
            freed: 0,
            launch_origin: None,
            apps_fp,
            sys_serial: 0,
            tray: Default::default(),
            cfg,
        }
    }

    /// Re-index installed applications when an application directory changed (package installed /
    /// removed / updated).
    pub fn rescan_apps_if_changed(&mut self) -> bool {
        let fp = aqua_apps::fingerprint();
        if self.apps_fp == Some(fp) {
            return false;
        }
        self.apps_fp = Some(fp);
        let apps = aqua_apps::scan();
        let ids = |v: &[aqua_apps::App]| {
            v.iter().map(|a| (a.id.clone(), a.name.clone(), a.exec.clone(), a.icon.clone())).collect::<Vec<_>>()
        };
        if ids(&apps) == ids(&self.apps) {
            return false;
        }
        tracing_log(&format!("applications changed: {} → {} entries", self.apps.len(), apps.len()));
        self.apps = apps;
        self.apply_config();
        true
    }

    /// Configuration changed on disk (System Settings): rebuild what depends on it.
    pub fn apply_config(&mut self) {
        let mut d = dock::Dock::new(&self.cfg, &self.apps);
        d.carry_over(&mut self.dock);
        self.dock = d;
        self.icons.set_policy(aqua_config::apple_icons::Policy {
            owners: dock::icon_owners(&self.dock, &self.apps),
            ..aqua_config::apple_icons::Policy::from_config(&self.cfg)
        });
        self.icons.invalidate();
        self.style.dark = self.cfg.dark;
        self.update_icon_look();
        self.control.dark = self.cfg.dark;
        self.layouts = self.cfg.keyboard.layouts.clone();
        self.cache.clear();
        self.serial += 1;
    }

    pub fn resize(&mut self, w: f32, h: f32, scale: f32, wallpaper: &Pixmap) {
        self.w = w;
        self.h = h;
        self.scale = scale;
        let dark = self.style.dark;
        self.style = style::Adaptive::from_wallpaper(wallpaper, w, h);
        self.style.dark = dark;
        self.cache.clear();
        self.icons.invalidate();
    }

    pub fn set_windows(&mut self, wins: Vec<WindowInfo>) {
        self.dock.track_windows(&wins);
        self.windows = wins;
        // Start the Dock's appear / disappear / reorder animations right away: the window
        // list can change between `tick()` and drawing (a restore from the Dock, the end of a
        // genie), and drawing the new list before `sync` has turned a removed item into a
        // shrinking ghost made the Dock snap narrower for a frame and jump back — the icons
        // flickered and seemed duplicated.
        dock::sync(self, 0.0);
    }

    pub fn focused_app(&self) -> Option<&WindowInfo> {
        self.windows.iter().find(|w| w.focused)
    }

    /// Name shown bold in the menu bar.
    pub fn active_app_name(&self) -> String {
        match self.focused_app() {
            Some(w) => self.app_display_name(&w.app_id),
            None => "Finder".into(),
        }
    }

    pub fn app_display_name(&self, app_id: &str) -> String {
        if let Some(d) = self.dock.items.iter().find(|d| d.matches(app_id)) {
            return d.name.clone();
        }
        if let Some(a) = aqua_apps::match_app_id(&self.apps, app_id) {
            return a.name.clone();
        }
        fallback_app_name(app_id)
    }

    /// 1×1 transparent pixmap for glass-only layers.
    pub(crate) fn blank_pixmap(&mut self) -> Arc<Pixmap> {
        let (pm, _) =
            self.cached(LayerId::SpotlightGlass, 0, 1.0 / self.scale.max(0.1), 1.0 / self.scale.max(0.1), |_, _| {});
        pm
    }

    /// Render helper with caching by state key.
    pub(crate) fn cached(
        &mut self,
        id: LayerId,
        key: u64,
        w: f32,
        h: f32,
        draw: impl FnOnce(&mut aqua_gfx::Canvas, &mut Self),
    ) -> (Arc<Pixmap>, u64) {
        if let Some(c) = self.cache.get_mut(&id) {
            if c.key == key {
                c.used = std::time::Instant::now();
                return (c.pm.clone(), c.serial);
            }
        }
        self.cache.remove(&id);
        let mut canvas = aqua_gfx::Canvas::new(w, h, self.scale);
        draw(&mut canvas, self);
        self.serial += 1;
        let pm = Arc::new(canvas.pm);
        self.cache.insert(id, Cached { key, pm: pm.clone(), serial: self.serial, used: std::time::Instant::now() });
        (pm, self.serial)
    }

    /// Advance animations; returns true if another frame is needed.
    pub fn tick(&mut self) -> bool {
        let now = std::time::Instant::now();
        let dt = (now - self.last_tick).as_secs_f32().min(0.1);
        self.last_tick = now;
        let mut anim = false;
        if self.icons.take_dirty() {
            self.icons.invalidate();
            self.cache.clear();
            anim = true;
        }
        let sys = aqua_sys::serial();
        if sys != self.sys_serial {
            self.sys_serial = sys;
            anim = true;
        }
        anim |= self.tray.poll();
        anim |= tray::handle_events(self);
        alert::reap(self);
        anim |= self.launchpad.animate(dt);
        anim |= self.control.animate(dt);
        anim |= self.dock.animate(dt);
        anim |= dock::sync(self, dt);
        anim |= self.spotlight.animate(dt);
        anim |= self.switcher.animate(dt);
        anim |= self.notes.animate(dt);
        anim |= self.clipboard.animate(dt);
        anim |= self.chars.animate(dt);
        anim |= self.shot.animating();
        anim |= alert::animating(self) || alert::hud_animating(self);
        anim |= self.locked && self.lockscreen.animating();
        anim |= self.animate_hiding(dt);
        anim |= menu::animating(self);
        anim
    }

    /// Build all layers in back-to-front order.
    pub fn trim_caches(&mut self) -> usize {
        let mut freed = 0;
        self.cache.retain(|_, c| {
            let keep = c.used.elapsed() < std::time::Duration::from_secs(5);
            if !keep && Arc::strong_count(&c.pm) == 1 {
                freed += c.pm.data().len();
            }
            keep
        });
        if self.last_icon_trim.elapsed() > std::time::Duration::from_secs(30) {
            self.last_icon_trim = std::time::Instant::now();
            let mut keep: std::collections::HashSet<String> =
                self.dock.items.iter().map(|i| i.icon.id.clone()).collect();
            keep.extend(self.windows.iter().map(|w| w.app_id.clone()));
            freed += self.icons.trim(std::time::Duration::from_secs(120), &keep);
        }
        freed
    }

    pub fn layers(&mut self) -> Vec<Layer> {
        self.freed += self.trim_caches();
        let mut out = Vec::new();
        out.extend(widgets::layers(self));
        if let Some(l) = mission::layer(self) {
            out.push(l);
        }
        if let Some(l) = mission::names_layer(self) {
            out.push(l);
        }
        if let Some(l) = mission::bar_layer(self) {
            out.push(l);
        }
        if self.bar_hide < 1.0 {
            let mut l = menubar::layer(self);
            let dy = self.bar_offset();
            l.rect.y -= dy;
            if dy > 0.0 {
                l.opacity = 1.0 - 0.35 * self.bar_hide;
            }
            out.push(l);
        }
        if self.dock_hide < 1.0 {
            out.push(dock::layer(self));
            if let Some(b) = dock::badges_layer(self) {
                out.push(b);
            }
            if self.dock_hide <= 0.0 {
                if let Some(t) = dock::tooltip_layer(self) {
                    out.push(t);
                }
            }
        }
        if let Some(l) = dock::drag_layer(self) {
            out.push(l);
        }
        if let Some(l) = launchpad::layer(self) {
            out.push(l);
        }
        if let Some(c) = control::layer(self) {
            out.push(c);
        }
        if let Some(l) = notifications::center_layer(self) {
            out.push(l);
        }
        if let Some(l) = notifications::banner_layer(self) {
            out.push(l);
        }
        if let Some(m) = menu::fade_layer(self) {
            out.push(m);
        }
        if let Some(m) = menu::layer(self) {
            out.push(m);
        }
        out.extend(spotlight::layers(self));
        if let Some(l) = switcher::layer(self) {
            out.push(l);
        }
        if let Some(l) = clipboard::layer(self) {
            out.push(l);
        }
        out.extend(charviewer::layers(self));
        if let Some(l) = alert::layer(self) {
            out.push(l);
        }
        out.extend(screenshot::layers(self));
        if let Some(l) = alert::hud_layer(self) {
            out.push(l);
        }
        self.darken_layers(&mut out);
        out
    }

    /// Lock screen layers (drawn instead of `layers()` while locked).
    pub fn lock_layers(&mut self, progress: f32) -> Vec<Layer> {
        let mut out = lockscreen::layers(self, progress);
        if let Some(l) = alert::hud_layer(self) {
            out.push(l);
        }
        out
    }

    fn darken_layers(&self, out: &mut [Layer]) {
        if self.style.dark {
            for l in out.iter_mut() {
                if let Some(g) = &l.glass {
                    l.glass = Some(style::darken(g));
                }
                for t in l.tiles.iter_mut() {
                    t.1 = style::darken(&t.1);
                }
            }
        }
    }

    /// Timers that may fire actions (alert countdowns).
    pub fn poll_actions(&mut self) -> Vec<Action> {
        alert::tick(self)
    }

    pub fn show_alert(&mut self, a: alert::Alert) {
        self.close_transients();
        self.alert = Some(a);
    }

    pub fn show_hud(&mut self, kind: alert::HudKind, value: f32, label: &str) {
        alert::show_hud(self, kind, value, label);
    }

    /// A Dock icon is pressed: the compositor must deliver motion and the release here.
    pub fn pointer_captured(&self) -> bool {
        self.dock.press.is_some()
    }

    /// An icon is being dragged out of the Dock (grabbing hand cursor).
    pub fn dragging_icon(&self) -> bool {
        self.dock.dragging()
    }

    pub fn toggle_clipboard(&mut self) {
        let was = self.clipboard.open;
        self.close_transients();
        if !was {
            self.clipboard.toggle();
        }
    }

    /// Switch system appearance; invalidates every cached layer.
    pub fn set_dark(&mut self, dark: bool) {
        self.style.dark = dark;
        self.control.dark = dark;
        self.update_icon_look();
        self.cache.clear();
    }

    pub fn toggle_launchpad(&mut self) {
        self.menu.open = None;
        if self.spotlight.open {
            self.spotlight.toggle();
        }
        self.launchpad.toggle();
    }

    pub fn toggle_notification_center(&mut self) {
        self.menu.open = None;
        if self.control.open {
            self.control.toggle();
        }
        self.notes.toggle_center();
    }

    /// Bounce an app's Dock icon (attention request / system bell).
    pub fn bounce_app(&mut self, app_id: &str) {
        let (items, geo) = dock::geometry(self);
        if let Some(i) = items.iter().enumerate().position(|(i, it)| !geo.ghost[i] && it.matches(app_id)) {
            let k = geo.keys[i].clone();
            if !self.dock.bounce.iter().any(|b| b.0 == k) {
                self.dock.bounce.push((k, 0.0));
            }
        }
    }

    /// Post a notification (banner + Notification Center history).
    pub fn notify(&mut self, mut n: notifications::Note) {
        let now = clock::now();
        n.time = (now.hour, now.minute);
        if n.app_name.is_empty() {
            n.app_name = self.app_display_name(&n.app_id);
        }
        self.notes.push(n);
    }

    pub fn toggle_spotlight(&mut self) {
        menu::dismiss(self);
        self.spotlight.origin = None;
        if self.bar_hide <= 0.0 && self.bar_shown() {
            if let Some((_, r)) = menubar::layout(self)
                .into_iter()
                .find(|(it, _)| matches!(it, menubar::Item::Status(menubar::Status::Search)))
            {
                let d = r.h.min(r.w) - 4.0;
                self.spotlight.origin = Some(Rect::new(r.cx() - d / 2.0, r.cy() - d / 2.0, d, d));
            }
        }
        if self.launchpad.open {
            self.launchpad.toggle();
        }
        if self.control.open {
            self.control.toggle();
        }
        self.spotlight.toggle();
    }

    /// Close menus and modal panels (e.g. when Mission Control opens).
    pub fn close_transients(&mut self) {
        menu::dismiss(self);
        if self.clipboard.open {
            self.clipboard.toggle();
        }
        if self.launchpad.visible() {
            self.toggle_launchpad();
        }
        if self.control.visible() {
            self.control.toggle();
        }
        if self.spotlight.open {
            self.toggle_spotlight();
        }
        if self.notes.center_open {
            self.toggle_notification_center();
        }
    }

    pub fn has_modal(&self) -> bool {
        self.locked
            || alert::active(self)
            || self.shot.active()
            || self.clipboard.open
            || self.launchpad.visible()
            || self.menu.open.is_some()
            || self.control.visible()
            || self.spotlight.open
            || self.notes.center_open
            || self.notes.replying.is_some()
    }
}

fn aqua_sys_user() -> String {
    std::env::var("USER").unwrap_or_else(|_| "you".into())
}

fn tracing_log(msg: &str) {
    eprintln!("aqua-shell: {msg}");
}

/// Display name for an app id no desktop entry knows: the last component of a reverse-DNS id
/// (`org.gnome.Maps` → `Maps`), without script/binary suffixes (`menutest.py` → `Menutest`).
pub fn fallback_app_name(app_id: &str) -> String {
    let mut id = app_id;
    for suf in [".py", ".sh", ".pl", ".rb", ".js", ".exe", ".AppImage", ".appimage", ".bin", ".x86_64"] {
        if let Some(b) = id.strip_suffix(suf) {
            id = b;
            break;
        }
    }
    let mut s = id.rsplit('.').next().unwrap_or(id).replace(['-', '_'], " ");
    if let Some(f) = s.get(0..1) {
        s = f.to_uppercase() + &s[1..];
    }
    s
}

#[cfg(test)]
mod name_tests {
    #[test]
    fn fallback_names() {
        use super::fallback_app_name as n;
        assert_eq!(n("org.gnome.Maps"), "Maps");
        assert_eq!(n("Menutest.py"), "Menutest");
        assert_eq!(n("my-tool.AppImage"), "My tool");
        assert_eq!(n("designer"), "Designer");
    }
}
