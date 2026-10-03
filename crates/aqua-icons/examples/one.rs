//! Render one built-in icon: `one NAME PX STYLE OUT.png` (STYLE as in `sheet`).
use aqua_icons::look::{Look, Style};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let px: u32 = a[2].parse().unwrap();
    let style = a[3].as_str();
    let dark = style == "dark" || style.ends_with("-dark");
    let look = Look {
        style: Style::from_config(style.trim_end_matches("-dark"), dark),
        dark,
        tint: a
            .get(5)
            .map(|t| {
                let v: Vec<f32> = t.split(',').map(|x| x.parse().unwrap()).collect();
                (v[0], v[1], v[2])
            })
            .unwrap_or((0.55, 0.45, 0.85)),
        glass: true,
    };
    let fonts = aqua_gfx::Fonts::load(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts")));
    aqua_icons::builtin::draw_look(&a[1], px, &fonts, &look).unwrap().save_png(&a[4]).unwrap();
}
