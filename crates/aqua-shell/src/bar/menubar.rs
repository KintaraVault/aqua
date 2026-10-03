//! Top menu bar: system menu, bold active-app name, app menus, status items, clock.
//! The bar itself is transparent; foreground adapts to the wallpaper.
use crate::{clock, hash_of, menu, Action, Layer, LayerId, Shell};
use aqua_gfx::{symbols, Rect, Weight};

#[derive(Default)]
pub struct MenuBar {
    pub cc_open: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Apple,
    AppName,
    Menu(usize),
    Status(Status),
    /// Status item of another application, by tray key.
    Tray(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    Input,
    Battery,
    Wifi,
    Search,
    Control,
    Clock,
    /// Stop button of a running screen recording.
    Record,
}

pub const FONT: f32 = aqua_config::metrics::MENUBAR_FONT;

pub fn app_menus(app: &str) -> Vec<&'static str> {
    if app == "Finder" {
        vec!["File", "Edit", "View", "Go", "Window", "Help"]
    } else {
        vec!["File", "Edit", "View", "Window", "Help"]
    }
}

/// Status items can be hidden from Settings → Menu Bar (Control Centre and the clock always stay).
pub fn hidden(sh: &Shell, st: Status) -> bool {
    let name = match st {
        Status::Input => "input",
        Status::Battery if aqua_sys::snapshot().power.level.is_none() => return true,
        Status::Battery => "battery",
        Status::Wifi => "wifi",
        Status::Search => "search",
        Status::Record => return sh.shot.recording.is_none(),
        Status::Control | Status::Clock => return false,
    };
    sh.cfg.menubar_hidden.iter().any(|h| h == name)
}

/// Compute item rects (logical, relative to output).
pub fn layout(sh: &Shell) -> Vec<(Item, Rect)> {
    let h = sh.cfg.menubar_height;
    let f = &sh.fonts;
    let pad = aqua_config::metrics::MENU_ITEM_PAD;
    let mut v = vec![];
    let mut x = 10.0;
    let aw = 34.0;
    v.push((Item::Apple, Rect::new(x, 0.0, aw, h)));
    x += aw;
    let name = sh.active_app_name();
    let nw = f.measure(&name, FONT, Weight::Bold) + pad * 2.0;
    v.push((Item::AppName, Rect::new(x, 0.0, nw, h)));
    x += nw;
    for (i, m) in app_menus(&name).iter().enumerate() {
        let w = f.measure(m, FONT, Weight::Regular) + pad * 2.0;
        v.push((Item::Menu(i), Rect::new(x, 0.0, w, h)));
        x += w;
    }
    let n = clock::now();
    let clock_s = clock::menubar_string_fmt(&n, sh.cfg.clock_24h, sh.cfg.clock_seconds, sh.cfg.clock_date);
    let mut rx = sh.w - 10.0;
    let cw = f.measure(&clock_s, FONT, Weight::Regular) + pad * 2.0;
    rx -= cw;
    v.push((Item::Status(Status::Clock), Rect::new(rx, 0.0, cw, h)));
    for (st, w) in [
        (Status::Control, 36.0),
        (Status::Search, 34.0),
        (Status::Wifi, 36.0),
        (Status::Battery, 44.0),
        (Status::Input, 36.0),
        (Status::Record, 34.0),
    ] {
        if hidden(sh, st) {
            continue;
        }
        rx -= w;
        v.push((Item::Status(st), Rect::new(rx, 0.0, w, h)));
    }
    for it in crate::tray::visible(sh).iter().rev() {
        if rx - crate::tray::SLOT < x + 16.0 {
            break;
        }
        rx -= crate::tray::SLOT;
        v.push((Item::Tray(it.key.clone()), Rect::new(rx, 0.0, crate::tray::SLOT, h)));
    }
    v
}

/// Menu-bar slot of a tray item.
pub fn tray_rect(sh: &Shell, key: &str) -> Option<Rect> {
    layout(sh).into_iter().find_map(|(it, r)| matches!(&it, Item::Tray(k) if k == key).then_some(r))
}

/// Tray item under the pointer.
pub fn tray_at(sh: &Shell, x: f32, y: f32) -> Option<(aqua_tray::TrayItem, Rect)> {
    if y >= sh.cfg.menubar_height {
        return None;
    }
    let (key, r) = layout(sh).into_iter().find_map(|(it, r)| match it {
        Item::Tray(k) if r.contains(x, y) => Some((k, r)),
        _ => None,
    })?;
    crate::tray::find(&key).map(|i| (i, r))
}

pub fn layer(sh: &mut Shell) -> Layer {
    let h = sh.cfg.menubar_height;
    let w = sh.w;
    let n = clock::now();
    let items = layout(sh);
    let solid = sh.fullscreen;
    let fg = if solid {
        if sh.style.dark {
            aqua_gfx::rgba(255, 255, 255, 1.0)
        } else {
            aqua_gfx::rgba(0, 0, 0, 1.0)
        }
    } else {
        sh.style.on_wallpaper(Rect::new(0.0, 0.0, w, h))
    };
    let open = sh.menu.open.clone();
    let snap = aqua_sys::snapshot();
    let wifi_on = snap.net.wifi_enabled && snap.net.active().is_some();
    let wifi_sig = snap.net.active().map(|n| n.signal).unwrap_or(0);
    let bat = snap.power.level;
    let badge = crate::sysinfo::layout_badge(sh.layouts.get(sh.layout_idx).map(|s| s.as_str()).unwrap_or("us"));
    let key = hash_of(&(
        (badge.clone(), wifi_on, wifi_sig / 25, bat.map(|b| (b * 100.0) as i32), snap.power.charging),
        sh.active_app_name(),
        (
            n.minute,
            if sh.cfg.clock_seconds { n.second } else { 0 },
            sh.cfg.clock_24h,
            sh.cfg.clock_date,
            sh.cfg.menubar_background,
            sh.cfg.menubar_hidden.clone(),
            solid,
            sh.style.dark,
        ),
        n.hour,
        n.day,
        format!("{open:?}"),
        sh.control.visible(),
        sh.shot.recording.is_some(),
        w as i32,
        fg.to_color_u8().red(),
        sh.tray.serial(),
    ));
    let (pm, serial) = sh.cached(LayerId::MenuBar, key, w, h, |c, sh| {
        let f = sh.fonts.clone();
        let dark_fg = fg.red() < 0.5;
        if solid {
            c.fill_rect(
                Rect::new(0.0, 0.0, w, h),
                if sh.style.dark { aqua_gfx::rgba(36, 36, 38, 0.97) } else { aqua_gfx::rgba(240, 240, 242, 0.97) },
            );
        } else if sh.cfg.menubar_background {
            let col = if dark_fg { aqua_gfx::rgba(255, 255, 255, 0.55) } else { aqua_gfx::rgba(0, 0, 0, 0.28) };
            c.fill_rect(Rect::new(0.0, 0.0, w, h), col);
        }
        for (it, r) in &items {
            let is_open = match (&open, it) {
                (Some(menu::MenuKind::Apple), Item::Apple) => true,
                (Some(menu::MenuKind::App(i)), Item::Menu(j)) => i == j,
                (Some(menu::MenuKind::AppName), Item::AppName) => true,
                (Some(menu::MenuKind::Wifi), Item::Status(Status::Wifi)) => true,
                (Some(menu::MenuKind::Battery), Item::Status(Status::Battery)) => true,
                (Some(menu::MenuKind::Input), Item::Status(Status::Input)) => true,
                (Some(menu::MenuKind::Tray(a)), Item::Tray(b)) => a == b,
                _ => false,
            } || (matches!(it, Item::Status(Status::Control)) && sh.control.visible());
            if is_open {
                let hl = Rect::new(r.x + 2.0, 3.0, r.w - 4.0, h - 6.0);
                let col = if dark_fg { aqua_gfx::rgba(0, 0, 0, 0.10) } else { aqua_gfx::rgba(255, 255, 255, 0.26) };
                c.fill_rrect(hl, hl.h / 2.0, col);
            }
            let base = h / 2.0 + 4.8;
            match it {
                Item::Apple => {
                    let aw = f.measure("\u{F8FF}", 16.5, Weight::Regular);
                    c.text(&f, r.cx() - aw / 2.0, base + 0.6, 16.5, Weight::Regular, fg, "\u{F8FF}");
                }
                Item::AppName => {
                    c.text(
                        &f,
                        r.x + aqua_config::metrics::MENU_ITEM_PAD,
                        base,
                        FONT,
                        Weight::Bold,
                        fg,
                        &sh.active_app_name(),
                    );
                }
                Item::Menu(i) => {
                    let name = sh.active_app_name();
                    let m = app_menus(&name)[*i];
                    c.text(&f, r.x + aqua_config::metrics::MENU_ITEM_PAD, base, FONT, Weight::Regular, fg, m);
                }
                Item::Tray(key) => {
                    let px = (crate::tray::ICON * c.scale).round() as u32;
                    if let Some(item) = crate::tray::find(key) {
                        if let Some(pm) = sh.tray.icon(&item, px, fg) {
                            let s = crate::tray::ICON;
                            c.draw_pixmap(&pm, Rect::new(r.cx() - s / 2.0, h / 2.0 - s / 2.0, s, s), 1.0);
                        }
                    }
                }
                Item::Status(st) => match st {
                    Status::Clock => {
                        c.text(
                            &f,
                            r.x + aqua_config::metrics::MENU_ITEM_PAD,
                            base,
                            FONT,
                            Weight::Regular,
                            fg,
                            &clock::menubar_string_fmt(&n, sh.cfg.clock_24h, sh.cfg.clock_seconds, sh.cfg.clock_date),
                        );
                    }
                    Status::Record => symbols::record_stop(c, Rect::new(r.cx() - 8.5, h / 2.0 - 8.5, 17.0, 17.0), fg),
                    Status::Control => {
                        symbols::control_center(c, Rect::new(r.cx() - 8.0, h / 2.0 - 7.5, 16.0, 16.0), fg)
                    }
                    Status::Search => symbols::search(c, Rect::new(r.cx() - 7.5, h / 2.0 - 7.5, 15.0, 15.0), fg),
                    Status::Wifi => {
                        let col = if wifi_on || snap.net.backend == aqua_sys::NetBackend::None {
                            fg
                        } else {
                            aqua_gfx::Color::from_rgba(fg.red(), fg.green(), fg.blue(), fg.alpha() * 0.35).unwrap_or(fg)
                        };
                        symbols::wifi(c, Rect::new(r.cx() - 9.0, h / 2.0 - 8.0, 18.0, 15.0), col)
                    }
                    Status::Battery => {
                        symbols::battery(c, Rect::new(r.cx() - 13.0, h / 2.0 - 6.0, 27.0, 12.0), bat.unwrap_or(1.0), fg)
                    }
                    Status::Input => {
                        let pill = Rect::new(r.cx() - 10.0, h / 2.0 - 8.5, 20.0, 17.0);
                        c.fill_rrect(pill, 4.5, fg);
                        let mut cut = aqua_gfx::Canvas::new(20.0, 17.0, c.scale);
                        cut.text_in(
                            &f,
                            Rect::new(0.0, 0.0, 20.0, 17.0),
                            0.5,
                            if badge.len() > 1 { 10.0 } else { 12.0 },
                            Weight::Bold,
                            aqua_gfx::rgba(0, 0, 0, 1.0),
                            &badge,
                        );
                        c.pm.draw_pixmap(
                            (pill.x * c.scale).round() as i32,
                            (pill.y * c.scale).round() as i32,
                            cut.pm.as_ref(),
                            &aqua_gfx::tiny_skia::PixmapPaint {
                                blend_mode: aqua_gfx::tiny_skia::BlendMode::DestinationOut,
                                ..Default::default()
                            },
                            aqua_gfx::tiny_skia::Transform::identity(),
                            None,
                        );
                    }
                },
            }
        }
    });
    Layer {
        id: LayerId::MenuBar,
        rect: Rect::new(0.0, 0.0, w, h),
        glass: None,
        tiles: vec![],
        content: pm,
        serial,
        opacity: 1.0,
        zoom: 1.0,
    }
}

pub fn click(sh: &mut Shell, x: f32, y: f32) -> Vec<Action> {
    for (it, r) in layout(sh) {
        if r.contains(x, y) {
            let kind = match it {
                Item::Tray(key) => {
                    return match crate::tray::find(&key) {
                        Some(item) => crate::tray::click(sh, &item, r),
                        None => vec![Action::Redraw],
                    };
                }
                Item::Apple => Some(menu::MenuKind::Apple),
                Item::AppName => Some(menu::MenuKind::AppName),
                Item::Menu(i) => Some(menu::MenuKind::App(i)),
                Item::Status(Status::Control) => {
                    sh.menu.open = None;
                    if sh.notes.center_open {
                        sh.notes.toggle_center();
                    }
                    sh.control.toggle();
                    return vec![Action::Redraw];
                }
                Item::Status(Status::Clock) => {
                    sh.toggle_notification_center();
                    return vec![Action::Redraw];
                }
                Item::Status(Status::Search) => {
                    sh.menu.open = None;
                    sh.toggle_spotlight();
                    return vec![Action::Redraw];
                }
                Item::Status(Status::Record) => {
                    menu::dismiss(sh);
                    return vec![Action::RecordStop, Action::Redraw];
                }
                Item::Status(Status::Wifi) => Some(menu::MenuKind::Wifi),
                Item::Status(Status::Battery) => Some(menu::MenuKind::Battery),
                Item::Status(Status::Input) => Some(menu::MenuKind::Input),
            };
            if let Some(k) = kind {
                if sh.menu.open.as_ref() == Some(&k) {
                    menu::dismiss(sh);
                } else {
                    sh.menu.open = Some(k);
                    sh.menu.anchor = r.x;
                    sh.menu.hover = None;
                }
            }
            return vec![Action::Redraw];
        }
    }
    menu::dismiss(sh);
    vec![Action::Redraw]
}
