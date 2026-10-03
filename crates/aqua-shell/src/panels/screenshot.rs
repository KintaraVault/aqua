//! Screenshot utility: ⌘⇧3 entire screen, ⌘⇧4 selected portion
//! (click instead of dragging, or press Space, to capture a window), ⌘⇧5 the
//! screenshot toolbar with options. Every capture is saved as PNG *and* copied to
//! the clipboard; a floating thumbnail slides in at the bottom-right corner.
//!
//! The shell only draws the interface and reports what to capture
//! ([`Action::ScreenshotTake`]); the compositor renders, crops, saves and copies.
use crate::{hash_of, style, Action, Key, Layer, LayerId, Shell};
use aqua_gfx::{rgba, Color, Pixmap, Rect, Weight};
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Mode {
    #[default]
    Off,
    /// Crosshair: drag a rectangle (⌘⇧4).
    Area,
    /// Camera: click a window (⌘⇧4 then Space).
    Window,
}

/// What the compositor should capture.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Full,
    /// Logical rectangle on the primary display.
    Area(Rect),
    /// A window's frame (logical rectangle, title bar included).
    Window(Rect),
}

/// Kinds offered by the toolbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Kind {
    #[default]
    Screen,
    Window,
    Portion,
    /// Record Entire Screen.
    RecScreen,
    /// Record Selected Portion.
    RecPortion,
}

impl Kind {
    pub fn is_record(self) -> bool {
        matches!(self, Kind::RecScreen | Kind::RecPortion)
    }
}

pub struct Thumb {
    pub pm: Arc<Pixmap>,
    pub path: String,
    pub shown: Instant,
    pub serial: u64,
}

#[derive(Default)]
pub struct ShotUi {
    pub mode: Mode,
    /// ⌘⇧5 toolbar visible.
    pub toolbar: bool,
    pub kind: Kind,
    /// Drag start and current pointer (logical).
    pub drag: Option<(f32, f32)>,
    pub cur: (f32, f32),
    /// Remembered "selected portion" of the toolbar.
    pub sel: Option<Rect>,
    pub hover: Option<usize>,
    pub options_open: bool,
    /// Window frames (logical, topmost first) — filled by the compositor.
    pub windows: Vec<Rect>,
    pub thumb: Option<Thumb>,
    /// Timer countdown: (started, seconds).
    pub countdown: Option<(Instant, f32)>,
    /// Options (mirrors the config; the compositor persists changes).
    pub save_to: String,
    pub timer: u32,
    pub show_thumb: bool,
    pub show_pointer: bool,
    /// The last capture was started from the ⌘⇧5 toolbar (the timer applies).
    pub from_toolbar: bool,
    pub thumb_serial: u64,
    /// Recording options (mirrors the config).
    pub rec_save: String,
    pub rec_mic: bool,
    pub rec_clicks: bool,
    pub rec_pointer: bool,
    /// A screen recording is running (set by the compositor).
    pub recording: Option<Instant>,
    /// Click rings drawn while recording with "Show Mouse Clicks".
    pub clicks: Vec<(f32, f32, Instant)>,
}

impl ShotUi {
    /// Any screenshot interface on screen that takes the pointer and keyboard.
    pub fn active(&self) -> bool {
        self.mode != Mode::Off || self.toolbar
    }
    pub fn begin(&mut self, mode: Mode) {
        self.mode = mode;
        self.toolbar = false;
        self.drag = None;
        self.options_open = false;
    }
    pub fn open_toolbar(&mut self) {
        self.toolbar = true;
        self.options_open = false;
        self.drag = None;
        self.mode = match self.kind {
            Kind::Screen | Kind::RecScreen => Mode::Off,
            Kind::Window => Mode::Window,
            Kind::Portion | Kind::RecPortion => Mode::Area,
        };
    }
    pub fn close(&mut self) {
        self.mode = Mode::Off;
        self.toolbar = false;
        self.drag = None;
        self.options_open = false;
    }
    pub fn set_thumb(&mut self, pm: Pixmap, path: String) {
        self.thumb_serial += 1;
        self.thumb = Some(Thumb { pm: Arc::new(pm), path, shown: Instant::now(), serial: self.thumb_serial });
    }
    pub fn animating(&self) -> bool {
        self.thumb.is_some() || self.countdown.is_some() || !self.clicks.is_empty()
    }
    /// Remember a click for the "Show Mouse Clicks" ring (only while recording).
    pub fn note_click(&mut self, x: f32, y: f32) {
        if self.recording.is_some() && self.rec_clicks {
            self.clicks.push((x, y, Instant::now()));
        }
    }
    /// Drop the thumbnail after it has been shown (5 s).
    pub fn tick(&mut self) {
        if self.thumb.as_ref().is_some_and(|t| t.shown.elapsed().as_secs_f32() > THUMB_SECS) {
            self.thumb = None;
        }
        self.clicks.retain(|c| c.2.elapsed().as_secs_f32() < CLICK_SECS);
        if self.recording.is_none() {
            self.clicks.clear();
        }
    }
}

const CLICK_SECS: f32 = 0.45;

const THUMB_SECS: f32 = 5.5;
const TB_H: f32 = 52.0;

fn norm(a: (f32, f32), b: (f32, f32)) -> Rect {
    let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
    let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

fn window_at(sh: &Shell, x: f32, y: f32) -> Option<Rect> {
    sh.shot.windows.iter().find(|r| r.contains(x, y)).copied()
}

/// Uniform-colour layer: a tiny pixmap stretched over `rect` by the renderer.
fn fill(sh: &mut Shell, slot: u8, rect: Rect, color: Color) -> Layer {
    let id = LayerId::Shot(slot);
    let s = 1.0 / sh.scale.max(0.1);
    let key =
        hash_of(&(color.red().to_bits(), color.green().to_bits(), color.blue().to_bits(), color.alpha().to_bits()));
    let (pm, serial) = sh.cached(id, key, 4.0 * s, 4.0 * s, |c, _| c.fill_rect(Rect::new(0.0, 0.0, 8.0, 8.0), color));
    Layer { id, rect, glass: None, tiles: vec![], content: pm, serial, opacity: 1.0, zoom: 1.0 }
}

/// Four layers outlining `r` (1 px border).
fn outline(sh: &mut Shell, first: u8, r: Rect, color: Color, wpx: f32) -> Vec<Layer> {
    vec![
        fill(sh, first, Rect::new(r.x, r.y, r.w, wpx), color),
        fill(sh, first + 1, Rect::new(r.x, r.bottom() - wpx, r.w, wpx), color),
        fill(sh, first + 2, Rect::new(r.x, r.y, wpx, r.h), color),
        fill(sh, first + 3, Rect::new(r.right() - wpx, r.y, wpx, r.h), color),
    ]
}

/// Four layers dimming everything outside `r`.
fn dim_outside(sh: &mut Shell, first: u8, r: Rect, color: Color) -> Vec<Layer> {
    let (w, h) = (sh.w, sh.h);
    vec![
        fill(sh, first, Rect::new(0.0, 0.0, w, r.y.max(0.0)), color),
        fill(sh, first + 1, Rect::new(0.0, r.bottom(), w, (h - r.bottom()).max(0.0)), color),
        fill(sh, first + 2, Rect::new(0.0, r.y, r.x.max(0.0), r.h), color),
        fill(sh, first + 3, Rect::new(r.right(), r.y, (w - r.right()).max(0.0), r.h), color),
    ]
}

fn label(sh: &mut Shell, slot: u8, x: f32, y: f32, text: &str) -> Layer {
    let f = sh.fonts.clone();
    let tw = f.measure(text, 11.5, Weight::Medium) + 14.0;
    let rect = Rect::new((x + 14.0).min(sh.w - tw - 4.0), (y + 16.0).min(sh.h - 26.0), tw, 21.0);
    let key = hash_of(&text);
    let t = text.to_string();
    let (pm, serial) = sh.cached(LayerId::Shot(slot), key, rect.w, rect.h, move |c, _| {
        c.fill_rrect(Rect::new(0.0, 0.0, tw, 21.0), 6.0, rgba(20, 20, 22, 0.78));
        c.text_in(&f, Rect::new(0.0, 0.0, tw, 21.0), 0.5, 11.5, Weight::Medium, rgba(255, 255, 255, 1.0), &t);
    });
    Layer { id: LayerId::Shot(slot), rect, glass: None, tiles: vec![], content: pm, serial, opacity: 1.0, zoom: 1.0 }
}

/// Toolbar buttons: (id, label). Ids ≥ 100 are separators.
const BTN_CLOSE: usize = 0;
const BTN_SCREEN: usize = 1;
const BTN_WINDOW: usize = 2;
const BTN_PORTION: usize = 3;
const BTN_OPTIONS: usize = 4;
const BTN_CAPTURE: usize = 5;
const BTN_REC_SCREEN: usize = 6;
const BTN_REC_PORTION: usize = 7;

fn toolbar_rect(sh: &Shell) -> Rect {
    let w = 580.0;
    Rect::new((sh.w - w) / 2.0, sh.h - TB_H - 96.0, w, TB_H)
}

fn buttons(sh: &Shell) -> Vec<(usize, Rect)> {
    let t = toolbar_rect(sh);
    let y = t.y + 8.0;
    let h = TB_H - 16.0;
    vec![
        (BTN_CLOSE, Rect::new(t.x + 10.0, y + 6.0, 24.0, 24.0)),
        (BTN_SCREEN, Rect::new(t.x + 48.0, y, 44.0, h)),
        (BTN_WINDOW, Rect::new(t.x + 96.0, y, 44.0, h)),
        (BTN_PORTION, Rect::new(t.x + 144.0, y, 44.0, h)),
        (BTN_REC_SCREEN, Rect::new(t.x + 206.0, y, 44.0, h)),
        (BTN_REC_PORTION, Rect::new(t.x + 254.0, y, 44.0, h)),
        (BTN_OPTIONS, Rect::new(t.x + 316.0, y, 112.0, h)),
        (BTN_CAPTURE, Rect::new(t.right() - 138.0, y, 128.0, h)),
    ]
}

/// Options popover rows: (label, is_header, action key).
fn option_rows(sh: &Shell) -> Vec<(String, bool, &'static str, bool)> {
    let o = &sh.shot;
    let save = |k: &str| o.save_to == k || (o.save_to.is_empty() && k == "pictures");
    if o.kind.is_record() {
        let rsave = |k: &str| o.rec_save == k || (o.rec_save != "desktop" && k == "movies");
        return vec![
            ("Save to".into(), true, "", false),
            ("Desktop".into(), false, "rsave:desktop", rsave("desktop")),
            ("Movies".into(), false, "rsave:movies", rsave("movies")),
            ("Timer".into(), true, "", false),
            ("None".into(), false, "timer:0", o.timer == 0),
            ("5 Seconds".into(), false, "timer:5", o.timer == 5),
            ("10 Seconds".into(), false, "timer:10", o.timer == 10),
            ("Microphone".into(), true, "", false),
            ("None".into(), false, "mic:0", !o.rec_mic),
            ("Default Microphone".into(), false, "mic:1", o.rec_mic),
            ("Options".into(), true, "", false),
            ("Show Mouse Clicks".into(), false, "clicks", o.rec_clicks),
            ("Show Mouse Pointer".into(), false, "rpointer", o.rec_pointer),
        ];
    }
    vec![
        ("Save to".into(), true, "", false),
        ("Desktop".into(), false, "save:desktop", save("desktop")),
        ("Pictures".into(), false, "save:pictures", save("pictures")),
        ("Clipboard only".into(), false, "save:clipboard", save("clipboard")),
        ("Timer".into(), true, "", false),
        ("None".into(), false, "timer:0", o.timer == 0),
        ("5 Seconds".into(), false, "timer:5", o.timer == 5),
        ("10 Seconds".into(), false, "timer:10", o.timer == 10),
        ("Options".into(), true, "", false),
        ("Show Floating Thumbnail".into(), false, "thumb", o.show_thumb),
        ("Show Mouse Pointer".into(), false, "pointer", o.show_pointer),
    ]
}

const ROW_H: f32 = 22.0;

fn options_rect(sh: &Shell) -> Rect {
    let b = buttons(sh).into_iter().find(|(i, _)| *i == BTN_OPTIONS).map(|(_, r)| r).unwrap_or_default();
    let n = option_rows(sh).len() as f32;
    let h = n * ROW_H + 12.0;
    Rect::new(b.x - 20.0, toolbar_rect(sh).y - h - 8.0, 210.0, h)
}

fn draw_icon(c: &mut aqua_gfx::Canvas, id: usize, r: Rect, col: Color) {
    let (cx, cy) = (r.cx(), r.cy());
    match id {
        BTN_SCREEN => {
            c.stroke_rrect(Rect::new(cx - 11.0, cy - 8.0, 22.0, 15.0), 2.5, col, 1.6);
            c.fill_rect(Rect::new(cx - 5.0, cy + 9.0, 10.0, 1.6), col);
        }
        BTN_WINDOW => {
            c.stroke_rrect(Rect::new(cx - 11.0, cy - 8.0, 22.0, 16.0), 2.5, col, 1.6);
            c.fill_rect(Rect::new(cx - 11.0, cy - 4.0, 22.0, 1.4), col);
            for i in 0..3 {
                c.fill_circle(cx - 8.0 + i as f32 * 3.0, cy - 6.0, 0.9, col);
            }
        }
        BTN_PORTION => {
            let rr = Rect::new(cx - 11.0, cy - 8.0, 22.0, 16.0);
            let mut x = rr.x;
            while x < rr.right() {
                let w = 3.0f32.min(rr.right() - x);
                c.fill_rect(Rect::new(x, rr.y, w, 1.5), col);
                c.fill_rect(Rect::new(x, rr.bottom() - 1.5, w, 1.5), col);
                x += 5.0;
            }
            let mut y = rr.y;
            while y < rr.bottom() {
                let h = 3.0f32.min(rr.bottom() - y);
                c.fill_rect(Rect::new(rr.x, y, 1.5, h), col);
                c.fill_rect(Rect::new(rr.right() - 1.5, y, 1.5, h), col);
                y += 5.0;
            }
        }
        BTN_REC_SCREEN | BTN_REC_PORTION => {
            aqua_gfx::symbols::record_screen(c, Rect::new(cx - 12.0, cy - 11.0, 24.0, 22.0), col, id == BTN_REC_PORTION)
        }
        _ => {}
    }
}

fn toolbar_layers(sh: &mut Shell) -> Vec<Layer> {
    let dark = sh.style.dark;
    let t = toolbar_rect(sh);
    let btns = buttons(sh);
    let kind = sh.shot.kind;
    let hover = sh.shot.hover;
    let timer = sh.shot.timer;
    let recording = sh.shot.recording.is_some();
    let key = hash_of(&(kind, hover, dark, timer, sh.shot.options_open, t.w as i32, t.y as i32, recording));
    let (pm, serial) = sh.cached(LayerId::Shot(40), key, t.w, t.h, |c, sh| {
        let f = sh.fonts.clone();
        let fg = style::text_primary(dark);
        let fg2 = style::text_secondary(dark);
        for (id, r) in &btns {
            let r = r.translate(-t.x, -t.y);
            let hot = hover == Some(*id);
            match *id {
                BTN_CLOSE => {
                    c.fill_circle(
                        r.cx(),
                        r.cy(),
                        10.0,
                        if hot { rgba(128, 128, 128, 0.45) } else { rgba(128, 128, 128, 0.25) },
                    );
                    let col = fg;
                    let p = 4.0;
                    let mut pb = aqua_gfx::tiny_skia::PathBuilder::new();
                    pb.move_to(r.cx() - p, r.cy() - p);
                    pb.line_to(r.cx() + p, r.cy() + p);
                    pb.move_to(r.cx() + p, r.cy() - p);
                    pb.line_to(r.cx() - p, r.cy() + p);
                    if let Some(path) = pb.finish() {
                        c.stroke_path(&path, &aqua_gfx::canvas::solid(col), 1.6);
                    }
                }
                BTN_SCREEN | BTN_WINDOW | BTN_PORTION | BTN_REC_SCREEN | BTN_REC_PORTION => {
                    let sel = matches!(
                        (*id, kind),
                        (BTN_SCREEN, Kind::Screen)
                            | (BTN_WINDOW, Kind::Window)
                            | (BTN_PORTION, Kind::Portion)
                            | (BTN_REC_SCREEN, Kind::RecScreen)
                            | (BTN_REC_PORTION, Kind::RecPortion)
                    );
                    if sel {
                        c.fill_rrect(r, 8.0, if dark { rgba(255, 255, 255, 0.22) } else { rgba(0, 0, 0, 0.12) });
                    } else if hot {
                        c.fill_rrect(r, 8.0, rgba(128, 128, 128, 0.15));
                    }
                    draw_icon(c, *id, r, if sel { fg } else { fg2 });
                }
                BTN_OPTIONS => {
                    if hot || sh.shot.options_open {
                        c.fill_rrect(r, 8.0, rgba(128, 128, 128, 0.15));
                    }
                    let lbl = if timer > 0 {
                        crate::trf("Options · {timer}s ▾", &[("timer", &timer)])
                    } else {
                        "Options ▾".to_string()
                    };
                    c.text_in(&f, r, 0.5, 13.0, Weight::Medium, fg, &lbl);
                }
                BTN_CAPTURE => {
                    c.fill_rrect(r, 8.0, if hot { style::accent(1.0) } else { style::accent(0.9) });
                    let lbl = if recording {
                        "Stop Recording"
                    } else if kind.is_record() {
                        "Record"
                    } else {
                        "Capture"
                    };
                    c.text_in(&f, r, 0.5, 13.0, Weight::Semibold, rgba(255, 255, 255, 1.0), lbl);
                }
                _ => {}
            }
        }
        let sep = style::separator(dark);
        c.fill_rect(Rect::new(40.0, 12.0, 1.0, TB_H - 24.0), sep);
        c.fill_rect(Rect::new(197.0, 12.0, 1.0, TB_H - 24.0), sep);
        c.fill_rect(Rect::new(307.0, 12.0, 1.0, TB_H - 24.0), sep);
    });
    let mut g = style::glass_menu(&sh.cfg.glass, dark);
    g.radius = 14.0;
    let mut v = vec![Layer {
        id: LayerId::Shot(40),
        rect: t,
        glass: Some(g),
        tiles: vec![],
        content: pm,
        serial,
        opacity: 1.0,
        zoom: 1.0,
    }];
    if sh.shot.options_open {
        let o = options_rect(sh);
        let rows = option_rows(sh);
        let key = hash_of(&(rows.iter().map(|r| (r.0.clone(), r.3)).collect::<Vec<_>>(), hover, dark));
        let (pm, serial) = sh.cached(LayerId::Shot(41), key, o.w, o.h, |c, sh| {
            let f = sh.fonts.clone();
            let fg = style::text_primary(dark);
            let fg2 = style::text_secondary(dark);
            for (i, (l, header, _, on)) in rows.iter().enumerate() {
                let r = Rect::new(6.0, 6.0 + i as f32 * ROW_H, o.w - 12.0, ROW_H);
                if *header {
                    c.text(&f, 12.0, r.y + 15.0, 11.0, Weight::Semibold, fg2, l);
                    continue;
                }
                let hot = hover == Some(1000 + i);
                if hot {
                    c.fill_rrect(r, 5.0, style::accent(0.9));
                }
                let col = if hot { rgba(255, 255, 255, 1.0) } else { fg };
                if *on {
                    aqua_gfx::symbols::checkmark(c, Rect::new(r.x + 6.0, r.y + 6.0, 10.0, 10.0), col, 1.6);
                }
                c.text(&f, r.x + 22.0, r.y + 15.5, 13.0, Weight::Regular, col, l);
            }
        });
        v.push(Layer {
            id: LayerId::Shot(41),
            rect: o,
            glass: Some(g),
            tiles: vec![],
            content: pm,
            serial,
            opacity: 1.0,
            zoom: 1.0,
        });
    }
    v
}

fn thumb_rect(sh: &Shell, t: &Thumb) -> Rect {
    let (iw, ih) = (t.pm.width() as f32, t.pm.height() as f32);
    let w = 180.0f32;
    let h = (w * ih / iw.max(1.0)).clamp(40.0, 180.0);
    let w = if h >= 180.0 { 180.0 * iw / ih.max(1.0) } else { w };
    let age = t.shown.elapsed().as_secs_f32();
    let slide = if age < 0.35 {
        1.0 - (1.0 - (1.0 - age / 0.35).powi(3))
    } else if age > THUMB_SECS - 0.4 {
        ((age - (THUMB_SECS - 0.4)) / 0.4).min(1.0).powi(2)
    } else {
        0.0
    };
    let x = sh.w - w - 22.0 + slide * (w + 40.0);
    Rect::new(x, sh.h - h - 90.0, w, h)
}

pub fn layers(sh: &mut Shell) -> Vec<Layer> {
    let mut out = vec![];
    sh.shot.tick();
    let (x, y) = sh.shot.cur;
    match sh.shot.mode {
        Mode::Area => {
            if let Some(a) = sh.shot.drag {
                let r = norm(a, (x, y));
                if sh.shot.toolbar {
                    out.extend(dim_outside(sh, 0, r, rgba(0, 0, 0, 0.35)));
                } else {
                    out.push(fill(sh, 0, r, rgba(160, 160, 160, 0.22)));
                }
                out.extend(outline(sh, 4, r, rgba(255, 255, 255, 0.9), 1.0));
                out.push(label(
                    sh,
                    20,
                    x,
                    y,
                    &format!("{} × {}", (r.w * sh.scale).round() as i32, (r.h * sh.scale).round() as i32),
                ));
            } else if let (true, Some(r)) = (sh.shot.toolbar, sh.shot.sel) {
                out.extend(dim_outside(sh, 0, r, rgba(0, 0, 0, 0.35)));
                out.extend(outline(sh, 4, r, rgba(255, 255, 255, 0.9), 1.0));
            } else if !sh.shot.toolbar || !toolbar_rect(sh).contains(x, y) {
                out.push(label(
                    sh,
                    20,
                    x,
                    y,
                    &format!("{}  {}", (x * sh.scale).round() as i32, (y * sh.scale).round() as i32),
                ));
            }
        }
        Mode::Window => {
            if let Some(r) = window_at(sh, x, y) {
                out.push(fill(sh, 0, r, rgba(80, 140, 255, 0.32)));
            } else {
                out.push(fill(sh, 0, Rect::new(0.0, 0.0, sh.w, sh.h), rgba(80, 140, 255, 0.18)));
            }
        }
        Mode::Off => {}
    }
    if sh.shot.toolbar {
        out.extend(toolbar_layers(sh));
    }
    if let Some((start, secs)) = sh.shot.countdown {
        let left = (secs - start.elapsed().as_secs_f32()).ceil().max(1.0) as i32;
        let r = Rect::new(sh.w / 2.0 - 40.0, sh.h / 2.0 - 40.0, 80.0, 80.0);
        let (pm, serial) = sh.cached(LayerId::Shot(42), left as u64, r.w, r.h, |c, sh| {
            let f = sh.fonts.clone();
            c.fill_rrect(Rect::new(0.0, 0.0, 80.0, 80.0), 18.0, rgba(20, 20, 22, 0.6));
            c.text_in(
                &f,
                Rect::new(0.0, 0.0, 80.0, 80.0),
                0.5,
                40.0,
                Weight::Semibold,
                rgba(255, 255, 255, 1.0),
                &left.to_string(),
            );
        });
        out.push(Layer {
            id: LayerId::Shot(42),
            rect: r,
            glass: None,
            tiles: vec![],
            content: pm,
            serial,
            opacity: 1.0,
            zoom: 1.0,
        });
    }
    let clicks = sh.shot.clicks.clone();
    for (i, (cx, cy, at)) in clicks.into_iter().enumerate().take(8) {
        let k = (at.elapsed().as_secs_f32() / CLICK_SECS).clamp(0.0, 1.0);
        let q = (k * 12.0) as u64;
        let rad = 14.0 + 14.0 * (q as f32 / 12.0);
        let r = Rect::new(cx - 30.0, cy - 30.0, 60.0, 60.0);
        let id = LayerId::Shot(50 + i as u8);
        let (pm, serial) = sh.cached(id, q, r.w, r.h, move |c, _| {
            let a = 1.0 - (q as f32 / 12.0);
            c.fill_circle(30.0, 30.0, rad, rgba(128, 128, 128, 0.28 * a));
            if let Some(p) = aqua_gfx::tiny_skia::PathBuilder::from_circle(30.0, 30.0, rad) {
                c.stroke_path(&p, &aqua_gfx::canvas::solid(rgba(40, 40, 40, 0.55 * a)), 2.0);
            }
        });
        out.push(Layer { id, rect: r, glass: None, tiles: vec![], content: pm, serial, opacity: 1.0, zoom: 1.0 });
    }
    if let Some(t) = sh.shot.thumb.as_ref() {
        let r = thumb_rect(sh, t);
        let pm_src = t.pm.clone();
        let key = t.serial;
        let (pm, serial) = sh.cached(LayerId::Shot(43), key, r.w + 8.0, r.h + 8.0, move |c, _| {
            c.fill_rrect(Rect::new(1.0, 2.0, r.w + 6.0, r.h + 6.0), 6.0, rgba(0, 0, 0, 0.25));
            c.fill_rrect(Rect::new(0.0, 0.0, r.w + 6.0, r.h + 6.0), 6.0, rgba(255, 255, 255, 1.0));
            c.draw_pixmap(&pm_src, Rect::new(3.0, 3.0, r.w, r.h), 1.0);
        });
        out.push(Layer {
            id: LayerId::Shot(43),
            rect: Rect::new(r.x - 3.0, r.y - 3.0, r.w + 8.0, r.h + 8.0),
            glass: None,
            tiles: vec![],
            content: pm,
            serial,
            opacity: 1.0,
            zoom: 1.0,
        });
    }
    out
}

pub fn wants_pointer(sh: &Shell, x: f32, y: f32) -> bool {
    sh.shot.active() || sh.shot.thumb.as_ref().is_some_and(|t| thumb_rect(sh, t).contains(x, y))
}

pub fn motion(sh: &mut Shell, x: f32, y: f32) {
    sh.shot.cur = (x, y);
    if !sh.shot.toolbar {
        sh.shot.hover = None;
        return;
    }
    sh.shot.hover = None;
    if sh.shot.options_open {
        let o = options_rect(sh);
        if o.contains(x, y) {
            let i = ((y - o.y - 6.0) / ROW_H).floor();
            if i >= 0.0 {
                let rows = option_rows(sh);
                if let Some(r) = rows.get(i as usize) {
                    if !r.1 {
                        sh.shot.hover = Some(1000 + i as usize);
                    }
                }
            }
            return;
        }
    }
    sh.shot.hover = buttons(sh).into_iter().find(|(_, r)| r.contains(x, y)).map(|(i, _)| i);
}

fn take(sh: &mut Shell, t: Target) -> Vec<Action> {
    sh.shot.from_toolbar = sh.shot.toolbar;
    sh.shot.close();
    vec![Action::ScreenshotTake(t), Action::Redraw]
}

/// Pointer button (press and release). None = not handled by the screenshot UI.
pub fn button(sh: &mut Shell, x: f32, y: f32, pressed: bool) -> Option<Vec<Action>> {
    sh.shot.cur = (x, y);
    if !sh.shot.active() {
        if pressed {
            if let Some(t) = sh.shot.thumb.as_ref() {
                if thumb_rect(sh, t).contains(x, y) {
                    let p = t.path.clone();
                    sh.shot.thumb = None;
                    if p.is_empty() {
                        return Some(vec![Action::Redraw]);
                    }
                    return Some(vec![
                        Action::Launch(format!("xdg-open '{}'", p.replace('\'', "'\\''"))),
                        Action::Redraw,
                    ]);
                }
            }
        }
        return None;
    }
    if sh.shot.toolbar {
        if sh.shot.options_open && options_rect(sh).contains(x, y) {
            if !pressed {
                return Some(vec![]);
            }
            let o = options_rect(sh);
            let i = ((y - o.y - 6.0) / ROW_H).floor();
            let rows = option_rows(sh);
            if let Some(r) = (i >= 0.0).then(|| rows.get(i as usize)).flatten() {
                if !r.1 {
                    let k = r.2;
                    match k {
                        "thumb" => sh.shot.show_thumb = !sh.shot.show_thumb,
                        "pointer" => sh.shot.show_pointer = !sh.shot.show_pointer,
                        "rpointer" => sh.shot.rec_pointer = !sh.shot.rec_pointer,
                        "clicks" => sh.shot.rec_clicks = !sh.shot.rec_clicks,
                        "mic:0" => sh.shot.rec_mic = false,
                        "mic:1" => sh.shot.rec_mic = true,
                        _ if k.starts_with("rsave:") => sh.shot.rec_save = k[6..].to_string(),
                        _ if k.starts_with("save:") => sh.shot.save_to = k[5..].to_string(),
                        _ if k.starts_with("timer:") => sh.shot.timer = k[6..].parse().unwrap_or(0),
                        _ => {}
                    }
                    return Some(vec![Action::ScreenshotOptions, Action::Redraw]);
                }
            }
            return Some(vec![Action::Redraw]);
        }
        if let Some((id, _)) = buttons(sh).into_iter().find(|(_, r)| r.contains(x, y)) {
            if !pressed {
                return Some(vec![]);
            }
            if id != BTN_OPTIONS {
                sh.shot.options_open = false;
            }
            match id {
                BTN_CLOSE => sh.shot.close(),
                BTN_SCREEN => {
                    sh.shot.kind = Kind::Screen;
                    sh.shot.mode = Mode::Off;
                }
                BTN_WINDOW => {
                    sh.shot.kind = Kind::Window;
                    sh.shot.mode = Mode::Window;
                }
                BTN_REC_SCREEN => {
                    sh.shot.kind = Kind::RecScreen;
                    sh.shot.mode = Mode::Off;
                }
                BTN_PORTION | BTN_REC_PORTION => {
                    sh.shot.kind = if id == BTN_PORTION { Kind::Portion } else { Kind::RecPortion };
                    sh.shot.mode = Mode::Area;
                    if sh.shot.sel.is_none() {
                        let (w, h) = (sh.w * 0.5, sh.h * 0.5);
                        sh.shot.sel = Some(Rect::new((sh.w - w) / 2.0, (sh.h - h) / 2.0 - 40.0, w, h));
                    }
                }
                BTN_OPTIONS => sh.shot.options_open = !sh.shot.options_open,
                BTN_CAPTURE => return Some(capture_current(sh)),
                _ => {}
            }
            return Some(vec![Action::Redraw]);
        }
        if sh.shot.options_open && pressed {
            sh.shot.options_open = false;
            return Some(vec![Action::Redraw]);
        }
        if toolbar_rect(sh).contains(x, y) {
            return Some(vec![]);
        }
    }
    match sh.shot.mode {
        Mode::Area => {
            if pressed {
                sh.shot.drag = Some((x, y));
                return Some(vec![Action::Redraw]);
            }
            let Some(a) = sh.shot.drag.take() else { return Some(vec![]) };
            let r = norm(a, (x, y));
            if r.w < 4.0 || r.h < 4.0 {
                if sh.shot.toolbar {
                    return Some(vec![Action::Redraw]);
                }
                return Some(match window_at(sh, x, y) {
                    Some(w) => take(sh, Target::Window(w)),
                    None => take(sh, Target::Full),
                });
            }
            if sh.shot.toolbar {
                sh.shot.sel = Some(r);
                return Some(vec![Action::Redraw]);
            }
            Some(take(sh, Target::Area(r)))
        }
        Mode::Window => {
            if !pressed {
                return Some(vec![]);
            }
            match window_at(sh, x, y) {
                Some(w) => Some(take(sh, Target::Window(w))),
                None => Some(vec![]),
            }
        }
        Mode::Off => Some(vec![]),
    }
}

fn record(sh: &mut Shell, t: Target) -> Vec<Action> {
    sh.shot.from_toolbar = true;
    sh.shot.close();
    vec![Action::RecordStart(t), Action::Redraw]
}

fn capture_current(sh: &mut Shell) -> Vec<Action> {
    if sh.shot.recording.is_some() {
        sh.shot.close();
        return vec![Action::RecordStop, Action::Redraw];
    }
    match sh.shot.kind {
        Kind::RecScreen => record(sh, Target::Full),
        Kind::RecPortion => match sh.shot.sel {
            Some(r) => record(sh, Target::Area(r)),
            None => record(sh, Target::Full),
        },
        Kind::Screen => take(sh, Target::Full),
        Kind::Portion => match sh.shot.sel {
            Some(r) => take(sh, Target::Area(r)),
            None => take(sh, Target::Full),
        },
        Kind::Window => {
            match window_at(sh, sh.shot.cur.0, sh.shot.cur.1).or_else(|| sh.shot.windows.first().copied()) {
                Some(w) => take(sh, Target::Window(w)),
                None => take(sh, Target::Full),
            }
        }
    }
}

pub fn key(sh: &mut Shell, key: Option<Key>, text: Option<&str>) -> (bool, Vec<Action>) {
    match key {
        Some(Key::Escape) => {
            if sh.shot.options_open {
                sh.shot.options_open = false;
            } else if sh.shot.drag.is_some() {
                sh.shot.drag = None;
            } else {
                sh.shot.close();
            }
            (true, vec![Action::Redraw])
        }
        Some(Key::Enter) if sh.shot.toolbar => (true, capture_current(sh)),
        _ if text == Some(" ") && !sh.shot.toolbar => {
            sh.shot.mode = if sh.shot.mode == Mode::Window { Mode::Area } else { Mode::Window };
            sh.shot.drag = None;
            (true, vec![Action::Redraw])
        }
        _ => (true, vec![]),
    }
}
