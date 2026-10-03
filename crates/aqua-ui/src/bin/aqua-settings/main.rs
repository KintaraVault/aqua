//! Aqua System Settings (Rust + Slint). `aqua-settings [--pane ID | ID | wifi:SSID]`.
//! Reads/writes ~/.config/aqua/config.toml (the compositor hot-reloads it) and talks to
//! NetworkManager/iwd, BlueZ, UPower, PipeWire and the backlight through aqua-sys.
use aqua_config::{Config, OutputCfg};
use aqua_ui::*;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

mod config;
mod datetime;
mod displays;
mod keyboard;
mod login;
mod shortcuts;
mod system;
#[cfg(test)]
mod tests;
mod util;

use config::*;
use datetime::*;
use displays::*;
use keyboard::*;
use login::*;
use shortcuts::*;
use system::*;
use util::*;

const PANES: &[(&str, &str)] = &[
    ("wifi", "wi-fi wifi wireless network internet ssid"),
    ("bluetooth", "bluetooth devices headphones pair"),
    ("network", "network ethernet wired vpn proxy internet"),
    ("battery", "battery energy power charging low power brightness"),
    ("general", "general about computer os memory chip storage disk software update upgrade packages date time zone ntp clock language region locale login items startup autostart"),
    ("appearance", "appearance dark light mode accent colour color theme radius corners"),
    ("accessibility", "accessibility reduce motion animation pointer cursor size large"),
    ("menubar", "menu bar menubar clock 24-hour seconds size date background controls spotlight wifi battery keyboard input status"),
    ("dock", "desktop dock magnification hide hot corners size widgets minimise genie click running app windows title bar double-click zoom animate full screen"),
    ("displays", "displays monitor resolution refresh rate hz vrr freesync gsync scale hidpi arrangement main"),
    ("wallpaper", "wallpaper background picture desktop"),
    ("notifications", "notifications do not disturb focus banners previews alerts"),
    ("sound", "sound volume audio output input microphone mute alert"),
    ("lock", "lock screen password idle sleep screen saver dim display off"),
    ("users", "users groups accounts password admin"),
    ("keyboard", "keyboard input sources layout language repeat shortcuts hotkeys keybindings bindings remap xkb"),
    ("mouse", "mouse pointer speed scroll natural"),
    ("trackpad", "trackpad touchpad pointer speed scroll natural tap gestures"),
];
const MENUBAR: [f32; 3] = [24.0, 26.0, 30.0];
const IDLE: [u32; 8] = [0, 60, 120, 300, 600, 1200, 1800, 3600];
const CORNERS: [&str; 6] = ["", "mission", "desktop", "launchpad", "notifications", "lock"];
const SWITCH: [&str; 4] = ["ctrl+space", "super+space", "alt+shift", "caps"];
const DOCK_CLICK: [&str; 5] = ["focus", "minimize", "cycle", "expose", "new"];
const TITLE_DBL: [&str; 3] = ["zoom", "minimize", "none"];
const SCALES: [f64; 6] = [0.0, 1.0, 1.25, 1.5, 1.75, 2.0];
const MB_AUTOHIDE: [&str; 3] = ["fullscreen", "always", "never"];
const APPLE_ICONS: [&str; 3] = ["all", "selected", "off"];
const ICON_STYLES: [&str; 5] = ["default", "auto", "dark", "clear", "tinted"];

fn main() -> Result<(), slint::PlatformError> {
    aqua_ui::init("org.aqua.settings");
    let mut pane = String::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--pane" {
            pane = args.next().unwrap_or_default();
        } else if !a.starts_with('-') {
            pane = a;
        }
    }
    let ui = SettingsWindow::new()?;
    aqua_ui::init_translations();
    aqua_ui::set_app_id();
    if aqua_ui::glass_supported() {
        ui.set_glass(true);
        aqua_ui::enable_glass(&ui.as_weak());
    }
    apply_theme!(ui);
    let cfg = Rc::new(RefCell::new(Config::load()));
    load_into(&ui, &cfg.borrow());
    let s = ui.global::<S>();
    let (login, full) = user_names();
    s.set_user_name(full.into());
    let about: Vec<KV> = aqua_sys::session::about()
        .into_iter()
        .chain([("User".to_string(), login.clone())])
        .collect::<Vec<_>>()
        .into_iter()
        .inspect(|(k, v)| {
            if k == "Name" && !v.is_empty() {
                s.set_host_name(v.clone().into());
            }
        })
        .map(|(k, v)| KV { key: tr(&k).into(), value: v.into() })
        .collect();
    if s.get_host_name().is_empty() {
        s.set_host_name("Aqua".into());
    }
    s.set_about(ModelRc::new(VecModel::from(about)));
    s.set_outputs(ModelRc::new(VecModel::from(outputs(&cfg.borrow()))));
    let (walls, cur) = wallpapers(&cfg.borrow());
    let walls = Rc::new(walls);
    s.set_wallpapers(ModelRc::new(VecModel::from((*walls).clone())));
    s.set_wallpaper_idx(cur);
    if let Some(ssid) = pane.strip_prefix("wifi:") {
        s.set_ask_ssid(ssid.into());
        pane = "wifi".into();
    }
    let pane = match pane.as_str() {
        "" => "appearance".to_string(),
        "network" | "wifi" | "bluetooth" | "battery" | "general" | "about" | "update" | "storage" | "datetime"
        | "language" | "login" | "appearance" | "menubar" | "dock" | "displays" | "wallpaper" | "sound" | "lock"
        | "users" | "keyboard" | "mouse" | "trackpad" | "notifications" | "accessibility" => pane,
        "shortcuts" | "keyboard-shortcuts" => {
            s.set_sheet("shortcuts".into());
            "keyboard".into()
        }
        "focus" | "dnd" => "notifications".into(),
        "energy" => "battery".into(),
        "display" => "displays".into(),
        "desktop" => "dock".into(),
        "date" | "time" => "datetime".into(),
        "software-update" => "update".into(),
        _ => "appearance".into(),
    };
    s.set_pane(pane.clone().into());
    s.set_login_name(login.clone().into());
    s.set_os_name(aqua_ui::sysdata::os_name().into());
    if let Some(p) = aqua_ui::sysdata::avatar() {
        if let Ok(img) = slint::Image::load_from_path(&p) {
            s.set_avatar(img);
            s.set_has_avatar(true);
        }
    }
    s.set_users(ModelRc::new(VecModel::from(
        aqua_ui::sysdata::users()
            .into_iter()
            .map(|(name, login, kind, current, initials)| UserRow {
                name: name.into(),
                login: login.into(),
                kind: kind.into(),
                current,
                initials: initials.into(),
            })
            .collect::<Vec<_>>(),
    )));
    shortcut_editor(&ui, cfg.clone());
    let refresh_volumes = {
        let ui = ui.as_weak();
        move || {
            let v: Vec<Volume> = aqua_ui::sysdata::volumes()
                .into_iter()
                .map(|(n, m, d, u)| Volume { name: n.into(), mount: m.into(), detail: d.into(), used: u })
                .collect();
            if let Some(u) = ui.upgrade() {
                u.global::<S>().set_volumes(ModelRc::new(VecModel::from(v)));
            }
        }
    };
    refresh_volumes();

    {
        use slint::winit_030::{winit, EventResult, WinitWindowAccessor};
        let w = ui.as_weak();
        let last_press: Rc<Cell<Option<std::time::Instant>>> = Rc::new(Cell::new(None));
        s.on_win_drag(move || {
            let Some(u) = w.upgrade() else { return };
            let now = std::time::Instant::now();
            if last_press.get().is_some_and(|t| now.duration_since(t) < std::time::Duration::from_millis(400)) {
                last_press.set(None);
                release_pointer_later(&u);
                u.global::<S>().invoke_win_title_double();
                return;
            }
            last_press.set(Some(now));
            u.window().with_winit_window(|win| {
                if let Err(e) = win.drag_window() {
                    eprintln!("aqua-settings: move failed: {e}");
                }
            });
            release_pointer_later(&u);
        });
        let w = ui.as_weak();
        s.on_win_resize(move |dir| {
            use winit::window::ResizeDirection as R;
            let d = match dir {
                0 => R::North,
                1 => R::South,
                2 => R::West,
                3 => R::East,
                4 => R::NorthWest,
                5 => R::NorthEast,
                6 => R::SouthWest,
                _ => R::SouthEast,
            };
            if let Some(u) = w.upgrade() {
                u.window().with_winit_window(|win| {
                    if let Err(e) = win.drag_resize_window(d) {
                        eprintln!("aqua-settings: resize ({d:?}) failed: {e}");
                    }
                });
                release_pointer_later(&u);
            }
        });
        s.on_win_close(|| {
            capture_marker(false);
            let _ = slint::quit_event_loop();
        });
        let w = ui.as_weak();
        s.on_win_minimize(move || {
            if let Some(u) = w.upgrade() {
                if u.window().with_winit_window(|w| w.set_minimized(true)).is_none() {
                    u.window().set_minimized(true);
                }
            }
        });
        let w = ui.as_weak();
        let c2 = cfg.clone();
        s.on_win_title_double(move || {
            let Some(u) = w.upgrade() else { return };
            let act = c2.borrow().titlebar_double_click.clone();
            match act.as_str() {
                "minimize" => {
                    if u.window().with_winit_window(|w| w.set_minimized(true)).is_none() {
                        u.window().set_minimized(true);
                    }
                }
                "none" => {}
                _ => {
                    if u.window().with_winit_window(|w| w.set_maximized(!w.is_maximized())).is_none() {
                        let win = u.window();
                        win.set_maximized(!win.is_maximized());
                    }
                }
            }
        });
        let w = ui.as_weak();
        s.on_win_zoom(move || {
            if let Some(u) = w.upgrade() {
                if u.window().with_winit_window(|w| w.set_maximized(!w.is_maximized())).is_none() {
                    let win = u.window();
                    win.set_maximized(!win.is_maximized());
                }
            }
        });
        let w = ui.as_weak();
        ui.window().on_winit_window_event(move |_, ev| {
            if let winit::event::WindowEvent::Focused(f) = ev {
                if let Some(u) = w.upgrade() {
                    if *f && u.window().is_minimized() {
                        u.window().set_minimized(false);
                    }
                    u.global::<S>().set_active(*f);
                }
            }
            EventResult::Propagate
        });
    }

    let history: Rc<RefCell<(Vec<String>, Vec<String>)>> = Rc::new(RefCell::new((vec![], vec![])));
    let on_enter: Rc<RefCell<Option<Box<dyn Fn(&str)>>>> = Rc::new(RefCell::new(None));
    let show_pane = {
        let ui = ui.as_weak();
        let history = history.clone();
        let on_enter = on_enter.clone();
        move |id: &str, record: bool| {
            let Some(u) = ui.upgrade() else { return };
            let s = u.global::<S>();
            let cur = s.get_pane().to_string();
            if cur == id {
                return;
            }
            if record {
                let mut h = history.borrow_mut();
                h.0.push(cur);
                h.1.clear();
            }
            s.set_pane(id.into());
            let h = history.borrow();
            s.set_can_back(!h.0.is_empty());
            s.set_can_forward(!h.1.is_empty());
            drop(h);
            if let Some(f) = on_enter.borrow().as_ref() {
                f(id);
            }
        }
    };
    let show_pane = Rc::new(show_pane);
    s.on_navigate({
        let show = show_pane.clone();
        move |id| show(id.as_str(), true)
    });
    s.on_go_back({
        let show = show_pane.clone();
        let history = history.clone();
        let ui = ui.as_weak();
        move || {
            let prev = history.borrow_mut().0.pop();
            if let Some(p) = prev {
                let cur = ui.upgrade().map(|u| u.global::<S>().get_pane().to_string()).unwrap_or_default();
                history.borrow_mut().1.push(cur);
                show(&p, false);
            }
        }
    });
    s.on_go_forward({
        let show = show_pane.clone();
        let history = history.clone();
        let ui = ui.as_weak();
        move || {
            let next = history.borrow_mut().1.pop();
            if let Some(n) = next {
                let cur = ui.upgrade().map(|u| u.global::<S>().get_pane().to_string()).unwrap_or_default();
                history.borrow_mut().0.push(cur);
                show(&n, false);
            }
        }
    });

    let save = {
        let ui = ui.as_weak();
        let cfg = cfg.clone();
        let seen = Rc::new(RefCell::new(Config::mtime()));
        move || {
            let ui = ui.unwrap();
            let mut c = cfg.borrow_mut();
            let mt = Config::mtime();
            if mt != *seen.borrow() {
                let outputs = c.outputs.clone();
                *c = Config::load();
                if c.outputs.is_empty() {
                    c.outputs = outputs;
                }
            }
            store_from(&ui, &mut c);
            let res = c.save();
            *seen.borrow_mut() = Config::mtime();
            match res {
                Ok(()) => ui.global::<S>().set_status("".into()),
                Err(e) => ui.global::<S>().set_status(trf("Could not save settings: {e}", &[("e", &e)]).into()),
            }
            apply_theme_from(&ui, &c);
        }
    };
    s.on_commit(save.clone());
    s.set_icon_apps(ModelRc::new(VecModel::from(icon_apps(&cfg.borrow()))));
    s.on_toggle_icon_app({
        let ui = ui.as_weak();
        let cfg = cfg.clone();
        move |id, on| {
            let Some(ui) = ui.upgrade() else { return };
            {
                let mut c = cfg.borrow_mut();
                let canon = aqua_config::apple_icons::canon;
                c.apple_icon_apps.retain(|x| canon(x) != canon(&id));
                if on {
                    c.apple_icon_apps.push(id.to_string());
                }
                if let Err(e) = c.save() {
                    ui.global::<S>().set_status(trf("Could not save settings: {e}", &[("e", &e)]).into());
                }
            }
            ui.global::<S>().set_icon_apps(ModelRc::new(VecModel::from(icon_apps(&cfg.borrow()))));
        }
    });
    s.on_search({
        let ui = ui.as_weak();
        move |q| {
            let q = q.to_lowercase();
            let m: Vec<bool> = PANES
                .iter()
                .map(|(_, kw)| q.trim().is_empty() || q.split_whitespace().all(|w| kw.contains(w)))
                .collect();
            let ui = ui.unwrap();
            if let Some(first) = m.iter().position(|x| *x) {
                let s = ui.global::<S>();
                let cur = s.get_pane();
                let parent = s.invoke_parent_of(cur.clone());
                if !PANES.iter().zip(&m).any(|((id, _), on)| *on && *id == parent.as_str()) {
                    s.invoke_navigate(PANES[first].0.into());
                }
            }
            ui.global::<S>().set_matches(ModelRc::new(VecModel::from(m)));
        }
    });
    s.on_wifi_toggle(|on| {
        aqua_sys::network::set_wifi_enabled(on);
        aqua_sys::invalidate();
    });
    s.on_wifi_scan(aqua_sys::network::scan);
    s.on_wifi_disconnect(|| {
        aqua_sys::network::disconnect();
        aqua_sys::invalidate();
    });
    s.on_wifi_connect({
        let ui = ui.as_weak();
        move |ssid, pw| {
            WIFI_CONNECTING.store(true, std::sync::atomic::Ordering::Relaxed);
            ui.unwrap().global::<S>().set_wifi_status(trf("Connecting to {ssid}…", &[("ssid", &ssid)]).into());
            let ui2 = ui.clone();
            let pw = if pw.is_empty() { None } else { Some(pw.to_string()) };
            aqua_sys::network::connect(ssid.to_string(), pw, move |r| {
                let msg = match r {
                    Ok(()) => trf("Connected to {ssid}", &[("ssid", &ssid)]),
                    Err(e) => trf("Could not join “{ssid}”: {e}", &[("ssid", &ssid), ("e", &e)]),
                };
                WIFI_CONNECTING.store(false, std::sync::atomic::Ordering::Relaxed);
                aqua_sys::invalidate();
                let _ = ui2.upgrade_in_event_loop(move |ui| ui.global::<S>().set_wifi_status(msg.into()));
            });
        }
    });
    s.on_bt_power(|on| {
        aqua_sys::bluetooth::set_powered(on);
        aqua_sys::invalidate();
    });
    s.on_bt_discover(|on| {
        aqua_sys::bluetooth::set_discovering(on);
        aqua_sys::invalidate();
    });
    s.on_bt_toggle(|p, c| {
        aqua_sys::bluetooth::toggle_device(p.to_string(), c);
        aqua_sys::invalidate();
    });
    s.on_set_low_power(|on| {
        aqua_sys::power::set_low_power(on);
        aqua_sys::invalidate();
    });
    s.on_set_volume(|v| {
        aqua_sys::audio::set_volume(v);
        aqua_sys::patch(|s| s.audio.volume = v);
    });
    s.on_set_muted(|m| {
        aqua_sys::audio::set_muted(m);
        aqua_sys::patch(|s| s.audio.muted = m);
    });
    s.on_set_input(aqua_sys::audio::set_input_volume);
    s.on_set_brightness(|v| {
        aqua_sys::backlight::set(v.max(0.02));
        aqua_sys::patch(|s| s.brightness = Some(v));
    });
    s.on_lock_now(lock_now);
    s.on_power_action(|a| {
        let r = match a.as_str() {
            "suspend" => aqua_sys::session::suspend(),
            "reboot" => aqua_sys::session::reboot(),
            _ => aqua_sys::session::power_off(),
        };
        if let Err(e) = r {
            eprintln!("aqua-settings: {a}: {e}");
        }
    });
    let refresh_outputs = wire_displays(&ui, &cfg, &walls);
    wire_keyboard(&ui, &cfg);
    let check_updates = wire_system(&ui, &cfg);
    let refresh_time = wire_datetime(&ui);
    let refresh_lang = wire_language(&ui);
    let refresh_login = wire_login(&ui, &cfg);
    *on_enter.borrow_mut() = Some(Box::new({
        let checked = Cell::new(false);
        let check = check_updates.clone();
        let time = refresh_time.clone();
        let lang = refresh_lang.clone();
        let login = refresh_login.clone();
        let vols = refresh_volumes.clone();
        move |id: &str| match id {
            "update" | "general" if !checked.get() => {
                checked.set(true);
                check();
            }
            "datetime" => time(),
            "language" => lang(),
            "login" => login(),
            "storage" | "about" => vols(),
            _ => {}
        }
    }));
    if let Some(f) = on_enter.borrow().as_ref() {
        f(&pane);
    }

    let timer = slint::Timer::default();
    let last = RefCell::new(0u64);
    let tick = RefCell::new(0u32);
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(1000), {
        let ui = ui.as_weak();
        let r = refresh_outputs.clone();
        let cfg = cfg.clone();
        let sig = RefCell::new(format!("{}|{:?}", outputs_text(), cfg.borrow().outputs));
        move || {
            if let Some(ui) = ui.upgrade() {
                refresh_sys(&ui, &mut last.borrow_mut());
                let mut t = tick.borrow_mut();
                *t += 1;
                if (*t).is_multiple_of(2) && ui.global::<S>().get_pane() == "displays" {
                    let now = format!("{}|{:?}", outputs_text(), cfg.borrow().outputs);
                    if *sig.borrow() != now {
                        *sig.borrow_mut() = now;
                        r();
                    }
                }
                if ui.global::<S>().get_pane() == "datetime" {
                    let h24 = ui.global::<S>().get_clock_24h();
                    ui.global::<S>().set_now_text(aqua_ui::sysdata::now_text(h24).into());
                }
            }
        }
    });
    refresh_sys(&ui, &mut 0);
    let _scale_watch = aqua_ui::watch_scale(ui.as_weak(), {
        let ui = ui.as_weak();
        move || {
            if let Some(ui) = ui.upgrade() {
                ui.set_repaint(ui.get_repaint() + 1);
            }
        }
    });
    ui.window().set_size(slint::LogicalSize::new(820.0, 640.0));
    let r = aqua_ui::run(ui.run());
    capture_marker(false);
    r
}
