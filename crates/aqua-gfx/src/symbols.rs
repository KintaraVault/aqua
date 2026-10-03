//! Vector re-creations of the system symbols used by the menu bar / control centre.
//! Each function draws inside the logical square/rect given.
use crate::canvas::solid;
use crate::{shapes, Canvas, Rect};
use std::f32::consts::PI;
use tiny_skia::{Color, PathBuilder};

fn arc(pb: &mut PathBuilder, cx: f32, cy: f32, r: f32, a0: f32, a1: f32) {
    let steps = 24;
    for i in 0..=steps {
        let a = a0 + (a1 - a0) * i as f32 / steps as f32;
        let (x, y) = (cx + r * a.cos(), cy + r * a.sin());
        if i == 0 {
            pb.move_to(x, y)
        } else {
            pb.line_to(x, y)
        }
    }
}

/// Wi-Fi fan (wedge-shaped, filled like SF "wifi").
pub fn wifi(c: &mut Canvas, r: Rect, color: Color) {
    let s = r.w.min(r.h);
    let cx = r.cx();
    let cy = r.y + r.h * 0.5 + s * 0.36;
    let half = PI / 4.0;
    let (a0, a1) = (-PI / 2.0 - half, -PI / 2.0 + half);
    let th = s * 0.13;
    for rad in [s * 0.78, s * 0.52].iter() {
        let mut pb = PathBuilder::new();
        arc(&mut pb, cx, cy, *rad, a0, a1);
        if let Some(p) = pb.finish() {
            c.stroke_path(&p, &solid(color), th);
        }
    }
    let mut pb = PathBuilder::new();
    let rr = s * 0.26;
    pb.move_to(cx, cy + s * 0.02);
    pb.line_to(cx + rr * a0.cos(), cy + rr * a0.sin());
    let mut tmp = PathBuilder::new();
    arc(&mut tmp, cx, cy, rr, a0, a1);
    for i in 0..=24 {
        let a = a0 + (a1 - a0) * i as f32 / 24.0;
        pb.line_to(cx + rr * a.cos(), cy + rr * a.sin());
    }
    pb.close();
    if let Some(p) = pb.finish() {
        c.fill_path(&p, &solid(color));
    }
}

pub fn battery(c: &mut Canvas, r: Rect, level: f32, color: Color) {
    let body = Rect::new(r.x, r.cy() - r.h * 0.5, r.w * 0.9, r.h);
    c.fill_rrect(
        body,
        r.h * 0.32,
        Color::from_rgba(color.red(), color.green(), color.blue(), color.alpha() * 0.4).unwrap(),
    );
    let fill = Rect::new(body.x + 1.5, body.y + 1.5, (body.w - 3.0) * level.clamp(0.0, 1.0), body.h - 3.0);
    c.fill_rrect(fill, (r.h - 3.0) * 0.3, color);
    let cap = Rect::new(body.right() + r.w * 0.025, r.cy() - r.h * 0.18, r.w * 0.06, r.h * 0.36);
    c.fill_rrect(
        cap,
        cap.w * 0.5,
        Color::from_rgba(color.red(), color.green(), color.blue(), color.alpha() * 0.45).unwrap(),
    );
}

pub fn search(c: &mut Canvas, r: Rect, color: Color) {
    let s = r.w.min(r.h);
    let rad = s * 0.3;
    let (cx, cy) = (r.x + s * 0.42, r.y + s * 0.42);
    if let Some(p) = PathBuilder::from_circle(cx, cy, rad) {
        c.stroke_path(&p, &solid(color), s * 0.12);
    }
    let mut pb = PathBuilder::new();
    pb.move_to(cx + rad * 0.72, cy + rad * 0.72);
    pb.line_to(r.x + s * 0.9, r.y + s * 0.9);
    if let Some(p) = pb.finish() {
        c.stroke_path(&p, &solid(color), s * 0.15);
    }
}

/// Control Centre glyph: two stacked toggles.
pub fn control_center(c: &mut Canvas, r: Rect, color: Color) {
    let s = r.w.min(r.h);
    let h = s * 0.36;
    let w = s * 0.95;
    let x = r.cx() - w / 2.0;
    for (i, knob_left) in [(0, false), (1, true)] {
        let y = r.y + s * 0.08 + i as f32 * (h + s * 0.12);
        let pill = Rect::new(x, y, w, h);
        c.stroke_rrect(pill, h / 2.0, color, s * 0.09);
        let kx = if knob_left { x + h / 2.0 } else { x + w - h / 2.0 };
        c.fill_circle(kx, y + h / 2.0, h * 0.36, color);
    }
}

pub fn chevron_right(c: &mut Canvas, r: Rect, color: Color, width: f32) {
    let mut pb = PathBuilder::new();
    pb.move_to(r.x + r.w * 0.35, r.y + r.h * 0.2);
    pb.line_to(r.x + r.w * 0.65, r.cy());
    pb.line_to(r.x + r.w * 0.35, r.y + r.h * 0.8);
    if let Some(p) = pb.finish() {
        c.stroke_path(&p, &solid(color), width);
    }
}

pub fn checkmark(c: &mut Canvas, r: Rect, color: Color, width: f32) {
    let mut pb = PathBuilder::new();
    pb.move_to(r.x + r.w * 0.18, r.y + r.h * 0.55);
    pb.line_to(r.x + r.w * 0.42, r.y + r.h * 0.78);
    pb.line_to(r.x + r.w * 0.85, r.y + r.h * 0.22);
    if let Some(p) = pb.finish() {
        c.stroke_path(&p, &solid(color), width);
    }
}

pub fn ellipsis(c: &mut Canvas, r: Rect, color: Color) {
    let d = r.w / 3.0;
    for i in 0..3 {
        c.fill_circle(r.x + d * (i as f32 + 0.5), r.cy(), r.h * 0.12, color);
    }
}

pub fn bluetooth(c: &mut Canvas, r: Rect, color: Color) {
    let s = r.h;
    let x = r.cx();
    let mut pb = PathBuilder::new();
    pb.move_to(x - s * 0.22, r.y + s * 0.3);
    pb.line_to(x + s * 0.22, r.y + s * 0.68);
    pb.line_to(x, r.y + s * 0.88);
    pb.line_to(x, r.y + s * 0.12);
    pb.line_to(x + s * 0.22, r.y + s * 0.32);
    pb.line_to(x - s * 0.22, r.y + s * 0.7);
    if let Some(p) = pb.finish() {
        c.stroke_path(&p, &solid(color), s * 0.08);
    }
}

pub fn moon(c: &mut Canvas, r: Rect, color: Color) {
    let s = r.w.min(r.h);
    let mut pb = PathBuilder::new();
    let (cx, cy, rad) = (r.cx(), r.cy(), s * 0.42);
    arc(&mut pb, cx, cy, rad, -PI * 0.35, PI * 1.35);
    let (cx2, cy2, rad2) = (cx + rad * 0.55, cy - rad * 0.45, rad * 0.85);
    for i in 0..=24 {
        let a = PI * 1.2 - (PI * 1.2 - (-PI * 0.15)) * (i as f32 / 24.0);
        pb.line_to(cx2 + rad2 * a.cos(), cy2 + rad2 * a.sin());
    }
    pb.close();
    if let Some(p) = pb.finish() {
        c.fill_path(&p, &solid(color));
    }
}

pub fn sun(c: &mut Canvas, r: Rect, color: Color) {
    let s = r.w.min(r.h);
    c.fill_circle(r.cx(), r.cy(), s * 0.2, color);
    for i in 0..8 {
        let a = i as f32 * PI / 4.0;
        let mut pb = PathBuilder::new();
        pb.move_to(r.cx() + a.cos() * s * 0.32, r.cy() + a.sin() * s * 0.32);
        pb.line_to(r.cx() + a.cos() * s * 0.45, r.cy() + a.sin() * s * 0.45);
        if let Some(p) = pb.finish() {
            c.stroke_path(&p, &solid(color), s * 0.08);
        }
    }
}

pub fn speaker(c: &mut Canvas, r: Rect, color: Color, waves: u32) {
    let s = r.h;
    let mut pb = PathBuilder::new();
    let x = r.x;
    pb.move_to(x, r.y + s * 0.36);
    pb.line_to(x + s * 0.22, r.y + s * 0.36);
    pb.line_to(x + s * 0.48, r.y + s * 0.12);
    pb.line_to(x + s * 0.48, r.y + s * 0.88);
    pb.line_to(x + s * 0.22, r.y + s * 0.64);
    pb.line_to(x, r.y + s * 0.64);
    pb.close();
    if let Some(p) = pb.finish() {
        c.fill_path(&p, &solid(color));
    }
    for i in 0..waves {
        let mut pb = PathBuilder::new();
        arc(&mut pb, x + s * 0.5, r.cy(), s * (0.2 + 0.17 * i as f32), -PI / 4.0, PI / 4.0);
        if let Some(p) = pb.finish() {
            c.stroke_path(&p, &solid(color), s * 0.08);
        }
    }
}

pub fn play(c: &mut Canvas, r: Rect, color: Color) {
    let mut pb = PathBuilder::new();
    pb.move_to(r.x + r.w * 0.2, r.y + r.h * 0.1);
    pb.line_to(r.x + r.w * 0.9, r.cy());
    pb.line_to(r.x + r.w * 0.2, r.y + r.h * 0.9);
    pb.close();
    if let Some(p) = pb.finish() {
        c.fill_path(&p, &solid(color));
    }
}

pub fn skip(c: &mut Canvas, r: Rect, color: Color, forward: bool) {
    for i in 0..2 {
        let ox = r.x + r.w * 0.5 * i as f32;
        let w = r.w * 0.5;
        let mut pb = PathBuilder::new();
        if forward {
            pb.move_to(ox, r.y + r.h * 0.15);
            pb.line_to(ox + w, r.cy());
            pb.line_to(ox, r.y + r.h * 0.85);
        } else {
            pb.move_to(ox + w, r.y + r.h * 0.15);
            pb.line_to(ox, r.cy());
            pb.line_to(ox + w, r.y + r.h * 0.85);
        }
        pb.close();
        if let Some(p) = pb.finish() {
            c.fill_path(&p, &solid(color));
        }
    }
}

pub fn airdrop(c: &mut Canvas, r: Rect, color: Color) {
    let s = r.w.min(r.h);
    for k in [0.42f32, 0.28].iter() {
        let mut pb = PathBuilder::new();
        arc(&mut pb, r.cx(), r.cy(), s * k, PI * 0.75, PI * 2.25);
        if let Some(p) = pb.finish() {
            c.stroke_path(&p, &solid(color), s * 0.07);
        }
    }
    c.fill_circle(r.cx(), r.cy(), s * 0.1, color);
}

pub fn screens(c: &mut Canvas, r: Rect, color: Color) {
    let s = r.w.min(r.h);
    c.stroke_rrect(Rect::new(r.x + s * 0.12, r.y + s * 0.18, s * 0.6, s * 0.48), s * 0.08, color, s * 0.07);
    c.stroke_rrect(Rect::new(r.x + s * 0.3, r.y + s * 0.34, s * 0.6, s * 0.48), s * 0.08, color, s * 0.07);
}

/// Generic rounded square placeholder.
pub fn square(c: &mut Canvas, r: Rect, color: Color) {
    if let Some(p) = shapes::squircle(r, r.w * 0.22) {
        c.fill_path(&p, &solid(color));
    }
}

fn stroke_lines(c: &mut Canvas, pts: &[&[(f32, f32)]], color: Color, width: f32) {
    let mut pb = PathBuilder::new();
    for line in pts {
        if let Some((x, y)) = line.first() {
            pb.move_to(*x, *y);
            for (x, y) in &line[1..] {
                pb.line_to(*x, *y);
            }
        }
    }
    if let Some(p) = pb.finish() {
        c.stroke_path(&p, &solid(color), width);
    }
}

/// The "A" of the Applications / App Store glyph (three rounded strokes).
pub fn apps_glyph(c: &mut Canvas, r: Rect, color: Color) {
    let s = r.w.min(r.h);
    let (x, y) = (r.cx() - s / 2.0, r.cy() - s / 2.0);
    let p = |u: f32, v: f32| (x + u * s, y + v * s);
    let w = s * 0.085;
    stroke_lines(
        c,
        &[&[p(0.30, 0.84), p(0.58, 0.18)], &[p(0.42, 0.18), p(0.70, 0.84)], &[p(0.14, 0.64), p(0.86, 0.64)]],
        color,
        w,
    );
}

/// Folder outline (Files).
pub fn folder(c: &mut Canvas, r: Rect, color: Color) {
    let s = r.w.min(r.h);
    let (x, y) = (r.cx() - s / 2.0, r.cy() - s / 2.0);
    let w = s * 0.075;
    let body = Rect::new(x + s * 0.08, y + s * 0.30, s * 0.84, s * 0.54);
    c.stroke_rrect(body, s * 0.08, color, w);
    let mut pb = PathBuilder::new();
    pb.move_to(x + s * 0.08, y + s * 0.36);
    pb.line_to(x + s * 0.08, y + s * 0.24);
    pb.quad_to(x + s * 0.08, y + s * 0.17, x + s * 0.15, y + s * 0.17);
    pb.line_to(x + s * 0.36, y + s * 0.17);
    pb.line_to(x + s * 0.44, y + s * 0.26);
    pb.line_to(x + s * 0.84, y + s * 0.26);
    pb.quad_to(x + s * 0.92, y + s * 0.26, x + s * 0.92, y + s * 0.34);
    if let Some(pp) = pb.finish() {
        c.stroke_path(&pp, &solid(color), w);
    }
    stroke_lines(c, &[&[(x + s * 0.08, y + s * 0.40), (x + s * 0.92, y + s * 0.40)]], color, w * 0.9);
}

/// Two stacked layers (Actions / Shortcuts).
pub fn layers(c: &mut Canvas, r: Rect, color: Color) {
    let s = r.w.min(r.h);
    let (x, y) = (r.cx() - s / 2.0, r.cy() - s / 2.0);
    let p = |u: f32, v: f32| (x + u * s, y + v * s);
    let w = s * 0.075;
    stroke_lines(c, &[&[p(0.5, 0.14), p(0.88, 0.36), p(0.5, 0.58), p(0.12, 0.36), p(0.5, 0.14)]], color, w);
    stroke_lines(c, &[&[p(0.20, 0.53), p(0.12, 0.58), p(0.5, 0.80), p(0.88, 0.58), p(0.80, 0.53)]], color, w);
}

/// Two overlapping documents (Clipboard).
pub fn documents(c: &mut Canvas, r: Rect, color: Color) {
    let s = r.w.min(r.h);
    let (x, y) = (r.cx() - s / 2.0, r.cy() - s / 2.0);
    let p = |u: f32, v: f32| (x + u * s, y + v * s);
    let w = s * 0.075;
    stroke_lines(
        c,
        &[&[p(0.28, 0.30), p(0.28, 0.16), p(0.56, 0.16), p(0.74, 0.34), p(0.74, 0.62), p(0.66, 0.62)]],
        color,
        w,
    );
    stroke_lines(c, &[&[p(0.56, 0.16), p(0.56, 0.34), p(0.74, 0.34)]], color, w * 0.85);
    stroke_lines(
        c,
        &[&[p(0.20, 0.36), p(0.48, 0.36), p(0.66, 0.54), p(0.66, 0.88), p(0.20, 0.88), p(0.20, 0.36)]],
        color,
        w,
    );
    stroke_lines(c, &[&[p(0.48, 0.36), p(0.48, 0.54), p(0.66, 0.54)]], color, w * 0.85);
}

/// Generic document with a folded corner, filled (file results).
pub fn document(c: &mut Canvas, r: Rect, paper: Color, line: Color) {
    let s = r.w.min(r.h);
    let (x, y) = (r.cx() - s * 0.36, r.y + (r.h - s) / 2.0 + s * 0.04);
    let (w, h) = (s * 0.72, s * 0.92);
    let f = w * 0.32;
    let mut pb = PathBuilder::new();
    pb.move_to(x, y);
    pb.line_to(x + w - f, y);
    pb.line_to(x + w, y + f);
    pb.line_to(x + w, y + h);
    pb.line_to(x, y + h);
    pb.close();
    if let Some(p) = pb.finish() {
        c.fill_path(&p, &solid(paper));
        c.stroke_path(&p, &solid(line), s * 0.03);
    }
    stroke_lines(c, &[&[(x + w - f, y), (x + w - f, y + f), (x + w, y + f)]], line, s * 0.03);
}

/// Screen-recording stop button (menu bar): circle with a square.
pub fn record_stop(c: &mut Canvas, r: Rect, color: Color) {
    let s = r.w.min(r.h);
    if let Some(p) = PathBuilder::from_circle(r.cx(), r.cy(), s * 0.44) {
        c.stroke_path(&p, &solid(color), s * 0.09);
    }
    let q = s * 0.34;
    c.fill_rrect(Rect::new(r.cx() - q / 2.0, r.cy() - q / 2.0, q, q), s * 0.06, color);
}

/// Video camera (Record Entire Screen / Record Selected Portion buttons).
pub fn record_screen(c: &mut Canvas, r: Rect, color: Color, portion: bool) {
    let s = r.w.min(r.h);
    let body = Rect::new(r.cx() - s * 0.46, r.cy() - s * 0.32, s * 0.92, s * 0.64);
    if portion {
        let mut x = body.x;
        while x < body.right() {
            let w = (s * 0.12).min(body.right() - x);
            c.fill_rect(Rect::new(x, body.y, w, s * 0.06), color);
            c.fill_rect(Rect::new(x, body.bottom() - s * 0.06, w, s * 0.06), color);
            x += s * 0.2;
        }
        let mut y = body.y;
        while y < body.bottom() {
            let h = (s * 0.12).min(body.bottom() - y);
            c.fill_rect(Rect::new(body.x, y, s * 0.06, h), color);
            c.fill_rect(Rect::new(body.right() - s * 0.06, y, s * 0.06, h), color);
            y += s * 0.2;
        }
    } else {
        c.stroke_rrect(body, s * 0.1, color, s * 0.07);
    }
    if let Some(p) = PathBuilder::from_circle(r.cx(), r.cy(), s * 0.14) {
        c.fill_path(&p, &solid(color));
    }
}
