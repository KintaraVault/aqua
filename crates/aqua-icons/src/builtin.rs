//! Procedural (Aqua) icons, drawn as glass layers on the 1024
//! grid. Plate icons use body coordinates 0..824 (the visible plate); Trash and folders
//! are free-form on the full 1024 grid. See [`crate::glass`] for the material.
use crate::glass::{Cov, Glass, Ic, B};
use aqua_gfx::canvas::lin_grad;
use aqua_gfx::tiny_skia::{BlendMode, Paint, PathBuilder};
use aqua_gfx::{rgba, Color, Fonts, Pixmap, Rect, Weight};
use std::f32::consts::PI;

fn hex(c: u32, a: f32) -> Color {
    rgba((c >> 16) as u8, (c >> 8) as u8, c as u8, a)
}

/// Vertical gradient between `y0` and `y1` (drawing coords).
fn vg(y0: f32, y1: f32, top: u32, bottom: u32) -> Paint<'static> {
    lin_grad(0.0, y0, 0.0, y1, &[(0.0, hex(top, 1.0)), (1.0, hex(bottom, 1.0))])
}

fn solid(c: Color) -> Paint<'static> {
    crate::glass::solid(c)
}

/// Plate background: a soft light-to-dark gradient.
fn bg(ic: &mut Ic, top: u32, bottom: u32) {
    ic.bg_v(hex(top, 1.0), hex(bottom, 1.0));
}

/// Frosted white glyph (white at the top, picking up a hint of the plate below).
fn frost(y0: f32, y1: f32, tint: u32) -> Paint<'static> {
    lin_grad(0.0, y0, 0.0, y1, &[(0.0, hex(0xffffff, 1.0)), (1.0, hex(tint, 1.0))])
}

fn line(pts: &[(f32, f32)]) -> aqua_gfx::tiny_skia::Path {
    let mut pb = PathBuilder::new();
    for (i, (x, y)) in pts.iter().enumerate() {
        if i == 0 {
            pb.move_to(*x, *y)
        } else {
            pb.line_to(*x, *y)
        }
    }
    pb.finish()
        .unwrap_or_else(|| PathBuilder::from_rect(aqua_gfx::tiny_skia::Rect::from_xywh(0.0, 0.0, 1.0, 1.0).unwrap()))
}

/// [`draw`] in an icon style (Dark / Clear / Tinted are rendered per layer).
pub fn draw_look(name: &str, px: u32, fonts: &Fonts, look: &crate::look::Look) -> Option<Pixmap> {
    crate::glass::with_look(look, || draw(name, px, fonts))
}

/// [`monogram`] in an icon style.
pub fn monogram_look(name: &str, px: u32, fonts: &Fonts, look: &crate::look::Look) -> Pixmap {
    crate::glass::with_look(look, || monogram(name, px, fonts))
}

pub fn draw(name: &str, px: u32, fonts: &Fonts) -> Option<Pixmap> {
    let px = px.max(16);
    if name == "trash" {
        return Some(trash(px));
    }
    if name == "downloads" || name == "folder" {
        return Some(folder(px, name == "downloads"));
    }
    let mut ic = Ic::plate(px);
    match name {
        "finder" => finder(&mut ic),
        "launchpad" | "apps" => launchpad(&mut ic),
        "safari" => safari(&mut ic),
        "messages" => messages(&mut ic),
        "mail" => mail(&mut ic),
        "maps" => maps(&mut ic),
        "photos" => photos(&mut ic),
        "facetime" => facetime(&mut ic),
        "calendar" => calendar(&mut ic, fonts),
        "notes" => notes(&mut ic),
        "reminders" => reminders(&mut ic),
        "music" => music(&mut ic),
        "terminal" => terminal(&mut ic),
        "calculator" => calculator(&mut ic),
        "textedit" => textedit(&mut ic, fonts),
        "preview" => preview(&mut ic),
        "activity" => activity(&mut ic),
        "appstore" => appstore(&mut ic),
        "settings" => settings(&mut ic),
        _ => return None,
    }
    Some(ic.finish(true))
}

pub fn monogram(name: &str, px: u32, fonts: &Fonts) -> Pixmap {
    let mut ic = Ic::plate(px.max(16));
    let h = name.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    let palette = [
        (0x8c97ff, 0x4f4fe0),
        (0x5fe0ad, 0x16a273),
        (0xffb066, 0xf56a1f),
        (0xff86b0, 0xe03a72),
        (0x6fd0ff, 0x1a7fec),
        (0xbf96ff, 0x7845e0),
    ];
    let (t, b) = palette[(h as usize) % palette.len()];
    bg(&mut ic, t, b);
    let letter: String = name.chars().next().map(|ch| ch.to_uppercase().collect()).unwrap_or_else(|| "?".into());
    let cov = ic.cov_text(fonts, Rect::new(0.0, 0.0, B, B), 0.5, 470.0, Weight::Semibold, &letter);
    ic.glass(&cov, &frost(150.0, 650.0, 0xeef0ff), Glass::frosted(0.96).tint(hex(b, 1.0)).shadow(0.3));
    ic.finish(true)
}

fn finder(ic: &mut Ic) {
    bg(ic, 0x3ad3ff, 0x0b97f4);
    let mut pb = PathBuilder::new();
    pb.move_to(338.0, 452.0);
    pb.cubic_to(360.0, 330.0, 392.0, 190.0, 416.0, 104.0);
    pb.quad_to(428.0, 62.0, 472.0, 62.0);
    pb.line_to(672.0, 62.0);
    pb.cubic_to(724.0, 62.0, 762.0, 100.0, 762.0, 152.0);
    pb.line_to(762.0, 672.0);
    pb.cubic_to(762.0, 724.0, 724.0, 762.0, 672.0, 762.0);
    pb.line_to(548.0, 762.0);
    pb.quad_to(504.0, 762.0, 494.0, 718.0);
    pb.cubic_to(474.0, 640.0, 446.0, 548.0, 436.0, 486.0);
    pb.line_to(356.0, 482.0);
    pb.quad_to(330.0, 480.0, 338.0, 452.0);
    pb.close();
    if let Some(p) = pb.finish() {
        let cov = ic.cov(&p);
        ic.glass(&cov, &frost(62.0, 762.0, 0xd9edff), Glass::frosted(0.97).tint(hex(0x00468f, 1.0)).shadow(0.28));
    }
    let ink = vg(200.0, 700.0, 0x15233a, 0x08101c);
    let eyes = ic
        .cov_rrect(Rect::new(212.0, 252.0, 32.0, 88.0), 16.0)
        .union(&ic.cov_rrect(Rect::new(556.0, 252.0, 32.0, 88.0), 16.0));
    ic.glass(&eyes, &ink, Glass::default().shadow(0.12).spec(0.25).lift(6.0, 3.0));
    let mut pb = PathBuilder::new();
    pb.move_to(170.0, 556.0);
    pb.cubic_to(300.0, 680.0, 500.0, 682.0, 630.0, 560.0);
    if let Some(p) = pb.finish() {
        let smile = ic.cov_stroke(&p, 30.0);
        ic.glass(&smile, &ink, Glass::default().shadow(0.12).spec(0.25).lift(6.0, 3.0));
    }
}

/// "Apps": a search field above a grid of coloured tiles.
fn launchpad(ic: &mut Ic) {
    bg(ic, 0xffffff, 0xe8e8ee);
    let pill = ic.cov_rrect(Rect::new(128.0, 150.0, 568.0, 126.0), 63.0);
    ic.glass(&pill, &vg(150.0, 276.0, 0xd3d3d8, 0xbdbdc3), Glass::default().shadow(0.10).lift(10.0, 5.0));
    let ring = sk_circle_stroke(ic, 214.0, 205.0, 30.0, 13.0);
    let handle = ic.cov_stroke(&line(&[(236.0, 227.0), (262.0, 253.0)]), 15.0);
    ic.glass(&ring.union(&handle), &solid(hex(0xffffff, 1.0)), Glass::flat());
    let tiles: [(u32, u32); 6] = [
        (0x2fa8ff, 0x0b7af2),
        (0x48dc6d, 0x22b14a),
        (0xff5579, 0xf0244e),
        (0xffaa3a, 0xf8860b),
        (0xbf6cf0, 0x9a42d8),
        (0xb0b0b5, 0x8d8d93),
    ];
    let (size, xs, ys) = (138.0, [140.0, 343.0, 546.0], [356.0, 556.0]);
    for (i, (t, b)) in tiles.iter().enumerate() {
        let (x, y) = (xs[i % 3], ys[i / 3]);
        let cov = ic.cov_squircle(Rect::new(x, y, size, size), size * 0.3);
        ic.glass(&cov, &vg(y, y + size, *t, *b), Glass::default().tint(hex(*b, 1.0)).shadow(0.35).lift(18.0, 10.0));
    }
}

fn sk_circle_stroke(ic: &Ic, cx: f32, cy: f32, r: f32, w: f32) -> Cov {
    match PathBuilder::from_circle(cx, cy, r) {
        Some(p) => ic.cov_stroke(&p, w),
        None => Cov::empty(ic.n(), ic.n()),
    }
}

fn safari(ic: &mut Ic) {
    bg(ic, 0xffffff, 0xeeeff3);
    let (cx, cy, r) = (B / 2.0, B / 2.0, 334.0);
    let disc = ic.cov_circle(cx, cy, r);
    let face = lin_grad(
        0.0,
        cy - r,
        0.0,
        cy + r,
        &[(0.0, hex(0x3fcbff, 1.0)), (0.55, hex(0x1d93fb, 1.0)), (1.0, hex(0x1a5fee, 1.0))],
    );
    ic.glass(&disc, &face, Glass::default().tint(hex(0x0b3d9a, 1.0)).shadow(0.25).lift(18.0, 10.0));
    let mut pb = PathBuilder::new();
    let mut pbm = PathBuilder::new();
    for i in 0..72 {
        let a = i as f32 * 2.0 * PI / 72.0;
        let major = i % 6 == 0;
        let (r0, r1) = if major { (r - 92.0, r - 34.0) } else { (r - 72.0, r - 34.0) };
        let b = if major { &mut pbm } else { &mut pb };
        b.move_to(cx + a.cos() * r0, cy + a.sin() * r0);
        b.line_to(cx + a.cos() * r1, cy + a.sin() * r1);
    }
    let mut ticks = Cov::empty(ic.n(), ic.n());
    if let Some(p) = pb.finish() {
        ticks = ticks.union(&ic.cov_stroke_butt(&p, 7.0));
    }
    if let Some(p) = pbm.finish() {
        ticks = ticks.union(&ic.cov_stroke_butt(&p, 9.0));
    }
    ic.fill_color(&ticks, hex(0xffffff, 0.92));
    let a = -PI / 4.0 - 0.06;
    let (dx, dy) = (a.cos(), a.sin());
    let (nx, ny) = (-dy, dx);
    let (len, w) = (262.0, 44.0);
    let red = ic.cov_poly(&[(cx + dx * len, cy + dy * len), (cx + nx * w, cy + ny * w), (cx - nx * w, cy - ny * w)]);
    let white = ic.cov_poly(&[(cx - dx * len, cy - dy * len), (cx + nx * w, cy + ny * w), (cx - nx * w, cy - ny * w)]);
    ic.glass(
        &white,
        &frost(cy, cy + len, 0xe8eef7),
        Glass::default().tint(hex(0x062c70, 1.0)).shadow(0.3).lift(12.0, 8.0),
    );
    ic.glass(
        &red,
        &vg(cy - len, cy, 0xff4d42, 0xe8211c),
        Glass::default().tint(hex(0x062c70, 1.0)).shadow(0.3).lift(12.0, 8.0),
    );
}

fn messages(ic: &mut Ic) {
    bg(ic, 0x6ff384, 0x1ec241);
    let bubble = ic.cov_oval(B / 2.0, 392.0, 308.0, 252.0, 0.0);
    let mut pb = PathBuilder::new();
    pb.move_to(200.0, 520.0);
    pb.cubic_to(200.0, 600.0, 170.0, 660.0, 128.0, 700.0);
    pb.cubic_to(220.0, 700.0, 300.0, 660.0, 340.0, 610.0);
    pb.close();
    let tail = pb.finish().map(|p| ic.cov(&p)).unwrap_or_else(|| Cov::empty(ic.n(), ic.n()));
    ic.glass(
        &bubble.union(&tail),
        &frost(140.0, 700.0, 0xe6f8e9),
        Glass::frosted(0.97).tint(hex(0x0b6a1f, 1.0)).shadow(0.3),
    );
}

fn mail(ic: &mut Ic) {
    bg(ic, 0x2bb0ff, 0x0b67f0);
    let r = Rect::new(98.0, 206.0, 628.0, 420.0);
    let env = ic.cov_rrect(r, 56.0);
    ic.glass(&env, &frost(r.y, r.bottom(), 0xcfe4ff), Glass::frosted(0.97).tint(hex(0x003b9c, 1.0)).shadow(0.3));
    let folds = ic
        .cov_stroke(&line(&[(130.0, 596.0), (330.0, 420.0)]), 7.0)
        .union(&ic.cov_stroke(&line(&[(694.0, 596.0), (494.0, 420.0)]), 7.0));
    ic.fill_color(&folds.intersect(&env), hex(0x7fb1ea, 0.35));
    let mut pb = PathBuilder::new();
    pb.move_to(118.0, 226.0);
    pb.line_to(706.0, 226.0);
    pb.line_to(470.0, 452.0);
    pb.quad_to(412.0, 504.0, 354.0, 452.0);
    pb.close();
    if let Some(p) = pb.finish() {
        let flap = ic.cov(&p).intersect(&env);
        ic.glass(
            &flap,
            &frost(226.0, 480.0, 0xe4f0ff),
            Glass::default().tint(hex(0x0a3f8a, 1.0)).shadow(0.16).spec(0.6).lift(14.0, 8.0),
        );
    }
}

fn maps(ic: &mut Ic) {
    bg(ic, 0x6fe282, 0x37c257);
    let mut rings = Cov::empty(ic.n(), ic.n());
    for r in [110.0, 160.0, 210.0] {
        rings = rings.union(&sk_circle_stroke(ic, 700.0, 120.0, r, 18.0));
    }
    ic.fill_color(&rings, hex(0xffffff, 0.30));
    let pink = ic.cov_poly(&[(0.0, 520.0), (230.0, 824.0), (0.0, 824.0)]);
    ic.glass(&pink, &vg(520.0, 824.0, 0xff8cc4, 0xf2629f), Glass::flat());
    let yellow = ic.cov_poly(&[(430.0, 824.0), (824.0, 600.0), (824.0, 824.0)]);
    ic.glass(&yellow, &vg(600.0, 824.0, 0xffdc3c, 0xffc400), Glass::flat());
    let road = ic
        .cov_stroke_butt(&line(&[(-40.0, 150.0), (880.0, 700.0)]), 96.0)
        .union(&ic.cov_stroke_butt(&line(&[(560.0, -40.0), (380.0, 880.0)]), 70.0));
    ic.glass(&road, &solid(hex(0xf7f7f7, 1.0)), Glass::default().shadow(0.12).spec(0.4).lift(10.0, 4.0));
    let hw_path = line(&[(268.0, -40.0), (268.0, 560.0)]);
    let casing = ic.cov_stroke_butt(&hw_path, 108.0);
    ic.fill_color(&casing, hex(0xffffff, 1.0));
    let hw = ic.cov_stroke_butt(&hw_path, 76.0);
    ic.glass(&hw, &vg(0.0, 560.0, 0x3aa0ff, 0x1673f2), Glass::flat().spec(0.4));
    let (cx, cy) = (292.0, 622.0);
    let ring = ic.cov_circle(cx, cy, 178.0);
    ic.glass(&ring, &frost(cy - 178.0, cy + 178.0, 0xe9eef5), Glass::frosted(0.92).shadow(0.32).lift(22.0, 12.0));
    let disc = ic.cov_circle(cx, cy, 138.0);
    ic.glass(&disc, &vg(cy - 138.0, cy + 138.0, 0x45b0ff, 0x0b6bf0), Glass::default().shadow(0.0).spec(0.7));
    let arrow = ic.cov_poly(&[(cx, cy - 92.0), (cx + 66.0, cy + 76.0), (cx, cy + 40.0), (cx - 66.0, cy + 76.0)]);
    ic.glass(
        &arrow,
        &solid(hex(0xffffff, 1.0)),
        Glass::default().tint(hex(0x003b8f, 1.0)).shadow(0.25).lift(10.0, 6.0).spec(0.3),
    );
}

fn photos(ic: &mut Ic) {
    bg(ic, 0xffffff, 0xf2f2f5);
    let cols = [0xff9a1a, 0xffcc12, 0x9ad936, 0x2fcf86, 0x15a8f2, 0x9a7cec, 0xf76fb6, 0xff4f5b];
    let (cx, cy) = (B / 2.0, B / 2.0);
    for (i, col) in cols.iter().enumerate() {
        let a = i as f32 * PI / 4.0 - PI / 2.0;
        let (px, py) = (cx + a.cos() * 128.0, cy + a.sin() * 128.0);
        let petal = ic.cov_oval(px, py, 82.0, 134.0, a.to_degrees() + 90.0);
        let mut paint = vg(py - 134.0, py + 134.0, *col, *col);
        paint.blend_mode = BlendMode::Multiply;
        ic.glass(&petal, &paint, Glass::frosted(0.9).shadow(0.0).spec(0.45));
    }
}

fn facetime(ic: &mut Ic) {
    bg(ic, 0x67f477, 0x20c33d);
    let body = ic.cov_squircle(Rect::new(112.0, 212.0, 432.0, 400.0), 92.0);
    let poly = [(590.0, 372.0), (712.0, 280.0), (712.0, 544.0), (590.0, 452.0)];
    let mut pb = PathBuilder::new();
    pb.move_to(poly[0].0, poly[0].1);
    for p in &poly[1..] {
        pb.line_to(p.0, p.1);
    }
    pb.close();
    let lens = match pb.finish() {
        Some(p) => ic.cov(&p).union(&ic.cov_stroke(&p, 56.0)),
        None => Cov::empty(ic.n(), ic.n()),
    };
    let g = Glass::frosted(0.97).tint(hex(0x0b6a1f, 1.0)).shadow(0.3);
    ic.glass(&body, &frost(212.0, 612.0, 0xe2f7e5), g);
    ic.glass(&lens, &frost(250.0, 574.0, 0xe2f7e5), g);
}

fn calendar(ic: &mut Ic, fonts: &Fonts) {
    bg(ic, 0xffffff, 0xf1f1f4);
    let (wd, day) = today();
    let w = ic.cov_text(fonts, Rect::new(0.0, 70.0, B, 160.0), 0.5, 150.0, Weight::Semibold, wd);
    ic.glass(&w, &vg(70.0, 230.0, 0xff4639, 0xf02d22), Glass::flat());
    let d = ic.cov_text(fonts, Rect::new(0.0, 238.0, B, 500.0), 0.5, 500.0, Weight::Regular, &day.to_string());
    ic.glass(&d, &vg(260.0, 740.0, 0x2a2a2d, 0x111113), Glass::flat());
}

/// (weekday abbreviation, day of month) computed from the system clock in local time.
pub fn today() -> (&'static str, u32) {
    let secs =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let off = local_offset_secs();
    let days = (secs + off).div_euclid(86400);
    let wd = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][(days.rem_euclid(7)) as usize];
    let (_, _, d) = civil_from_days(days);
    (wd, d)
}

pub fn local_offset_secs() -> i64 {
    if let Ok(v) = std::env::var("AQUA_TZ_OFFSET") {
        return v.parse().unwrap_or(0);
    }
    static OFF: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| {
        std::process::Command::new("date")
            .arg("+%z")
            .output()
            .ok()
            .and_then(|o| {
                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                let sign = if s.starts_with('-') { -1 } else { 1 };
                let d = s.trim_start_matches(['+', '-']);
                let h: i64 = d.get(0..2)?.parse().ok()?;
                let m: i64 = d.get(2..4)?.parse().ok()?;
                Some(sign * (h * 3600 + m * 60))
            })
            .unwrap_or(0)
    })
}

pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn notes(ic: &mut Ic) {
    bg(ic, 0xffffff, 0xf3f3f5);
    let band = ic.cov_rrect(Rect::new(-40.0, -40.0, B + 80.0, 242.0), 1.0);
    ic.glass(
        &band,
        &vg(0.0, 202.0, 0xffe45a, 0xffc908),
        Glass::default().tint(hex(0x9b7400, 1.0)).shadow(0.2).spec(0.5).lift(12.0, 6.0),
    );
    let mut dots = Cov::empty(ic.n(), ic.n());
    for i in 0..17 {
        dots = dots.union(&ic.cov_circle(40.0 + i as f32 * 46.5, 236.0, 6.5));
    }
    ic.fill_color(&dots, hex(0xb7b7bd, 1.0));
    let lines = ic
        .cov_rrect(Rect::new(108.0, 414.0, 608.0, 11.0), 5.5)
        .union(&ic.cov_rrect(Rect::new(108.0, 620.0, 608.0, 11.0), 5.5));
    ic.fill_color(&lines, hex(0xc4c4ca, 1.0));
}

fn reminders(ic: &mut Ic) {
    bg(ic, 0xffffff, 0xf1f1f4);
    let cols = [(0x3aa6ff, 0x0b78f0), (0xff5a52, 0xe8302a), (0xffaa2a, 0xf28a00)];
    for (i, (t, b)) in cols.iter().enumerate() {
        let y = 220.0 + i as f32 * 196.0;
        let halo = ic.cov_circle(160.0, y, 64.0);
        ic.fill_color(&halo, hex(*t, 0.32));
        let dot = ic.cov_circle(160.0, y, 46.0);
        ic.glass(
            &dot,
            &vg(y - 46.0, y + 46.0, *t, *b),
            Glass::default().tint(hex(*b, 1.0)).shadow(0.3).lift(10.0, 6.0),
        );
        let l = ic.cov_rrect(Rect::new(282.0, y - 6.0, 440.0, 12.0), 6.0);
        ic.fill_color(&l, hex(0xc7c7cc, 1.0));
    }
}

fn music(ic: &mut Ic) {
    bg(ic, 0xff5f7c, 0xf31c45);
    let f = |x: f32| x * B;
    let beam = ic.cov_poly(&[(f(0.345), f(0.24)), (f(0.745), f(0.158)), (f(0.745), f(0.288)), (f(0.345), f(0.37))]);
    let stems = ic
        .cov_rrect(Rect::new(f(0.345), f(0.26), f(0.07), f(0.48)), 8.0)
        .union(&ic.cov_rrect(Rect::new(f(0.675), f(0.18), f(0.07), f(0.48)), 8.0));
    let heads = ic.cov_oval(f(0.30), f(0.745), f(0.118), f(0.088), -22.0).union(&ic.cov_oval(
        f(0.63),
        f(0.665),
        f(0.118),
        f(0.088),
        -22.0,
    ));
    let note = beam.offset(-6.0 * ic.s).offset(6.0 * ic.s).union(&stems).union(&heads);
    ic.glass(&note, &frost(f(0.15), f(0.85), 0xffdde5), Glass::frosted(0.95).tint(hex(0x8a0020, 1.0)).shadow(0.3));
}

fn terminal(ic: &mut Ic) {
    bg(ic, 0x3c3c40, 0x151517);
    let screen = ic.cov_squircle(Rect::new(70.0, 70.0, B - 140.0, B - 140.0), 120.0);
    ic.glass(&screen, &vg(70.0, B - 70.0, 0x1b1b1e, 0x0b0b0c), Glass::flat().spec(0.35));
    let chev = ic.cov_stroke(&line(&[(206.0, 300.0), (330.0, 400.0), (206.0, 500.0)]), 56.0);
    let bar = ic.cov_rrect(Rect::new(376.0, 474.0, 220.0, 54.0), 27.0);
    ic.glass(&chev.union(&bar), &frost(300.0, 528.0, 0xd8dbe2), Glass::default().shadow(0.35).lift(14.0, 8.0));
}

fn calculator(ic: &mut Ic) {
    bg(ic, 0x4b4b50, 0x1d1d1f);
    let c = [(250.0, 250.0), (574.0, 250.0), (250.0, 574.0), (574.0, 574.0)];
    for (i, (x, y)) in c.iter().enumerate() {
        let key = ic.cov_circle(*x, *y, 140.0);
        let paint = if i == 3 {
            vg(y - 140.0, y + 140.0, 0xffb340, 0xff8a00)
        } else {
            vg(y - 140.0, y + 140.0, 0x9a9aa0, 0x6c6c72)
        };
        ic.glass(&key, &paint, Glass::default().shadow(0.4).lift(20.0, 10.0));
        let (w, t) = (130.0, 30.0);
        let h = |dy: f32| Rect::new(x - w / 2.0, y - t / 2.0 + dy, w, t);
        let sym = match i {
            0 => ic.cov_rrect(h(0.0), t / 2.0).union(&ic.cov_rrect(Rect::new(x - t / 2.0, y - w / 2.0, t, w), t / 2.0)),
            1 => ic.cov_rrect(h(0.0), t / 2.0),
            2 => ic
                .cov_stroke(&line(&[(x - 48.0, y - 48.0), (x + 48.0, y + 48.0)]), t)
                .union(&ic.cov_stroke(&line(&[(x + 48.0, y - 48.0), (x - 48.0, y + 48.0)]), t)),
            _ => ic.cov_rrect(h(-30.0), t / 2.0).union(&ic.cov_rrect(h(30.0), t / 2.0)),
        };
        ic.glass(&sym, &solid(hex(0xffffff, 1.0)), Glass::default().shadow(0.2).lift(8.0, 4.0).spec(0.3));
    }
}

fn textedit(ic: &mut Ic, fonts: &Fonts) {
    bg(ic, 0xf6f6f8, 0xd6d6dc);
    let paper = ic.cov_squircle(Rect::new(150.0, 96.0, 524.0, 632.0), 60.0);
    ic.glass(&paper, &vg(96.0, 728.0, 0xffffff, 0xf4f4f6), Glass::default().shadow(0.25));
    let aa = ic.cov_text(fonts, Rect::new(210.0, 140.0, 300.0, 150.0), 0.0, 128.0, Weight::Bold, "Aa");
    ic.fill_color(&aa, hex(0x1c1c1e, 1.0));
    let mut ln = Cov::empty(ic.n(), ic.n());
    for i in 0..6 {
        let y = 330.0 + i as f32 * 58.0;
        let w = if i % 3 == 2 { 250.0 } else { 400.0 };
        ln = ln.union(&ic.cov_rrect(Rect::new(212.0, y, w, 16.0), 8.0));
    }
    ic.fill_color(&ln, hex(0xb4b4ba, 1.0));
    let pen = ic.cov_stroke(&line(&[(596.0, 690.0), (720.0, 300.0)]), 64.0);
    ic.glass(
        &pen,
        &lin_grad(560.0, 0.0, 760.0, 0.0, &[(0.0, hex(0x3a3a3e, 1.0)), (1.0, hex(0x141416, 1.0))]),
        Glass::default().shadow(0.35).lift(18.0, 10.0),
    );
    let nib = ic.cov_poly(&[(566.0, 676.0), (626.0, 696.0), (574.0, 772.0)]);
    ic.glass(&nib, &vg(676.0, 772.0, 0xe9c26a, 0xb98a2c), Glass::default().shadow(0.2).spec(0.6));
}

fn preview(ic: &mut Ic) {
    bg(ic, 0x45a8ff, 0x166ae8);
    let mut pb = PathBuilder::new();
    pb.move_to(232.0, 330.0);
    pb.line_to(592.0, 330.0);
    pb.line_to(592.0, 640.0);
    pb.cubic_to(592.0, 760.0, 232.0, 760.0, 232.0, 640.0);
    pb.close();
    if let Some(p) = pb.finish() {
        let body = ic.cov(&p);
        ic.glass(
            &body,
            &lin_grad(
                232.0,
                0.0,
                592.0,
                0.0,
                &[(0.0, hex(0xb9defd, 1.0)), (0.45, hex(0xf2f9ff, 1.0)), (1.0, hex(0x9fcdf8, 1.0))],
            ),
            Glass::frosted(0.92).tint(hex(0x003c94, 1.0)).shadow(0.3),
        );
    }
    let rim = ic.cov_oval(412.0, 300.0, 204.0, 104.0, 0.0);
    ic.glass(&rim, &vg(196.0, 404.0, 0x3c3c40, 0x111113), Glass::default().shadow(0.3).spec(0.6));
    let lens = ic.cov_oval(412.0, 296.0, 156.0, 70.0, 0.0);
    ic.glass(&lens, &vg(226.0, 366.0, 0x8fd0ff, 0xe9f6ff), Glass::default().shadow(0.0).spec(0.8));
}

fn activity(ic: &mut Ic) {
    bg(ic, 0x2b2b2e, 0x0a0a0b);
    let mut grid = Cov::empty(ic.n(), ic.n());
    for i in 1..8 {
        let v = i as f32 * B / 8.0;
        grid = grid
            .union(&ic.cov_rrect(Rect::new(v - 2.5, 0.0, 5.0, B), 0.0))
            .union(&ic.cov_rrect(Rect::new(0.0, v - 2.5, B, 5.0), 0.0));
    }
    ic.fill_color(&grid, hex(0x1f5a32, 0.55));
    let pts = [
        (30.0, 470.0),
        (130.0, 470.0),
        (178.0, 130.0),
        (238.0, 720.0),
        (286.0, 470.0),
        (360.0, 470.0),
        (420.0, 360.0),
        (480.0, 470.0),
        (560.0, 540.0),
        (630.0, 410.0),
        (690.0, 470.0),
        (794.0, 470.0),
    ];
    let path = line(&pts);
    let glow = ic.cov_stroke(&path, 26.0).blur(ic.dev(18.0));
    ic.fill_color(&glow, hex(0x35e06a, 0.65));
    let l = ic.cov_stroke(&path, 22.0);
    ic.glass(&l, &solid(hex(0x3ae86a, 1.0)), Glass::flat().spec(0.5));
}

fn appstore(ic: &mut Ic) {
    bg(ic, 0x2aa4ff, 0x0a59ef);
    let f = |x: f32| x * B;
    let g = Glass::frosted(0.84).tint(hex(0x002a85, 1.0)).shadow(0.26);
    let paint = frost(f(0.15), f(0.85), 0xd9e8ff);
    let w = f(0.085);
    let a = ic.cov_stroke(&line(&[(f(0.545), f(0.175)), (f(0.235), f(0.79))]), w);
    let b = ic.cov_stroke(&line(&[(f(0.455), f(0.175)), (f(0.765), f(0.79))]), w);
    let c = ic.cov_stroke(&line(&[(f(0.17), f(0.625)), (f(0.83), f(0.625))]), w);
    ic.glass(&a, &paint, g);
    ic.glass(&b, &paint, g);
    ic.glass(&c, &paint, g);
}

fn gear(ic: &Ic, cx: f32, cy: f32, r_out: f32, r_in: f32, teeth: usize) -> Cov {
    let mut pb = PathBuilder::new();
    let n = teeth * 4;
    for i in 0..n {
        let a = (i as f32 + 0.5) * 2.0 * PI / n as f32;
        let r = if (i / 2) % 2 == 0 { r_out } else { r_in };
        let (x, y) = (cx + a.cos() * r, cy + a.sin() * r);
        if i == 0 {
            pb.move_to(x, y)
        } else {
            pb.line_to(x, y)
        }
    }
    pb.close();
    match pb.finish() {
        Some(p) => ic.cov(&p).offset(-ic.dev(5.0)).offset(ic.dev(5.0)),
        None => Cov::empty(ic.n(), ic.n()),
    }
}

fn settings(ic: &mut Ic) {
    bg(ic, 0xb9b9bf, 0x7c7c83);
    let (cx, cy) = (B / 2.0, B / 2.0);
    let outer = gear(ic, cx, cy, 336.0, 300.0, 36).minus(&ic.cov_circle(cx, cy, 252.0));
    ic.glass(&outer, &vg(80.0, 744.0, 0xf7f7f9, 0xc9c9ce), Glass::default().shadow(0.35).lift(18.0, 10.0));
    let well = ic.cov_circle(cx, cy, 252.0);
    ic.fill_color(&well, hex(0x5a5a60, 0.55));
    let inner = gear(ic, cx, cy, 236.0, 214.0, 24).minus(&ic.cov_circle(cx, cy, 168.0));
    ic.glass(&inner, &vg(170.0, 650.0, 0xeeeef1, 0xbdbdc3), Glass::default().shadow(0.3).lift(12.0, 6.0));
    let mut spokes = Cov::empty(ic.n(), ic.n());
    for i in 0..3 {
        let a = i as f32 * 2.0 * PI / 3.0 - PI / 2.0 + PI / 3.0;
        spokes =
            spokes.union(&ic.cov_stroke_butt(&line(&[(cx, cy), (cx + a.cos() * 190.0, cy + a.sin() * 190.0)]), 46.0));
    }
    let hub = ic.cov_circle(cx, cy, 74.0);
    ic.glass(&spokes.union(&hub), &vg(220.0, 600.0, 0xf2f2f4, 0xc4c4ca), Glass::default().shadow(0.3).lift(12.0, 6.0));
    let pin = ic.cov_circle(cx, cy, 34.0);
    ic.glass(&pin, &vg(cy - 34.0, cy + 34.0, 0x6a6a70, 0x45454a), Glass::flat().spec(0.4));
}

fn trash(px: u32) -> Pixmap {
    let mut ic = Ic::free(px);
    let (x0, x1, top, bot) = (262.0, 762.0, 230.0, 912.0);
    let mut pb = PathBuilder::new();
    pb.move_to(x0, top);
    pb.line_to(x1, top);
    pb.line_to(x1 - 34.0, bot - 44.0);
    pb.quad_to(x1 - 40.0, bot, x1 - 92.0, bot);
    pb.line_to(x0 + 92.0, bot);
    pb.quad_to(x0 + 40.0, bot, x0 + 34.0, bot - 44.0);
    pb.close();
    let Some(path) = pb.finish() else { return ic.finish(false) };
    let body = ic.cov(&path);
    let glass = lin_grad(
        x0,
        0.0,
        x1,
        0.0,
        &[
            (0.0, rgba(214, 218, 226, 0.62)),
            (0.35, rgba(246, 247, 250, 0.72)),
            (0.7, rgba(236, 238, 243, 0.66)),
            (1.0, rgba(196, 200, 210, 0.62)),
        ],
    );
    ic.glass(&body, &glass, Glass::flat().spec(1.0));
    let mut ribs = Cov::empty(ic.n(), ic.n());
    for i in 0..7 {
        let x = x0 + 72.0 + i as f32 * 59.5;
        let xb = x + (x - 512.0) * -0.07;
        ribs = ribs.union(&ic.cov_stroke(&line(&[(x, top + 70.0), (xb, bot - 60.0)]), 16.0));
    }
    ic.glass(&ribs.intersect(&body), &solid(rgba(255, 255, 255, 0.55)), Glass::flat().spec(0.6));
    let rim = ic.cov_oval(512.0, top, 268.0, 44.0, 0.0);
    let hole = ic.cov_oval(512.0, top + 4.0, 236.0, 28.0, 0.0);
    ic.glass(&rim.clone().minus(&hole), &vg(top - 44.0, top + 44.0, 0xfafbfd, 0xc9ccd4), Glass::flat().spec(1.0));
    ic.fill_color(&hole, rgba(110, 116, 128, 0.42));
    ic.finish(false)
}

fn folder(px: u32, downloads: bool) -> Pixmap {
    let mut ic = Ic::free(px);
    let back = ic
        .cov_rrect(Rect::new(104.0, 196.0, 350.0, 170.0), 42.0)
        .union(&ic.cov_rrect(Rect::new(104.0, 254.0, 816.0, 580.0), 52.0));
    ic.glass(&back, &vg(196.0, 834.0, 0x58b3ef, 0x3f9ce3), Glass::flat().spec(0.6));
    let front = ic.cov_rrect(Rect::new(104.0, 330.0, 816.0, 504.0), 52.0);
    ic.glass(
        &front,
        &vg(330.0, 834.0, 0x9ad8fd, 0x68c0f7),
        Glass::default().tint(hex(0x0b4f8f, 1.0)).shadow(0.3).lift(16.0, -6.0).spec(1.0),
    );
    if downloads {
        let ring = sk_circle_stroke(&ic, 512.0, 590.0, 142.0, 20.0);
        let mut pb = PathBuilder::new();
        pb.move_to(512.0, 505.0);
        pb.line_to(512.0, 665.0);
        pb.move_to(450.0, 608.0);
        pb.line_to(512.0, 670.0);
        pb.line_to(574.0, 608.0);
        let arrow = pb.finish().map(|p| ic.cov_stroke(&p, 24.0)).unwrap_or_else(|| Cov::empty(ic.n(), ic.n()));
        let glyph = ring.union(&arrow);
        let lip = glyph.shift(0.0, ic.dev(4.0)).minus(&glyph);
        ic.fill_color(&lip, rgba(255, 255, 255, 0.5));
        ic.fill_color(&glyph, hex(0x3d9be0, 0.9));
    }
    ic.finish(false)
}
