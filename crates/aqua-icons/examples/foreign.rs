//! Foreign icons in every style: a Linux theme glyph on our plate (native styled path,
//! and the generic restyle), plus bitmaps run through the restyle.
use aqua_gfx::tiny_skia::{FillRule, PathBuilder, Transform};
use aqua_icons::look::{apply, Look, Style};

fn monitor_glyph() -> aqua_gfx::Pixmap {
    let mut g = aqua_gfx::Pixmap::new(256, 256).unwrap();
    let id = Transform::identity();
    let circ = |r: f32| PathBuilder::from_circle(128.0, 128.0, r).unwrap();
    g.fill_path(&circ(122.0), &aqua_gfx::canvas::solid(aqua_gfx::rgba(40, 44, 52, 1.0)), FillRule::Winding, id, None);
    g.fill_path(
        &circ(108.0),
        &aqua_gfx::canvas::lin_grad(
            0.0,
            20.0,
            0.0,
            236.0,
            &[(0.0, aqua_gfx::rgba(250, 250, 252, 1.0)), (1.0, aqua_gfx::rgba(205, 208, 214, 1.0))],
        ),
        FillRule::Winding,
        id,
        None,
    );
    let band = aqua_gfx::tiny_skia::Rect::from_xywh(34.0, 100.0, 188.0, 56.0).unwrap();
    g.fill_rect(
        band,
        &aqua_gfx::canvas::lin_grad(
            0.0,
            100.0,
            0.0,
            156.0,
            &[(0.0, aqua_gfx::rgba(90, 94, 102, 1.0)), (1.0, aqua_gfx::rgba(60, 63, 70, 1.0))],
        ),
        id,
        None,
    );
    for k in 0..15 {
        let x = 42.0 + k as f32 * 12.0;
        let hgt = [18.0, 30.0, 44.0, 26.0, 60.0, 36.0, 22.0, 50.0, 70.0, 28.0, 40.0, 64.0, 24.0, 34.0, 20.0][k];
        let r = aqua_gfx::tiny_skia::Rect::from_xywh(x, 128.0 - hgt / 2.0, 6.0, hgt).unwrap();
        g.fill_rect(r, &aqua_gfx::canvas::solid(aqua_gfx::rgba(185, 190, 198, 1.0)), id, None);
    }
    g
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "/tmp/foreign.png".into());
    let px: u32 = std::env::args().nth(2).and_then(|v| v.parse().ok()).unwrap_or(128);
    let fonts = aqua_gfx::Fonts::load(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts")));
    let glyph = monitor_glyph();
    let mac = ["activity", "settings", "calendar", "finder", "photos"];
    let looks =
        [("default", false), ("dark", true), ("clear", false), ("clear", true), ("tinted", false), ("tinted", true)];
    let cols = 2 + mac.len() as u32;
    let cell = px + px / 6;
    let mut sheet = aqua_gfx::Pixmap::new(cell * cols, cell * looks.len() as u32).unwrap();
    let bg = aqua_gfx::canvas::lin_grad(
        0.0,
        0.0,
        (cell * cols) as f32,
        0.0,
        &[(0.0, aqua_gfx::rgba(120, 160, 230, 1.0)), (1.0, aqua_gfx::rgba(70, 110, 200, 1.0))],
    );
    sheet.fill_rect(
        aqua_gfx::tiny_skia::Rect::from_xywh(0.0, 0.0, sheet.width() as f32, sheet.height() as f32).unwrap(),
        &bg,
        Transform::identity(),
        None,
    );
    for (r, (st, dark)) in looks.iter().enumerate() {
        let look = Look { style: Style::from_config(st, *dark), dark: *dark, tint: (0.55, 0.45, 0.85), glass: true };
        let mut row: Vec<aqua_gfx::Pixmap> = vec![];
        row.push(if look.style == Style::Default {
            aqua_icons::normalize::plate(&glyph, px)
        } else {
            aqua_icons::normalize::plate_look(&glyph, px, &look)
        });
        let k = if px <= 160 {
            3
        } else if px <= 520 {
            2
        } else {
            1
        };
        let mut p = aqua_icons::normalize::plate(&glyph, px * k);
        apply(&mut p, &look, false);
        row.push(aqua_icons::glass::downsample(&p, k));
        for n in mac {
            let mut p = aqua_icons::builtin::draw(n, px * k, &fonts).unwrap();
            apply(&mut p, &look, false);
            row.push(aqua_icons::glass::downsample(&p, k));
        }
        for (c, p) in row.iter().enumerate() {
            sheet.draw_pixmap(
                (c as u32 * cell + px / 12) as i32,
                (r as u32 * cell + px / 12) as i32,
                p.as_ref(),
                &Default::default(),
                Transform::identity(),
                None,
            );
        }
    }
    sheet.save_png(&out).unwrap();
}
