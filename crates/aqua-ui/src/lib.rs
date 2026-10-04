//! Aqua system apps (Rust + Slint): System Settings, file chooser, polkit agent, greeter.
slint::include_modules!();
pub mod finder;
pub mod sysdata;

pub use aqua_i18n::{ntr, plural, tr, trf};

/// Switch every app's sidebar between the floating island and the full-height solid style
/// (saved to aqua.toml; open windows follow within a second via `apply_theme!`).
pub fn set_sidebar_style(solid: bool) {
    let mut cfg = aqua_config::Config::load();
    let v = if solid { "solid" } else { "floating" };
    if cfg.sidebar_style != v {
        cfg.sidebar_style = v.into();
        if let Err(e) = cfg.save() {
            eprintln!("cannot save sidebar style: {e}");
        }
    }
}

/// Apply the user's appearance (dark mode, accent) from aqua-config to a window's Theme global.
#[macro_export]
macro_rules! apply_theme {
    ($ui:expr) => {{
        let cfg = aqua_config::Config::load();
        let t = $ui.global::<$crate::Theme>();
        t.set_dark($crate::is_dark(&cfg));
        t.set_accent($crate::accent_color(&cfg.accent));
        t.set_solid_sidebar(cfg.solid_sidebar());
        t.set_glass_controls(cfg.glass_controls);
        t.set_glass_lights(cfg.glass_traffic_lights);
        t.set_motion(!cfg.reduce_motion);
        {
            let weak = $ui.as_weak();
            let last = std::cell::Cell::new(aqua_config::Config::mtime());
            let tick = std::cell::Cell::new(0u32);
            let timer = slint::Timer::default();
            timer.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(1), move || {
                let mt = aqua_config::Config::mtime();
                tick.set(tick.get().wrapping_add(1));
                if mt == last.get() && tick.get() % 30 != 0 {
                    return;
                }
                last.set(mt);
                let Some(u) = weak.upgrade() else { return };
                let cfg = aqua_config::Config::load();
                let t = u.global::<$crate::Theme>();
                let d = $crate::is_dark(&cfg);
                if t.get_dark() != d {
                    t.set_dark(d);
                }
                let a = $crate::accent_color(&cfg.accent);
                if t.get_accent() != a {
                    t.set_accent(a);
                }
                if t.get_solid_sidebar() != cfg.solid_sidebar() {
                    t.set_solid_sidebar(cfg.solid_sidebar());
                }
                if t.get_glass_controls() != cfg.glass_controls {
                    t.set_glass_controls(cfg.glass_controls);
                }
                if t.get_glass_lights() != cfg.glass_traffic_lights {
                    t.set_glass_lights(cfg.glass_traffic_lights);
                }
                if t.get_motion() == cfg.reduce_motion {
                    t.set_motion(!cfg.reduce_motion);
                }
            });
            std::mem::forget(timer);
        }
        $ui.global::<$crate::WinCtl>().on_tile_menu(|| {
            $crate::aqua_msg("tilemenu");
        });
        let weak = $ui.as_weak();
        $ui.global::<$crate::Backdrop>().on_capture(move || {
            if let Some(u) = weak.upgrade() {
                let dark = u.global::<$crate::Theme>().get_dark();
                let b = u.global::<$crate::Backdrop>();
                match $crate::blurred_snapshot(u.window(), dark) {
                    Some((img, w, h)) => {
                        b.set_image(img);
                        b.set_win_w(w);
                        b.set_win_h(h);
                        b.set_valid(true);
                    }
                    None => b.set_valid(false),
                }
            }
        });
    }};
}

/// A strongly blurred, downscaled copy of what the window currently shows (for the glass menus, see
/// `Backdrop` in common.slint).
pub fn blurred_snapshot(win: &slint::Window, dark: bool) -> Option<(slint::Image, f32, f32)> {
    let snap = win.take_snapshot().ok()?;
    let (w, h) = (snap.width() as usize, snap.height() as usize);
    if w == 0 || h == 0 {
        return None;
    }
    let scale = win.scale_factor().max(0.5);
    let f = ((8.0 * scale).round() as usize).max(2);
    let (sw, sh) = (w.div_ceil(f), h.div_ceil(f));
    let bg = if dark { [38.0f32, 38.0, 42.0] } else { [236.0f32, 238.0, 243.0] };
    let src = snap.as_bytes();
    let mut a = vec![0f32; sw * sh * 3];
    for sy in 0..sh {
        for sx in 0..sw {
            let mut acc = [0f32; 3];
            let mut n = 0f32;
            for y in (sy * f..((sy + 1) * f).min(h)).step_by(2) {
                for x in (sx * f..((sx + 1) * f).min(w)).step_by(2) {
                    let i = (y * w + x) * 4;
                    let al = src[i + 3] as f32 / 255.0;
                    for k in 0..3 {
                        acc[k] += src[i + k] as f32 * al + bg[k] * (1.0 - al);
                    }
                    n += 1.0;
                }
            }
            let o = (sy * sw + sx) * 3;
            for k in 0..3 {
                a[o + k] = acc[k] / n.max(1.0);
            }
        }
    }
    let pass = |from: &[f32], to: &mut [f32], horiz: bool| {
        let r = 2isize;
        for y in 0..sh as isize {
            for x in 0..sw as isize {
                let mut acc = [0f32; 3];
                for d in -r..=r {
                    let (xx, yy) = if horiz {
                        ((x + d).clamp(0, sw as isize - 1), y)
                    } else {
                        (x, (y + d).clamp(0, sh as isize - 1))
                    };
                    let i = (yy as usize * sw + xx as usize) * 3;
                    for k in 0..3 {
                        acc[k] += from[i + k];
                    }
                }
                let o = (y as usize * sw + x as usize) * 3;
                for k in 0..3 {
                    to[o + k] = acc[k] / (2 * r + 1) as f32;
                }
            }
        }
    };
    let mut tmp = vec![0f32; a.len()];
    for _ in 0..3 {
        pass(&a, &mut tmp, true);
        pass(&tmp, &mut a, false);
    }
    let mut out = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(sw as u32, sh as u32);
    for (i, p) in out.make_mut_slice().iter_mut().enumerate() {
        *p = slint::Rgba8Pixel {
            r: a[i * 3].round() as u8,
            g: a[i * 3 + 1].round() as u8,
            b: a[i * 3 + 2].round() as u8,
            a: 255,
        };
    }
    Some((slint::Image::from_rgba8(out), w as f32 / scale, h as f32 / scale))
}

pub fn is_dark(cfg: &aqua_config::Config) -> bool {
    cfg.resolve_dark()
}

pub const ACCENTS: [&str; 9] = ["multicolor", "blue", "purple", "pink", "red", "orange", "yellow", "green", "graphite"];

pub fn accent_color(name: &str) -> slint::Color {
    let rgb = match name {
        "purple" => 0xa550a7,
        "pink" => 0xf74f9e,
        "red" => 0xff5257,
        "orange" => 0xf7821b,
        "yellow" => 0xffc600,
        "green" => 0x62ba46,
        "graphite" => 0x8c8c8c,
        _ => 0x007aff,
    };
    slint::Color::from_rgb_u8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// Login name and full name (GECOS) of the current user.
pub fn user_names() -> (String, String) {
    let login = std::env::var("USER").unwrap_or_else(|_| "user".into());
    let full = std::fs::read_to_string("/etc/passwd")
        .ok()
        .and_then(|p| {
            p.lines()
                .find(|l| l.starts_with(&format!("{login}:")))
                .and_then(|l| l.split(':').nth(4))
                .map(|g| g.split(',').next().unwrap_or("").to_string())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| login.clone());
    (login, full)
}

/// Common start-up: xdg app id and the renderer.
pub fn init(app_id: &str) {
    // `~/.cache/aqua/logs/<app id>.log` + crash reports (see aqua-log).
    aqua_log::init(app_id);
    if std::env::var_os("SLINT_BACKEND").is_none() {
        let gl = std::env::var_os("AQUA_UI_SOFTWARE").is_none() && gl_available();
        std::env::set_var("SLINT_BACKEND", if gl { "winit-femtovg" } else { "winit-software" });
        if gl {
            std::env::set_var("AQUA_UI_AUTO_GL", "1");
        }
    }
    *APP_ID.lock().unwrap() = app_id.to_string();
}

/// Is there an EGL/GL driver we can load (Mesa, NVIDIA)?
fn gl_available() -> bool {
    let names: [&[u8]; 3] = [b"libEGL.so.1\0", b"libGL.so.1\0", b"libGLX.so.0\0"];
    names.iter().any(|n| unsafe {
        let h = libc::dlopen(n.as_ptr() as *const libc::c_char, libc::RTLD_LAZY | libc::RTLD_LOCAL);
        if h.is_null() {
            false
        } else {
            libc::dlclose(h);
            true
        }
    })
}

/// Run the event loop; if the automatically chosen OpenGL renderer fails to start,
/// restart the app with the software renderer instead of exiting.
pub fn run(r: Result<(), slint::PlatformError>) -> Result<(), slint::PlatformError> {
    if let Err(e) = &r {
        if std::env::var_os("AQUA_UI_AUTO_GL").is_some() {
            eprintln!("aqua: OpenGL renderer failed ({e}); falling back to software rendering");
            use std::os::unix::process::CommandExt;
            if let Ok(exe) = std::env::current_exe() {
                let err = std::process::Command::new(exe)
                    .args(std::env::args_os().skip(1))
                    .env("SLINT_BACKEND", "winit-software")
                    .env_remove("AQUA_UI_AUTO_GL")
                    .exec();
                eprintln!("aqua: restart failed: {err}");
            }
        }
    }
    r
}

static APP_ID: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// Apply the app id; Slint needs the platform to exist, so call this right after creating
/// the first window and before showing it.
pub fn set_app_id() {
    let id = APP_ID.lock().unwrap().clone();
    if !id.is_empty() {
        let _ = slint::set_xdg_app_id(id);
    }
}

/// Work around stale partial repaints when the compositor changes the fractional scale of a
/// running window: poll the scale factor and bump a `repaint` property on change.
pub fn watch_scale(
    window: slint::Weak<impl slint::ComponentHandle + 'static>,
    bump: impl Fn() + 'static,
) -> slint::Timer {
    let t = slint::Timer::default();
    let last = std::cell::Cell::new(0.0f32);
    t.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(300), move || {
        if let Some(w) = window.upgrade() {
            let s = w.window().scale_factor();
            if (s - last.get()).abs() > 0.001 {
                if last.get() != 0.0 {
                    bump();
                    w.window().request_redraw();
                }
                last.set(s);
            }
        }
    });
    t
}

/// Ask the compositor for background blur behind the whole window (KDE blur protocol via winit;
/// Aqua turns it into glass).
pub fn glass_supported() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").map(|d| d.to_lowercase().contains("aqua")).unwrap_or(false)
        && std::env::var_os("WAYLAND_DISPLAY").is_some()
        && !std::env::var("SLINT_BACKEND").map(|b| b.contains("software")).unwrap_or(false)
        && std::env::var_os("AQUA_NO_GLASS").is_none()
}

pub fn enable_glass<C: slint::ComponentHandle + 'static>(w: &slint::Weak<C>) {
    fn attempt<C: slint::ComponentHandle + 'static>(w: slint::Weak<C>, tries: u32) {
        use slint::winit_030::WinitWindowAccessor;
        let Some(u) = w.upgrade() else { return };
        if u.window().with_winit_window(|win| win.set_blur(true)).is_none() && tries < 40 {
            slint::Timer::single_shot(std::time::Duration::from_millis(50), move || attempt(w, tries + 1));
        }
    }
    let w = w.clone();
    slint::Timer::single_shot(std::time::Duration::from_millis(30), move || attempt(w, 0));
}

/// Send one command line to the running compositor's control socket
/// (`$AQUA_SOCKET`, else `$XDG_RUNTIME_DIR/aqua[-$WAYLAND_DISPLAY].sock`).
pub fn aqua_msg(line: &str) -> bool {
    use std::io::Write;
    let path = aqua_config::paths::control_socket();
    let Ok(mut s) = std::os::unix::net::UnixStream::connect(path) else { return false };
    let _ = s.set_write_timeout(Some(std::time::Duration::from_millis(500)));
    let mut l = line.replace('\n', " ");
    l.push('\n');
    s.write_all(l.as_bytes()).is_ok()
}

/// Apply `AQUA_LANG` to the Slint UI (without it Slint follows the system locale). Call
/// after the first window was created.
pub fn init_translations() {
    if let Ok(l) = std::env::var("AQUA_LANG") {
        let base = aqua_i18n::base_lang(&l);
        let _ = slint::select_bundled_translation(if base == "en" { "" } else { base });
    }
}

/// Menus may open as their own windows (placed by Aqua next to their owner, so they can
/// extend past the window — see the compositor's `wm::popups`). Elsewhere (other
/// compositors, X11, tests) menus stay inside the window. `AQUA_INWINDOW_MENUS=1` forces
/// the in-window fallback.
pub fn native_menus() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").map(|d| d.to_lowercase().contains("aqua")).unwrap_or(false)
        && std::env::var_os("WAYLAND_DISPLAY").is_some()
        && !std::env::var("SLINT_BACKEND").map(|b| b.contains("x11")).unwrap_or(false)
        && std::env::var_os("AQUA_INWINDOW_MENUS").is_none()
}

/// Window title asking Aqua to show a menu window at (`x`, `y`) relative to the app's
/// active window (`sub`: relative to its top-most open menu), at `alt_x` if it does not fit
/// to the right.
pub fn popup_title(x: f32, y: f32, alt_x: Option<f32>, sub: bool) -> String {
    let mut t = format!("aqua-popup:{}:{}", x.round() as i32, y.round() as i32);
    if alt_x.is_some() || sub {
        t.push_str(&format!(":{}", alt_x.map(|a| (a.round() as i32).to_string()).unwrap_or_default()));
    }
    if sub {
        t.push_str(":sub");
    }
    t
}

#[cfg(test)]
mod popup_tests {
    #[test]
    fn popup_titles() {
        assert_eq!(super::popup_title(10.4, 20.6, None, false), "aqua-popup:10:21");
        assert_eq!(super::popup_title(256.0, 40.0, Some(-236.0), true), "aqua-popup:256:40:-236:sub");
        assert_eq!(super::popup_title(5.0, 6.0, None, true), "aqua-popup:5:6::sub");
    }
}
