//! Desktop widgets (glass): calendar month + analog clock.
use crate::{clock, hash_of, style, Layer, LayerId, Shell};
use aqua_gfx::{rgba, Rect, Weight};

#[derive(Default)]
pub struct Widgets {
    pub hidden: bool,
}

const S: f32 = 168.0;

pub fn layers(sh: &mut Shell) -> Vec<Layer> {
    if sh.widgets.hidden || !sh.cfg.show_widgets {
        return vec![];
    }
    let n = clock::now();
    let y = sh.cfg.menubar_height + 14.0;
    let clock_r = Rect::new(18.0, y, S, S);
    let cal_r = Rect::new(18.0 + S + 16.0, y, S, S);
    let light = !sh.style.dark && sh.style.lum(cal_r) > 0.8;
    let fg = if light { rgba(29, 29, 31, 0.9) } else { rgba(255, 255, 255, 0.95) };
    let fg2 = if light { rgba(29, 29, 31, 0.55) } else { rgba(255, 255, 255, 0.7) };
    let g = style::glass_tile(&sh.cfg.glass, 26.0);
    let mut out = vec![];

    let key = hash_of(&(n.hour, n.minute, light));
    let (pm, serial) = sh.cached(LayerId::Widgets(0), key, S, S, |c, _sh| {
        let (cx, cy, r) = (S / 2.0, S / 2.0, S / 2.0 - 14.0);
        c.fill_circle(cx, cy, r, rgba(255, 255, 255, if light { 0.55 } else { 0.18 }));
        for i in 0..60 {
            let a = i as f32 / 60.0 * std::f32::consts::TAU;
            let (r0, w) = if i % 5 == 0 { (r - 10.0, 2.2) } else { (r - 5.0, 1.0) };
            let mut pb = aqua_gfx::tiny_skia::PathBuilder::new();
            pb.move_to(cx + a.sin() * r0, cy - a.cos() * r0);
            pb.line_to(cx + a.sin() * (r - 2.0), cy - a.cos() * (r - 2.0));
            if let Some(p) = pb.finish() {
                c.stroke_path(&p, &aqua_gfx::canvas::solid(fg2), w);
            }
        }
        let hand = |c: &mut aqua_gfx::Canvas, frac: f32, len: f32, w: f32, col| {
            let a = frac * std::f32::consts::TAU;
            let mut pb = aqua_gfx::tiny_skia::PathBuilder::new();
            pb.move_to(cx - a.sin() * 8.0, cy + a.cos() * 8.0);
            pb.line_to(cx + a.sin() * len, cy - a.cos() * len);
            if let Some(p) = pb.finish() {
                c.stroke_path(&p, &aqua_gfx::canvas::solid(col), w);
            }
        };
        let hf = ((n.hour % 12) as f32 + n.minute as f32 / 60.0) / 12.0;
        hand(c, hf, r * 0.52, 5.0, fg);
        hand(c, n.minute as f32 / 60.0, r * 0.8, 3.5, fg);
        c.fill_circle(cx, cy, 4.5, rgba(255, 149, 0, 1.0));
    });
    out.push(Layer {
        id: LayerId::Widgets(0),
        rect: clock_r,
        glass: Some(g),
        tiles: vec![],
        content: pm,
        serial,
        opacity: 1.0,
        zoom: 1.0,
    });

    let key = hash_of(&(n.year, n.month, n.day, light));
    let (pm, serial) = sh.cached(LayerId::Widgets(1), key, S, S, |c, sh| {
        let f = sh.fonts.clone();
        c.text(&f, 18.0, 28.0, 12.0, Weight::Bold, rgba(255, 69, 58, 1.0), clock::MONTHS_LONG[(n.month - 1) as usize]);
        let cw = (S - 28.0) / 7.0;
        for (i, d) in ["M", "T", "W", "T", "F", "S", "S"].iter().enumerate() {
            c.text_in(&f, Rect::new(14.0 + i as f32 * cw, 36.0, cw, 14.0), 0.5, 9.5, Weight::Semibold, fg2, d);
        }
        let first = clock::first_weekday(&n);
        let dim = clock::days_in_month(n.year, n.month);
        for d in 1..=dim {
            let idx = first + d as usize - 1;
            let (col, row) = (idx % 7, idx / 7);
            let r = Rect::new(14.0 + col as f32 * cw, 54.0 + row as f32 * 18.0, cw, 18.0);
            if d == n.day {
                c.fill_circle(r.cx(), r.cy(), 8.5, rgba(255, 69, 58, 1.0));
                c.text_in(&f, r, 0.5, 10.5, Weight::Bold, rgba(255, 255, 255, 1.0), &d.to_string());
            } else {
                c.text_in(&f, r, 0.5, 10.5, Weight::Semibold, fg, &d.to_string());
            }
        }
    });
    out.push(Layer {
        id: LayerId::Widgets(1),
        rect: cal_r,
        glass: Some(g),
        tiles: vec![],
        content: pm,
        serial,
        opacity: 1.0,
        zoom: 1.0,
    });
    out
}
