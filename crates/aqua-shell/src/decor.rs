//! Server-side window decorations: Aqua-style titlebar with traffic lights.
use aqua_config::metrics;
use aqua_gfx::tiny_skia::PathBuilder;
use aqua_gfx::{rgba, Canvas, Color, Fonts, Pixmap, Rect, Weight};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Button {
    Close,
    Minimize,
    Zoom,
}

pub const BUTTONS: [Button; 3] = [Button::Close, Button::Minimize, Button::Zoom];

/// Traffic-light rects relative to the titlebar origin.
pub fn button_rect(b: Button) -> Rect {
    let i = BUTTONS.iter().position(|x| *x == b).unwrap() as f32;
    let d = metrics::TRAFFIC_LIGHT;
    let x = 14.0 + i * (d + metrics::TRAFFIC_GAP);
    Rect::new(x, (metrics::TITLEBAR_HEIGHT - d) / 2.0, d, d)
}

pub fn button_at(x: f32, y: f32) -> Option<Button> {
    BUTTONS.iter().copied().find(|b| button_rect(*b).inset(-3.0).contains(x, y))
}

/// Draw only the traffic lights (also used by CSD-less overlays).
pub fn draw_traffic_lights(c: &mut Canvas, focused: bool, hover: bool) {
    let cols: [(u32, u32); 3] = [(0xff5f57, 0xe2463f), (0xfebc2e, 0xe1a116), (0x28c840, 0x1aab29)];
    for (i, b) in BUTTONS.iter().enumerate() {
        let r = button_rect(*b);
        let (fill, edge) = if focused || hover { cols[i] } else { (0xd9d9dc, 0xc6c6c9) };
        let h = |c: u32| rgba((c >> 16) as u8, (c >> 8) as u8, c as u8, 1.0);
        c.fill_circle(r.cx(), r.cy(), r.w / 2.0, h(edge));
        c.fill_circle(r.cx(), r.cy(), r.w / 2.0 - 0.6, h(fill));
        if hover {
            let g = rgba(0, 0, 0, 0.55);
            let mut pb = PathBuilder::new();
            let k = r.w * 0.26;
            match b {
                Button::Close => {
                    pb.move_to(r.cx() - k, r.cy() - k);
                    pb.line_to(r.cx() + k, r.cy() + k);
                    pb.move_to(r.cx() + k, r.cy() - k);
                    pb.line_to(r.cx() - k, r.cy() + k);
                }
                Button::Minimize => {
                    pb.move_to(r.cx() - k * 1.2, r.cy());
                    pb.line_to(r.cx() + k * 1.2, r.cy());
                }
                Button::Zoom => {
                    let k = r.w * 0.3;
                    let mut t = PathBuilder::new();
                    t.move_to(r.cx() - k, r.cy() - k);
                    t.line_to(r.cx() + k * 0.45, r.cy() - k);
                    t.line_to(r.cx() - k, r.cy() + k * 0.45);
                    t.close();
                    t.move_to(r.cx() + k, r.cy() + k);
                    t.line_to(r.cx() - k * 0.45, r.cy() + k);
                    t.line_to(r.cx() + k, r.cy() - k * 0.45);
                    t.close();
                    if let Some(p) = t.finish() {
                        c.fill_path(&p, &aqua_gfx::canvas::solid(g));
                    }
                }
            }
            if let Some(p) = pb.finish() {
                c.stroke_path(&p, &aqua_gfx::canvas::solid(g), 1.4);
            }
        }
    }
}

/// Titlebar content (transparent background – the compositor draws glass behind it).
pub fn titlebar(fonts: &Fonts, w: f32, scale: f32, title: &str, focused: bool, hover: bool, dark: bool) -> Pixmap {
    let h = metrics::TITLEBAR_HEIGHT;
    let mut c = Canvas::new(w, h, scale);
    draw_traffic_lights(&mut c, focused, hover);
    let col: Color = match (dark, focused) {
        (false, true) => rgba(38, 38, 40, 0.88),
        (false, false) => rgba(38, 38, 40, 0.42),
        (true, true) => rgba(255, 255, 255, 0.86),
        (true, false) => rgba(255, 255, 255, 0.40),
    };
    let left = 14.0 + 3.0 * (metrics::TRAFFIC_LIGHT + metrics::TRAFFIC_GAP) + 10.0;
    let avail = w - 2.0 * left;
    if avail > 30.0 {
        c.text_in(fonts, Rect::new(left, 0.0, avail, h), 0.5, 13.5, Weight::Semibold, col, title);
    }
    c.fill_rect(
        Rect::new(0.0, h - 1.0 / scale, w, 1.0 / scale),
        if dark { rgba(0, 0, 0, 0.45) } else { rgba(0, 0, 0, 0.08) },
    );
    c.pm
}
