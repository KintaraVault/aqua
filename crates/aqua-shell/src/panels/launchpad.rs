//! "Applications" panel: a glass sheet with
//! the title (typing filters it), a "…" options button, a segmented category control
//! (All + the categories that have apps) and a smoothly scrolling icon grid with
//! section headers.
use crate::scroll::Smooth;
use crate::{hash_of, style, Action, Key, Layer, LayerId, Shell};
use aqua_apps::{App, Section};
use aqua_gfx::{rgba, symbols, Rect, Weight};
use aqua_icons::IconRequest;

#[derive(Default)]
pub struct Launchpad {
    pub open: bool,
    pub t: f32,
    pub query: String,
    /// Selected category (None = All).
    pub tab: Option<Section>,
    pub hover: Option<usize>,
    pub hover_tab: Option<usize>,
    pub scroll: Smooth,
    /// Keyboard selection (cell index).
    pub sel: Option<usize>,
    /// Animated x / width of the segmented control's selection pill.
    pub pill: (f32, f32),
    pub pill_target: (f32, f32),
    /// "…" options popover.
    pub menu_open: bool,
    pub hover_menu: Option<usize>,
    pub hover_more: bool,
    /// Options: group "All" by category, larger icons.
    pub ungrouped: bool,
    pub large: bool,
}

impl Launchpad {
    pub fn visible(&self) -> bool {
        self.open || self.t > 0.001
    }
    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.query.clear();
            self.scroll.reset();
            self.hover = None;
            self.sel = None;
            self.menu_open = false;
        }
    }
    pub fn animate(&mut self, dt: f32) -> bool {
        let mut anim = self.scroll.animate(dt);
        let k = 1.0 - (-18.0 * dt).exp();
        let (px, pw) = self.pill;
        let (tx, tw) = self.pill_target;
        if (px - tx).abs() > 0.2 || (pw - tw).abs() > 0.2 {
            self.pill = if pw <= 0.0 { (tx, tw) } else { (px + (tx - px) * k, pw + (tw - pw) * k) };
            anim = true;
        } else {
            self.pill = (tx, tw);
        }
        let target = if self.open { 1.0 } else { 0.0 };
        if (self.t - target).abs() < 0.001 {
            self.t = target;
            return anim;
        }
        let speed = 7.5;
        self.t += (target - self.t) * (1.0 - (-speed * dt).exp());
        if (self.t - target).abs() < 0.01 {
            self.t = target;
        }
        true
    }
}

pub fn panel_rect(sh: &Shell) -> Rect {
    let w = (sh.w * 0.575).clamp(640.0, 1240.0).min(sh.w - 40.0);
    let h = (sh.h * 0.66).clamp(420.0, 920.0).min(sh.h - 130.0);
    Rect::new((sh.w - w) / 2.0, (sh.h - h) / 2.0 - sh.h * 0.03, w, h)
}

struct Metrics {
    pad: f32,
    tabs_y: f32,
    tab_h: f32,
    grid_y: f32,
    cols: usize,
    col_w: f32,
    row_h: f32,
    icon: f32,
}

fn metrics(sh: &Shell, r: Rect) -> Metrics {
    let pad = 24.0;
    let cols = if sh.launchpad.large { 6 } else { 7 };
    let col_w = (r.w - 2.0 * pad) / cols as f32;
    let icon = (col_w * if sh.launchpad.large { 0.56 } else { 0.50 }).min(96.0);
    Metrics { pad, tabs_y: 76.0, tab_h: 36.0, grid_y: 126.0, cols, col_w, row_h: icon + 50.0, icon }
}

/// Categories offered in the segmented control (those that have apps), in order.
fn categories(sh: &Shell) -> Vec<Section> {
    Section::ALL.iter().copied().filter(|s| sh.apps.iter().any(|a| a.section() == *s)).collect()
}

/// Segments: (category or None for "All", rect relative to the panel).
fn segments(sh: &Shell, r: Rect) -> Vec<(Option<Section>, Rect)> {
    let m = metrics(sh, r);
    let cats = categories(sh);
    let avail = r.w - 2.0 * m.pad;
    let max = ((avail / 104.0).floor() as usize).max(2);
    let mut v: Vec<Option<Section>> = vec![None];
    v.extend(cats.iter().copied().map(Some).take(max - 1));
    if let Some(t) = sh.launchpad.tab {
        if !v.contains(&Some(t)) {
            if let Some(last) = v.last_mut() {
                *last = Some(t);
            }
        }
    }
    let n = v.len() as f32;
    let sw = (avail - 8.0) / n;
    v.into_iter()
        .enumerate()
        .map(|(i, s)| (s, Rect::new(m.pad + 4.0 + i as f32 * sw, m.tabs_y + 4.0, sw, m.tab_h - 8.0)))
        .collect()
}

/// Sections shown: (header, apps).
fn sections(sh: &Shell) -> Vec<(Option<String>, Vec<App>)> {
    let q = sh.launchpad.query.to_lowercase();
    let matches = |a: &App| {
        q.is_empty()
            || a.name.to_lowercase().contains(&q)
            || a.keywords.iter().any(|k| k.to_lowercase().starts_with(&q))
    };
    let apps: Vec<App> = sh.apps.iter().filter(|a| matches(a)).cloned().collect();
    if !q.is_empty() {
        return vec![(None, apps)];
    }
    if let Some(sec) = sh.launchpad.tab {
        return vec![(None, apps.into_iter().filter(|a| a.section() == sec).collect())];
    }
    if sh.launchpad.ungrouped {
        return vec![(None, apps)];
    }
    let mut out = vec![];
    for sec in Section::ALL {
        let v: Vec<App> = apps.iter().filter(|a| a.section() == sec).cloned().collect();
        if !v.is_empty() {
            out.push((Some(sec.title().to_string()), v));
        }
    }
    out
}

/// Grid cells (panel-relative, before scroll) for every app in order, the section
/// headers and the content height.
fn cells(sh: &Shell, r: Rect) -> (Vec<(App, Rect)>, Vec<(String, f32)>, f32) {
    let m = metrics(sh, r);
    let mut y = m.grid_y + 6.0;
    let mut out = vec![];
    let mut headers = vec![];
    for (i, (hdr, apps)) in sections(sh).into_iter().enumerate() {
        if i > 0 {
            y += 14.0;
        }
        if let Some(hh) = hdr {
            headers.push((hh, y));
            y += 34.0;
        }
        for (k, a) in apps.into_iter().enumerate() {
            let (cx, cy) = (k % m.cols, k / m.cols);
            out.push((a, Rect::new(m.pad + cx as f32 * m.col_w, y + cy as f32 * m.row_h, m.col_w, m.row_h)));
        }
        if let Some((_, last)) = out.last() {
            y = last.bottom() + 2.0;
        }
    }
    (out, headers, y)
}

/// Options popover rows: (label, checked).
fn menu_rows(sh: &Shell) -> Vec<(String, bool)> {
    let mut v = vec![
        ("Group by Category".to_string(), !sh.launchpad.ungrouped),
        ("Larger Icons".to_string(), sh.launchpad.large),
    ];
    for c in categories(sh) {
        v.push((c.title().to_string(), sh.launchpad.tab == Some(c)));
    }
    v
}

const MENU_ROW: f32 = 26.0;

fn more_rect(r: Rect) -> Rect {
    Rect::new(r.w - 24.0 - 34.0, 22.0, 34.0, 30.0)
}

fn menu_rect(sh: &Shell, r: Rect) -> Rect {
    let n = menu_rows(sh).len() as f32;
    let mr = more_rect(r);
    Rect::new(mr.right() - 230.0, mr.bottom() + 6.0, 230.0, n * MENU_ROW + 12.0 + 9.0)
}

fn menu_row_at(sh: &Shell, r: Rect, lx: f32, ly: f32) -> Option<usize> {
    let mr = menu_rect(sh, r);
    if !mr.contains(lx, ly) {
        return None;
    }
    let mut y = ly - mr.y - 6.0;
    if y >= 2.0 * MENU_ROW {
        y -= 9.0;
        if y < 2.0 * MENU_ROW {
            return None;
        }
    }
    let i = (y / MENU_ROW).floor();
    (i >= 0.0 && (i as usize) < menu_rows(sh).len()).then_some(i as usize)
}

pub fn layer(sh: &mut Shell) -> Option<Layer> {
    if !sh.launchpad.visible() {
        return None;
    }
    let r = panel_rect(sh);
    let m = metrics(sh, r);
    let dark = sh.style.is_dark_glass(r) || sh.style.dark;
    let (cells, headers, content_h) = cells(sh, r);
    sh.launchpad.scroll.set_max(content_h - r.h + 24.0);
    let segs = segments(sh, r);
    if let Some((_, sr)) = segs.iter().find(|(s, _)| *s == sh.launchpad.tab) {
        sh.launchpad.pill_target = (sr.x, sr.w);
        if sh.launchpad.pill.1 <= 0.0 {
            sh.launchpad.pill = sh.launchpad.pill_target;
        }
    }
    let lp = &sh.launchpad;
    let key = hash_of(&(
        (lp.query.clone(), lp.tab, lp.hover, lp.hover_tab, (lp.scroll.pos * 2.0) as i32, lp.sel),
        (
            (lp.pill.0 * 2.0) as i32,
            (lp.pill.1 * 2.0) as i32,
            lp.menu_open,
            lp.hover_menu,
            lp.hover_more,
            lp.ungrouped,
            lp.large,
        ),
        (cells.len(), r.w as i32, r.h as i32, dark, sh.icons_serial()),
    ));
    let scroll = lp.scroll.pos;
    let max_scroll = lp.scroll.max;
    let pill = lp.pill;
    let (pm, serial) = sh.cached(LayerId::Launchpad, key, r.w, r.h, |c, sh| {
        let f = sh.fonts.clone();
        let fg = style::text_primary(dark);
        let fg2 = style::text_secondary(dark);
        let d = m.icon * 1.24;
        let ipx = (d * sh.scale).round() as u32;
        for (i, (app, cell)) in cells.iter().enumerate() {
            let cell = cell.translate(0.0, -scroll);
            if cell.bottom() < m.grid_y - 4.0 || cell.y > r.h {
                continue;
            }
            let req = IconRequest { id: app.id.clone(), name: app.name.clone(), icon: app.icon.clone() };
            let icon = sh.icons.get(&req, ipx);
            let ir = Rect::new(cell.cx() - d / 2.0, cell.y + 2.0 + (m.icon - d) / 2.0, d, d);
            if sh.launchpad.hover == Some(i) || sh.launchpad.sel == Some(i) {
                let a = if sh.launchpad.sel == Some(i) { 0.20 } else { 0.10 };
                c.fill_rrect(
                    Rect::new(cell.x + 6.0, cell.y - 6.0, cell.w - 12.0, cell.h - 2.0),
                    18.0,
                    rgba(255, 255, 255, if dark { a } else { a * 2.0 }),
                );
            }
            c.draw_pixmap(&icon, ir, 1.0);
            c.text_in(
                &f,
                Rect::new(cell.x + 2.0, cell.y + m.icon + 10.0, cell.w - 4.0, 26.0),
                0.5,
                16.0,
                Weight::Regular,
                fg,
                &app.name,
            );
        }
        for (h, y) in &headers {
            let y = y - scroll;
            if y > m.grid_y - 30.0 && y < r.h {
                c.text(&f, m.pad + 6.0, y + 20.0, 17.0, Weight::Semibold, fg2, h);
            }
        }
        if cells.is_empty() {
            c.text_in(&f, Rect::new(0.0, m.grid_y + 40.0, r.w, 40.0), 0.5, 18.0, Weight::Regular, fg2, "No Results");
        }
        {
            use aqua_gfx::tiny_skia as sk;
            let sc = c.scale;
            let fade = |pm: &mut sk::Pixmap, y0: f32, y1: f32| {
                let stops = vec![
                    sk::GradientStop::new(0.0, sk::Color::from_rgba8(0, 0, 0, 255)),
                    sk::GradientStop::new(1.0, sk::Color::from_rgba8(0, 0, 0, 0)),
                ];
                if let Some(sh) = sk::LinearGradient::new(
                    sk::Point::from_xy(0.0, y0 * sc),
                    sk::Point::from_xy(0.0, y1 * sc),
                    stops,
                    sk::SpreadMode::Pad,
                    sk::Transform::identity(),
                ) {
                    let p = sk::Paint { shader: sh, blend_mode: sk::BlendMode::DestinationOut, ..Default::default() };
                    let (a, b) = (y0.min(y1), y0.max(y1));
                    if let Some(rr) = sk::Rect::from_xywh(0.0, a * sc, r.w * sc, (b - a) * sc) {
                        pm.fill_rect(rr, &p, sk::Transform::identity(), None);
                    }
                }
            };
            if scroll > 0.5 {
                fade(&mut c.pm, m.grid_y - 2.0, m.grid_y + 26.0);
            }
            if scroll < max_scroll - 0.5 {
                fade(&mut c.pm, r.h, r.h - 48.0);
            }
        }
        let clear = aqua_gfx::tiny_skia::Rect::from_xywh(0.0, 0.0, r.w * c.scale, (m.grid_y - 2.0) * c.scale).unwrap();
        let p = aqua_gfx::tiny_skia::Paint { blend_mode: aqua_gfx::tiny_skia::BlendMode::Clear, ..Default::default() };
        c.pm.fill_rect(clear, &p, aqua_gfx::tiny_skia::Transform::identity(), None);
        symbols::apps_glyph(c, Rect::new(m.pad + 2.0, 24.0, 26.0, 26.0), fg2);
        let q = &sh.launchpad.query;
        let tx = m.pad + 40.0;
        if q.is_empty() {
            c.text(&f, tx, 48.0, 29.0, Weight::Bold, fg, "Applications");
        } else {
            let adv = c.text(&f, tx, 48.0, 29.0, Weight::Bold, fg, q);
            c.fill_rect(Rect::new(tx + adv + 2.0, 22.0, 1.8, 32.0), style::accent(0.95));
        }
        let mr = more_rect(r);
        if sh.launchpad.hover_more || sh.launchpad.menu_open {
            c.fill_rrect(mr, 9.0, rgba(255, 255, 255, if dark { 0.14 } else { 0.35 }));
        }
        symbols::ellipsis(c, Rect::new(mr.cx() - 11.0, mr.cy() - 7.0, 22.0, 14.0), fg);
        let track = Rect::new(m.pad, m.tabs_y, r.w - 2.0 * m.pad, m.tab_h);
        c.fill_rrect(track, 11.0, if dark { rgba(255, 255, 255, 0.09) } else { rgba(255, 255, 255, 0.28) });
        c.fill_rrect(Rect::new(pill.0, m.tabs_y + 4.0, pill.1, m.tab_h - 8.0), 8.0, rgba(255, 255, 255, 0.96));
        for (i, (s, sr)) in segs.iter().enumerate() {
            let sel = *s == sh.launchpad.tab;
            let hov = sh.launchpad.hover_tab == Some(i) && !sel;
            if hov {
                c.fill_rrect(*sr, 8.0, rgba(255, 255, 255, if dark { 0.08 } else { 0.18 }));
            }
            let label = s.map(|s| s.short()).unwrap_or("All");
            let col = if sel { rgba(0, 0, 0, 0.88) } else { fg2 };
            c.text_in(&f, *sr, 0.5, 15.0, if sel { Weight::Semibold } else { Weight::Regular }, col, label);
        }
        if sh.launchpad.menu_open {
            let mn = menu_rect(sh, r);
            c.fill_rrect(mn.translate(0.0, 3.0), 12.0, rgba(0, 0, 0, 0.18));
            c.fill_rrect(mn, 12.0, if dark { rgba(44, 44, 48, 0.97) } else { rgba(246, 246, 248, 0.97) });
            let mfg = style::text_primary(dark);
            let mut y = mn.y + 6.0;
            for (i, (l, on)) in menu_rows(sh).iter().enumerate() {
                if i == 2 {
                    c.fill_rect(Rect::new(mn.x + 10.0, y + 4.0, mn.w - 20.0, 1.0), style::separator(dark));
                    y += 9.0;
                }
                let rr = Rect::new(mn.x + 5.0, y, mn.w - 10.0, MENU_ROW);
                let hot = sh.launchpad.hover_menu == Some(i);
                if hot {
                    c.fill_rrect(rr, 6.0, style::accent(0.92));
                }
                let col = if hot { rgba(255, 255, 255, 1.0) } else { mfg };
                if *on {
                    symbols::checkmark(c, Rect::new(rr.x + 7.0, rr.y + 8.0, 11.0, 11.0), col, 1.7);
                }
                c.text_in(&f, Rect::new(rr.x + 24.0, rr.y, rr.w - 28.0, rr.h), 0.0, 14.0, Weight::Regular, col, l);
                y += MENU_ROW;
            }
        }
    });
    let t = sh.launchpad.t;
    let ease = 1.0 - (1.0 - t).powi(3);
    let glass = style::glass_panel(&sh.cfg.glass, 34.0);
    Some(Layer {
        id: LayerId::Launchpad,
        rect: r,
        glass: Some(glass),
        tiles: vec![],
        content: pm,
        serial,
        opacity: ease,
        zoom: 0.94 + 0.06 * ease,
    })
}

pub fn hover(sh: &mut Shell, x: f32, y: f32) {
    if !sh.launchpad.open {
        return;
    }
    let r = panel_rect(sh);
    let m = metrics(sh, r);
    let (lx, ly) = (x - r.x, y - r.y);
    sh.launchpad.hover_more = more_rect(r).contains(lx, ly);
    if sh.launchpad.menu_open {
        sh.launchpad.hover_menu = menu_row_at(sh, r, lx, ly);
        sh.launchpad.hover = None;
        sh.launchpad.hover_tab = None;
        if menu_rect(sh, r).contains(lx, ly) {
            return;
        }
    }
    let (cells, ..) = cells(sh, r);
    let scroll = sh.launchpad.scroll.pos;
    sh.launchpad.hover = if ly > m.grid_y && r.contains(x, y) {
        cells.iter().position(|(_, c)| c.translate(0.0, -scroll).contains(lx, ly))
    } else {
        None
    };
    sh.launchpad.hover_tab = segments(sh, r).iter().position(|(_, sr)| sr.contains(lx, ly));
}

fn launch_cell(sh: &mut Shell, r: Rect, app: &App, cell: Rect) -> Vec<Action> {
    let cmd = app.launch_command();
    let c = cell.translate(r.x, r.y - sh.launchpad.scroll.pos);
    let d = c.w.min(c.h) * 0.62;
    sh.launch_origin = Some(Rect::new(c.cx() - d / 2.0, c.y + 4.0, d, d));
    sh.launchpad.toggle();
    vec![Action::Launch(cmd)]
}

fn select_tab(sh: &mut Shell, t: Option<Section>) {
    sh.launchpad.tab = t;
    sh.launchpad.scroll.reset();
    sh.launchpad.sel = None;
}

pub fn click(sh: &mut Shell, x: f32, y: f32) -> Option<Vec<Action>> {
    if !sh.launchpad.open {
        return None;
    }
    let r = panel_rect(sh);
    let (lx, ly) = (x - r.x, y - r.y);
    if sh.launchpad.menu_open {
        if let Some(i) = menu_row_at(sh, r, lx, ly) {
            match i {
                0 => sh.launchpad.ungrouped = !sh.launchpad.ungrouped,
                1 => sh.launchpad.large = !sh.launchpad.large,
                k => {
                    let cats = categories(sh);
                    if let Some(c) = cats.get(k - 2).copied() {
                        let t = if sh.launchpad.tab == Some(c) { None } else { Some(c) };
                        select_tab(sh, t);
                    }
                }
            }
            sh.launchpad.scroll.reset();
            sh.launchpad.menu_open = false;
            return Some(vec![Action::Redraw]);
        }
        sh.launchpad.menu_open = false;
        if menu_rect(sh, r).contains(lx, ly) || more_rect(r).contains(lx, ly) {
            return Some(vec![Action::Redraw]);
        }
    }
    if !r.contains(x, y) {
        sh.launchpad.toggle();
        return Some(vec![Action::Redraw]);
    }
    if more_rect(r).contains(lx, ly) {
        sh.launchpad.menu_open = true;
        sh.launchpad.hover_menu = None;
        return Some(vec![Action::Redraw]);
    }
    if let Some((s, _)) = segments(sh, r).into_iter().find(|(_, sr)| sr.contains(lx, ly)) {
        select_tab(sh, s);
        return Some(vec![Action::Redraw]);
    }
    let m = metrics(sh, r);
    if ly > m.grid_y {
        let (cells, ..) = cells(sh, r);
        let scroll = sh.launchpad.scroll.pos;
        if let Some((app, cell)) = cells.iter().find(|(_, c)| c.translate(0.0, -scroll).contains(lx, ly)) {
            let (app, cell) = (app.clone(), *cell);
            return Some(launch_cell(sh, r, &app, cell));
        }
    }
    Some(vec![])
}

pub fn scroll(sh: &mut Shell, x: f32, y: f32, dy: f32, wheel: bool) -> bool {
    if !sh.launchpad.open || !panel_rect(sh).contains(x, y) {
        return false;
    }
    sh.launchpad.scroll.scroll(dy, wheel);
    sh.launchpad.hover = None;
    true
}

/// Move the keyboard selection to the nearest cell in a direction.
fn step(sh: &mut Shell, dx: i32, dy: i32) {
    let r = panel_rect(sh);
    let m = metrics(sh, r);
    let (cells, ..) = cells(sh, r);
    if cells.is_empty() {
        return;
    }
    let cur = match sh.launchpad.sel {
        Some(i) if i < cells.len() => i,
        _ => {
            sh.launchpad.sel = Some(0);
            return;
        }
    };
    let c0 = cells[cur].1;
    let next = if dx != 0 {
        let j = cur as i32 + dx;
        (j >= 0 && (j as usize) < cells.len()).then_some(j as usize)
    } else {
        cells
            .iter()
            .enumerate()
            .filter(|(_, (_, c))| if dy > 0 { c.y > c0.y + 1.0 } else { c.y < c0.y - 1.0 })
            .min_by(|a, b| {
                let da = (a.1 .1.y - c0.y).abs() * 4.0 + (a.1 .1.x - c0.x).abs();
                let db = (b.1 .1.y - c0.y).abs() * 4.0 + (b.1 .1.x - c0.x).abs();
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i)
    };
    if let Some(n) = next {
        sh.launchpad.sel = Some(n);
        let c = cells[n].1;
        let s = sh.launchpad.scroll.target;
        if c.y - s < m.grid_y + 4.0 {
            sh.launchpad.scroll.glide(c.y - m.grid_y - 40.0);
        } else if c.bottom() - s > r.h - 10.0 {
            sh.launchpad.scroll.glide(c.bottom() - r.h + 30.0);
        }
    }
}

pub fn key(sh: &mut Shell, key: Option<Key>, text: Option<&str>) -> (bool, Vec<Action>) {
    match key {
        Some(Key::Escape) => {
            if sh.launchpad.menu_open {
                sh.launchpad.menu_open = false;
            } else if sh.launchpad.query.is_empty() {
                sh.launchpad.toggle();
            } else {
                sh.launchpad.query.clear();
                sh.launchpad.sel = None;
            }
        }
        Some(Key::Backspace) => {
            sh.launchpad.query.pop();
            sh.launchpad.sel = None;
        }
        Some(Key::Left) => step(sh, -1, 0),
        Some(Key::Right) => step(sh, 1, 0),
        Some(Key::Up) => step(sh, 0, -1),
        Some(Key::Down) => step(sh, 0, 1),
        Some(Key::Tab) => {
            let r = panel_rect(sh);
            let segs = segments(sh, r);
            let i = segs.iter().position(|(s, _)| *s == sh.launchpad.tab).unwrap_or(0);
            let t = segs[(i + 1) % segs.len()].0;
            select_tab(sh, t);
        }
        Some(Key::Enter) => {
            let r = panel_rect(sh);
            let (cells, ..) = cells(sh, r);
            let i = sh.launchpad.sel.filter(|i| *i < cells.len()).unwrap_or(0);
            if let Some((app, cell)) = cells.get(i).cloned() {
                return (true, launch_cell(sh, r, &app, cell));
            }
        }
        _ => {}
    }
    if let Some(t) = text {
        if !t.chars().any(|c| c.is_control()) {
            sh.launchpad.query.push_str(t);
            sh.launchpad.scroll.reset();
            sh.launchpad.sel = None;
        }
    }
    (true, vec![Action::Redraw])
}
