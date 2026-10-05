//! Config ⇄ UI: loading the settings into the window and storing them back.
use super::*;
use aqua_config::GlassStyle;

/// Set while System Settings is joining a Wi-Fi network (see `on_wifi_connect`).
pub static WIFI_CONNECTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Installed apps that have a counterpart (Appearance → App Icons).
pub fn icon_apps(cfg: &Config) -> Vec<IconApp> {
    use aqua_config::apple_icons::{canon, lookup};
    let mut seen = std::collections::HashSet::new();
    let mut v: Vec<IconApp> = aqua_apps::scan()
        .into_iter()
        .filter_map(|a| {
            let c = lookup(&a.id, &a.icon).or_else(|| lookup(a.exec.split_whitespace().next().unwrap_or(""), ""))?;
            if !seen.insert(canon(&a.id)) {
                return None;
            }
            let enabled = cfg.apple_icon_apps.iter().any(|x| canon(x) == canon(&a.id));
            Some(IconApp { id: a.id.clone().into(), name: a.name.clone().into(), apple: c.apple.into(), enabled })
        })
        .collect();
    v.sort_by_key(|a| a.name.to_lowercase());
    v
}

pub fn idle_idx(s: u32) -> i32 {
    if s == 0 {
        return 0;
    }
    (1..IDLE.len()).min_by_key(|&i| (IDLE[i] as i64 - s as i64).abs()).unwrap_or(0) as i32
}
pub fn lerp_inv(v: f64, a: f64, b: f64) -> f32 {
    ((v - a) / (b - a)).clamp(0.0, 1.0) as f32
}
pub fn lerp(t: f32, a: f64, b: f64) -> f64 {
    a + (b - a) * t.clamp(0.0, 1.0) as f64
}

/// Liquid Glass sliders: (slider value of the style, apply a slider value to the style), in
/// the order clear, blur, refraction, edges, dispersion, vibrancy.
pub type GlassKnob = (fn(&GlassStyle) -> f32, fn(&mut GlassStyle, f32));
pub const GLASS_KNOBS: [GlassKnob; 6] = [
    (|g| 1.0 - lerp_inv(g.tint.3 as f64, 0.0, 0.35), |g, v| g.tint.3 = lerp(1.0 - v, 0.0, 0.35) as f32),
    (|g| lerp_inv(g.blur as f64, 0.0, 30.0), |g, v| g.blur = lerp(v, 0.0, 30.0) as f32),
    (|g| lerp_inv(g.refraction as f64, 0.0, 90.0), |g, v| g.refraction = lerp(v, 0.0, 90.0) as f32),
    (|g| lerp_inv(g.rim as f64, 0.0, 2.0), |g, v| g.rim = lerp(v, 0.0, 2.0) as f32),
    (|g| lerp_inv(g.dispersion as f64, 0.0, 20.0), |g, v| g.dispersion = lerp(v, 0.0, 20.0) as f32),
    (|g| lerp_inv(g.saturation as f64, 0.8, 2.0), |g, v| g.saturation = lerp(v, 0.8, 2.0) as f32),
];

fn glass_sliders(s: &S<'_>) -> [f32; 6] {
    [
        s.get_glass_clear(),
        s.get_glass_blur(),
        s.get_glass_refraction(),
        s.get_glass_edges(),
        s.get_glass_dispersion(),
        s.get_glass_vibrancy(),
    ]
}

/// Show `g` on the Liquid Glass sliders.
pub fn load_glass(s: &S<'_>, g: &GlassStyle) {
    let v: Vec<f32> = GLASS_KNOBS.iter().map(|(get, _)| get(g)).collect();
    s.set_glass_clear(v[0]);
    s.set_glass_blur(v[1]);
    s.set_glass_refraction(v[2]);
    s.set_glass_edges(v[3]);
    s.set_glass_dispersion(v[4]);
    s.set_glass_vibrancy(v[5]);
}

/// Write the sliders the user moved into `g` (untouched ones keep their exact value, also
/// one set beyond a slider's range in the file).
pub fn store_glass(s: &S<'_>, g: &mut GlassStyle) {
    for ((get, set), v) in GLASS_KNOBS.iter().zip(glass_sliders(s)) {
        if (get(g) - v).abs() > 1e-3 {
            set(g, v);
        }
    }
}

pub fn load_into(ui: &SettingsWindow, cfg: &Config) {
    let s = ui.global::<S>();
    load_glass(&s, &cfg.glass);
    s.set_appearance(match cfg.appearance.as_str() {
        "auto" => 0,
        "dark" => 2,
        "light" => 1,
        _ if cfg.dark => 2,
        _ => 1,
    });
    s.set_accent(ACCENTS.iter().position(|a| *a == cfg.accent).unwrap_or(0) as i32);
    s.set_minimize_idx(if cfg.minimize_effect == "scale" { 1 } else { 0 });
    s.set_show_widgets(cfg.show_widgets);
    s.set_clock_24h(cfg.clock_24h);
    s.set_clock_seconds(cfg.clock_seconds);
    s.set_clock_date(cfg.clock_date);
    s.set_menubar_bg(cfg.menubar_background);
    s.set_global_menu(cfg.global_menu);
    s.set_style_apps(cfg.style_apps);
    s.set_stage_manager(cfg.stage_manager);
    let hid = |n: &str| cfg.menubar_hidden.iter().any(|h| h == n);
    s.set_mb_wifi(!hid("wifi"));
    s.set_mb_battery(!hid("battery"));
    s.set_mb_input(!hid("input"));
    s.set_mb_search(!hid("search"));
    s.set_alert_sound(cfg.alert_sound);
    s.set_window_radius(lerp_inv(cfg.window_radius as f64, 0.0, 30.0));
    s.set_sidebar_style(match cfg.sidebar_style.as_str() { "floating" => 1, "solid" => 2, _ => 0 });
    s.set_glass_controls(cfg.glass_controls);
    s.set_glass_lights(cfg.glass_traffic_lights);
    s.set_window_glass(cfg.window_glass);
    s.set_dock_size(lerp_inv(cfg.dock_icon_size as f64, 32.0, 96.0));
    s.set_dock_magnify(cfg.dock_magnify);
    s.set_dock_mag(lerp_inv(cfg.dock_magnification as f64, 1.0, 2.5));
    s.set_menubar_size(if cfg.menubar_height < 25.0 {
        0
    } else if cfg.menubar_height < 28.0 {
        1
    } else {
        2
    });
    s.set_dock_autohide(cfg.dock_autohide);
    s.set_menubar_autohide_idx(MB_AUTOHIDE.iter().position(|x| *x == cfg.menubar_autohide).unwrap_or(0) as i32);
    s.set_apple_icons_idx(APPLE_ICONS.iter().position(|x| *x == cfg.apple_icons).unwrap_or(0) as i32);
    s.set_icon_style_idx(ICON_STYLES.iter().position(|x| *x == cfg.icon_style).unwrap_or(0) as i32);
    s.set_icon_glass(cfg.icon_glass);
    let hc: Vec<i32> =
        cfg.hot_corners.iter().map(|c| CORNERS.iter().position(|x| x == c).unwrap_or(0) as i32).collect();
    s.set_hc0(hc[0]);
    s.set_hc1(hc[1]);
    s.set_hc2(hc[2]);
    s.set_hc3(hc[3]);
    s.set_dim_idx(idle_idx(cfg.idle.dim_secs));
    s.set_off_idx(idle_idx(cfg.idle.screen_off_secs));
    s.set_lock_idx(idle_idx(cfg.idle.lock_secs));
    s.set_suspend_idx(idle_idx(cfg.idle.suspend_secs));
    s.set_lock_on_sleep(cfg.idle.lock_on_sleep);
    s.set_lock_on_start(cfg.lock_on_start);
    s.set_layouts(cfg.keyboard.layouts.join(", ").into());
    s.set_switch_idx(SWITCH.iter().position(|x| *x == cfg.keyboard.switch).unwrap_or(0) as i32);
    s.set_repeat_delay(1.0 - lerp_inv(cfg.keyboard.repeat_delay as f64, 150.0, 1000.0));
    s.set_repeat_rate(lerp_inv(cfg.keyboard.repeat_rate as f64, 5.0, 60.0));
    s.set_numlock(cfg.keyboard.numlock);
    s.set_xkb_options(cfg.keyboard.options.clone().into());
    s.set_natural_scroll(cfg.pointer.natural_scroll);
    s.set_tap_to_click(cfg.pointer.tap_to_click);
    s.set_tp_speed(lerp_inv(cfg.pointer.speed, -1.0, 1.0));
    s.set_gestures(cfg.pointer.gestures);
    s.set_dwt(cfg.pointer.disable_while_typing);
    s.set_mouse_natural(cfg.pointer.mouse_natural_scroll);
    s.set_mouse_speed(lerp_inv(cfg.pointer.mouse_speed, -1.0, 1.0));
    s.set_scroll_speed(lerp_inv(cfg.pointer.scroll_factor, 0.25, 3.0));
    s.set_pointer_accel(cfg.pointer.accel_profile != "flat");
    s.set_dock_click_idx(DOCK_CLICK.iter().position(|x| *x == cfg.dock_click).unwrap_or(0) as i32);
    s.set_dock_bounce(cfg.dock_bounce);
    s.set_dock_keep_order(cfg.dock_keep_order);
    s.set_title_dbl_idx(TITLE_DBL.iter().position(|x| *x == cfg.titlebar_double_click).unwrap_or(0) as i32);
    s.set_animate_windows(cfg.animate_windows);
    s.set_tile_by_drag(cfg.tile_by_drag);
    s.set_tile_margins(cfg.tile_margins);
    s.set_reduce_motion(cfg.reduce_motion);
    s.set_cursor_size(lerp_inv(cfg.cursor_size as f64, 1.0, 4.0));
    s.set_dnd(cfg.do_not_disturb);
    s.set_note_previews(cfg.notification_previews);
}

pub fn store_from(ui: &SettingsWindow, cfg: &mut Config) {
    let s = ui.global::<S>();
    cfg.appearance = ["auto", "light", "dark"][s.get_appearance().clamp(0, 2) as usize].into();
    cfg.dark = match s.get_appearance() {
        2 => true,
        1 => false,
        _ => is_dark(cfg),
    };
    cfg.accent = ACCENTS[s.get_accent().clamp(0, 8) as usize].into();
    cfg.minimize_effect = if s.get_minimize_idx() == 1 { "scale" } else { "genie" }.into();
    cfg.show_widgets = s.get_show_widgets();
    cfg.clock_24h = s.get_clock_24h();
    cfg.clock_seconds = s.get_clock_seconds();
    cfg.clock_date = s.get_clock_date();
    cfg.menubar_background = s.get_menubar_bg();
    cfg.global_menu = s.get_global_menu();
    cfg.style_apps = s.get_style_apps();
    cfg.stage_manager = s.get_stage_manager();
    cfg.menubar_hidden = [
        ("wifi", s.get_mb_wifi()),
        ("battery", s.get_mb_battery()),
        ("input", s.get_mb_input()),
        ("search", s.get_mb_search()),
    ]
    .iter()
    .filter(|(_, on)| !on)
    .map(|(n, _)| n.to_string())
    .collect();
    cfg.alert_sound = s.get_alert_sound();
    cfg.window_radius = lerp(s.get_window_radius(), 0.0, 30.0).round() as f32;
    cfg.sidebar_style = match s.get_sidebar_style() { 1 => "floating", 2 => "solid", _ => "aqua" }.into();
    cfg.glass_controls = s.get_glass_controls();
    cfg.glass_traffic_lights = s.get_glass_lights();
    cfg.window_glass = s.get_window_glass();
    store_glass(&s, &mut cfg.glass);
    cfg.dock_icon_size = lerp(s.get_dock_size(), 32.0, 96.0).round() as f32;
    cfg.dock_magnify = s.get_dock_magnify();
    if s.get_dock_magnify() {
        cfg.dock_magnification = (lerp(s.get_dock_mag(), 1.0, 2.5) * 100.0).round() as f32 / 100.0;
    }
    cfg.menubar_height = MENUBAR[s.get_menubar_size().clamp(0, 2) as usize];
    cfg.dock_autohide = s.get_dock_autohide();
    cfg.menubar_autohide = MB_AUTOHIDE[s.get_menubar_autohide_idx().clamp(0, 2) as usize].into();
    cfg.apple_icons = APPLE_ICONS[s.get_apple_icons_idx().clamp(0, 2) as usize].into();
    cfg.icon_style = ICON_STYLES[s.get_icon_style_idx().clamp(0, 4) as usize].into();
    cfg.icon_glass = s.get_icon_glass();
    for (i, v) in [s.get_hc0(), s.get_hc1(), s.get_hc2(), s.get_hc3()].into_iter().enumerate() {
        cfg.hot_corners[i] = CORNERS[v.clamp(0, 5) as usize].into();
    }
    let t = |i: i32| IDLE[i.clamp(0, 7) as usize];
    cfg.idle.dim_secs = t(s.get_dim_idx());
    cfg.idle.screen_off_secs = t(s.get_off_idx());
    cfg.idle.lock_secs = t(s.get_lock_idx());
    cfg.idle.suspend_secs = t(s.get_suspend_idx());
    cfg.idle.lock_on_sleep = s.get_lock_on_sleep();
    cfg.lock_on_start = s.get_lock_on_start();
    cfg.keyboard.switch = SWITCH[s.get_switch_idx().clamp(0, 3) as usize].into();
    cfg.keyboard.repeat_delay = lerp(1.0 - s.get_repeat_delay(), 150.0, 1000.0).round() as i32;
    cfg.keyboard.repeat_rate = lerp(s.get_repeat_rate(), 5.0, 60.0).round() as i32;
    cfg.keyboard.numlock = s.get_numlock();
    cfg.keyboard.options = s.get_xkb_options().trim().to_string();
    cfg.pointer.natural_scroll = s.get_natural_scroll();
    cfg.pointer.tap_to_click = s.get_tap_to_click();
    cfg.pointer.speed = (lerp(s.get_tp_speed(), -1.0, 1.0) * 100.0).round() / 100.0;
    cfg.pointer.gestures = s.get_gestures();
    cfg.pointer.disable_while_typing = s.get_dwt();
    cfg.pointer.mouse_natural_scroll = s.get_mouse_natural();
    cfg.pointer.mouse_speed = (lerp(s.get_mouse_speed(), -1.0, 1.0) * 100.0).round() / 100.0;
    cfg.pointer.scroll_factor = (lerp(s.get_scroll_speed(), 0.25, 3.0) * 100.0).round() / 100.0;
    cfg.pointer.accel_profile = if s.get_pointer_accel() { "adaptive" } else { "flat" }.into();
    cfg.dock_click = DOCK_CLICK[s.get_dock_click_idx().clamp(0, 4) as usize].into();
    cfg.dock_bounce = s.get_dock_bounce();
    cfg.dock_keep_order = s.get_dock_keep_order();
    cfg.titlebar_double_click = TITLE_DBL[s.get_title_dbl_idx().clamp(0, 2) as usize].into();
    cfg.animate_windows = s.get_animate_windows();
    cfg.tile_by_drag = s.get_tile_by_drag();
    cfg.tile_margins = s.get_tile_margins();
    cfg.reduce_motion = s.get_reduce_motion();
    cfg.cursor_size = ((lerp(s.get_cursor_size(), 1.0, 4.0) * 2.0).round() / 2.0) as f32;
    cfg.do_not_disturb = s.get_dnd();
    cfg.notification_previews = s.get_note_previews();
}

pub fn output_cfg<'a>(cfg: &'a mut Config, name: &str) -> &'a mut OutputCfg {
    if let Some(i) = cfg.outputs.iter().position(|o| o.name == name) {
        return &mut cfg.outputs[i];
    }
    cfg.outputs.push(OutputCfg { name: name.into(), ..Default::default() });
    cfg.outputs.last_mut().unwrap()
}

pub fn wallpapers(cfg: &Config) -> (Vec<WallItem>, i32) {
    let mut paths: Vec<std::path::PathBuf> = vec![];
    let home = dirs::home_dir().unwrap_or_default();
    for d in [
        std::path::PathBuf::from("/usr/share/backgrounds"),
        "/usr/share/wallpapers".into(),
        home.join("Pictures"),
        home.join(".local/share/backgrounds"),
    ] {
        let mut stack = vec![(d, 0)];
        while let Some((dir, depth)) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            let mut ents: Vec<_> = rd.flatten().map(|e| e.path()).collect();
            ents.sort();
            for p in ents {
                if p.is_dir() && depth < 3 {
                    stack.push((p, depth + 1));
                } else if matches!(
                    p.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref(),
                    Some("jpg" | "jpeg" | "png" | "webp")
                ) && paths.len() < 23
                {
                    paths.push(p);
                }
            }
        }
    }
    let mut v = vec![WallItem { path: "".into(), label: tr("Aqua (dynamic)").into(), img: Default::default() }];
    let mut cur = 0;
    for (i, p) in paths.iter().enumerate() {
        if cfg.wallpaper.as_deref() == Some(p.as_path()) {
            cur = i as i32 + 1;
        }
        v.push(WallItem {
            path: p.to_string_lossy().to_string().into(),
            label: p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default().into(),
            img: slint::Image::load_from_path(p).unwrap_or_default(),
        });
    }
    (v, cur)
}

pub fn refresh_sys(ui: &SettingsWindow, last: &mut u64) {
    let snap = aqua_sys::snapshot();
    if snap.serial == *last && *last != 0 {
        return;
    }
    *last = snap.serial.max(1);
    let s = ui.global::<S>();
    let n = &snap.net;
    s.set_has_wifi(n.has_wifi);
    s.set_wifi_enabled(n.wifi_enabled);
    s.set_net_backend(format!("{:?}", n.backend).into());
    let nets: Vec<WifiNet> = n
        .networks
        .iter()
        .map(|x| WifiNet {
            ssid: x.ssid.clone().into(),
            signal: x.signal as i32,
            secure: x.secure,
            active: x.active,
            known: x.known,
        })
        .collect();
    s.set_networks(ModelRc::new(VecModel::from(nets)));
    s.set_wired(n.wired.clone().unwrap_or_default().into());
    // While a join is in progress the status line shows its progress, not the periodic state.
    if !WIFI_CONNECTING.load(std::sync::atomic::Ordering::Relaxed) {
        s.set_wifi_status(
            match (n.has_wifi, n.wifi_enabled, n.active()) {
                (false, _, _) => tr("No Wi-Fi hardware").to_string(),
                (_, false, _) => tr("Off").into(),
                (_, _, Some(a)) => trf("Connected to {ssid}", &[("ssid", &a.ssid)]),
                _ => tr("Not connected").into(),
            }
            .into(),
        );
    }
    let bt = &snap.bt;
    s.set_bt_available(bt.available);
    s.set_bt_powered(bt.powered);
    s.set_bt_discovering(bt.discovering);
    s.set_bt_adapter(bt.adapter.clone().into());
    let devs: Vec<BtDev> = bt
        .devices
        .iter()
        .map(|d| BtDev {
            path: d.path.clone().into(),
            name: d.name.clone().into(),
            kind: d.icon.clone().into(),
            connected: d.connected,
            paired: d.paired,
            battery: d.battery.map(|b| b as i32).unwrap_or(-1),
        })
        .collect();
    s.set_bt_devices(ModelRc::new(VecModel::from(devs)));
    let p = &snap.power;
    s.set_has_battery(p.level.is_some());
    s.set_battery_level(p.level.unwrap_or(1.0));
    s.set_charging(p.charging);
    s.set_low_power(p.low_power);
    s.set_power_text(
        match p.level {
            None => tr("Powered by AC adapter — no battery present").to_string(),
            Some(l) => format!(
                "{:.0}% · {}{}",
                l * 100.0,
                if p.charging {
                    tr("Charging")
                } else if p.on_ac {
                    tr("On power adapter")
                } else {
                    tr("On battery")
                },
                p.minutes
                    .map(|m| format!(
                        " · {}",
                        trf("{time} remaining", &[("time", &format!("{}:{:02}", m / 60, m % 60))])
                    ))
                    .unwrap_or_default()
            ),
        }
        .into(),
    );
    let a = &snap.audio;
    s.set_audio_available(a.available);
    s.set_audio_device(a.device.clone().into());
    s.set_audio_backend(a.backend.into());
    s.set_volume(a.volume.min(1.0));
    s.set_muted(a.muted);
    s.set_input_volume(a.input_volume.min(1.0));
    s.set_has_brightness(snap.brightness.is_some());
    s.set_brightness(snap.brightness.unwrap_or(1.0));
}

pub fn lock_now() {
    if let Ok(ctl) = std::env::var("AQUA_CTL") {
        let _ = std::fs::write(ctl, "lock\n");
    } else {
        let _ = std::process::Command::new("loginctl").arg("lock-session").status();
    }
}
