//! Renders every built-in icon to a contact sheet for visual review:
//! `cargo run --release -p aqua-icons --example sheet -- out.png [px] [style]`
use aqua_icons::look::{apply, Look, Style};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = args.get(1).cloned().unwrap_or_else(|| "/tmp/sheet.png".into());
    let px: u32 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(128);
    let style = args.get(3).map(|s| s.as_str()).unwrap_or("default");
    let fonts = aqua_gfx::Fonts::load(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts")));
    let names = [
        "finder",
        "launchpad",
        "safari",
        "messages",
        "mail",
        "maps",
        "photos",
        "facetime",
        "calendar",
        "reminders",
        "notes",
        "music",
        "appstore",
        "settings",
        "terminal",
        "calculator",
        "textedit",
        "preview",
        "activity",
        "downloads",
        "trash",
    ];
    let cols = 7u32;
    let rows = (names.len() as u32).div_ceil(cols) + 1;
    let cell = px + px / 6;
    let (w, h) = (cell * cols, cell * rows);
    let mut sheet = aqua_gfx::Pixmap::new(w, h).unwrap();
    let bgp = aqua_gfx::canvas::lin_grad(
        0.0,
        0.0,
        w as f32,
        h as f32,
        &[
            (0.0, aqua_gfx::rgba(196, 170, 140, 1.0)),
            (0.5, aqua_gfx::rgba(120, 110, 130, 1.0)),
            (1.0, aqua_gfx::rgba(88, 100, 140, 1.0)),
        ],
    );
    sheet.fill_rect(
        aqua_gfx::tiny_skia::Rect::from_xywh(0.0, 0.0, w as f32, h as f32).unwrap(),
        &bgp,
        aqua_gfx::tiny_skia::Transform::identity(),
        None,
    );
    let dark = style == "dark" || style.ends_with("-dark");
    let st = Style::from_config(style.trim_end_matches("-dark"), dark);
    let look = Look { style: st, dark, tint: (0.55, 0.45, 0.85), glass: true };
    let t0 = std::time::Instant::now();
    for (i, n) in names.iter().enumerate() {
        let t = std::time::Instant::now();
        let mut pm = aqua_icons::builtin::draw_look(n, px, &fonts, &look).unwrap();
        if std::env::var("T").is_ok() {
            eprintln!("{n}: {:?}", t.elapsed());
        }
        apply(&mut pm, &look, true);
        let (x, y) = ((i as u32 % cols) * cell + px / 12, (i as u32 / cols) * cell + px / 12);
        sheet.draw_pixmap(
            x as i32,
            y as i32,
            pm.as_ref(),
            &Default::default(),
            aqua_gfx::tiny_skia::Transform::identity(),
            None,
        );
    }
    eprintln!("{} icons at {px}px in {:?}", names.len(), t0.elapsed());
    let mono = aqua_icons::builtin::monogram_look("Krita", px, &fonts, &look);
    let y = (rows - 1) * cell + px / 12;
    sheet.draw_pixmap(
        (px / 12) as i32,
        y as i32,
        mono.as_ref(),
        &Default::default(),
        aqua_gfx::tiny_skia::Transform::identity(),
        None,
    );
    let mut glyph = aqua_gfx::Pixmap::new(256, 256).unwrap();
    let mut pb = aqua_gfx::tiny_skia::PathBuilder::new();
    pb.push_circle(128.0, 128.0, 120.0);
    glyph.fill_path(
        &pb.finish().unwrap(),
        &aqua_gfx::canvas::solid(aqua_gfx::rgba(40, 160, 225, 1.0)),
        aqua_gfx::tiny_skia::FillRule::Winding,
        aqua_gfx::tiny_skia::Transform::identity(),
        None,
    );
    let mut plated = aqua_icons::normalize::plate(&glyph, px);
    apply(&mut plated, &look, false);
    sheet.draw_pixmap(
        (cell + px / 12) as i32,
        y as i32,
        plated.as_ref(),
        &Default::default(),
        aqua_gfx::tiny_skia::Transform::identity(),
        None,
    );
    sheet.save_png(&out).unwrap();
}
