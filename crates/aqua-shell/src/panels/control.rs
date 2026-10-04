//! Control Centre: liquid-glass tiles (toggles, now playing, sliders, quick actions).
use crate::{hash_of, style, Action, Layer, LayerId, Shell};
use aqua_gfx::{rgba, symbols, Canvas, Rect, Weight};

pub struct ControlCenter {
    pub open: bool,
    pub t: f32,
    pub wifi: bool,
    pub bluetooth: bool,
    pub airdrop: bool,
    pub focus: bool,
    /// Stage Manager is on (mirrored from the compositor).
    pub stage: bool,
    pub dark: bool,
    pub brightness: f32,
    pub volume: f32,
    pub hover: Option<usize>,
    pub drag: Option<usize>,
}

impl Default for ControlCenter {
    fn default() -> Self {
        Self {
            open: false,
            t: 0.0,
            wifi: true,
            bluetooth: true,
            airdrop: false,
            focus: false,
            stage: false,
            dark: false,
            brightness: 0.8,
            volume: 0.6,
            hover: None,
            drag: None,
        }
    }
}

impl ControlCenter {
    pub fn visible(&self) -> bool {
        self.open || self.t > 0.001
    }
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }
    pub fn animate(&mut self, dt: f32) -> bool {
        let target = if self.open { 1.0 } else { 0.0 };
        if (self.t - target).abs() < 0.001 {
            self.t = target;
            return false;
        }
        self.t += (target - self.t) * (1.0 - (-9.0 * dt).exp());
        if (self.t - target).abs() < 0.01 {
            self.t = target;
        }
        true
    }
}

const U: f32 = 70.0;
const G: f32 = 14.0;

#[derive(Clone, Copy, PartialEq)]
enum T {
    Wifi,
    Bt,
    AirDrop,
    Playing,
    Focus,
    Stage,
    Mirror,
    Display,
    Sound,
    DarkMode,
    Calc,
    Timer,
    Shot,
}

fn tiles(origin: (f32, f32)) -> Vec<(T, Rect, bool)> {
    let (ox, oy) = origin;
    let cell = |c: f32, r: f32, w: f32, h: f32| {
        Rect::new(ox + c * (U + G), oy + r * (U + G), w * U + (w - 1.0) * G, h * U + (h - 1.0) * G)
    };
    let row_s = |r: f32| oy + 3.0 * (U + G) + (r - 3.0) * (U * 0.82 + G);
    let slider = |r: f32| Rect::new(ox, row_s(r), 4.0 * U + 3.0 * G, U * 0.82);
    let last_y = row_s(5.0);
    let circ = |c: f32| Rect::new(ox + c * (U + G), last_y, U, U);
    vec![
        (T::Wifi, cell(0.0, 0.0, 2.0, 1.0), false),
        (T::Playing, cell(2.0, 0.0, 2.0, 2.0), false),
        (T::Bt, cell(0.0, 1.0, 1.0, 1.0), true),
        (T::AirDrop, cell(1.0, 1.0, 1.0, 1.0), true),
        (T::Focus, cell(0.0, 2.0, 2.0, 1.0), false),
        (T::Stage, cell(2.0, 2.0, 1.0, 1.0), true),
        (T::Mirror, cell(3.0, 2.0, 1.0, 1.0), true),
        (T::Display, slider(3.0), false),
        (T::Sound, slider(4.0), false),
        (T::DarkMode, circ(0.0), true),
        (T::Calc, circ(1.0), true),
        (T::Timer, circ(2.0), true),
        (T::Shot, circ(3.0), true),
    ]
}

fn panel(sh: &Shell) -> Rect {
    let w = 4.0 * U + 3.0 * G;
    let h = 3.0 * (U + G) + 2.0 * (U * 0.82 + G) + U + 44.0;
    let slide = (1.0 - sh.control.t) * 18.0;
    Rect::new(sh.w - w - 18.0, sh.cfg.menubar_height + 14.0 - slide, w, h)
}

pub fn layer(sh: &mut Shell) -> Option<Layer> {
    if !sh.control.visible() {
        return None;
    }
    let p = panel(sh);
    let ts = tiles((p.x, p.y));
    let snap = aqua_sys::snapshot();
    if sh.control.drag.is_none() {
        sh.control.wifi = snap.net.wifi_enabled;
        sh.control.bluetooth = snap.bt.powered;
        if let Some(b) = snap.brightness {
            sh.control.brightness = b;
        }
        if snap.audio.available {
            sh.control.volume = if snap.audio.muted { 0.0 } else { snap.audio.volume };
        }
    }
    let wifi_name = snap.net.active().map(|n| n.ssid.clone()).or(snap.net.wired.clone()).unwrap_or_else(|| {
        if snap.net.wifi_enabled {
            "Not Connected".into()
        } else {
            "Off".into()
        }
    });
    let cc = &sh.control;
    let light_bg = !sh.style.dark && sh.style.lum(p) > 0.80;
    let media = snap.media.clone();
    let key = hash_of(&(
        (wifi_name.clone(), snap.bt.devices.iter().filter(|d| d.connected).count(), snap.audio.device.clone()),
        (media.title.clone(), media.artist.clone(), media.app.clone(), media.playing),
        cc.wifi,
        cc.bluetooth,
        cc.airdrop,
        (cc.focus, cc.stage),
        cc.dark,
        (cc.brightness * 100.0) as i32,
        (cc.volume * 100.0) as i32,
        cc.hover,
        p.x as i32,
        light_bg,
    ));
    let fg = if light_bg { rgba(29, 29, 31, 0.92) } else { rgba(255, 255, 255, 0.97) };
    let fg2 = if light_bg { rgba(29, 29, 31, 0.6) } else { rgba(255, 255, 255, 0.72) };
    let (pm, serial) = sh.cached(LayerId::ControlCenter, key, p.w, p.h, |c, sh| {
        let f = sh.fonts.clone();
        let cc = &sh.control;
        for (i, (t, r, _round)) in ts.iter().enumerate() {
            let r = r.translate(-p.x, -p.y);
            if cc.hover == Some(i) {
                c.fill_rrect(r, if r.w == r.h { r.w / 2.0 } else { 22.0 }, rgba(255, 255, 255, 0.10));
            }
            let toggle_circle = |c: &mut Canvas, cx: f32, cy: f32, on: bool| {
                if on {
                    c.fill_circle(cx, cy, 17.0, style::accent(1.0));
                } else {
                    c.fill_circle(cx, cy, 17.0, rgba(255, 255, 255, 0.22));
                }
            };
            match t {
                T::Wifi | T::Focus => {
                    let on = if *t == T::Wifi { cc.wifi } else { cc.focus };
                    let (cx, cy) = (r.x + 34.0, r.cy());
                    toggle_circle(c, cx, cy, on);
                    let ic = Rect::new(cx - 9.0, cy - 8.0, 18.0, 16.0);
                    if *t == T::Wifi {
                        symbols::wifi(c, ic, fg)
                    } else {
                        symbols::moon(c, ic, fg)
                    }
                    let (title, sub) = if *t == T::Wifi {
                        ("Wi-Fi", wifi_name.as_str())
                    } else {
                        ("Focus", if on { "Do Not Disturb" } else { "" })
                    };
                    c.text(&f, r.x + 60.0, r.cy() - 1.0, 13.5, Weight::Semibold, fg, title);
                    c.text(&f, r.x + 60.0, r.cy() + 15.0, 12.0, Weight::Regular, fg2, sub);
                }
                T::Bt | T::AirDrop => {
                    let on = if *t == T::Bt { cc.bluetooth } else { cc.airdrop };
                    let (cx, cy) = (r.cx(), r.cy());
                    if on {
                        c.fill_circle(cx, cy, r.w / 2.0 - 2.0, style::accent(1.0));
                    }
                    let ic = Rect::new(cx - 11.0, cy - 11.0, 22.0, 22.0);
                    if *t == T::Bt {
                        symbols::bluetooth(c, ic, fg)
                    } else {
                        symbols::airdrop(c, ic, fg)
                    }
                }
                T::Playing => {
                    let art = Rect::new(r.x + 16.0, r.y + 16.0, 52.0, 52.0);
                    c.fill_rrect(art, 12.0, rgba(255, 255, 255, 0.22));
                    if !media.app.is_empty() {
                        let initial: String = media.app.chars().take(1).collect();
                        c.text_in(&f, art, 0.5, 24.0, Weight::Bold, fg, &initial);
                        c.text_in(
                            &f,
                            Rect::new(art.right() + 8.0, art.y, r.right() - art.right() - 14.0, 18.0),
                            0.0,
                            11.5,
                            Weight::Medium,
                            fg2,
                            &media.app,
                        );
                    }
                    let (title, artist) = if media.player.is_empty() || media.title.is_empty() {
                        (
                            if media.player.is_empty() { "Not Playing".to_string() } else { media.app.clone() },
                            String::new(),
                        )
                    } else {
                        (media.title.clone(), media.artist.clone())
                    };
                    c.text_in(
                        &f,
                        Rect::new(r.x + 16.0, r.bottom() - 62.0, r.w - 28.0, 18.0),
                        0.0,
                        13.5,
                        Weight::Semibold,
                        fg,
                        &title,
                    );
                    if !artist.is_empty() {
                        c.text_in(
                            &f,
                            Rect::new(r.x + 16.0, r.bottom() - 46.0, r.w - 28.0, 16.0),
                            0.0,
                            12.0,
                            Weight::Regular,
                            fg2,
                            &artist,
                        );
                    }
                    let dim = |on: bool| if on { fg } else { fg2 };
                    symbols::skip(c, Rect::new(r.x + 22.0, r.bottom() - 26.0, 20.0, 14.0), dim(media.can_prev), false);
                    if media.playing {
                        let pr = Rect::new(r.cx() - 7.0, r.bottom() - 28.0, 14.0, 17.0);
                        c.fill_rrect(Rect::new(pr.x, pr.y, 4.5, pr.h), 1.5, fg);
                        c.fill_rrect(Rect::new(pr.right() - 4.5, pr.y, 4.5, pr.h), 1.5, fg);
                    } else {
                        symbols::play(
                            c,
                            Rect::new(r.cx() - 8.0, r.bottom() - 28.0, 18.0, 18.0),
                            dim(!media.player.is_empty()),
                        );
                    }
                    symbols::skip(
                        c,
                        Rect::new(r.right() - 42.0, r.bottom() - 26.0, 20.0, 14.0),
                        dim(media.can_next),
                        true,
                    );
                }
                T::Stage => {
                    if cc.stage {
                        c.fill_circle(r.cx(), r.cy(), r.w / 2.0 - 2.0, style::accent(1.0));
                    }
                    let x = r.cx() - 12.0;
                    for k in 0..3 {
                        c.fill_rrect(Rect::new(x, r.cy() - 10.0 + k as f32 * 7.5, 5.0, 5.0), 1.5, fg);
                    }
                    c.stroke_rrect(Rect::new(x + 8.0, r.cy() - 10.0, 16.0, 20.0), 3.0, fg, 1.8);
                }
                T::Mirror => symbols::screens(c, Rect::new(r.cx() - 12.0, r.cy() - 12.0, 24.0, 24.0), fg),
                T::Display | T::Sound => {
                    let v = if *t == T::Display { cc.brightness } else { cc.volume };
                    c.text(
                        &f,
                        r.x + 16.0,
                        r.y + 22.0,
                        13.5,
                        Weight::Semibold,
                        fg,
                        if *t == T::Display { "Display" } else { "Sound" },
                    );
                    let tr = Rect::new(r.x + 16.0, r.y + 34.0, r.w - 32.0, 22.0);
                    c.fill_rrect(tr, 11.0, rgba(255, 255, 255, 0.22));
                    let fill = Rect::new(tr.x, tr.y, (tr.w * v).max(22.0), tr.h);
                    c.fill_rrect(fill, 11.0, rgba(255, 255, 255, 0.95));
                    let ic = Rect::new(tr.x + 5.0, tr.y + 4.0, 14.0, 14.0);
                    let icol = rgba(90, 90, 96, 0.9);
                    if *t == T::Display {
                        symbols::sun(c, ic, icol)
                    } else {
                        symbols::speaker(c, ic, icol, 1)
                    }
                }
                T::DarkMode => {
                    if cc.dark {
                        c.fill_circle(r.cx(), r.cy(), r.w / 2.0 - 2.0, style::accent(1.0));
                    }
                    c.fill_circle(r.cx(), r.cy(), 11.0, fg);
                    c.fill_circle(r.cx() + 4.0, r.cy(), 8.0, aqua_gfx::Color::TRANSPARENT);
                    let mut pb = aqua_gfx::tiny_skia::PathBuilder::new();
                    pb.push_circle(r.cx(), r.cy(), 9.0);
                    if let Some(pp) = pb.finish() {
                        c.fill_path(&pp, &aqua_gfx::canvas::solid(rgba(30, 60, 140, 0.55)));
                    }
                    c.fill_circle(r.cx() - 3.0, r.cy(), 9.0, fg);
                }
                T::Calc => {
                    let b = Rect::new(r.cx() - 9.0, r.cy() - 12.0, 18.0, 24.0);
                    c.stroke_rrect(b, 3.0, fg, 1.8);
                    c.fill_rect(Rect::new(b.x + 4.0, b.y + 4.0, 10.0, 4.0), fg);
                    for k in 0..6 {
                        c.fill_circle(b.x + 5.0 + (k % 3) as f32 * 4.0, b.y + 12.0 + (k / 3) as f32 * 5.0, 1.2, fg);
                    }
                }
                T::Timer => {
                    let mut pb = aqua_gfx::tiny_skia::PathBuilder::new();
                    pb.push_circle(r.cx(), r.cy() + 1.0, 11.0);
                    pb.move_to(r.cx(), r.cy() + 1.0);
                    pb.line_to(r.cx() + 5.0, r.cy() - 4.0);
                    if let Some(pp) = pb.finish() {
                        c.stroke_path(&pp, &aqua_gfx::canvas::solid(fg), 1.8);
                    }
                }
                T::Shot => {
                    let b = Rect::new(r.cx() - 12.0, r.cy() - 10.0, 24.0, 20.0);
                    c.stroke_rrect(b, 4.0, fg, 1.8);
                    c.fill_circle(r.cx(), r.cy(), 4.5, fg);
                }
            }
        }
        let lbl = "Edit Controls";
        let w = f.measure(lbl, 12.5, Weight::Medium) + 22.0;
        let pr = Rect::new((p.w - w) / 2.0, p.h - 30.0, w, 22.0);
        c.fill_rrect(pr, 11.0, rgba(40, 40, 48, 0.42));
        c.stroke_rrect(pr, 11.0, rgba(255, 255, 255, 0.35), 1.0);
        c.text_in(&f, pr, 0.5, 12.5, Weight::Medium, rgba(255, 255, 255, 0.95), lbl);
    });
    let base = sh.cfg.glass;
    let tile_glass: Vec<(Rect, aqua_config::GlassStyle)> =
        ts.iter().map(|(_, r, round)| (*r, style::glass_tile(&base, if *round { r.w / 2.0 } else { 24.0 }))).collect();
    let t = sh.control.t;
    Some(Layer {
        id: LayerId::ControlCenter,
        rect: p,
        glass: None,
        tiles: tile_glass,
        content: pm,
        serial,
        opacity: t,
        zoom: 0.97 + 0.03 * t,
    })
}

pub fn hover(sh: &mut Shell, x: f32, y: f32) {
    if !sh.control.open {
        return;
    }
    let p = panel(sh);
    sh.control.hover = tiles((p.x, p.y)).iter().position(|(_, r, _)| r.contains(x, y));
}

pub fn click(sh: &mut Shell, x: f32, y: f32) -> Option<Vec<Action>> {
    if !sh.control.open {
        return None;
    }
    let p = panel(sh);
    let ts = tiles((p.x, p.y));
    let Some((t, r, _)) = ts.iter().find(|(_, r, _)| r.contains(x, y)) else {
        if p.contains(x, y) && y > p.bottom() - 34.0 {
            sh.control.open = false;
            return Some(vec![Action::Redraw, Action::OpenSettings("dock".into())]);
        }
        if !p.contains(x, y) {
            sh.control.toggle();
        }
        return Some(vec![Action::Redraw]);
    };
    let cc = &mut sh.control;
    let mut out = vec![Action::Redraw];
    match t {
        T::Wifi => {
            if x - r.x > 60.0 {
                cc.open = false;
                out.push(Action::OpenSettings("wifi".into()));
            } else {
                cc.wifi = !cc.wifi;
                aqua_sys::network::set_wifi_enabled(cc.wifi);
            }
        }
        T::Bt => {
            cc.bluetooth = !cc.bluetooth;
            aqua_sys::bluetooth::set_powered(cc.bluetooth);
        }
        T::AirDrop => {
            cc.open = false;
            out.push(Action::OpenSettings("bluetooth".into()));
        }
        T::Focus => {
            cc.focus = !cc.focus;
            out.push(Action::SetFocusMode(cc.focus));
        }
        T::Stage => {
            cc.stage = !cc.stage;
            out.push(Action::SetStageManager(cc.stage));
        }
        T::Mirror => {
            cc.open = false;
            out.push(Action::OpenSettings("displays".into()));
        }
        T::Calc => {
            cc.open = false;
            out.push(Action::Launch("gnome-calculator || kcalc || galculator || qalculate-gtk".into()));
        }
        T::Timer => {
            cc.open = false;
            out.push(Action::Launch("gnome-clocks || kclock || xdg-open https://time.is".into()));
        }
        T::Playing => {
            let m = aqua_sys::snapshot().media;
            if m.player.is_empty() {
                cc.open = false;
                out.push(Action::Launch("rhythmbox || lollypop || amberol || elisa || spotify || vlc".into()));
            } else if y > r.bottom() - 36.0 && x < r.x + 50.0 {
                aqua_sys::media::previous(&m.player);
            } else if y > r.bottom() - 36.0 && x > r.right() - 50.0 {
                aqua_sys::media::next(&m.player);
            } else if y > r.bottom() - 36.0 {
                aqua_sys::media::play_pause(&m.player);
            } else {
                cc.open = false;
                out.push(Action::Activate(m.app.to_lowercase()));
            }
        }
        T::DarkMode => {
            let d = !cc.dark;
            sh.set_dark(d);
            out.push(Action::SetDark(d));
        }
        T::Display | T::Sound => {
            let v = ((x - r.x - 16.0) / (r.w - 32.0)).clamp(0.0, 1.0);
            if *t == T::Display {
                cc.brightness = v;
                aqua_sys::backlight::set(v);
            } else {
                cc.volume = v;
                aqua_sys::audio::set_volume(v);
            }
        }
        T::Shot => {
            cc.open = false;
            out.push(Action::Screenshot);
        }
    }
    Some(out)
}
