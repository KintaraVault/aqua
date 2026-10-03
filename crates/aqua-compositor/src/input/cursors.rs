//! Pointer shapes drawn with tiny-skia.
//!
//! Every cursor lives in a 32×32 logical box; `hotspot` is in the same units.
use aqua_gfx::tiny_skia::{FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Stroke, Transform as T};
use smithay::input::pointer::CursorIcon;

pub const BOX: f32 = 32.0;

/// Which drawing a CSS cursor name maps to.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum Shape {
    Arrow,
    Pointer,
    Text,
    VText,
    Crosshair,
    Ew,
    Ns,
    Nwse,
    Nesw,
    Col,
    Row,
    Move,
    Grab,
    Grabbing,
    NotAllowed,
    Wait,
    Progress,
    Help,
    Copy,
    ZoomIn,
    ZoomOut,
}

pub fn shape(icon: CursorIcon) -> Shape {
    use CursorIcon as C;
    match icon {
        C::Pointer => Shape::Pointer,
        C::Text => Shape::Text,
        C::VerticalText => Shape::VText,
        C::Crosshair | C::Cell => Shape::Crosshair,
        C::EResize | C::WResize | C::EwResize => Shape::Ew,
        C::NResize | C::SResize | C::NsResize => Shape::Ns,
        C::NwResize | C::SeResize | C::NwseResize => Shape::Nwse,
        C::NeResize | C::SwResize | C::NeswResize => Shape::Nesw,
        C::ColResize => Shape::Col,
        C::RowResize => Shape::Row,
        C::Move | C::AllScroll => Shape::Move,
        C::Grab => Shape::Grab,
        C::Grabbing => Shape::Grabbing,
        C::NotAllowed | C::NoDrop => Shape::NotAllowed,
        C::Wait => Shape::Wait,
        C::Progress => Shape::Progress,
        C::Help => Shape::Help,
        C::Copy | C::Alias => Shape::Copy,
        C::ZoomIn => Shape::ZoomIn,
        C::ZoomOut => Shape::ZoomOut,
        _ => Shape::Arrow,
    }
}

pub fn hotspot(s: Shape) -> (f32, f32) {
    match s {
        Shape::Arrow | Shape::NotAllowed | Shape::Progress | Shape::Help | Shape::Copy => (4.0, 4.0),
        Shape::Pointer => (12.5, 5.0),
        Shape::ZoomIn | Shape::ZoomOut => (13.0, 13.0),
        _ => (16.0, 16.0),
    }
}

fn poly(pts: &[(f32, f32)]) -> Path {
    let mut pb = PathBuilder::new();
    for (i, &(x, y)) in pts.iter().enumerate() {
        if i == 0 {
            pb.move_to(x, y)
        } else {
            pb.line_to(x, y)
        }
    }
    pb.close();
    pb.finish().unwrap()
}

fn rrect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Path {
    let mut pb = PathBuilder::new();
    let r = r.min(w / 2.0).min(h / 2.0);
    let k = 0.5523 * r;
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish().unwrap()
}

fn circle(cx: f32, cy: f32, r: f32) -> Path {
    PathBuilder::from_circle(cx, cy, r).unwrap()
}

/// The "?" of the Help badge as a filled outline (hook + stem), centred at (22.5, 22.5).
fn question_hook() -> Path {
    let mut pb = PathBuilder::new();
    pb.move_to(20.4, 21.0);
    pb.cubic_to(20.4, 19.4, 21.4, 18.6, 22.6, 18.6);
    pb.cubic_to(23.9, 18.6, 24.8, 19.5, 24.8, 20.6);
    pb.cubic_to(24.8, 21.6, 24.2, 22.1, 23.5, 22.6);
    pb.cubic_to(23.1, 22.9, 23.0, 23.2, 23.0, 23.8);
    let p = pb.finish().unwrap();
    let st = Stroke { width: 1.3, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
    let r = p.stroke(&st, 4.0);
    r.unwrap_or(p)
}

fn arrow_path(dx: f32, dy: f32, k: f32) -> Path {
    let pts = [(0.0, 0.0), (0.0, 16.6), (3.9, 12.9), (6.6, 19.2), (9.3, 18.0), (6.7, 11.9), (12.0, 11.9)];
    let v: Vec<(f32, f32)> = pts.iter().map(|(x, y)| (dx + x * k, dy + y * k)).collect();
    poly(&v)
}

/// Horizontal double-headed arrow centred at (16,16).
fn double_arrow(len: f32, bar: bool) -> Vec<Path> {
    let h = len / 2.0;
    let (c, head, hw, sw) = (16.0, 5.5, 5.0, 1.2);
    let mut v = vec![poly(&[
        (c - h, c),
        (c - h + head, c - hw),
        (c - h + head, c - sw),
        (c + h - head, c - sw),
        (c + h - head, c - hw),
        (c + h, c),
        (c + h - head, c + hw),
        (c + h - head, c + sw),
        (c - h + head, c + sw),
        (c - h + head, c + hw),
    ])];
    if bar {
        v.push(rrect(c - 1.3, c - 8.0, 2.6, 16.0, 0.6));
    }
    v
}

struct Layer {
    paths: Vec<Path>,
    fill: [u8; 4],
    stroke: [u8; 4],
    width: f32,
    ts: T,
}

fn layer(paths: Vec<Path>, fill: [u8; 4], stroke: [u8; 4], width: f32) -> Layer {
    Layer { paths, fill, stroke, width, ts: T::identity() }
}

/// Black shape with white outline (the default look).
fn dark(paths: Vec<Path>) -> Layer {
    layer(paths, [0, 0, 0, 255], [255, 255, 255, 255], 2.6)
}

/// White shape with black outline (hands, magnifier).
fn light(paths: Vec<Path>) -> Layer {
    layer(paths, [255, 255, 255, 255], [0, 0, 0, 255], 1.3)
}

fn paint(c: [u8; 4]) -> Paint<'static> {
    let mut p = Paint { anti_alias: true, ..Default::default() };
    p.set_color_rgba8(c[0], c[1], c[2], c[3]);
    p
}

fn hand(fingers: [f32; 4], thumb: bool) -> Vec<Path> {
    let mut v = vec![];
    let xs = [9.6, 13.1, 16.6, 20.1];
    for (i, &x) in xs.iter().enumerate() {
        let up = fingers[i];
        let top = 14.0 - up;
        v.push(rrect(x, top, 3.4, 12.0 + up, 1.7));
    }
    v.push(rrect(9.4, 13.0, 14.2, 12.6, 4.0));
    if thumb {
        v.push(poly(&[(10.5, 22.0), (5.2, 16.8), (5.6, 14.9), (7.6, 15.0), (11.5, 18.0)]));
    }
    v
}

fn beachball(cx: f32, cy: f32, r: f32) -> Vec<(Path, [u8; 4])> {
    let cols: [[u8; 4]; 6] = [
        [252, 74, 74, 255],
        [255, 166, 0, 255],
        [255, 222, 33, 255],
        [77, 205, 82, 255],
        [41, 146, 255, 255],
        [176, 82, 222, 255],
    ];
    let mut out = vec![];
    for (i, c) in cols.iter().enumerate() {
        let a0 = (i as f32) * std::f32::consts::TAU / 6.0 - 1.2;
        let a1 = a0 + std::f32::consts::TAU / 6.0;
        let mut pb = PathBuilder::new();
        pb.move_to(cx, cy);
        let steps = 8;
        for s in 0..=steps {
            let a = a0 + (a1 - a0) * s as f32 / steps as f32;
            let rr = r;
            pb.line_to(cx + rr * a.cos(), cy + rr * a.sin());
        }
        pb.close();
        out.push((pb.finish().unwrap(), *c));
    }
    out
}

/// Render `s` into a pixmap of `BOX*scale` pixels.
pub fn draw(s: Shape, scale: f32) -> Pixmap {
    let px = (BOX * scale).ceil().max(1.0) as u32;
    let mut pm = Pixmap::new(px, px).unwrap();
    let base = T::from_scale(scale, scale);
    let mut layers: Vec<Layer> = vec![];
    let mut balls: Vec<(f32, f32, f32)> = vec![];
    let badge_arrow = || dark(vec![arrow_path(4.0, 4.0, 1.0)]);
    match s {
        Shape::Arrow => layers.push(badge_arrow()),
        Shape::Pointer => layers.push(light(hand([7.5, 1.0, 0.5, 0.0], true))),
        Shape::Grab => layers.push(light(hand([5.0, 6.0, 5.0, 3.0], true))),
        Shape::Grabbing => layers.push(light(hand([0.0, 0.0, 0.0, 0.0], false))),
        Shape::Text | Shape::VText => {
            let mut l = dark(vec![
                rrect(15.2, 8.0, 1.6, 16.0, 0.3),
                rrect(11.6, 7.0, 3.8, 1.6, 0.6),
                rrect(16.6, 7.0, 3.8, 1.6, 0.6),
                rrect(11.6, 23.4, 3.8, 1.6, 0.6),
                rrect(16.6, 23.4, 3.8, 1.6, 0.6),
                rrect(13.2, 15.2, 5.6, 1.6, 0.3),
            ]);
            l.width = 2.0;
            if s == Shape::VText {
                l.ts = T::from_rotate_at(90.0, 16.0, 16.0);
            }
            layers.push(l);
        }
        Shape::Crosshair => {
            let mut l = dark(vec![rrect(15.3, 5.0, 1.4, 22.0, 0.2), rrect(5.0, 15.3, 22.0, 1.4, 0.2)]);
            l.width = 2.0;
            layers.push(l);
        }
        Shape::Ew | Shape::Ns | Shape::Nwse | Shape::Nesw | Shape::Col | Shape::Row => {
            let bar = matches!(s, Shape::Col | Shape::Row);
            let mut l = dark(double_arrow(if bar { 24.0 } else { 22.0 }, bar));
            let deg = match s {
                Shape::Ns | Shape::Row => 90.0,
                Shape::Nwse => 45.0,
                Shape::Nesw => -45.0,
                _ => 0.0,
            };
            l.ts = T::from_rotate_at(deg, 16.0, 16.0);
            layers.push(l);
        }
        Shape::Move => {
            let mut v = double_arrow(24.0, false);
            let r = T::from_rotate_at(90.0, 16.0, 16.0);
            let extra: Vec<Path> = double_arrow(24.0, false).into_iter().filter_map(|p| p.transform(r)).collect();
            v.extend(extra);
            layers.push(dark(v));
        }
        Shape::NotAllowed => {
            layers.push(badge_arrow());
            let mut ring = PathBuilder::new();
            ring.push_circle(22.0, 22.0, 6.2);
            ring.push_circle(22.0, 22.0, 4.4);
            let mut l = layer(
                vec![ring.finish().unwrap(), poly(&[(18.4, 19.6), (19.6, 18.4), (25.6, 24.4), (24.4, 25.6)])],
                [0, 0, 0, 255],
                [255, 255, 255, 255],
                2.0,
            );
            l.fill = [20, 20, 20, 255];
            layers.push(l);
        }
        Shape::Help | Shape::Copy => {
            layers.push(badge_arrow());
            let col = if s == Shape::Copy { [52, 199, 89, 255] } else { [0, 122, 255, 255] };
            layers.push(layer(vec![circle(22.5, 22.5, 5.6)], col, [255, 255, 255, 255], 2.0));
            let glyph = if s == Shape::Copy {
                vec![rrect(21.7, 19.3, 1.6, 6.4, 0.3), rrect(19.3, 21.7, 6.4, 1.6, 0.3)]
            } else {
                vec![circle(22.5, 25.6, 0.85), question_hook()]
            };
            layers.push(layer(glyph, [255, 255, 255, 255], [0, 0, 0, 0], 0.0));
        }
        Shape::ZoomIn | Shape::ZoomOut => {
            let mut ring = PathBuilder::new();
            ring.push_circle(13.0, 13.0, 7.5);
            ring.push_circle(13.0, 13.0, 5.8);
            let handle = poly(&[(18.0, 19.6), (19.6, 18.0), (26.5, 24.9), (24.9, 26.5)]);
            layers.push(layer(vec![circle(13.0, 13.0, 6.0)], [255, 255, 255, 230], [0, 0, 0, 0], 0.0));
            layers.push(dark(vec![ring.finish().unwrap(), handle]));
            let mut g = vec![rrect(9.6, 12.3, 6.8, 1.4, 0.3)];
            if s == Shape::ZoomIn {
                g.push(rrect(12.3, 9.6, 1.4, 6.8, 0.3));
            }
            layers.push(layer(g, [0, 0, 0, 255], [0, 0, 0, 0], 0.0));
        }
        Shape::Wait => balls.push((16.0, 16.0, 8.5)),
        Shape::Progress => {
            layers.push(badge_arrow());
            balls.push((22.0, 22.0, 5.5));
        }
    }

    let mut sh = Pixmap::new(px, px).unwrap();
    let sp = paint([0, 0, 0, 70]);
    for l in &layers {
        let ts = base.pre_concat(l.ts).pre_translate(0.0, 1.0);
        for p in &l.paths {
            if l.width > 0.0 {
                sh.stroke_path(
                    p,
                    &sp,
                    &Stroke { width: l.width + 0.8, line_join: LineJoin::Round, ..Default::default() },
                    ts,
                    None,
                );
            }
            sh.fill_path(p, &sp, FillRule::EvenOdd, ts, None);
        }
    }
    for &(cx, cy, r) in &balls {
        sh.fill_path(&circle(cx, cy + 1.0, r + 1.5), &sp, FillRule::Winding, base, None);
    }
    aqua_gfx::blur::blur(&mut sh, 1.6 * scale);
    pm.draw_pixmap(0, 0, sh.as_ref(), &Default::default(), T::identity(), None);

    for l in &layers {
        let ts = base.pre_concat(l.ts);
        if l.width > 0.0 && l.stroke[3] > 0 {
            let st =
                Stroke { width: l.width, line_join: LineJoin::Round, line_cap: LineCap::Round, ..Default::default() };
            let p = paint(l.stroke);
            for path in &l.paths {
                pm.stroke_path(path, &p, &st, ts, None);
            }
        }
        let f = paint(l.fill);
        for path in &l.paths {
            pm.fill_path(path, &f, FillRule::EvenOdd, ts, None);
        }
    }
    for &(cx, cy, r) in &balls {
        pm.fill_path(&circle(cx, cy, r + 1.2), &paint([255, 255, 255, 255]), FillRule::Winding, base, None);
        for (p, c) in beachball(cx, cy, r) {
            pm.fill_path(&p, &paint(c), FillRule::Winding, base, None);
        }
        pm.fill_path(
            &circle(cx - r * 0.25, cy - r * 0.35, r * 0.45),
            &paint([255, 255, 255, 70]),
            FillRule::Winding,
            base,
            None,
        );
    }
    pm
}

/// Resize-edge cursor for a window border drag.
pub fn for_edges(e: crate::wm::grabs::resize_grab::ResizeEdge) -> CursorIcon {
    use crate::wm::grabs::resize_grab::ResizeEdge as E;
    if e == E::TOP_LEFT || e == E::BOTTOM_RIGHT {
        CursorIcon::NwseResize
    } else if e == E::TOP_RIGHT || e == E::BOTTOM_LEFT {
        CursorIcon::NeswResize
    } else if e.intersects(E::LEFT | E::RIGHT) {
        CursorIcon::EwResize
    } else {
        CursorIcon::NsResize
    }
}
