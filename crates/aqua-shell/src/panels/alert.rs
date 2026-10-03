//! Modal alerts in the Aqua style (icon, bold title, informative text,
//! pill buttons, optional help "?" and auto-confirm countdown), plus the
//! "About" panels and the volume/brightness HUD.
use crate::{hash_of, style, Action, Key, Layer, LayerId, Shell};
use aqua_gfx::{rgba, Canvas, Fonts, Rect, Weight};
use aqua_icons::IconRequest;
use std::time::Instant;

#[derive(Clone, Debug, PartialEq, Hash)]
pub enum AlertIcon {
    Warning,
    Computer,
    App { id: String, name: String, icon: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Button {
    pub label: String,
    pub actions: Vec<Action>,
    pub default: bool,
}

#[derive(Clone)]
pub struct Alert {
    pub icon: AlertIcon,
    pub title: String,
    /// Informative text; `{n}` is replaced by the countdown seconds.
    pub body: String,
    /// Extra "label: value" rows (About panels).
    pub info: Vec<(String, String)>,
    pub buttons: Vec<Button>,
    pub countdown: Option<(Instant, u32)>,
    pub opened: Instant,
    pub closing: Option<Instant>,
    pub hover: Option<usize>,
    pub help_url: Option<String>,
}

impl Alert {
    pub fn new(icon: AlertIcon, title: &str, body: &str) -> Self {
        Self {
            icon,
            title: title.into(),
            body: body.into(),
            info: vec![],
            buttons: vec![],
            countdown: None,
            opened: Instant::now(),
            closing: None,
            hover: None,
            help_url: None,
        }
    }
    pub fn button(mut self, label: &str, actions: Vec<Action>, default: bool) -> Self {
        self.buttons.push(Button { label: label.into(), actions, default });
        self
    }
    pub fn countdown(mut self, secs: u32) -> Self {
        self.countdown = Some((Instant::now(), secs));
        self
    }
    fn remaining(&self) -> Option<u32> {
        self.countdown.map(|(t, s)| s.saturating_sub(t.elapsed().as_secs() as u32))
    }
    fn progress(&self) -> f32 {
        let slow = 1.0;
        if let Some(c) = self.closing {
            return 1.0 - (c.elapsed().as_secs_f32() / (0.16 * slow)).min(1.0);
        }
        (self.opened.elapsed().as_secs_f32() / (0.22 * slow)).min(1.0)
    }
}

/// Standard confirmation for Restart / Shut Down / Log Out (60 s countdown).
pub fn power_confirm(what: Action, user: &str) -> Alert {
    let (title, body, button) = match what {
        Action::Restart => (
            "Are you sure you want to restart your computer now?",
            "If you do nothing, the computer will restart automatically in {n} seconds.",
            "Restart",
        ),
        Action::ShutDown => (
            "Are you sure you want to shut down your computer now?",
            "If you do nothing, the computer will shut down automatically in {n} seconds.",
            "Shut Down",
        ),
        _ => ("", "", "Log Out"),
    };
    let (title, body) = if button == "Log Out" {
        (
            crate::tr("Are you sure you want to quit all applications and log out now?").to_string(),
            crate::trf(
                "If you do nothing, {user} will be logged out automatically in {n} seconds.",
                &[("user", &user)],
            ),
        )
    } else {
        (crate::tr(title).to_string(), crate::tr(body).to_string())
    };
    let now = match what {
        Action::Restart => Action::RestartNow,
        Action::ShutDown => Action::ShutDownNow,
        _ => Action::LogOutNow,
    };
    let mut a = Alert::new(AlertIcon::Computer, &title, &body)
        .button("Cancel", vec![], false)
        .button(button, vec![now], true)
        .countdown(60);
    a.help_url = Some("https://support.apple.com/guide/mac-help/".into());
    a
}

pub fn wrap(f: &Fonts, s: &str, size: f32, w: Weight, max: f32) -> Vec<String> {
    // Whole sentences are translated here: the words measured below are not catalog keys.
    let s = crate::tr(s);
    let mut out = vec![];
    for para in s.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            let cand = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if f.measure(&cand, size, w) > max && !line.is_empty() {
                out.push(std::mem::replace(&mut line, word.to_string()));
            } else {
                line = cand;
            }
        }
        out.push(line);
    }
    out
}

const W: f32 = 272.0;

struct Geo {
    panel: Rect,
    buttons: Vec<Rect>,
    help: Rect,
    title: Vec<String>,
    body: Vec<String>,
}

fn geo(sh: &Shell, a: &Alert) -> Geo {
    let f = &sh.fonts;
    let inner = W - 40.0;
    let title = wrap(f, &a.title, 14.0, Weight::Bold, inner);
    let body_s = a.body.replace("{n}", &a.remaining().map(|n| n.to_string()).unwrap_or_default());
    let body = if body_s.is_empty() { vec![] } else { wrap(f, &body_s, 12.0, Weight::Regular, inner) };
    let mut h = 22.0
        + 64.0
        + 14.0
        + title.len() as f32 * 18.0
        + 6.0
        + body.len() as f32 * 16.0
        + a.info.len() as f32 * 18.0
        + 18.0;
    let stacked = a.buttons.len() > 2;
    let bh = 30.0;
    h += if stacked { a.buttons.len() as f32 * (bh + 8.0) } else { bh } + 16.0;
    let panel = Rect::new((sh.w - W) / 2.0, (sh.h - h) / 2.0 - sh.h * 0.06, W, h);
    let by = panel.bottom() - 16.0 - if stacked { a.buttons.len() as f32 * (bh + 8.0) - 8.0 } else { bh };
    let n = a.buttons.len().max(1) as f32;
    let buttons = a
        .buttons
        .iter()
        .enumerate()
        .map(|(i, _)| {
            if stacked {
                Rect::new(panel.x + 16.0, by + i as f32 * (bh + 8.0), W - 32.0, bh)
            } else {
                let bw = (W - 32.0 - (n - 1.0) * 10.0) / n;
                Rect::new(panel.x + 16.0 + i as f32 * (bw + 10.0), by, bw, bh)
            }
        })
        .collect();
    Geo { help: Rect::new(panel.right() - 34.0, panel.y + 12.0, 22.0, 22.0), panel, buttons, title, body }
}

fn draw_computer(c: &mut Canvas, r: Rect) {
    let scr = Rect::new(r.x + 2.0, r.y + 6.0, r.w - 4.0, r.h * 0.62);
    c.fill_rrect(scr, 5.0, rgba(200, 200, 206, 1.0));
    c.fill_rrect_vgrad(
        Rect::new(scr.x + 3.0, scr.y + 3.0, scr.w - 6.0, scr.h - 10.0),
        3.0,
        rgba(60, 120, 230, 1.0),
        rgba(170, 90, 200, 1.0),
    );
    c.fill_rect(Rect::new(r.cx() - 7.0, scr.bottom(), 14.0, r.h * 0.18), rgba(180, 180, 188, 1.0));
    c.fill_rrect(Rect::new(r.cx() - 16.0, scr.bottom() + r.h * 0.17, 32.0, 4.0), 2.0, rgba(170, 170, 178, 1.0));
}

fn draw_warning(c: &mut Canvas, f: &Fonts, r: Rect) {
    use aqua_gfx::tiny_skia::PathBuilder;
    let mut pb = PathBuilder::new();
    pb.move_to(r.cx(), r.y + 4.0);
    pb.line_to(r.right() - 2.0, r.bottom() - 6.0);
    pb.line_to(r.x + 2.0, r.bottom() - 6.0);
    pb.close();
    if let Some(p) = pb.finish() {
        c.fill_path(
            &p,
            &aqua_gfx::canvas::lin_grad(
                0.0,
                r.y,
                0.0,
                r.bottom(),
                &[(0.0, rgba(255, 214, 10, 1.0)), (1.0, rgba(255, 179, 0, 1.0))],
            ),
        );
    }
    c.text_in(f, Rect::new(r.x, r.y + 18.0, r.w, r.h - 22.0), 0.5, 34.0, Weight::Bold, rgba(40, 30, 0, 0.9), "!");
}

pub fn layer(sh: &mut Shell) -> Option<Layer> {
    let a = sh.alert.clone()?;
    let g = geo(sh, &a);
    let dark = sh.style.dark;
    let p = g.panel;
    let rem = a.remaining();
    let key = hash_of(&(
        a.title.clone(),
        a.body.clone(),
        rem,
        a.hover,
        dark,
        a.icon.clone(),
        a.buttons.len(),
        sh.icons_serial(),
        p.h as i32,
    ));
    let (pm, serial) = sh.cached(LayerId::Alert, key, p.w, p.h, |c, sh| {
        let f = sh.fonts.clone();
        let fg = style::text_primary(dark);
        let fg2 = style::text_secondary(dark);
        let ir = Rect::new(p.w / 2.0 - 32.0, 22.0, 64.0, 64.0);
        match &a.icon {
            AlertIcon::Warning => draw_warning(c, &f, ir),
            AlertIcon::Computer => draw_computer(c, ir),
            AlertIcon::App { id, name, icon } => {
                let px = (64.0 * sh.scale).round() as u32;
                let pmx = sh.icons.get(&IconRequest { id: id.clone(), name: name.clone(), icon: icon.clone() }, px);
                c.draw_pixmap(&pmx, ir, 1.0);
            }
        }
        if a.help_url.is_some() {
            let hr = g.help.translate(-p.x, -p.y);
            c.stroke_rrect(hr, hr.h / 2.0, style::separator(dark), 1.0);
            c.text_in(&f, hr, 0.5, 12.0, Weight::Semibold, fg2, "?");
        }
        let mut y = ir.bottom() + 14.0;
        for l in &g.title {
            c.text_in(&f, Rect::new(0.0, y, p.w, 18.0), 0.5, 14.0, Weight::Bold, fg, l);
            y += 18.0;
        }
        y += 6.0;
        for l in &g.body {
            c.text_in(&f, Rect::new(0.0, y, p.w, 16.0), 0.5, 12.0, Weight::Regular, fg, l);
            y += 16.0;
        }
        for (k, v) in &a.info {
            let half = p.w / 2.0;
            c.text_in(&f, Rect::new(0.0, y, half - 4.0, 18.0), 1.0, 11.5, Weight::Semibold, fg, k);
            let v = f.ellipsize(v, 11.5, Weight::Regular, half - 20.0);
            c.text_in(&f, Rect::new(half + 4.0, y, half - 4.0, 18.0), 0.0, 11.5, Weight::Regular, fg2, &v);
            y += 18.0;
        }
        for (i, (b, r)) in a.buttons.iter().zip(&g.buttons).enumerate() {
            let r = r.translate(-p.x, -p.y);
            let hot = a.hover == Some(i);
            if b.default {
                c.fill_rrect(r, r.h / 2.0, style::accent(if hot { 0.85 } else { 1.0 }));
                c.text_in(&f, r, 0.5, 13.0, Weight::Medium, rgba(255, 255, 255, 1.0), &b.label);
            } else {
                let bg = if dark {
                    rgba(255, 255, 255, if hot { 0.22 } else { 0.14 })
                } else {
                    rgba(0, 0, 0, if hot { 0.11 } else { 0.06 })
                };
                c.fill_rrect(r, r.h / 2.0, bg);
                c.text_in(&f, r, 0.5, 13.0, Weight::Medium, fg, &b.label);
            }
        }
    });
    let t = a.progress();
    let ease = 1.0 - (1.0 - t).powi(3);
    let mut glass = style::glass_menu(&sh.cfg.glass, dark);
    glass.radius = 26.0;
    glass.shadow = 0.38;
    Some(Layer {
        id: LayerId::Alert,
        rect: p,
        glass: Some(glass),
        tiles: vec![],
        content: pm,
        serial,
        opacity: ease,
        zoom: 0.9 + 0.1 * ease,
    })
}

pub fn animating(sh: &Shell) -> bool {
    sh.alert.as_ref().map(|a| a.progress() < 1.0 || a.countdown.is_some() || a.closing.is_some()).unwrap_or(false)
}

/// Advance timers; returns actions when the countdown expired.
pub fn tick(sh: &mut Shell) -> Vec<Action> {
    let Some(a) = &sh.alert else { return vec![] };
    if let Some(c) = a.closing {
        if c.elapsed().as_secs_f32() > 0.17 {
            sh.alert = None;
        }
        return vec![];
    }
    if a.remaining() == Some(0) {
        let acts = a.buttons.iter().find(|b| b.default).map(|b| b.actions.clone()).unwrap_or_default();
        dismiss(sh);
        return acts;
    }
    vec![]
}

fn dismiss(sh: &mut Shell) {
    if let Some(a) = sh.alert.as_mut().filter(|a| a.closing.is_none()) {
        a.closing = Some(Instant::now());
        a.countdown = None;
    }
}

/// Drop an alert whose fade-out finished (called every frame).
pub fn reap(sh: &mut Shell) {
    if sh.alert.as_ref().and_then(|a| a.closing).map(|c| c.elapsed().as_secs_f32() > 0.17).unwrap_or(false) {
        sh.alert = None;
    }
}

/// An alert is up and modal (not fading out).
pub fn active(sh: &Shell) -> bool {
    sh.alert.as_ref().map(|a| a.closing.is_none()).unwrap_or(false)
}

pub fn hover(sh: &mut Shell, x: f32, y: f32) {
    let Some(a) = sh.alert.clone() else { return };
    let g = geo(sh, &a);
    if let Some(al) = &mut sh.alert {
        al.hover = g.buttons.iter().position(|r| r.contains(x, y));
    }
}

pub fn click(sh: &mut Shell, x: f32, y: f32) -> Option<Vec<Action>> {
    let a = sh.alert.clone()?;
    if a.closing.is_some() {
        return None;
    }
    let g = geo(sh, &a);
    if let Some(i) = g.buttons.iter().position(|r| r.contains(x, y)) {
        let acts = a.buttons[i].actions.clone();
        dismiss(sh);
        let mut v = vec![Action::Redraw];
        v.extend(acts);
        return Some(v);
    }
    if g.help.contains(x, y) {
        if let Some(u) = &a.help_url {
            return Some(vec![Action::Launch(format!("xdg-open '{u}'"))]);
        }
    }
    Some(vec![Action::Beep])
}

pub fn key(sh: &mut Shell, key: Option<Key>) -> (bool, Vec<Action>) {
    let Some(a) = sh.alert.clone().filter(|a| a.closing.is_none()) else { return (false, vec![]) };
    match key {
        Some(Key::Escape) => {
            let acts = a
                .buttons
                .iter()
                .find(|b| b.label == "Cancel" || b.label == "OK")
                .map(|b| b.actions.clone())
                .unwrap_or_default();
            dismiss(sh);
            (true, acts)
        }
        Some(Key::Enter) => {
            let acts = a
                .buttons
                .iter()
                .find(|b| b.default)
                .or(a.buttons.first())
                .map(|b| b.actions.clone())
                .unwrap_or_default();
            dismiss(sh);
            (true, acts)
        }
        _ => (true, vec![]),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HudKind {
    Volume,
    Brightness,
    Mute,
    Layout,
}

pub struct Hud {
    pub kind: HudKind,
    pub value: f32,
    pub label: String,
    pub at: Instant,
}

impl Hud {
    fn alpha(&self) -> f32 {
        let s = self.at.elapsed().as_secs_f32();
        if s < 0.15 {
            s / 0.15
        } else if s < 1.4 {
            1.0
        } else {
            (1.0 - (s - 1.4) / 0.35).max(0.0)
        }
    }
}

pub fn hud_animating(sh: &Shell) -> bool {
    sh.hud.as_ref().map(|h| h.at.elapsed().as_secs_f32() < 1.8).unwrap_or(false)
}

pub fn show_hud(sh: &mut Shell, kind: HudKind, value: f32, label: &str) {
    sh.hud = Some(Hud { kind, value, label: label.into(), at: Instant::now() });
}

pub fn hud_layer(sh: &mut Shell) -> Option<Layer> {
    let h = sh.hud.as_ref()?;
    let al = h.alpha();
    if al <= 0.0 {
        return None;
    }
    let r = Rect::new(sh.w - 320.0, sh.cfg.menubar_height + 10.0, 300.0, 56.0);
    let dark = sh.style.dark;
    let key = hash_of(&(h.kind, (h.value * 100.0) as i32, h.label.clone(), dark));
    let (kind, value, label) = (h.kind, h.value, h.label.clone());
    let (pm, serial) = sh.cached(LayerId::Hud, key, r.w, r.h, |c, sh| {
        let f = sh.fonts.clone();
        let fg = style::text_primary(dark);
        let title = match kind {
            HudKind::Volume | HudKind::Mute => "Sound",
            HudKind::Brightness => "Display",
            HudKind::Layout => "Input Source",
        };
        c.text(&f, 16.0, 22.0, 13.0, Weight::Semibold, fg, title);
        if kind == HudKind::Layout {
            c.text(&f, 16.0, 44.0, 15.0, Weight::Medium, fg, &label);
            return;
        }
        let tr = Rect::new(16.0, 30.0, r.w - 32.0, 14.0);
        c.fill_rrect(tr, 7.0, rgba(128, 128, 128, 0.3));
        let v = if kind == HudKind::Mute { 0.0 } else { value };
        if v > 0.0 {
            c.fill_rrect(
                Rect::new(tr.x, tr.y, (tr.w * v).max(14.0), tr.h),
                7.0,
                if dark { rgba(255, 255, 255, 0.95) } else { rgba(255, 255, 255, 1.0) },
            );
        }
    });
    let mut g = style::glass_menu(&sh.cfg.glass, dark);
    g.radius = 18.0;
    Some(Layer {
        id: LayerId::Hud,
        rect: r,
        glass: Some(g),
        tiles: vec![],
        content: pm,
        serial,
        opacity: al,
        zoom: 0.96 + 0.04 * al,
    })
}
