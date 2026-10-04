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
