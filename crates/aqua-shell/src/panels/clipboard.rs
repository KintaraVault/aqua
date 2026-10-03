//! Clipboard history panel (⇧⌘V): Spotlight-style glass panel with a search field,
//! a "…" options button and one row per copied item (thumbnail for images).
use crate::{hash_of, style, Action, Key, Layer, LayerId, Shell};
use aqua_gfx::{rgba, symbols, Canvas, Pixmap, Rect, Weight};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClipKind {
    Text,
    Url,
    Image,
    Files,
    Color,
}

#[derive(Clone)]
pub struct ClipItem {
    pub id: u64,
    pub kind: ClipKind,
    pub title: String,
    pub detail: String,
    pub time: (u32, u32),
    /// Image preview (premultiplied RGBA, any size).
    pub thumb: Option<Arc<Pixmap>>,
}

#[derive(Default)]
pub struct ClipboardPanel {
    pub open: bool,
    pub t: f32,
    pub query: String,
    pub sel: usize,
    pub hover: Option<usize>,
    pub hover_copy: bool,
    pub options: bool,
    /// Newest first.
    pub items: Vec<ClipItem>,
    pub serial: u64,
}

impl ClipboardPanel {
    pub fn visible(&self) -> bool {
        self.open || self.t > 0.001
    }
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.query.clear();
        self.sel = 0;
        self.options = false;
    }
    pub fn animate(&mut self, dt: f32) -> bool {
        let target = if self.open { 1.0 } else { 0.0 };
        if (self.t - target).abs() < 0.001 {
            self.t = target;
            return false;
        }
        self.t += (target - self.t) * (1.0 - (-12.0 * dt).exp());
        if (self.t - target).abs() < 0.01 {
            self.t = target;
        }
        true
    }
    pub fn push(&mut self, it: ClipItem) {
        self.items.retain(|x| x.id != it.id);
        self.items.insert(0, it);
        self.serial += 1;
    }
    pub fn retain_ids(&mut self, keep: &[u64]) {
        self.items.retain(|x| keep.contains(&x.id));
        self.serial += 1;
    }
    /// Move an item to the top (it was pasted again).
    pub fn bump(&mut self, id: u64) {
        if let Some(i) = self.items.iter().position(|x| x.id == id) {
            let mut it = self.items.remove(i);
            it.time = crate::clock::now_hm();
            self.items.insert(0, it);
            self.serial += 1;
        }
    }
    fn filtered(&self) -> Vec<usize> {
        let q = self.query.to_lowercase();
        self.items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                q.is_empty() || it.title.to_lowercase().contains(&q) || it.detail.to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect()
    }
}

const HEAD: f32 = 58.0;
const ROW: f32 = 58.0;
const MAX_ROWS: usize = 7;

fn panel_rect(sh: &Shell, n: usize) -> Rect {
    let w = (sh.w * 0.42).clamp(500.0, 640.0);
    let rows = n.clamp(1, MAX_ROWS) as f32;
    Rect::new((sh.w - w) / 2.0, sh.h * 0.16, w, HEAD + 8.0 + rows * ROW + 10.0)
}

fn fmt_time(t: (u32, u32), h24: bool) -> String {
    if h24 {
        format!("{:02}:{:02}", t.0, t.1)
    } else {
        let h = t.0 % 12;
        format!("{}:{:02} {}", if h == 0 { 12 } else { h }, t.1, if t.0 < 12 { "AM" } else { "PM" })
    }
}

fn kind_icon(c: &mut Canvas, f: &aqua_gfx::Fonts, r: Rect, it: &ClipItem, dark: bool) {
    if let Some(th) = &it.thumb {
        c.fill_rrect(r, 8.0, rgba(255, 255, 255, if dark { 0.08 } else { 0.5 }));
        let (tw, tht) = (th.width() as f32, th.height() as f32);
        let s = (r.w / tw).min(r.h / tht);
        let (dw, dh) = (tw * s, tht * s);
        c.draw_pixmap(th, Rect::new(r.x + (r.w - dw) / 2.0, r.y + (r.h - dh) / 2.0, dw, dh), 1.0);
        return;
    }
    let (bg, glyph) = match it.kind {
        ClipKind::Text => (rgba(142, 142, 147, 1.0), "Aa"),
        ClipKind::Url => (rgba(10, 132, 255, 1.0), "↗"),
        ClipKind::Files => (rgba(52, 170, 220, 1.0), ""),
        ClipKind::Color => (parse_color(&it.title).unwrap_or(rgba(255, 149, 0, 1.0)), ""),
        ClipKind::Image => (rgba(255, 149, 0, 1.0), ""),
    };
    c.fill_rrect(r, 9.0, bg);
    let w = rgba(255, 255, 255, 1.0);
    if it.kind == ClipKind::Files {
        let d = Rect::new(r.cx() - 8.0, r.cy() - 10.0, 16.0, 20.0);
        c.stroke_rrect(d, 3.0, w, 1.6);
        for k in 0..3 {
            c.fill_rect(Rect::new(d.x + 4.0, d.y + 6.0 + k as f32 * 4.0, 8.0, 1.4), w);
        }
    } else if it.kind == ClipKind::Image {
        let d = Rect::new(r.cx() - 10.0, r.cy() - 8.0, 20.0, 16.0);
        c.stroke_rrect(d, 3.0, w, 1.6);
        c.fill_circle(d.x + 6.0, d.y + 5.5, 2.0, w);
    }
    if !glyph.is_empty() {
        c.text_in(
            f,
            r,
            0.5,
            if glyph == "Aa" { 15.0 } else { 17.0 },
            Weight::Semibold,
            rgba(255, 255, 255, 1.0),
            glyph,
        );
    }
}

fn parse_color(s: &str) -> Option<aqua_gfx::Color> {
    let h = s.trim().strip_prefix('#')?;
    let v = u32::from_str_radix(h, 16).ok()?;
    match h.len() {
        6 => Some(rgba((v >> 16) as u8, (v >> 8) as u8, v as u8, 1.0)),
        3 => Some(rgba(((v >> 8) & 15) as u8 * 17, ((v >> 4) & 15) as u8 * 17, (v & 15) as u8 * 17, 1.0)),
        _ => None,
    }
}

fn dots_rect(p: Rect) -> Rect {
    Rect::new(p.right() - 48.0, 13.0 + p.y, 34.0, 32.0)
}
fn copy_rect(row: Rect) -> Rect {
    Rect::new(row.right() - 40.0, row.y + (row.h - 28.0) / 2.0, 28.0, 28.0)
}
fn options_rect(p: Rect) -> Rect {
    Rect::new(p.right() - 190.0, p.y + 50.0, 176.0, 40.0)
}

pub fn layer(sh: &mut Shell) -> Option<Layer> {
    if !sh.clipboard.visible() {
        return None;
    }
    let idx = sh.clipboard.filtered();
    let r = panel_rect(sh, idx.len());
    let dark = sh.style.dark || sh.style.is_dark_glass(r);
    let cp = &sh.clipboard;
    let sel = cp.sel.min(idx.len().saturating_sub(1));
    let first = sel.saturating_sub(MAX_ROWS - 1);
    let key = hash_of(&(
        cp.query.clone(),
        sel,
        cp.hover,
        cp.hover_copy,
        cp.options,
        cp.serial,
        idx.len(),
        r.w as i32,
        dark,
        sh.cfg.clock_24h,
    ));
    let (pm, serial) = sh.cached(LayerId::Clipboard, key, r.w, r.h, |c, sh| {
        let f = sh.fonts.clone();
        let fg = style::text_primary(dark);
        let fg2 = style::text_secondary(dark);
        let cp = &sh.clipboard;
        let sf = Rect::new(14.0, 12.0, r.w - 76.0, 34.0);
        c.fill_rrect(sf, 17.0, rgba(255, 255, 255, if dark { 0.10 } else { 0.42 }));
        symbols::search(c, Rect::new(sf.x + 12.0, sf.cy() - 7.5, 15.0, 15.0), fg2);
        if cp.query.is_empty() {
            c.text(&f, sf.x + 36.0, sf.cy() + 6.0, 16.0, Weight::Regular, fg2, "Clipboard");
            c.fill_rect(Rect::new(sf.x + 35.0, sf.cy() - 9.0, 1.5, 18.0), style::accent(0.95));
        } else {
            let tw = c.text(&f, sf.x + 36.0, sf.cy() + 6.0, 16.0, Weight::Regular, fg, &cp.query);
            c.fill_rect(Rect::new(sf.x + 37.0 + tw, sf.cy() - 9.0, 1.5, 18.0), style::accent(0.95));
        }
        let dr = dots_rect(Rect::new(0.0, 0.0, r.w, r.h));
        c.fill_circle(dr.cx(), dr.cy(), 16.0, rgba(255, 255, 255, if dark { 0.10 } else { 0.42 }));
        symbols::ellipsis(c, Rect::new(dr.cx() - 8.0, dr.cy() - 2.0, 16.0, 4.0), fg);
        let y0 = HEAD + 8.0;
        if idx.is_empty() {
            let msg = if cp.items.is_empty() { "Nothing copied yet" } else { "No Results" };
            c.text_in(&f, Rect::new(0.0, y0, r.w, ROW), 0.5, 15.0, Weight::Medium, fg2, msg);
        }
        for (slot, &ii) in idx.iter().enumerate().skip(first).take(MAX_ROWS) {
            let it = &cp.items[ii];
            let row = Rect::new(8.0, y0 + (slot - first) as f32 * ROW, r.w - 16.0, ROW - 4.0);
            let selected = slot == sel;
            if selected {
                c.fill_rrect(row, 14.0, rgba(255, 255, 255, if dark { 0.14 } else { 0.48 }));
            } else if cp.hover == Some(slot) {
                c.fill_rrect(row, 14.0, rgba(255, 255, 255, if dark { 0.07 } else { 0.24 }));
            }
            kind_icon(c, &f, Rect::new(row.x + 10.0, row.y + 8.0, 38.0, 38.0), it, dark);
            let tx = row.x + 60.0;
            let maxw = row.w - 60.0 - 56.0;
            let title = f.ellipsize(&it.title.replace(['\n', '\t'], " "), 15.0, Weight::Semibold, maxw);
            c.text(&f, tx, row.y + 24.0, 15.0, Weight::Semibold, fg, &title);
            let detail = format!("{} · Copied {}", it.detail, fmt_time(it.time, sh.cfg.clock_24h));
            c.text(&f, tx, row.y + 42.0, 12.5, Weight::Regular, fg2, &detail);
            if selected || cp.hover == Some(slot) {
                let cr = copy_rect(row);
                let hot = cp.hover == Some(slot) && cp.hover_copy;
                c.fill_circle(
                    cr.cx(),
                    cr.cy(),
                    14.0,
                    rgba(
                        255,
                        255,
                        255,
                        if hot {
                            0.5
                        } else if dark {
                            0.12
                        } else {
                            0.4
                        },
                    ),
                );
                c.stroke_rrect(Rect::new(cr.cx() - 5.0, cr.cy() - 6.0, 10.0, 11.0), 2.5, fg, 1.4);
                c.stroke_rrect(Rect::new(cr.cx() - 2.0, cr.cy() - 3.0, 10.0, 11.0), 2.5, fg, 1.4);
            }
        }
        if cp.options {
            let o = options_rect(Rect::new(0.0, 0.0, r.w, r.h));
            c.fill_rrect(o, 10.0, if dark { rgba(50, 50, 56, 0.96) } else { rgba(250, 250, 252, 0.97) });
            c.stroke_rrect(o, 10.0, style::separator(dark), 1.0);
            c.fill_rrect(Rect::new(o.x + 5.0, o.y + 5.0, o.w - 10.0, o.h - 10.0), 6.0, style::accent(0.9));
            c.text(&f, o.x + 14.0, o.cy() + 5.0, 13.5, Weight::Regular, rgba(255, 255, 255, 1.0), "Clear History");
        }
    });
    let t = sh.clipboard.t;
    let ease = 1.0 - (1.0 - t).powi(3);
    let mut g = style::glass_panel(&sh.cfg.glass, 28.0);
    g.max_luma = 0.86;
    g.tint = aqua_config::Rgba(1.0, 1.0, 1.0, if dark { 0.10 } else { 0.30 });
    Some(Layer {
        id: LayerId::Clipboard,
        rect: r.translate(0.0, (1.0 - ease) * -10.0),
        glass: Some(g),
        tiles: vec![],
        content: pm,
        serial,
        opacity: ease,
        zoom: 0.95 + 0.05 * ease,
    })
}

fn row_at(sh: &Shell, x: f32, y: f32) -> Option<(usize, bool)> {
    let idx = sh.clipboard.filtered();
    let p = panel_rect(sh, idx.len());
    if !p.contains(x, y) {
        return None;
    }
    let first = sh.clipboard.sel.min(idx.len().saturating_sub(1)).saturating_sub(MAX_ROWS - 1);
    let ly = y - p.y - HEAD - 8.0;
    if ly < 0.0 {
        return None;
    }
    let slot = first + (ly / ROW) as usize;
    if slot >= idx.len() {
        return None;
    }
    let row = Rect::new(p.x + 8.0, p.y + HEAD + 8.0 + (slot - first) as f32 * ROW, p.w - 16.0, ROW - 4.0);
    Some((slot, copy_rect(row).contains(x, y)))
}

pub fn hover(sh: &mut Shell, x: f32, y: f32) {
    if sh.clipboard.open {
        let h = row_at(sh, x, y);
        sh.clipboard.hover = h.map(|h| h.0);
        sh.clipboard.hover_copy = h.map(|h| h.1).unwrap_or(false);
    }
}

fn activate(sh: &mut Shell, slot: usize, paste: bool) -> Vec<Action> {
    let idx = sh.clipboard.filtered();
    let Some(&ii) = idx.get(slot) else { return vec![Action::Redraw] };
    let id = sh.clipboard.items[ii].id;
    sh.clipboard.toggle();
    vec![Action::ClipboardUse(id, paste), Action::Redraw]
}

pub fn click(sh: &mut Shell, x: f32, y: f32) -> Option<Vec<Action>> {
    if !sh.clipboard.open {
        return None;
    }
    let idx = sh.clipboard.filtered();
    let p = panel_rect(sh, idx.len());
    if sh.clipboard.options {
        sh.clipboard.options = false;
        if options_rect(p).contains(x, y) {
            sh.clipboard.items.clear();
            sh.clipboard.serial += 1;
            return Some(vec![Action::ClipboardClear, Action::Redraw]);
        }
        return Some(vec![Action::Redraw]);
    }
    if !p.contains(x, y) {
        sh.clipboard.toggle();
        return Some(vec![Action::Redraw]);
    }
    if dots_rect(p).contains(x, y) {
        sh.clipboard.options = true;
        return Some(vec![Action::Redraw]);
    }
    Some(match row_at(sh, x, y) {
        Some((slot, copy)) => activate(sh, slot, !copy),
        None => vec![Action::Redraw],
    })
}

pub fn key(sh: &mut Shell, key: Option<Key>, text: Option<&str>) -> (bool, Vec<Action>) {
    let n = sh.clipboard.filtered().len();
    let cp = &mut sh.clipboard;
    match key {
        Some(Key::Escape) => {
            if cp.options {
                cp.options = false;
            } else if cp.query.is_empty() {
                cp.toggle();
            } else {
                cp.query.clear();
            }
        }
        Some(Key::Backspace) => {
            cp.query.pop();
            cp.sel = 0;
        }
        Some(Key::Down) | Some(Key::Tab) if n > 0 => cp.sel = (cp.sel + 1).min(n - 1),
        Some(Key::Up) => cp.sel = cp.sel.saturating_sub(1),
        Some(Key::Enter) => {
            let s = cp.sel;
            return (true, activate(sh, s, true));
        }
        _ => {}
    }
    if let Some(t) = text {
        if !t.chars().any(|c| c.is_control()) {
            cp.query.push_str(t);
            cp.sel = 0;
        }
    }
    (true, vec![Action::Redraw])
}
