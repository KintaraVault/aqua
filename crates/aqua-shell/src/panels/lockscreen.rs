//! Lock screen: date, huge clock, user avatar, glass password pill,
//! status icons (input source, battery, Wi-Fi, power) in the top-right corner.
use crate::{clock, hash_of, style, Action, Key, Layer, LayerId, Shell};
use aqua_config::Rgba;
use aqua_gfx::{rgba, symbols, Canvas, Rect, Weight};
use std::time::Instant;

#[derive(Default)]
pub struct LockScreen {
    /// (login, full name)
    pub user: (String, String),
    pub password: String,
    pub busy: bool,
    pub fail_at: Option<Instant>,
    pub lockout_until: Option<Instant>,
    pub show_hint: bool,
    pub power_menu: bool,
    pub hover: Option<Hit>,
    pub caps: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hit {
    Field,
    Help,
    Input,
    Power,
    Sleep,
    Restart,
    ShutDown,
}

impl LockScreen {
    pub fn reset(&mut self) {
        self.password.clear();
        self.busy = false;
        self.fail_at = None;
        self.show_hint = false;
        self.power_menu = false;
    }
    pub fn fail(&mut self) {
        self.password.clear();
        self.fail_at = Some(Instant::now());
        self.show_hint = true;
    }
    /// Horizontal shake offset after a wrong password (damped sine, 0.5 s).
    pub fn shake(&self) -> f32 {
        match self.fail_at {
            Some(t) => {
                let s = t.elapsed().as_secs_f32();
                if s > 0.5 {
                    0.0
                } else {
                    (s * 38.0).sin() * 14.0 * (1.0 - s / 0.5)
                }
            }
            None => 0.0,
        }
    }
    pub fn animating(&self) -> bool {
        self.busy || self.fail_at.map(|t| t.elapsed().as_secs_f32() < 0.55).unwrap_or(false)
    }
    pub fn locked_out(&self) -> Option<u64> {
        self.lockout_until.and_then(|t| t.checked_duration_since(Instant::now())).map(|d| d.as_secs() + 1)
    }
}

fn initials(name: &str) -> String {
    name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase()
}

struct Geo {
    avatar: Rect,
    field: Rect,
    help: Rect,
    input: Rect,
    power: Rect,
    menu: Rect,
}

fn geo(sh: &Shell) -> Geo {
    let (w, h) = (sh.w, sh.h);
    let fw = 210.0;
    let field = Rect::new((w - fw) / 2.0 - 18.0, h - 118.0, fw, 34.0);
    let avatar = Rect::new(w / 2.0 - 30.0, field.y - 112.0, 60.0, 60.0);
    let help = Rect::new(field.right() + 8.0, field.y + 2.0, 30.0, 30.0);
    let power = Rect::new(w - 44.0, 8.0, 30.0, 26.0);
    let input = Rect::new(w - 260.0, 8.0, 120.0, 26.0);
    let menu = Rect::new(w - 200.0, 40.0, 186.0, 3.0 * 30.0 + 12.0);
    Geo { avatar, field, help, input, power, menu }
}

fn menu_rows(g: &Geo) -> [(Hit, &'static str, Rect); 3] {
    let r = |i: f32| Rect::new(g.menu.x + 6.0, g.menu.y + 6.0 + i * 30.0, g.menu.w - 12.0, 30.0);
    [(Hit::Sleep, "Sleep", r(0.0)), (Hit::Restart, "Restart", r(1.0)), (Hit::ShutDown, "Shut Down", r(2.0))]
}

/// Lock screen layers. `progress` 0..1 fades/zooms the screen in.
pub fn layers(sh: &mut Shell, progress: f32) -> Vec<Layer> {
    let (w, h) = (sh.w, sh.h);
    let g = geo(sh);
    let n = clock::now();
    let snap = aqua_sys::snapshot();
    let ls = &sh.lockscreen;
    let layout =
        crate::sysinfo::layout_name(sh.layouts.get(sh.layout_idx).map(|s| s.as_str()).unwrap_or("us")).to_string();
    let shake = ls.shake();
    let lockout = ls.locked_out();
    let key = hash_of(&(
        (n.minute, n.hour, n.day, ls.password.len(), ls.busy, ls.show_hint, ls.power_menu, ls.hover, ls.user.clone()),
        ((shake * 4.0) as i32, lockout, layout.clone(), snap.serial, ls.caps, w as i32, h as i32, sh.cfg.clock_24h),
    ));
    let (pm, serial) = sh.cached(LayerId::Lock, key, w, h, |c, sh| {
        let f = sh.fonts.clone();
        let ls = &sh.lockscreen;
        let white = rgba(255, 255, 255, 0.96);
        let soft = rgba(255, 255, 255, 0.78);
        c.fill_rrect_vgrad(Rect::new(0.0, 0.0, w, h * 0.45), 0.0, rgba(0, 0, 0, 0.22), rgba(0, 0, 0, 0.0));
        c.fill_rrect_vgrad(Rect::new(0.0, h * 0.62, w, h * 0.38), 0.0, rgba(0, 0, 0, 0.0), rgba(0, 0, 0, 0.30));
        let date = format!(
            "{} {} {}",
            crate::tr(clock::WEEKDAYS[n.weekday]),
            n.day,
            crate::tr(clock::MONTHS[(n.month - 1) as usize])
        );
        let top = h * 0.085;
        c.text_in(&f, Rect::new(0.0, top, w, 34.0), 0.5, 25.0, Weight::Semibold, soft, &date);
        let hour = if sh.cfg.clock_24h {
            n.hour
        } else {
            let x = n.hour % 12;
            if x == 0 {
                12
            } else {
                x
            }
        };
        let time =
            if sh.cfg.clock_24h { format!("{:02}:{:02}", hour, n.minute) } else { format!("{}:{:02}", hour, n.minute) };
        let size = (h * 0.205).clamp(90.0, 240.0);
        c.text_in(
            &f,
            Rect::new(0.0, top + 34.0, w, size * 1.02),
            0.5,
            size,
            Weight::Bold,
            rgba(255, 255, 255, 0.88),
            &time,
        );

        let st_col = rgba(255, 255, 255, 0.95);
        let pr = g.power;
        draw_power(c, Rect::new(pr.cx() - 8.0, pr.cy() - 8.0, 16.0, 16.0), st_col);
        let mut x = pr.x - 8.0;
        if snap.net.wifi_enabled || snap.net.backend == aqua_sys::NetBackend::None {
            x -= 22.0;
            symbols::wifi(c, Rect::new(x, pr.cy() - 7.5, 18.0, 15.0), st_col);
        }
        if let Some(l) = snap.power.level {
            x -= 34.0;
            symbols::battery(c, Rect::new(x, pr.cy() - 6.0, 27.0, 12.0), l, st_col);
        }
        x -= 26.0;
        draw_keyboard(c, Rect::new(x, pr.cy() - 7.0, 20.0, 14.0), st_col);
        let lw = f.measure(&layout, 13.5, Weight::Medium);
        x -= lw + 8.0;
        c.text(&f, x, pr.cy() + 5.0, 13.5, Weight::Medium, st_col, &layout);

        let a = g.avatar;
        c.fill_circle(a.cx(), a.cy(), a.w / 2.0, rgba(255, 255, 255, 0.30));
        c.fill_circle(a.cx(), a.cy(), a.w / 2.0 - 1.0, rgba(160, 164, 176, 0.85));
        let name = if ls.user.1.is_empty() { ls.user.0.clone() } else { ls.user.1.clone() };
        c.text_in(&f, a, 0.5, 24.0, Weight::Semibold, white, &initials(&name));
        c.text_in(&f, Rect::new(0.0, a.bottom() + 8.0, w, 24.0), 0.5, 16.0, Weight::Semibold, white, &name);

        let fr = g.field.translate(shake, 0.0);
        if ls.hover == Some(Hit::Field) {
            c.fill_rrect(fr, fr.h / 2.0, rgba(255, 255, 255, 0.06));
        }
        if let Some(secs) = lockout {
            c.text_in(
                &f,
                fr,
                0.5,
                13.0,
                Weight::Regular,
                soft,
                &crate::trf("Try again in {secs} s", &[("secs", &secs)]),
            );
        } else if ls.busy {
            let (cx, cy) = (fr.cx(), fr.cy());
            let t =
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
            let phase = ((t / 80) % 12) as usize;
            for i in 0..12 {
                let ang = i as f32 / 12.0 * std::f32::consts::TAU;
                let al = 0.25 + 0.75 * (((i + 12 - phase) % 12) as f32 / 12.0);
                c.fill_circle(cx + ang.cos() * 8.0, cy + ang.sin() * 8.0, 1.6, rgba(255, 255, 255, al));
            }
        } else if ls.password.is_empty() {
            c.text(&f, fr.x + 16.0, fr.cy() + 5.0, 14.0, Weight::Regular, rgba(255, 255, 255, 0.62), "Enter Password");
        } else {
            let nd = ls.password.chars().count().min(22);
            for i in 0..nd {
                c.fill_circle(fr.x + 18.0 + i as f32 * 9.5, fr.cy(), 3.3, white);
            }
        }
        if ls.caps && !ls.busy {
            let cr = Rect::new(fr.right() - 26.0, fr.cy() - 8.0, 16.0, 16.0);
            c.fill_rrect(cr, 4.0, rgba(255, 255, 255, 0.85));
            c.text_in(&f, cr, 0.5, 11.0, Weight::Bold, rgba(60, 60, 70, 1.0), "⇪");
        }
        let hb = g.help.translate(shake, 0.0);
        c.text_in(
            &f,
            hb,
            0.5,
            15.0,
            Weight::Semibold,
            rgba(255, 255, 255, if ls.hover == Some(Hit::Help) { 1.0 } else { 0.85 }),
            "?",
        );
        let hint = if ls.show_hint && ls.fail_at.is_some() {
            "Incorrect password. Try again."
        } else if ls.show_hint {
            "Type your login password and press Return"
        } else {
            "Your password is required to log in"
        };
        c.text_in(&f, Rect::new(0.0, g.field.bottom() + 12.0, w, 18.0), 0.5, 12.5, Weight::Regular, soft, hint);

        if ls.power_menu {
            let m = g.menu;
            c.fill_rrect(m, 12.0, rgba(40, 40, 46, 0.55));
            c.stroke_rrect(m, 12.0, rgba(255, 255, 255, 0.18), 1.0);
            for (hit, label, r) in menu_rows(&g) {
                if ls.hover == Some(hit) {
                    c.fill_rrect(r, 7.0, style::accent(0.9));
                }
                c.text(&f, r.x + 12.0, r.cy() + 5.0, 13.5, Weight::Regular, white, label);
            }
        }
    });
    let ease = 1.0 - (1.0 - progress).powi(3);
    let mut out = vec![];
    let back = aqua_config::GlassStyle {
        blur: 24.0 * ease,
        tint: Rgba(0.05, 0.05, 0.08, 0.10 * ease),
        saturation: 1.15,
        refraction: 0.0,
        bevel: 0.0,
        rim: 0.0,
        radius: 0.0,
        shadow: 0.0,
        max_luma: 0.85,
    };
    let pill = aqua_config::GlassStyle {
        blur: 18.0,
        tint: Rgba(1.0, 1.0, 1.0, 0.16),
        saturation: 1.4,
        refraction: 4.0,
        bevel: 8.0,
        rim: 0.5,
        radius: g.field.h / 2.0,
        shadow: 0.0,
        max_luma: 0.7,
    };
    let help = aqua_config::GlassStyle { radius: g.help.h / 2.0, ..pill };
    let shake = sh.lockscreen.shake();
    out.push(Layer {
        id: LayerId::Lock,
        rect: Rect::new(0.0, 0.0, w, h),
        glass: Some(back),
        tiles: vec![(g.field.translate(shake, 0.0), pill), (g.help.translate(shake, 0.0), help)],
        content: pm,
        serial,
        opacity: ease,
        zoom: 1.04 - 0.04 * ease,
    });
    out
}

fn draw_power(c: &mut Canvas, r: Rect, col: aqua_gfx::Color) {
    use aqua_gfx::tiny_skia::PathBuilder;
    let (cx, cy, rad) = (r.cx(), r.cy() + 1.0, r.w * 0.42);
    let mut pb = PathBuilder::new();
    let a0 = -60f32.to_radians() - std::f32::consts::FRAC_PI_2;
    let steps = 40;
    for i in 0..=steps {
        let a = a0 + (i as f32 / steps as f32) * (300f32.to_radians());
        let (x, y) = (cx + -(a.cos() * rad), cy + a.sin() * rad);
        if i == 0 {
            pb.move_to(x, y)
        } else {
            pb.line_to(x, y)
        }
    }
    pb.move_to(cx, cy - rad - 1.5);
    pb.line_to(cx, cy - 1.0);
    if let Some(p) = pb.finish() {
        c.stroke_path(&p, &aqua_gfx::canvas::solid(col), 1.8);
    }
}

fn draw_keyboard(c: &mut Canvas, r: Rect, col: aqua_gfx::Color) {
    c.stroke_rrect(r, 3.0, col, 1.4);
    for row in 0..2 {
        for k in 0..5 {
            c.fill_rect(Rect::new(r.x + 3.0 + k as f32 * 3.0, r.y + 3.0 + row as f32 * 3.2, 1.6, 1.6), col);
        }
    }
    c.fill_rect(Rect::new(r.x + 5.0, r.bottom() - 4.2, r.w - 10.0, 1.6), col);
}

fn hit(sh: &Shell, x: f32, y: f32) -> Option<Hit> {
    let g = geo(sh);
    if sh.lockscreen.power_menu {
        for (h, _, r) in menu_rows(&g) {
            if r.contains(x, y) {
                return Some(h);
            }
        }
    }
    if g.field.contains(x, y) {
        Some(Hit::Field)
    } else if g.help.contains(x, y) {
        Some(Hit::Help)
    } else if g.power.contains(x, y) {
        Some(Hit::Power)
    } else if g.input.contains(x, y) {
        Some(Hit::Input)
    } else {
        None
    }
}

pub fn hover(sh: &mut Shell, x: f32, y: f32) {
    sh.lockscreen.hover = hit(sh, x, y);
}

pub fn click(sh: &mut Shell, x: f32, y: f32) -> Vec<Action> {
    let h = hit(sh, x, y);
    let ls = &mut sh.lockscreen;
    let was_menu = ls.power_menu;
    ls.power_menu = false;
    match h {
        Some(Hit::Help) => ls.show_hint = !ls.show_hint,
        Some(Hit::Power) => ls.power_menu = !was_menu,
        Some(Hit::Input) => {
            if sh.layouts.len() > 1 {
                return vec![Action::SwitchLayout((sh.layout_idx + 1) % sh.layouts.len()), Action::Redraw];
            }
        }
        Some(Hit::Sleep) => return vec![Action::Sleep],
        Some(Hit::Restart) => return vec![Action::Restart],
        Some(Hit::ShutDown) => return vec![Action::ShutDown],
        _ => {}
    }
    vec![Action::Redraw]
}

/// Keyboard input on the lock screen.
pub fn key(sh: &mut Shell, key: Option<Key>, text: Option<&str>) -> Vec<Action> {
    let ls = &mut sh.lockscreen;
    if ls.busy || ls.locked_out().is_some() {
        return vec![Action::Redraw];
    }
    match key {
        Some(Key::Escape) => {
            ls.password.clear();
            ls.power_menu = false;
        }
        Some(Key::Backspace) => {
            ls.password.pop();
        }
        Some(Key::Enter) if !ls.password.is_empty() => {
            let pw = std::mem::take(&mut ls.password);
            ls.password = "•".repeat(pw.chars().count());
            return vec![Action::Unlock(pw)];
        }
        _ => {}
    }
    if let Some(t) = text {
        if !t.chars().any(|c| c.is_control()) {
            ls.fail_at = None;
            ls.password.push_str(t);
        }
    }
    vec![Action::Redraw]
}
