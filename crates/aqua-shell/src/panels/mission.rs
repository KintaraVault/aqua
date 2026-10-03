//! Mission Control overlay: the compositor lays the windows out as thumbnails and
//! feeds their target rects here; the shell draws the hover outline, the title pill
//! under the hovered window and the Spaces bar at the top.
use crate::{hash_of, style, Layer, LayerId, Shell};
use aqua_gfx::{rgba, Rect, Weight};
use aqua_icons::IconRequest;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Item {
    pub id: u64,
    /// Thumbnail rect (logical, output coords) at the end of the animation.
    pub rect: Rect,
    pub title: String,
    pub app_id: String,
}

#[derive(Default)]
pub struct Mission {
    /// 0 = closed, 1 = fully open (eased).
    pub progress: f32,
    pub items: Vec<Item>,
    pub hover: Option<u64>,
    /// Spaces bar state (mirrored from the compositor).
    pub desks: usize,
    pub cur: usize,
    /// 0 = collapsed name pills, 1 = expanded desktop thumbnails.
    pub expand: f32,
    pub hover_desk: Option<usize>,
    pub hover_close: bool,
    pub hover_plus: bool,
    /// Desktop under a window being dragged.
    pub drop_desk: Option<usize>,
}

impl Mission {
    pub fn visible(&self) -> bool {
        self.progress > 0.0
    }
}

const PILL_H: f32 = 30.0;

pub fn desk_name(i: usize) -> String {
    crate::trf("Desktop {n}", &[("n", &(i + 1))])
}

/// Collapsed Spaces bar: one glass capsule with every desktop's name.
pub fn spaces_rect(sh: &Shell) -> Rect {
    let w: f32 = name_widths(sh).iter().sum::<f32>() + 8.0;
    Rect::new((sh.w - w) / 2.0, sh.cfg.menubar_height + 10.0, w, 34.0)
}

fn name_widths(sh: &Shell) -> Vec<f32> {
    (0..sh.mission.desks.max(1)).map(|i| sh.fonts.measure(&desk_name(i), 13.0, Weight::Semibold) + 28.0).collect()
}

/// Name segments inside the collapsed capsule.
pub fn name_rects(sh: &Shell) -> Vec<Rect> {
    let r = spaces_rect(sh);
    let mut x = r.x + 4.0;
    name_widths(sh)
        .into_iter()
        .map(|w| {
            let o = Rect::new(x, r.y + 4.0, w, r.h - 8.0);
            x += w;
            o
        })
        .collect()
}

/// Size of one desktop thumbnail in the expanded bar.
pub fn thumb_size(sh: &Shell) -> (f32, f32) {
    let tw = (sh.w * 0.112).round();
    (tw, (tw * sh.h / sh.w).round())
}

/// Expanded Spaces bar: desktop thumbnails (logical, final position).
pub fn desk_rects(sh: &Shell) -> Vec<Rect> {
    let n = sh.mission.desks.max(1);
    let (tw, th) = thumb_size(sh);
    let gap = 30.0;
    let total = n as f32 * tw + (n as f32 - 1.0) * gap;
    let x0 = (sh.w - total) / 2.0;
    let y = sh.cfg.menubar_height + 16.0 - (1.0 - sh.mission.expand) * 14.0;
    (0..n).map(|i| Rect::new(x0 + i as f32 * (tw + gap), y, tw, th)).collect()
}

/// Bottom edge of the expanded bar (thumbnail + label).
pub fn bar_bottom(sh: &Shell) -> f32 {
    let (_, th) = thumb_size(sh);
    sh.cfg.menubar_height + 16.0 + th + 28.0
}

/// "+" (add desktop) button at the right end of the bar.
pub fn plus_rect(sh: &Shell) -> Rect {
    let (_, th) = thumb_size(sh);
    let s = 44.0;
    Rect::new(sh.w - 40.0 - s, sh.cfg.menubar_height + 16.0 + (th - s) / 2.0, s, s)
}

/// Close badge on the hovered desktop thumbnail.
pub fn close_rect(desk: Rect) -> Rect {
    Rect::new(desk.x - 9.0, desk.y - 9.0, 22.0, 22.0)
}

fn pill_rect(sh: &Shell, it: &Item) -> Rect {
    let tw = sh.fonts.measure(&it.title, 13.0, Weight::Medium).min(320.0);
    let w = tw + 24.0 + 26.0;
    Rect::new(it.rect.cx() - w / 2.0, it.rect.bottom() + 12.0, w, PILL_H)
}

pub fn layer(sh: &mut Shell) -> Option<Layer> {
    if !sh.mission.visible() {
        return None;
    }
    let full = Rect::new(0.0, 0.0, sh.w, sh.h);
    let hover = sh.mission.hover.and_then(|h| sh.mission.items.iter().find(|i| i.id == h).cloned());
    let settled = sh.mission.progress >= 0.999;
    let hov = if settled { hover.clone() } else { None };
    let dark = sh.style.dark;
    let pill = hov.as_ref().map(|it| pill_rect(sh, it));
    let key = hash_of(&(
        hov.as_ref().map(|i| (i.id, i.title.clone(), (i.rect.x as i32, i.rect.y as i32, i.rect.w as i32))),
        sh.w as i32,
        dark,
        sh.icons_serial(),
    ));
    let (pm, serial) = sh.cached(LayerId::Mission, key, full.w, full.h, |c, sh| {
        let f = sh.fonts.clone();
        if let (Some(it), Some(p)) = (&hov, pill) {
            let o = Rect::new(it.rect.x - 4.0, it.rect.y - 4.0, it.rect.w + 8.0, it.rect.h + 8.0);
            c.stroke_rrect(o, 16.0, style::accent(0.95), 3.5);
            let app = aqua_apps::match_app_id(&sh.apps, &it.app_id);
            let req = IconRequest {
                id: it.app_id.clone(),
                name: app.map(|a| a.name.clone()).unwrap_or_else(|| sh.app_display_name(&it.app_id)),
                icon: app.map(|a| a.icon.clone()).unwrap_or_else(|| it.app_id.clone()),
            };
            let icon = sh.icons.get(&req, (20.0 * sh.scale).round() as u32);
            c.draw_pixmap(&icon, Rect::new(p.x + 9.0, p.y + 5.0, 20.0, 20.0), 1.0);
            let fg = style::text_primary(dark);
            c.text_in(&f, Rect::new(p.x + 34.0, p.y, p.w - 44.0, p.h), 0.0, 13.0, Weight::Medium, fg, &it.title);
        }
    });
    let mut tiles = vec![];
    if let Some(p) = pill {
        tiles.push((p, aqua_config::GlassStyle { radius: PILL_H / 2.0, ..style::glass_menu(&sh.cfg.glass, dark) }));
    }
    Some(Layer {
        id: LayerId::Mission,
        rect: full,
        glass: None,
        tiles,
        content: pm,
        serial,
        opacity: sh.mission.progress,
        zoom: 1.0,
    })
}

/// Collapsed Spaces bar (names in a glass capsule); fades out as the bar expands.
pub fn names_layer(sh: &mut Shell) -> Option<Layer> {
    let op = sh.mission.progress * (1.0 - sh.mission.expand);
    if op <= 0.002 {
        return None;
    }
    let sp = spaces_rect(sh);
    let rects = name_rects(sh);
    let (cur, n) = (sh.mission.cur, sh.mission.desks.max(1));
    let key = hash_of(&(n, cur, sp.w as i32, sh.w as i32));
    let (pm, serial) = sh.cached(LayerId::MissionNames, key, sp.w, sp.h, |c, sh| {
        let f = sh.fonts.clone();
        for (i, r) in rects.iter().enumerate() {
            let r = Rect::new(r.x - sp.x, r.y - sp.y, r.w, r.h);
            if i == cur && n > 1 {
                c.fill_rrect(r, r.h / 2.0, rgba(255, 255, 255, 0.28));
            }
            let a = if i == cur { 0.98 } else { 0.78 };
            c.text_in(&f, r, 0.5, 13.0, Weight::Semibold, rgba(255, 255, 255, a), &desk_name(i));
        }
    });
    Some(Layer {
        id: LayerId::MissionNames,
        rect: sp,
        glass: None,
        tiles: vec![(sp, style::glass_clear(&sh.cfg.glass, 17.0))],
        content: pm,
        serial,
        opacity: op,
        zoom: 1.0,
    })
}

/// Expanded Spaces bar: labels, current/drop outlines, close badge and "+" button.
pub fn bar_layer(sh: &mut Shell) -> Option<Layer> {
    let op = sh.mission.progress * sh.mission.expand;
    if op <= 0.002 {
        return None;
    }
    let full = Rect::new(0.0, 0.0, sh.w, bar_bottom(sh) + 6.0);
    let desks = desk_rects(sh);
    let plus = plus_rect(sh);
    let m = &sh.mission;
    let (cur, hover, hclose, hplus, drop) = (m.cur, m.hover_desk, m.hover_close, m.hover_plus, m.drop_desk);
    let n = desks.len();
    let key = hash_of(&(n, cur, hover, hclose, hplus, drop, desks.first().map(|r| r.y as i32), sh.w as i32));
    let (pm, serial) = sh.cached(LayerId::MissionBar, key, full.w, full.h, |c, sh| {
        let f = sh.fonts.clone();
        for (i, r) in desks.iter().enumerate() {
            if Some(i) == drop {
                let o = Rect::new(r.x - 4.0, r.y - 4.0, r.w + 8.0, r.h + 8.0);
                c.stroke_rrect(o, 12.0, style::accent(0.95), 3.0);
            } else if i == cur {
                let o = Rect::new(r.x - 3.0, r.y - 3.0, r.w + 6.0, r.h + 6.0);
                c.stroke_rrect(o, 11.0, rgba(255, 255, 255, 0.92), 2.0);
            }
            let lr = Rect::new(r.x - 10.0, r.bottom() + 6.0, r.w + 20.0, 18.0);
            let a = if i == cur { 1.0 } else { 0.82 };
            c.text_in(&f, lr, 0.5, 12.0, Weight::Medium, rgba(255, 255, 255, a), &desk_name(i));
            if Some(i) == hover && n > 1 && drop.is_none() {
                let cr = close_rect(*r);
                c.fill_circle(cr.cx(), cr.cy() + 1.0, cr.w / 2.0 + 1.0, rgba(0, 0, 0, 0.25));
                let bg = if hclose { rgba(110, 110, 116, 1.0) } else { rgba(150, 150, 156, 0.96) };
                c.fill_circle(cr.cx(), cr.cy(), cr.w / 2.0, bg);
                cross(c, cr, rgba(255, 255, 255, 1.0));
            }
        }
        if hplus {
            c.fill_rrect(plus, plus.h / 2.0, rgba(255, 255, 255, 0.18));
        }
        let pc = Rect::new(plus.cx() - 9.0, plus.cy() - 9.0, 18.0, 18.0);
        c.fill_rrect(Rect::new(pc.cx() - 1.25, pc.y, 2.5, pc.h), 1.25, rgba(255, 255, 255, 0.95));
        c.fill_rrect(Rect::new(pc.x, pc.cy() - 1.25, pc.w, 2.5), 1.25, rgba(255, 255, 255, 0.95));
    });
    let tiles = vec![(
        plus,
        aqua_config::GlassStyle { radius: plus.h / 2.0, ..style::glass_clear(&sh.cfg.glass, plus.h / 2.0) },
    )];
    Some(Layer { id: LayerId::MissionBar, rect: full, glass: None, tiles, content: pm, serial, opacity: op, zoom: 1.0 })
}

fn cross(c: &mut aqua_gfx::Canvas, r: Rect, col: aqua_gfx::Color) {
    let mut pb = aqua_gfx::tiny_skia::PathBuilder::new();
    let k = r.w * 0.3;
    pb.move_to(r.x + k, r.y + k);
    pb.line_to(r.right() - k, r.bottom() - k);
    pb.move_to(r.right() - k, r.y + k);
    pb.line_to(r.x + k, r.bottom() - k);
    if let Some(p) = pb.finish() {
        c.stroke_path(&p, &aqua_gfx::canvas::solid(col), 1.8);
    }
}
