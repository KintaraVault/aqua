//! aqua-preview: renders the shell with the software compositor to a PNG.
//! Usage: aqua-preview [out.png] [--w 1440 --h 900 --scale 2] [--launchpad] [--control] [--menu apple|file] [--window] [--rclick X Y]
use aqua_shell::{soft, Shell};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let get = |k: &str, d: f32| {
        args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(d)
    };
    let out = args.get(1).filter(|a| !a.starts_with("--")).cloned().unwrap_or_else(|| "preview.png".into());
    let (w, h, scale) = (get("--w", 1440.0), get("--h", 900.0), get("--scale", 2.0));
    let mut cfg = aqua_config::Config::load();
    cfg.fetch_icons = args.iter().any(|a| a == "--fetch");
    let wp = aqua_wallpaper::wallpaper(cfg.wallpaper.as_deref(), (w * scale) as u32, (h * scale) as u32);
    let mut sh = Shell::new(cfg, w, h, scale, &wp);
    let mut windows = vec![];
    if args.iter().any(|a| a == "--window") {
        sh.set_windows(vec![aqua_shell::WindowInfo {
            id: 1,
            app_id: "weston-terminal".into(),
            title: "Terminal — zsh — 80×24".into(),
            focused: true,
            minimized: false,
        }]);
        let (ww, wh) = (720.0, 460.0);
        let mut c = aqua_gfx::Canvas::new(ww, wh, scale);
        c.fill_rect(aqua_gfx::Rect::new(0.0, 0.0, ww, wh), aqua_gfx::rgba(255, 255, 255, 1.0));
        let tb = aqua_shell::decor::titlebar(&sh.fonts, ww, scale, "Terminal — zsh — 80×24", true, false, false);
        c.blit(&tb, 0.0, 0.0);
        let f = sh.fonts.clone();
        c.text(
            &f,
            14.0,
            62.0,
            13.0,
            aqua_gfx::Weight::Regular,
            aqua_gfx::rgba(30, 30, 30, 1.0),
            "user@aqua ~ % cargo run --release",
        );
        windows.push(soft::SoftWindow { rect: aqua_gfx::Rect::new(380.0, 180.0, ww, wh), content: c.pm, radius: 16.0 });
    }
    if args.iter().any(|a| a == "--launchpad") {
        sh.toggle_launchpad();
        sh.launchpad.t = 1.0;
    }
    if let Some(i) = args.iter().position(|a| a == "--spotlight") {
        sh.toggle_spotlight();
        sh.spotlight.origin = None;
        if let Some(q) = args.get(i + 1).filter(|a| !a.starts_with("--")) {
            sh.spotlight.query = q.clone();
        }
        if let Some(f) = args
            .iter()
            .position(|a| a == "--filter")
            .and_then(|j| args.get(j + 1))
            .and_then(|v| v.parse::<usize>().ok())
        {
            sh.spotlight.set_filter(Some(aqua_shell::spotlight::Filter::ALL[(f.clamp(1, 4)) - 1]));
            std::thread::sleep(std::time::Duration::from_millis(400));
        }
        for _ in 0..90 {
            sh.spotlight.animate(1.0 / 60.0);
            let _ = sh.layers();
        }
    }
    if let Some(i) = args.iter().position(|a| a == "--tab") {
        let n: usize = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(0);
        sh.launchpad.tab = aqua_apps::Section::ALL.get(n).copied();
    }
    if args.iter().any(|a| a == "--lpmenu") {
        sh.launchpad.menu_open = true;
    }
    if let Some(i) = args.iter().position(|a| a == "--scroll") {
        let n: f32 = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(0.0);
        let _ = sh.layers();
        sh.launchpad.scroll.max = 1e6;
        sh.launchpad.scroll.jump(n);
    }
    if args.iter().any(|a| a == "--control") {
        sh.control.toggle();
        sh.control.t = 1.0;
    }
    if let Some(i) = args.iter().position(|a| a == "--menu") {
        sh.menu.open = Some(match args.get(i + 1).map(|s| s.as_str()) {
            Some("file") => aqua_shell::menu::MenuKind::App(0),
            Some("app") => aqua_shell::menu::MenuKind::AppName,
            _ => aqua_shell::menu::MenuKind::Apple,
        });
        sh.menu.anchor = if args.get(i + 1).map(|s| s == "file").unwrap_or(false) { 100.0 } else { 10.0 };
        sh.menu.hover = Some(2);
    }
    if let Some(i) = args.iter().position(|a| a == "--hover") {
        let x: f32 = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(0.0);
        let y: f32 = args.get(i + 2).and_then(|v| v.parse().ok()).unwrap_or(0.0);
        sh.pointer_motion(x, y);
    }
    if let Some(i) = args.iter().position(|a| a == "--rclick") {
        let x: f32 = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(0.0);
        let y: f32 = args.get(i + 2).and_then(|v| v.parse().ok()).unwrap_or(0.0);
        let (hit, _) = sh.pointer_secondary(x, y);
        if !hit {
            sh.open_desktop_menu(x, y);
        }
        sh.pointer_motion(x + 30.0, y + 40.0);
    }
    if args.iter().any(|a| a == "--launchpad") {
        for _ in 0..30 {
            sh.launchpad.animate(1.0 / 60.0);
            let _ = sh.layers();
        }
    }
    if let Some(i) = args.iter().position(|a| a == "--dockdrag") {
        let idx: usize = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(1);
        let tx: f32 = args.get(i + 2).and_then(|v| v.parse().ok()).unwrap_or(0.0);
        let ty: f32 = args.get(i + 3).and_then(|v| v.parse().ok()).unwrap_or(0.0);
        let frames = get("--frames", 30.0) as usize;
        aqua_shell::dock::sync(&mut sh, 0.016);
        let (_, geo) = aqua_shell::dock::geometry(&sh);
        let r = geo.slots[idx];
        let _ = sh.pointer_button(r.cx(), r.cy(), true);
        for k in 1..=10 {
            let f = k as f32 / 10.0;
            sh.pointer_motion(r.cx() + (tx - r.cx()) * f, r.cy() + (ty - r.cy()) * f);
            aqua_shell::dock::sync(&mut sh, 0.016);
        }
        if args.iter().any(|a| a == "--release") {
            let _ = sh.pointer_button(tx, ty, false);
        }
        for _ in 0..frames {
            aqua_shell::dock::sync(&mut sh, 0.016);
        }
    }
    if let Some(i) = args.iter().position(|a| a == "--dockadd") {
        let name = args.get(i + 1).cloned().unwrap_or_default();
        let frames = get("--frames", 6.0) as usize;
        aqua_shell::dock::sync(&mut sh, 0.016);
        let mut c = sh.cfg.clone();
        c.dock.insert(
            2,
            aqua_config::DockItem {
                name: name.clone(),
                app: name.to_lowercase(),
                exec: "/bin/true".into(),
                icon: name.to_lowercase(),
                ids: vec![],
            },
        );
        sh.cfg = c;
        sh.apply_config();
        for _ in 0..frames {
            aqua_shell::dock::sync(&mut sh, 0.016);
        }
    }
    if let Some(i) = args.iter().position(|a| a == "--shotui") {
        if args.get(i + 1).map(|s| s == "rec").unwrap_or(false) {
            sh.shot.kind = aqua_shell::screenshot::Kind::RecScreen;
        }
        sh.shot.open_toolbar();
        sh.shot.options_open = args.iter().any(|a| a == "--opts");
    }
    if args.iter().any(|a| a == "--recording") {
        sh.shot.recording = Some(std::time::Instant::now());
    }
    let wait = get("--wait", 0.0);
    if wait > 0.0 {
        let _ = sh.layers();
        std::thread::sleep(std::time::Duration::from_secs_f32(wait));
        sh.tick();
    }
    if let Some(i) = args.iter().position(|a| a == "--icons") {
        sh.cfg.icon_style = args.get(i + 1).cloned().unwrap_or_default();
        sh.apply_config();
    }
    let t = std::time::Instant::now();
    let layers = sh.layers();
    let img = soft::compose(&wp, &windows, &layers, scale);
    eprintln!("rendered {} layers in {:?}", layers.len(), t.elapsed());
    img.save_png(&out).unwrap();
}
