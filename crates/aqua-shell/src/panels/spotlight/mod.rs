//! Spotlight: a glass search field with four round filter buttons next to
//! it — Applications (⌘1), Files (⌘2), Actions (⌘3) and Clipboard (⌘4). Typing or
//! choosing a filter tucks the buttons into the field, which widens and grows a result
//! list underneath (scrollable, gliding selection). Without a filter the results mix
//! an inline calculator, apps (Aqua's own included), System Settings panes, actions
//! and files.
mod calc;
mod search;

use crate::scroll::Smooth;
use crate::{hash_of, style, Action, Key, Layer, LayerId, Shell};
use aqua_gfx::{rgba, symbols, Color, Rect, Weight};
use aqua_icons::IconRequest;
pub use calc::calc;
use calc::fmt_num;
use search::*;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Filter {
    Apps,
    Files,
    Actions,
    Clipboard,
}

impl Filter {
    pub const ALL: [Filter; 4] = [Filter::Apps, Filter::Files, Filter::Actions, Filter::Clipboard];
    pub fn label(self) -> &'static str {
        match self {
            Filter::Apps => "Applications",
            Filter::Files => "Files",
            Filter::Actions => "Actions",
            Filter::Clipboard => "Clipboard",
        }
    }
    fn placeholder(self) -> &'static str {
        match self {
            Filter::Apps => "Search Applications",
            Filter::Files => "Search Files",
            Filter::Actions => "Search Actions",
            Filter::Clipboard => "Search Clipboard",
        }
    }
    fn glyph(self, c: &mut aqua_gfx::Canvas, r: Rect, col: Color) {
        match self {
            Filter::Apps => symbols::apps_glyph(c, r, col),
            Filter::Files => symbols::folder(c, r, col),
            Filter::Actions => symbols::layers(c, r, col),
            Filter::Clipboard => symbols::documents(c, r, col),
        }
    }
}

#[derive(Default)]
pub struct Spotlight {
    pub open: bool,
    pub t: f32,
    pub query: String,
    pub sel: usize,
    pub hover: Option<usize>,
    /// Where the panel morphs from / back into (the menu-bar magnifier), logical.
    pub origin: Option<Rect>,
    /// Animated panel height and selection-highlight position (results grow/shrink and
    /// the highlight glides between rows).
    pub h: f32,
    pub target_h: f32,
    pub sel_y: f32,
    pub target_sel_y: f32,
    pub reduce_motion: bool,
    /// Active filter (⌘1…⌘4 or the round buttons).
    pub filter: Option<Filter>,
    /// 1 = the filter buttons stand next to the field, 0 = tucked into it.
    pub btn_t: f32,
    pub btn_target: f32,
    pub hover_btn: Option<usize>,
    pub scroll: Smooth,
    /// Width of the search field (animated between compact and full width).
    pub field_w: f32,
    closed_at: Option<Instant>,
}

impl Spotlight {
    pub fn visible(&self) -> bool {
        self.open || self.t > 0.001
    }
    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.query.clear();
            self.sel = 0;
            self.hover = None;
            self.hover_btn = None;
            self.filter = None;
            self.scroll.reset();
            self.closed_at = None;
            if self.t <= 0.001 {
                self.h = 0.0;
                self.sel_y = 0.0;
                self.btn_t = 0.0;
                self.field_w = 0.0;
            }
            files::warm();
        } else {
            self.closed_at = Some(Instant::now());
        }
    }
    pub fn set_filter(&mut self, f: Option<Filter>) {
        self.filter = f;
        self.sel = 0;
        self.scroll.reset();
        self.hover = None;
        if f == Some(Filter::Files) {
            files::warm();
        }
    }
    pub fn animate(&mut self, dt: f32) -> bool {
        let mut anim = false;
        let k = if self.reduce_motion { 1.0 } else { 1.0 - (-16.0 * dt).exp() };
        if self.target_h > 0.0 && (self.h - self.target_h).abs() > 0.25 {
            self.h += (self.target_h - self.h) * k;
            anim = true;
        } else {
            self.h = self.target_h;
        }
        if (self.sel_y - self.target_sel_y).abs() > 0.25 {
            self.sel_y +=
                (self.target_sel_y - self.sel_y) * if self.reduce_motion { 1.0 } else { 1.0 - (-22.0 * dt).exp() };
            anim = true;
        } else {
            self.sel_y = self.target_sel_y;
        }
        anim |= self.scroll.animate(dt);
        let bt = if self.open && self.t > 0.55 {
            self.btn_target
        } else if self.open {
            self.btn_t.min(self.btn_target)
        } else {
            0.0
        };
        if (self.btn_t - bt).abs() > 0.002 {
            self.btn_t += (bt - self.btn_t) * if self.reduce_motion { 1.0 } else { 1.0 - (-13.0 * dt).exp() };
            anim = true;
        } else {
            self.btn_t = bt;
        }
        if self.open && self.filter == Some(Filter::Files) && files::building() {
            anim = true;
        }
        if let Some(c) = self.closed_at {
            if !self.open && c.elapsed() > Duration::from_secs(120) {
                files::drop_index();
                self.closed_at = None;
            }
        }
        let target = if self.open { 1.0 } else { 0.0 };
        if (self.t - target).abs() < 0.001 {
            self.t = target;
            return anim;
        }
        if self.reduce_motion {
            self.t = target;
            return true;
        }
        let rate = if self.open { 10.0 } else { 13.0 };
        self.t += (target - self.t) * (1.0 - (-rate * dt).exp());
        if (self.t - target).abs() < 0.005 {
            self.t = target;
        }
        true
    }
}

const BAR_H: f32 = 56.0;
const BTN: f32 = 56.0;
const BTN_GAP: f32 = 12.0;
const ROW_H: f32 = 44.0;
const MAX_VISIBLE: usize = 8;
const LIST_TOP: f32 = BAR_H + 12.0;

/// Full width of Spotlight (field + buttons, or the expanded field).
fn full_width(sh: &Shell) -> f32 {
    (sh.w * 0.44).clamp(560.0, 720.0).min(sh.w - 40.0)
}

fn compact_field_w(sh: &Shell) -> f32 {
    full_width(sh) - 4.0 * (BTN + BTN_GAP)
}

/// Do the filter buttons belong next to the field right now?
fn buttons_wanted(sh: &Shell) -> bool {
    sh.spotlight.query.is_empty() && sh.spotlight.filter.is_none()
}

/// Rows area height (logical) for `n` rows; filters always show a list area.
fn list_h(sh: &Shell, n: usize) -> f32 {
    if n == 0 {
        return if sh.spotlight.filter.is_some() { 64.0 } else { 0.0 };
    }
    n.min(MAX_VISIBLE) as f32 * ROW_H + 10.0
}

pub fn panel_rect(sh: &Shell, n: usize) -> Rect {
    let w = full_width(sh);
    let lh = list_h(sh, n);
    let h = BAR_H + if lh > 0.0 { 12.0 + lh } else { 0.0 };
    Rect::new((sh.w - w) / 2.0, sh.h * 0.2, w, h)
}

/// Final rectangles of the four filter buttons (when shown).
fn button_rects(sh: &Shell) -> Vec<Rect> {
    let full = panel_rect(sh, 0);
    let x0 = full.x + compact_field_w(sh) + BTN_GAP;
    (0..4).map(|i| Rect::new(x0 + i as f32 * (BTN + BTN_GAP), full.y, BTN, BTN)).collect()
}

/// Animated button rect (slides out of the field's right end, scales up).
fn button_anim(sh: &Shell, i: usize, field_right: f32) -> (Rect, f32) {
    let fin = button_rects(sh)[i];
    let e = sh.spotlight.btn_t.clamp(0.0, 1.0);
    let e = 1.0 - (1.0 - e).powi(3);
    let from_cx = field_right - BTN * 0.6;
    let cx = from_cx + (fin.cx() - from_cx) * e;
    let d = BTN * (0.35 + 0.65 * e);
    (Rect::new(cx - d / 2.0, fin.cy() - d / 2.0, d, d), e)
}

/// Glass "blob" + content layers.
pub fn layers(sh: &mut Shell) -> Vec<Layer> {
    if !sh.spotlight.visible() {
        sh.spotlight.h = 0.0;
        sh.spotlight.target_h = 0.0;
        return vec![];
    }
    sh.spotlight.reduce_motion = sh.cfg.reduce_motion;
    sh.spotlight.btn_target = if buttons_wanted(sh) { 1.0 } else { 0.0 };
    let rows = rows(sh);
    let n = rows.len();
    let full = panel_rect(sh, n);
    let sel = sh.spotlight.sel.min(n.saturating_sub(1));
    sh.spotlight.scroll.set_max((n as f32 - MAX_VISIBLE as f32).max(0.0) * ROW_H);
    sh.spotlight.target_h = full.h;
    if sh.spotlight.h <= 0.0 {
        sh.spotlight.h = full.h;
    }
    sh.spotlight.target_sel_y = sel as f32 * ROW_H;
    if n == 0 {
        sh.spotlight.sel_y = sh.spotlight.target_sel_y;
    }
    let fw_c = compact_field_w(sh);
    let e = sh.spotlight.btn_t.clamp(0.0, 1.0);
    let field_w = full.w + (fw_c - full.w) * (1.0 - (1.0 - e).powi(2));
    sh.spotlight.field_w = field_w;
    let tall = sh.spotlight.h.max(BAR_H).round();
    let r = Rect::new(full.x, full.y, if tall > BAR_H + 1.0 { full.w } else { field_w }, tall);
    let dark = sh.style.is_dark_glass(r);
    let t = sh.spotlight.t;
    let et = 1.0 - (1.0 - t).powi(3);
    let from = sh.spotlight.origin.unwrap_or_else(|| Rect::new(r.cx() - r.w * 0.35, r.y + 4.0, r.w * 0.7, BAR_H - 8.0));
    let lerp = |a: f32, b: f32| a + (b - a) * et;
    let m = Rect::new(lerp(from.x, r.x), lerp(from.y, r.y), lerp(from.w, r.w), lerp(from.h, r.h));
    let radius_to = if tall <= BAR_H + 1.0 { BAR_H / 2.0 } else { 26.0 };
    let tint = aqua_config::Rgba(1.0, 1.0, 1.0, if dark { 0.12 } else { 0.34 });
    let mut g = style::glass_panel(&sh.cfg.glass, lerp(from.h.min(from.w) / 2.0, radius_to).min(m.h / 2.0));
    g.max_luma = 0.88;
    g.tint = tint;
    let glass_alpha = (t * 4.0).min(1.0);
    let blank = sh.blank_pixmap();
    let mut out = vec![Layer {
        id: LayerId::SpotlightGlass,
        rect: m,
        glass: Some(g),
        tiles: vec![],
        content: blank,
        serial: 0,
        opacity: glass_alpha,
        zoom: 1.0,
    }];
    let mut content = content_layer(sh, &rows, Rect::new(full.x, full.y, full.w, tall), sel, dark, field_w);
    let ct = ((t - 0.45) / 0.55).clamp(0.0, 1.0);
    content.opacity = ct * ct * (3.0 - 2.0 * ct);
    content.zoom = 0.97 + 0.03 * et;
    out.push(content);
    if sh.spotlight.btn_t > 0.01 && tall <= BAR_H + 1.0 {
        let field_right = full.x + field_w;
        for (i, f) in Filter::ALL.iter().enumerate() {
            let (br, be) = button_anim(sh, i, field_right);
            let hot = sh.spotlight.hover_btn == Some(i);
            let mut bg = style::glass_panel(&sh.cfg.glass, br.w / 2.0);
            bg.max_luma = 0.88;
            bg.tint = aqua_config::Rgba(1.0, 1.0, 1.0, if dark { 0.12 } else { 0.34 } + if hot { 0.12 } else { 0.0 });
            let id = LayerId::SpotlightButton(i as u8);
            let key = hash_of(&(*f as u8, hot, dark));
            let fg = style::text_primary(dark);
            let ff = *f;
            let (pm, serial) = sh.cached(id, key, BTN, BTN, move |c, _| {
                ff.glyph(c, Rect::new(BTN * 0.29, BTN * 0.29, BTN * 0.42, BTN * 0.42), fg);
            });
            out.push(Layer {
                id,
                rect: br,
                glass: Some(bg),
                tiles: vec![],
                content: pm,
                serial,
                opacity: be * content_opacity(t),
                zoom: 1.0,
            });
        }
    }
    out
}

fn content_opacity(t: f32) -> f32 {
    let ct = ((t - 0.45) / 0.55).clamp(0.0, 1.0);
    ct * ct * (3.0 - 2.0 * ct)
}

fn content_layer(sh: &mut Shell, rows: &[Row], r: Rect, sel: usize, dark: bool, field_w: f32) -> Layer {
    let sp = &sh.spotlight;
    let sel_y = sp.sel_y;
    let scroll = sp.scroll.pos;
    let filter = sp.filter;
    let building = filter == Some(Filter::Files) && files::building();
    let key = hash_of(&(
        (sp.query.clone(), sel, (sel_y * 2.0) as i32, (scroll * 2.0) as i32, sp.hover, format!("{rows:?}")),
        (r.w as i32, r.h as i32, (field_w * 2.0) as i32, dark, filter, building, sh.icons_serial()),
    ));
    let query = sp.query.clone();
    let rows2 = rows.to_vec();
    let (pm, serial) = sh.cached(LayerId::Spotlight, key, r.w, r.h, |c, sh| {
        let f = sh.fonts.clone();
        let fg = style::text_primary(dark);
        let fg2 = style::text_secondary(dark);
        if r.h > BAR_H + 2.0 {
            c.fill_rect(Rect::new(18.0, BAR_H + 4.0, r.w - 36.0, 1.0), style::separator(dark));
            if rows2.is_empty() {
                let msg = if building { "Indexing Files…" } else { "No Results" };
                c.text_in(&f, Rect::new(0.0, LIST_TOP, r.w, 52.0), 0.5, 15.0, Weight::Regular, fg2, msg);
            } else {
                let ipx = (30.0 * sh.scale).round() as u32;
                c.fill_rrect(
                    Rect::new(10.0, LIST_TOP + sel_y - scroll, r.w - 20.0, ROW_H - 2.0),
                    12.0,
                    style::accent(0.92),
                );
                let first = (scroll / ROW_H).floor().max(0.0) as usize;
                for (i, row) in rows2.iter().enumerate().skip(first).take(MAX_VISIBLE + 2) {
                    let rr = Rect::new(10.0, LIST_TOP + i as f32 * ROW_H - scroll, r.w - 20.0, ROW_H - 2.0);
                    if rr.y > r.h || rr.bottom() < LIST_TOP - ROW_H {
                        continue;
                    }
                    let selected = (LIST_TOP + sel_y - scroll - rr.y).abs() < ROW_H * 0.5;
                    if !selected && sh.spotlight.hover == Some(i) {
                        c.fill_rrect(rr, 12.0, rgba(255, 255, 255, if dark { 0.10 } else { 0.30 }));
                    }
                    let (tfg, tfg2) =
                        if selected { (rgba(255, 255, 255, 1.0), rgba(255, 255, 255, 0.8)) } else { (fg, fg2) };
                    let ir = Rect::new(rr.x + 10.0, rr.y + (rr.h - 30.0) / 2.0, 30.0, 30.0);
                    let title: String = match row {
                        Row::Calc(v) => {
                            let icon = sh.icons.get(
                                &IconRequest {
                                    id: "org.gnome.Calculator".into(),
                                    name: "Calculator".into(),
                                    icon: "builtin:calculator".into(),
                                },
                                ipx,
                            );
                            c.draw_pixmap(&icon, ir, 1.0);
                            format!("= {v}")
                        }
                        Row::App(ai) => {
                            let a = &sh.apps[*ai];
                            let icon = sh.icons.get(
                                &IconRequest { id: a.id.clone(), name: a.name.clone(), icon: a.icon.clone() },
                                ipx,
                            );
                            c.draw_pixmap(&icon, ir, 1.0);
                            a.name.clone()
                        }
                        Row::Setting(_, t) => {
                            let icon = sh.icons.get(
                                &IconRequest {
                                    id: "org.aqua.settings".into(),
                                    name: "System Settings".into(),
                                    icon: "builtin:settings".into(),
                                },
                                ipx,
                            );
                            c.draw_pixmap(&icon, ir, 1.0);
                            t.to_string()
                        }
                        Row::Act(i) => {
                            c.fill_rrect(
                                ir.inset(1.0),
                                8.0,
                                if selected { rgba(255, 255, 255, 0.22) } else { rgba(128, 128, 140, 0.28) },
                            );
                            symbols::layers(c, ir.inset(6.0), tfg);
                            ACTIONS[*i].1.to_string()
                        }
                        Row::File(p, dir) => {
                            if *dir {
                                let icon = sh.icons.get(
                                    &IconRequest {
                                        id: "folder".into(),
                                        name: "Folder".into(),
                                        icon: "builtin:folder".into(),
                                    },
                                    ipx,
                                );
                                c.draw_pixmap(&icon, ir, 1.0);
                            } else {
                                symbols::document(c, ir, rgba(255, 255, 255, 0.96), rgba(120, 120, 128, 0.7));
                            }
                            p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
                        }
                        Row::Clip(_, t, _) => {
                            c.fill_rrect(
                                ir.inset(1.0),
                                8.0,
                                if selected { rgba(255, 255, 255, 0.22) } else { rgba(128, 128, 140, 0.28) },
                            );
                            symbols::documents(c, ir.inset(5.0), tfg);
                            t.lines().next().unwrap_or("").chars().take(80).collect()
                        }
                    };
                    let kind = match row {
                        Row::File(p, _) => p.parent().map(tilde).unwrap_or_default(),
                        Row::Clip(_, _, d) => d.chars().take(40).collect(),
                        _ => row_kind(row).to_string(),
                    };
                    let kw = f.measure(&kind, 13.0, Weight::Regular).min(rr.w * 0.42);
                    c.text_in(
                        &f,
                        Rect::new(rr.x + 52.0, rr.y, rr.w - 52.0 - kw - 24.0, rr.h),
                        0.0,
                        16.0,
                        if matches!(row, Row::Calc(_)) { Weight::Medium } else { Weight::Regular },
                        tfg,
                        &title,
                    );
                    c.text_in(
                        &f,
                        Rect::new(rr.right() - kw - 14.0, rr.y, kw, rr.h),
                        1.0,
                        13.0,
                        Weight::Regular,
                        tfg2,
                        &kind,
                    );
                }
                if scroll < sh.spotlight.scroll.max - 0.5 {
                    fade(c, r.w, r.h, r.h - 30.0);
                }
            }
            let p =
                aqua_gfx::tiny_skia::Paint { blend_mode: aqua_gfx::tiny_skia::BlendMode::Clear, ..Default::default() };
            if let Some(cr) = aqua_gfx::tiny_skia::Rect::from_xywh(0.0, 0.0, r.w * c.scale, (BAR_H + 3.5) * c.scale) {
                c.pm.fill_rect(cr, &p, aqua_gfx::tiny_skia::Transform::identity(), None);
            }
            if scroll > 0.5 {
                fade(c, r.w, LIST_TOP - 6.0, LIST_TOP + 14.0);
            }
            c.fill_rect(Rect::new(18.0, BAR_H + 4.0, r.w - 36.0, 1.0), style::separator(dark));
        }
        symbols::search(c, Rect::new(20.0, (BAR_H - 22.0) / 2.0, 22.0, 22.0), fg2);
        let mut tx = 54.0;
        if let Some(fl) = filter {
            let lw = f.measure(fl.label(), 14.0, Weight::Medium);
            let chip = Rect::new(tx - 4.0, (BAR_H - 30.0) / 2.0, lw + 40.0, 30.0);
            c.fill_rrect(chip, 15.0, if dark { rgba(255, 255, 255, 0.16) } else { rgba(255, 255, 255, 0.55) });
            fl.glyph(c, Rect::new(chip.x + 8.0, chip.y + 6.0, 18.0, 18.0), fg);
            c.text_in(
                &f,
                Rect::new(chip.x + 30.0, chip.y, lw + 4.0, chip.h),
                0.0,
                14.0,
                Weight::Medium,
                fg,
                fl.label(),
            );
            tx = chip.right() + 10.0;
        }
        let avail = field_w - tx - 20.0;
        if query.is_empty() {
            c.fill_rect(Rect::new(tx, 16.0, 1.6, 24.0), style::accent(0.95));
            let ph = filter.map(|f| f.placeholder()).unwrap_or("Spotlight Search");
            c.text_in(&f, Rect::new(tx + 4.0, 0.0, avail.max(10.0), BAR_H), 0.0, 21.0, Weight::Regular, fg2, ph);
        } else {
            let full_w = f.measure(&query, 21.0, Weight::Regular);
            let shown: String = if full_w > avail {
                let mut s = query.clone();
                while !s.is_empty() && f.measure(&format!("…{s}"), 21.0, Weight::Regular) > avail {
                    s.remove(0);
                }
                format!("…{s}")
            } else {
                query.clone()
            };
            let w = c.text(&f, tx, 36.0, 21.0, Weight::Regular, fg, &shown);
            c.fill_rect(Rect::new(tx + w + 2.0, 16.0, 1.6, 24.0), style::accent(0.95));
        }
    });
    Layer { id: LayerId::Spotlight, rect: r, glass: None, tiles: vec![], content: pm, serial, opacity: 1.0, zoom: 1.0 }
}

/// Erase content between y0 (fully) and y1 (not at all) — soft list edges.
fn fade(c: &mut aqua_gfx::Canvas, w: f32, y0: f32, y1: f32) {
    use aqua_gfx::tiny_skia as sk;
    let sc = c.scale;
    let stops = vec![
        sk::GradientStop::new(0.0, sk::Color::from_rgba8(0, 0, 0, 255)),
        sk::GradientStop::new(1.0, sk::Color::from_rgba8(0, 0, 0, 0)),
    ];
    if let Some(shd) = sk::LinearGradient::new(
        sk::Point::from_xy(0.0, y0 * sc),
        sk::Point::from_xy(0.0, y1 * sc),
        stops,
        sk::SpreadMode::Pad,
        sk::Transform::identity(),
    ) {
        let p = sk::Paint { shader: shd, blend_mode: sk::BlendMode::DestinationOut, ..Default::default() };
        let (a, b) = (y0.min(y1), y0.max(y1));
        if let Some(rr) = sk::Rect::from_xywh(0.0, a * sc, w * sc, (b - a) * sc) {
            c.pm.fill_rect(rr, &p, sk::Transform::identity(), None);
        }
    }
}

fn row_at(sh: &Shell, x: f32, y: f32) -> Option<usize> {
    let rows = rows(sh);
    let r = panel_rect(sh, rows.len());
    if !r.contains(x, y) {
        return None;
    }
    let ly = y - r.y - LIST_TOP + sh.spotlight.scroll.pos;
    if y - r.y < LIST_TOP || ly < 0.0 {
        return None;
    }
    let i = (ly / ROW_H) as usize;
    (i < rows.len()).then_some(i)
}

fn button_at(sh: &Shell, x: f32, y: f32) -> Option<usize> {
    if sh.spotlight.btn_t < 0.5 || !buttons_wanted(sh) {
        return None;
    }
    button_rects(sh).iter().position(|b| ((x - b.cx()).powi(2) + (y - b.cy()).powi(2)).sqrt() <= b.w / 2.0)
}

pub fn hover(sh: &mut Shell, x: f32, y: f32) {
    if sh.spotlight.open {
        sh.spotlight.hover = row_at(sh, x, y);
        sh.spotlight.hover_btn = button_at(sh, x, y);
    }
}

/// Is the point over Spotlight (field, list or buttons)?
fn over(sh: &Shell, x: f32, y: f32) -> bool {
    let n = rows(sh).len();
    let r = panel_rect(sh, n);
    let field = Rect::new(
        r.x,
        r.y,
        if n == 0 && sh.spotlight.filter.is_none() { sh.spotlight.field_w.max(compact_field_w(sh)) } else { r.w },
        r.h,
    );
    field.contains(x, y) || button_at(sh, x, y).is_some()
}

fn run_action(sh: &mut Shell, id: &str) -> Vec<Action> {
    match id {
        "screenshot" => vec![Action::Screenshot],
        "screenshot-ui" => vec![Action::ScreenshotUi("ui".into())],
        "record" => vec![Action::ScreenshotUi("record".into())],
        "dark" => vec![Action::SetDark(!sh.style.dark)],
        "dnd" => vec![Action::SetFocusMode(!sh.notes.dnd)],
        "lock" => vec![Action::Lock],
        "mission" => vec![Action::MissionControl],
        "apps" => {
            sh.launchpad.toggle();
            vec![Action::Redraw]
        }
        "terminal" => vec![Action::OpenTerminal],
        "clipboard" => vec![Action::ShowClipboard],
        "chars" => vec![Action::ShowChars],
        "notifications" => {
            sh.toggle_notification_center();
            vec![Action::Redraw]
        }
        "control" => {
            sh.control.toggle();
            vec![Action::Redraw]
        }
        "widgets" => vec![Action::ToggleWidgets],
        "trash" => vec![Action::EmptyTrash],
        "about" => vec![Action::ShowAbout(String::new())],
        "sleep" => vec![Action::Sleep],
        "restart" => vec![Action::Restart],
        "shutdown" => vec![Action::ShutDown],
        "logout" => vec![Action::LogOut],
        _ => vec![],
    }
}

fn activate(sh: &mut Shell, i: usize) -> Vec<Action> {
    let rows = rows(sh);
    let pr = panel_rect(sh, rows.len());
    let icon_rect = Rect::new(
        pr.x + 20.0,
        pr.y + LIST_TOP + i as f32 * ROW_H - sh.spotlight.scroll.pos + (ROW_H - 2.0 - 30.0) / 2.0,
        30.0,
        30.0,
    );
    let Some(row) = rows.get(i).cloned() else { return vec![Action::Redraw] };
    sh.spotlight.toggle();
    match row {
        Row::App(ai) => {
            sh.launch_origin = Some(icon_rect);
            vec![Action::Launch(sh.apps[ai].launch_command())]
        }
        Row::Calc(v) => vec![Action::CopyText(v)],
        Row::Setting(p, _) => vec![Action::OpenSettings(p.to_string())],
        Row::Act(k) => run_action(sh, ACTIONS[k].0),
        Row::File(p, _) => {
            files::touch(&p);
            vec![Action::Launch(crate::dock::open_cmd(&p.to_string_lossy()))]
        }
        Row::Clip(id, ..) => vec![Action::ClipboardUse(id, true)],
    }
}

pub fn click(sh: &mut Shell, x: f32, y: f32) -> Option<Vec<Action>> {
    if !sh.spotlight.open {
        return None;
    }
    if let Some(b) = button_at(sh, x, y) {
        sh.spotlight.set_filter(Some(Filter::ALL[b]));
        return Some(vec![Action::Redraw]);
    }
    if !over(sh, x, y) {
        sh.spotlight.toggle();
        return Some(vec![Action::Redraw]);
    }
    Some(match row_at(sh, x, y) {
        Some(i) => activate(sh, i),
        None => vec![Action::Redraw],
    })
}

pub fn scroll(sh: &mut Shell, x: f32, y: f32, dy: f32, wheel: bool) -> bool {
    if !sh.spotlight.open {
        return false;
    }
    let n = rows(sh).len();
    if !panel_rect(sh, n).contains(x, y) {
        return true;
    }
    sh.spotlight.scroll.scroll(dy, wheel);
    sh.spotlight.hover = None;
    true
}

/// Keep the selected row inside the visible window.
fn reveal(sh: &mut Shell) {
    let top = sh.spotlight.sel as f32 * ROW_H;
    let win = MAX_VISIBLE as f32 * ROW_H;
    let s = sh.spotlight.scroll.target;
    if top < s {
        sh.spotlight.scroll.glide(top);
    } else if top + ROW_H > s + win {
        sh.spotlight.scroll.glide(top + ROW_H - win);
    }
}

pub fn key(sh: &mut Shell, key: Option<Key>, text: Option<&str>) -> (bool, Vec<Action>) {
    let n = rows(sh).len();
    match key {
        Some(Key::Escape) => {
            if !sh.spotlight.query.is_empty() {
                sh.spotlight.query.clear();
                sh.spotlight.sel = 0;
                sh.spotlight.scroll.reset();
            } else if sh.spotlight.filter.is_some() {
                sh.spotlight.set_filter(None);
            } else {
                sh.spotlight.toggle();
            }
        }
        Some(Key::Backspace) => {
            if sh.spotlight.query.is_empty() {
                sh.spotlight.set_filter(None);
            } else {
                sh.spotlight.query.pop();
                sh.spotlight.sel = 0;
                sh.spotlight.scroll.reset();
            }
        }
        Some(Key::Cmd(c @ '1'..='4')) => {
            let f = Filter::ALL[(c as u8 - b'1') as usize];
            let f = if sh.spotlight.filter == Some(f) { None } else { Some(f) };
            sh.spotlight.set_filter(f);
        }
        Some(Key::Down) | Some(Key::Tab) if n > 0 => {
            sh.spotlight.sel = (sh.spotlight.sel + 1).min(n - 1);
            reveal(sh);
        }
        Some(Key::Up) => {
            sh.spotlight.sel = sh.spotlight.sel.saturating_sub(1);
            reveal(sh);
        }
        Some(Key::Enter) => {
            let i = sh.spotlight.sel;
            return (true, activate(sh, i));
        }
        _ => {}
    }
    if let Some(t) = text {
        if !t.chars().any(|c| c.is_control()) {
            sh.spotlight.query.push_str(t);
            sh.spotlight.sel = 0;
            sh.spotlight.scroll.reset();
        }
    }
    (true, vec![Action::Redraw])
}

/// A small in-memory index of the user's files (name, path, mtime), built in a
/// background thread when Spotlight opens and dropped two minutes after it closed.
pub mod files {
    use super::*;

    struct Entry {
        lower: String,
        path: PathBuf,
        dir: bool,
        mtime: u64,
    }

    #[derive(Default)]
    struct Index {
        entries: Vec<Entry>,
        built: Option<Instant>,
        building: bool,
    }

    const MAX_ENTRIES: usize = 60_000;
    const MAX_DEPTH: usize = 7;
    const SKIP: &[&str] = &[
        "node_modules",
        "target",
        "__pycache__",
        "venv",
        "site-packages",
        "Trash",
        "snap",
        "go",
        "build",
        "dist-newstyle",
        "vendor",
    ];

    fn index() -> &'static Arc<Mutex<Index>> {
        static I: std::sync::OnceLock<Arc<Mutex<Index>>> = std::sync::OnceLock::new();
        I.get_or_init(|| Arc::new(Mutex::new(Index::default())))
    }

    pub fn building() -> bool {
        index().lock().map(|i| i.building).unwrap_or(false)
    }

    pub fn drop_index() {
        if let Ok(mut i) = index().lock() {
            if !i.building {
                i.entries = Vec::new();
                i.built = None;
            }
        }
    }

    /// Start (re)building when the index is missing or older than a minute.
    pub fn warm() {
        let idx = index().clone();
        {
            let Ok(mut i) = idx.lock() else { return };
            if i.building || i.built.is_some_and(|b| b.elapsed() < Duration::from_secs(60)) {
                return;
            }
            i.building = true;
        }
        let spawned = std::thread::Builder::new().name("aqua-file-index".into()).spawn(move || {
            let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
            let mut out = Vec::new();
            walk(&home, 0, &mut out);
            out.shrink_to_fit();
            if let Ok(mut i) = idx.lock() {
                i.entries = out;
                i.built = Some(Instant::now());
                i.building = false;
            }
        });
        if spawned.is_err() {
            if let Ok(mut i) = index().lock() {
                i.building = false;
            }
        }
    }

    fn walk(d: &std::path::Path, depth: usize, out: &mut Vec<Entry>) {
        if depth > MAX_DEPTH || out.len() >= MAX_ENTRIES {
            return;
        }
        let Ok(rd) = std::fs::read_dir(d) else { return };
        let mut dirs = vec![];
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            let dir = ft.is_dir();
            if dir && SKIP.contains(&name.as_str()) {
                continue;
            }
            let mtime = e
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            out.push(Entry { lower: name.to_lowercase(), path: e.path(), dir, mtime });
            if dir {
                dirs.push(e.path());
            }
            if out.len() >= MAX_ENTRIES {
                return;
            }
        }
        for p in dirs {
            walk(&p, depth + 1, out);
        }
    }

    /// Best matches for `q` (empty query: recently modified documents).
    pub fn search(q: &str, limit: usize) -> Vec<(PathBuf, bool)> {
        let Ok(i) = index().lock() else { return vec![] };
        if q.is_empty() {
            let mut v: Vec<&Entry> = i.entries.iter().filter(|e| !e.dir).collect();
            v.sort_by_key(|e| std::cmp::Reverse(e.mtime));
            return v.into_iter().take(limit).map(|e| (e.path.clone(), e.dir)).collect();
        }
        let mut v: Vec<(i32, usize, &Entry)> = i
            .entries
            .iter()
            .filter_map(|e| {
                let s = if e.lower.starts_with(q) {
                    0
                } else if e.lower.split(|c: char| !c.is_alphanumeric()).any(|w| w.starts_with(q)) {
                    1
                } else if e.lower.contains(q) {
                    2
                } else {
                    return None;
                };
                Some((s, e.path.components().count(), e))
            })
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(b.2.mtime.cmp(&a.2.mtime)));
        v.into_iter().take(limit).map(|(_, _, e)| (e.path.clone(), e.dir)).collect()
    }

    /// Opened from Spotlight: rank it as recent.
    pub fn touch(p: &std::path::Path) {
        if let Ok(mut i) = index().lock() {
            let now =
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            if let Some(e) = i.entries.iter_mut().find(|e| e.path == p) {
                e.mtime = now;
            }
        }
    }
}
