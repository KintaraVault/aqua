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

/// Left inset of the first traffic light (macOS 26 titled window: AX frame x=8, 16 pt hit
/// box, 14 pt visible disc → disc starts at 9 pt).
pub const TRAFFIC_INSET: f32 = 9.0;

/// Traffic-light rects relative to the titlebar origin.
pub fn button_rect(b: Button) -> Rect {
    let i = BUTTONS.iter().position(|x| *x == b).unwrap() as f32;
    let d = metrics::TRAFFIC_LIGHT;
    let x = TRAFFIC_INSET + i * (d + metrics::TRAFFIC_GAP);
    Rect::new(x, (metrics::TITLEBAR_HEIGHT - d) / 2.0, d, d)
}

pub fn button_at(x: f32, y: f32) -> Option<Button> {
    BUTTONS.iter().copied().find(|b| button_rect(*b).inset(-3.0).contains(x, y))
}

/// Draw only the traffic lights.
///
/// Colours are sampled from macOS 26 window captures (`assets/mac/ui/apps/*/…-win0-noshadow.png`):
/// active discs are flat sRGB fills with a 1 px darker rim (slightly deeper in dark mode);
/// inactive discs are a black overlay (light ≈ 14 %/18 % rim, dark ≈ 80 %/100 % rim).
/// `glass` is kept for config compatibility; macOS 26 itself draws them flat.
pub fn draw_traffic_lights(c: &mut Canvas, focused: bool, hover: bool, dark: bool, glass: bool) {
    let _ = glass;
    // (fill, rim light, rim dark)
    let cols: [(u32, u32, u32); 3] =
        [(0xff5c5f, 0xff3236, 0xec1f23), (0xfac800, 0xf8b600, 0xf2b000), (0x34c759, 0x01b92f, 0x00a61c)];
    let h = |c: u32, a: f32| rgba((c >> 16) as u8, (c >> 8) as u8, c as u8, a);
    for (i, b) in BUTTONS.iter().enumerate() {
        let r = button_rect(*b);
        let lit = focused || hover;
        let (fill, rim) = if lit {
            let (f, rl, rd) = cols[i];
            (h(f, 1.0), h(if dark { rd } else { rl }, 1.0))
        } else if dark {
            (rgba(0, 0, 0, 0.80), rgba(0, 0, 0, 1.0))
        } else {
            (rgba(0, 0, 0, 0.14), rgba(0, 0, 0, 0.18))
        };
        if lit {
            c.fill_circle(r.cx(), r.cy(), r.w / 2.0, rim);
            c.fill_circle(r.cx(), r.cy(), r.w / 2.0 - 1.0, fill);
        } else {
            // Translucent: draw rim ring and fill without overlap so alphas don't stack.
            c.fill_circle(r.cx(), r.cy(), r.w / 2.0, fill);
            let mut pb = PathBuilder::new();
            pb.push_circle(r.cx(), r.cy(), r.w / 2.0 - 0.5);
            if let Some(p) = pb.finish() {
                let a = (rim.alpha() - fill.alpha()).max(0.0) / (1.0 - fill.alpha()).max(0.01);
                c.stroke_path(&p, &aqua_gfx::canvas::solid(rgba(0, 0, 0, a)), 1.0);
            }
        }
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
pub fn titlebar(fonts: &Fonts, w: f32, scale: f32, title: &str, focused: bool, hover: bool, dark: bool, glass: bool) -> Pixmap {
    let h = metrics::TITLEBAR_HEIGHT;
    let mut c = Canvas::new(w, h, scale);
    draw_traffic_lights(&mut c, focused, hover, dark, glass);
    let col: Color = match (dark, focused) {
        (false, true) => rgba(38, 38, 40, 0.88),
        (false, false) => rgba(38, 38, 40, 0.42),
        (true, true) => rgba(255, 255, 255, 0.86),
        (true, false) => rgba(255, 255, 255, 0.40),
    };
    // macOS 26 (TextEdit capture): title starts 15 pt after the zoom disc, left-aligned.
    let left = TRAFFIC_INSET + 3.0 * metrics::TRAFFIC_LIGHT + 2.0 * metrics::TRAFFIC_GAP + 15.0;
    let avail = w - left - 12.0;
    // No separator line: macOS 26 titlebars blend into the content.
    if avail > 30.0 {
        c.text_in(fonts, Rect::new(left, 0.0, avail, h), 0.0, 13.0, Weight::Bold, col, title);
    }
    c.pm
}
