//! Headless smoke tests: build a real `Shell`, drive it through its public API and render
//! every panel without a compositor.
use aqua_shell::{menu::MenuKind, Action, Key, LayerId, Shell};
use std::path::PathBuf;
use std::sync::Once;

fn sandbox() -> PathBuf {
    static INIT: Once = Once::new();
    let dir = std::env::temp_dir().join(format!("aqua-shell-tests-{}", std::process::id()));
    INIT.call_once(|| {
        std::fs::create_dir_all(dir.join("config")).unwrap();
        std::env::set_var("AQUA_CONFIG", dir.join("config/config.toml"));
        std::env::set_var("XDG_CONFIG_HOME", dir.join("config"));
        std::env::set_var("XDG_CACHE_HOME", dir.join("cache"));
    });
    dir
}

fn shell(w: f32, h: f32, scale: f32) -> Shell {
    let dir = sandbox();
    let cfg = aqua_config::Config {
        fetch_icons: false,
        icon_cache: Some(dir.join("icons")),
        font_dir: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts")),
        ..Default::default()
    };
    let wp = aqua_wallpaper::generate((w * scale) as u32, (h * scale) as u32, aqua_wallpaper::Palette::Tahoe);
    Shell::new(cfg, w, h, scale, &wp)
}

fn ids(sh: &mut Shell) -> Vec<LayerId> {
    sh.layers().iter().map(|l| l.id).collect()
}

/// Let time-based animations finish (they advance with the wall clock).
fn settle(sh: &mut Shell) {
    let t0 = std::time::Instant::now();
    while t0.elapsed().as_millis() < 900 {
        sh.tick();
        let _ = sh.layers();
        std::thread::sleep(std::time::Duration::from_millis(8));
    }
}

#[test]
fn desktop_has_menu_bar_and_dock() {
    let mut sh = shell(1280.0, 800.0, 1.0);
    let l = ids(&mut sh);
    assert!(l.contains(&LayerId::MenuBar), "{l:?}");
    assert!(l.contains(&LayerId::Dock), "{l:?}");
    for layer in sh.layers() {
        assert!(layer.rect.w >= 0.0 && layer.rect.h >= 0.0);
        assert!((0.0..=1.0).contains(&layer.opacity), "{:?} opacity {}", layer.id, layer.opacity);
    }
}

#[test]
fn renders_at_various_sizes_and_scales() {
    for (w, h, s) in [(800.0, 600.0, 1.0), (1440.0, 900.0, 2.0), (1920.0, 1080.0, 1.25), (640.0, 400.0, 1.5)] {
        let mut sh = shell(w, h, s);
        settle(&mut sh);
        let wp = aqua_wallpaper::generate(400, 300, aqua_wallpaper::Palette::Tahoe);
        sh.resize(w * 0.75, h * 0.75, s, &wp);
        assert!(ids(&mut sh).contains(&LayerId::MenuBar));
    }
}

#[test]
fn launchpad_opens_and_escape_closes() {
    let mut sh = shell(1280.0, 800.0, 1.0);
    sh.toggle_launchpad();
    settle(&mut sh);
    assert!(ids(&mut sh).contains(&LayerId::Launchpad));
    let (consumed, _) = sh.key(Some(Key::Escape), None);
    assert!(consumed);
    settle(&mut sh);
    assert!(!ids(&mut sh).contains(&LayerId::Launchpad));
}

#[test]
fn spotlight_accepts_typing() {
    let mut sh = shell(1280.0, 800.0, 1.0);
    sh.toggle_spotlight();
    for c in ["2", "+", "2"] {
        sh.key(None, Some(c));
    }
    assert_eq!(sh.spotlight.query, "2+2");
    sh.key(Some(Key::Backspace), None);
    assert_eq!(sh.spotlight.query, "2+");
    settle(&mut sh);
    assert!(ids(&mut sh).iter().any(|l| matches!(l, LayerId::Spotlight | LayerId::SpotlightGlass)));
    sh.key(Some(Key::Escape), None);
    assert!(sh.spotlight.query.is_empty(), "first Esc clears the query");
    assert!(sh.spotlight.visible());
    sh.key(Some(Key::Escape), None);
    settle(&mut sh);
    assert!(!ids(&mut sh).contains(&LayerId::Spotlight));
}

#[test]
fn system_menu_opens_from_menu_bar() {
    let mut sh = shell(1280.0, 800.0, 1.0);
    let y = sh.cfg.menubar_height / 2.0;
    let _ = sh.layers();
    sh.pointer_motion(14.0, y);
    let _ = sh.pointer_button(14.0, y, true);
    let _ = sh.pointer_button(14.0, y, false);
    assert_eq!(sh.menu.open, Some(MenuKind::Apple));
    settle(&mut sh);
    assert!(ids(&mut sh).contains(&LayerId::Menu));
    sh.key(Some(Key::Escape), None);
    assert!(sh.menu.open.is_none());
}

#[test]
fn control_centre_and_notification_centre_render() {
    let mut sh = shell(1280.0, 800.0, 1.0);
    sh.control.toggle();
    settle(&mut sh);
    assert!(ids(&mut sh).contains(&LayerId::ControlCenter));
    sh.control.toggle();
    sh.toggle_notification_center();
    settle(&mut sh);
    assert!(ids(&mut sh).contains(&LayerId::NotificationCenter));
}

#[test]
fn notifications_show_a_banner() {
    let mut sh = shell(1280.0, 800.0, 1.0);
    sh.notify(aqua_shell::notifications::Note {
        id: 1,
        app_id: "org.example.App".into(),
        app_name: "Example".into(),
        summary: "Hello".into(),
        body: "World".into(),
        timeout: 5.0,
        ..Default::default()
    });
    settle(&mut sh);
    assert!(ids(&mut sh).contains(&LayerId::Banner));
}

#[test]
fn power_confirmation_alert() {
    let mut sh = shell(1280.0, 800.0, 1.0);
    sh.show_alert(aqua_shell::alert::power_confirm(Action::LogOut, "user"));
    settle(&mut sh);
    assert!(sh.has_modal());
    assert!(ids(&mut sh).contains(&LayerId::Alert));
    sh.key(Some(Key::Escape), None);
    settle(&mut sh);
    assert!(!ids(&mut sh).contains(&LayerId::Alert));
}

#[test]
fn hud_shows_volume() {
    let mut sh = shell(1280.0, 800.0, 1.0);
    sh.show_hud(aqua_shell::alert::HudKind::Volume, 0.5, "");
    settle(&mut sh);
    let _ = sh.layers();
}

#[test]
fn full_screen_keeps_bar_and_dock_hidden_at_the_edges() {
    let mut sh = shell(1280.0, 800.0, 1.0);
    sh.fullscreen = true;
    for (x, y) in [(640.0, 0.0), (640.0, 799.0), (10.0, 0.5), (640.0, 798.5)] {
        sh.pointer_motion(x, y);
        assert!(!sh.bar_shown(), "menu bar revealed at {x},{y}");
        assert!(!sh.dock_shown(), "Dock revealed at {x},{y}");
    }
    settle(&mut sh);
    let l = ids(&mut sh);
    assert!(!l.contains(&LayerId::MenuBar) && !l.contains(&LayerId::Dock), "{l:?}");

    // Leaving full screen brings both back; an auto-hidden Dock still peeks out at the edge.
    sh.fullscreen = false;
    sh.cfg.dock_autohide = true;
    sh.pointer_motion(640.0, 400.0);
    assert!(sh.bar_shown() && !sh.dock_shown());
    sh.pointer_motion(640.0, 799.0);
    assert!(sh.dock_shown(), "auto-hidden Dock reveals at the bottom edge");
}

/// Drawn centre of every Dock icon (what the next frame would show).
fn drawn(sh: &Shell) -> Vec<(String, f32)> {
    let (_, g) = aqua_shell::dock::geometry(sh);
    g.keys.iter().enumerate().map(|(i, k)| (k.clone(), g.slots[i].cx() + g.dx[i])).collect()
}

fn pinned_apps(sh: &Shell) -> Vec<String> {
    sh.cfg.dock.iter().map(|d| d.app.clone()).collect()
}

/// Dragging a Dock icon across its neighbours: they slide smoothly (no one-frame jump to
/// the new slot and back — the "blink"), and the drop reorders the kept icons.
#[test]
fn dock_drag_reorders_smoothly() {
    let mut sh = shell(1280.0, 800.0, 1.0);
    sh.cfg.dock_magnification = 1.0;
    for _ in 0..3 {
        aqua_shell::dock::sync(&mut sh, 0.016);
    }
    let before = pinned_apps(&sh);
    assert!(before.len() >= 4, "{before:?}");
    let (_, g) = aqua_shell::dock::geometry(&sh);
    let (from, to) = (g.slots[0], g.slots[3]);
    let dragged = g.keys[0].clone();
    let step_max = sh.cfg.dock_icon_size * 0.6;
    let _ = sh.pointer_button(from.cx(), from.cy(), true);
    let mut prev = drawn(&sh);
    let n = 24;
    for k in 1..=n {
        let f = k as f32 / n as f32;
        sh.pointer_motion(from.cx() + (to.cx() + 4.0 - from.cx()) * f, from.cy());
        // The compositor may draw right after the motion, before the next tick.
        let now = drawn(&sh);
        for (key, x) in &now {
            if *key == dragged {
                continue;
            }
            if let Some((_, px)) = prev.iter().find(|(k, _)| k == key) {
                assert!((x - px).abs() <= step_max, "icon {key} jumped {px} → {x} at step {k}");
            }
        }
        aqua_shell::dock::sync(&mut sh, 0.016);
        prev = drawn(&sh);
    }
    let _ = sh.pointer_button(to.cx() + 4.0, to.cy(), false);
    for _ in 0..40 {
        aqua_shell::dock::sync(&mut sh, 0.016);
    }
    let after = pinned_apps(&sh);
    assert_ne!(before, after, "the drop did not reorder the Dock");
    assert_eq!(after.iter().position(|a| *a == before[0]), Some(3), "{before:?} → {after:?}");
    assert_eq!(after.len(), before.len());
}

/// A kept icon dropped after the running-only apps stays the last kept icon instead of
/// silently snapping back, and running-only apps can be reordered among themselves.
#[test]
fn dock_drag_respects_running_apps() {
    let mut sh = shell(1280.0, 800.0, 1.0);
    sh.cfg.dock_magnification = 1.0;
    sh.cfg.dock_keep_order = true;
    let win = |id: u64, app: &str| aqua_shell::WindowInfo {
        id,
        app_id: app.into(),
        title: app.into(),
        focused: false,
        minimized: false,
    };
    sh.set_windows(vec![win(1, "org.example.alpha"), win(2, "org.example.beta")]);
    for _ in 0..40 {
        aqua_shell::dock::sync(&mut sh, 0.016);
    }
    let kept = pinned_apps(&sh);
    let (items, g) = aqua_shell::dock::geometry(&sh);
    let sep = items.iter().position(|i| i.kind == aqua_shell::dock::Kind::Separator).unwrap();
    let alpha = items.iter().position(|i| i.app == "org.example.alpha").unwrap();
    let beta = items.iter().position(|i| i.app == "org.example.beta").unwrap();
    assert!(alpha < beta && beta < sep);
    // Drag beta before alpha.
    let (b, a) = (g.slots[beta], g.slots[alpha]);
    let _ = sh.pointer_button(b.cx(), b.cy(), true);
    for k in 1..=12 {
        sh.pointer_motion(b.cx() + (a.x + 2.0 - b.cx()) * k as f32 / 12.0, b.cy());
        aqua_shell::dock::sync(&mut sh, 0.016);
    }
    let _ = sh.pointer_button(a.x + 2.0, a.cy(), false);
    for _ in 0..40 {
        aqua_shell::dock::sync(&mut sh, 0.016);
    }
    let (items, _) = aqua_shell::dock::geometry(&sh);
    let alpha2 = items.iter().position(|i| i.app == "org.example.alpha").unwrap();
    let beta2 = items.iter().position(|i| i.app == "org.example.beta").unwrap();
    assert!(beta2 < alpha2, "running-only apps were not reordered");
    assert_eq!(pinned_apps(&sh), kept, "reordering running apps must not pin them");
    // Drag the first kept icon to the far right of the app area.
    let (_, g) = aqua_shell::dock::geometry(&sh);
    let first = g.slots[0];
    let end = g.slots[sep - 1];
    let _ = sh.pointer_button(first.cx(), first.cy(), true);
    for k in 1..=20 {
        sh.pointer_motion(first.cx() + (end.right() - 2.0 - first.cx()) * k as f32 / 20.0, first.cy());
        aqua_shell::dock::sync(&mut sh, 0.016);
    }
    let _ = sh.pointer_button(end.right() - 2.0, end.cy(), false);
    for _ in 0..40 {
        aqua_shell::dock::sync(&mut sh, 0.016);
    }
    let after = pinned_apps(&sh);
    assert_eq!(after.last(), kept.first(), "{kept:?} → {after:?}");
}

fn app(id: &str, name: &str, exec: &str, icon: &str) -> aqua_apps::App {
    aqua_apps::App {
        id: id.into(),
        name: name.into(),
        exec: exec.into(),
        icon: icon.into(),
        categories: vec![],
        wm_class: None,
        path: PathBuf::new(),
        terminal: false,
        keywords: vec![],
        workdir: None,
    }
}

/// Several windows of one app share an icon; different apps launched through the same
/// wrapper (Steam games and the Steam client, Flatpak apps) each get their own, with
/// their own icon, and the icon's menu lists exactly its own windows.
#[test]
fn dock_groups_instances_and_separates_apps() {
    let mut sh = shell(1440.0, 900.0, 1.0);
    sh.apps = vec![
        app("Dota 2", "Dota 2", "steam steam://rungameid/570", "steam_icon_570"),
        app("steam", "Steam", "/usr/bin/steam %U", "steam"),
        app("The Outlast Trials", "The Outlast Trials", "steam steam://rungameid/1304930", "steam_icon_1304930"),
        app("org.example.Chat", "Chat", "flatpak run --command=chat org.example.Chat", "org.example.Chat"),
        app("org.example.Notes", "Notes", "flatpak run org.example.Notes", "org.example.Notes"),
        app("org.example.Editor", "Editor", "example-editor %F", "org.example.Editor"),
    ];
    let w = |id: u64, app: &str, title: &str| aqua_shell::WindowInfo {
        id,
        app_id: app.into(),
        title: title.into(),
        focused: id == 5,
        minimized: false,
    };
    sh.set_windows(vec![
        w(1, "steam", "Steam"),
        w(2, "steam_app_570", "Dota 2"),
        w(3, "steam_app_570", "Dota 2 — console"),
        w(4, "steam_app_1304930", "TOT"),
        w(5, "org.example.Editor", "a.txt"),
        w(6, "org.example.Editor", "b.txt"),
        w(7, "example-editor", "c.txt"),
        w(8, "org.example.Chat", "Chat"),
        w(9, "org.example.Notes", "Notes"),
        w(10, "steam_app_999", "Unknown game"),
    ]);
    let items = aqua_shell::dock::visible_items(&sh);
    let apps: Vec<_> = items.iter().filter(|i| i.kind == aqua_shell::dock::Kind::App).collect();
    for win in &sh.windows {
        let owners: Vec<_> = apps.iter().filter(|i| i.matches(&win.app_id)).map(|i| i.name.clone()).collect();
        assert_eq!(owners.len(), 1, "window {} ({}) belongs to {owners:?}", win.id, win.app_id);
    }
    let icon_of = |app_id: &str| apps.iter().find(|i| i.matches(app_id)).map(|i| i.icon.icon.clone()).unwrap();
    assert_eq!(icon_of("steam_app_570"), "steam_icon_570");
    assert_eq!(icon_of("steam_app_1304930"), "steam_icon_1304930");
    assert_eq!(icon_of("steam_app_999"), "steam_icon_999");
    assert_eq!(icon_of("org.example.Chat"), "org.example.Chat");
    assert_eq!(icon_of("org.example.Notes"), "org.example.Notes");
    let distinct = |ids: &[&str]| {
        let names: std::collections::HashSet<_> =
            ids.iter().map(|a| apps.iter().position(|i| i.matches(a)).unwrap()).collect();
        names.len()
    };
    assert_eq!(distinct(&["steam", "steam_app_570", "steam_app_1304930", "steam_app_999"]), 4);
    assert_eq!(distinct(&["org.example.Chat", "org.example.Notes"]), 2);
    assert_eq!(distinct(&["org.example.Editor", "example-editor"]), 1, "one app, one icon");
    // the editor's Dock menu lists its three windows and nothing else
    let idx = items.iter().position(|i| i.matches("org.example.Editor")).unwrap();
    let entries = aqua_shell::menu::entries(&sh, &MenuKind::Dock(idx));
    let wins: Vec<_> = entries
        .iter()
        .flatten()
        .filter(|e| matches!(e.action, Some(Action::FocusWindow(_)) | Some(Action::Restore(_))))
        .map(|e| e.label.clone())
        .collect();
    assert_eq!(wins, vec!["a.txt", "b.txt", "c.txt"]);
}

/// One app whose windows use different ids (Telegram: "org.telegram.desktop" on Wayland,
/// WM_CLASS "TelegramDesktop" on X11) stays under its single icon — pinned or not — and
/// several windows with the same title are told apart in the icon's menu.
#[test]
fn dock_groups_app_id_spellings_and_numbers_same_titles() {
    use aqua_shell::dock::Kind;
    let mut sh = shell(1440.0, 900.0, 1.0);
    let mut tg = app("org.telegram.desktop", "Telegram", "Telegram -- %u", "org.telegram.desktop");
    tg.wm_class = Some("TelegramDesktop".into());
    let mut files = app("org.gnome.Nautilus", "Files", "nautilus --new-window", "org.gnome.Nautilus");
    files.wm_class = Some("org.gnome.Nautilus".into());
    sh.apps = vec![tg, files, app("org.example.Chat", "Chat", "chat", "org.example.Chat")];
    // not kept in the Dock to begin with (the default "Messages" icon lists Telegram)
    sh.dock.items.retain(|i| !i.matches("org.telegram.desktop"));
    let w = |id: u64, app: &str, title: &str| aqua_shell::WindowInfo {
        id,
        app_id: app.into(),
        title: title.into(),
        focused: false,
        minimized: id == 6,
    };
    let wins = vec![
        w(1, "org.telegram.desktop", "Telegram"),
        w(2, "TelegramDesktop", "Saved Messages"),
        w(4, "org.gnome.Nautilus", "Home"),
        w(5, "org.gnome.Nautilus", "Home"),
        w(6, "org.gnome.Nautilus", "Home"),
        w(7, "org.example.Chat", "Chat"),
    ];
    let check = |sh: &Shell, pinned: bool| {
        let items = aqua_shell::dock::visible_items(sh);
        let apps: Vec<_> = items.iter().filter(|i| i.kind == Kind::App).collect();
        for win in &sh.windows {
            let owners: Vec<_> = apps.iter().filter(|i| i.matches(&win.app_id)).map(|i| i.name.clone()).collect();
            assert_eq!(owners.len(), 1, "pinned={pinned}: window {} ({}) belongs to {owners:?}", win.id, win.app_id);
        }
        let tg: Vec<_> = apps.iter().filter(|i| i.matches("TelegramDesktop") || i.matches("org.telegram.desktop")).collect();
        assert_eq!(tg.len(), 1, "pinned={pinned}: one Telegram icon");
        assert_eq!(tg[0].pinned, pinned);
        assert!(!apps.iter().any(|i| i.matches("org.example.Chat") && i.matches("org.gnome.Nautilus")));
        let it = aqua_shell::dock::app_item_for(sh, "TelegramDesktop").unwrap();
        assert!(it.matches("org.telegram.desktop"));
        let idx = items.iter().position(|i| i.matches("org.gnome.Nautilus")).unwrap();
        let labels: Vec<_> = aqua_shell::menu::entries(sh, &MenuKind::Dock(idx))
            .iter()
            .flatten()
            .filter(|e| matches!(e.action, Some(Action::FocusWindow(_)) | Some(Action::Restore(_))))
            .map(|e| e.label.clone())
            .collect();
        assert_eq!(labels, vec!["Home", "Home (2)", "Home (3)"]);
    };
    sh.set_windows(wins.clone());
    check(&sh, false);
    // pinned the way a user config pins it: app id + launcher, no aliases
    let mut pin = sh.dock.items.iter().find(|i| i.kind == Kind::App).cloned().unwrap();
    pin.name = "Telegram".into();
    pin.app = "org.telegram.desktop".into();
    pin.exec = "Telegram --".into();
    pin.pinned = true;
    pin.aliases = vec![];
    sh.dock.items.insert(0, pin);
    sh.set_windows(vec![]);
    sh.set_windows(wins);
    check(&sh, true);
}
