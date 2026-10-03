//! Styled (Dark / Clear / Tinted) foreground: the "glass glyph".
//!
//! The styled icons are not a colour filter. The foreground layers are
//! merged into one glyph which is then rendered as a piece of glass on the style's
//! plate: in Clear and Tinted the glyph turns monochrome, its tonal detail becoming
//! *opacity* (bright parts dense white, dark parts thin frosted glass) so that complex
//! artwork keeps its structure instead of turning to mush; it is lit like a
//! physical object — a soft shadow lifts it off the plate, a specular edge follows its
//! outline (bright towards the light, a faint bounce opposite), the lower edges sink
//! into a slight inner shade and a sheen runs from top to bottom. Dark keeps the
//! colours but gets the same lighting.
use crate::glass::{specular, Cov};
use crate::look::{Look, Style};
use aqua_gfx::Pixmap;

/// Where the glyph sits.
pub struct Ctx<'a> {
    /// The icon has a plate (otherwise it is free-form: folders, Trash).
    pub plate: bool,
    /// The original plate colour (decides whether dark ink must turn light).
    pub plate_col: Option<[f32; 3]>,
    /// Device pixels per unit of the 1024 icon grid.
    pub unit: f32,
    /// Vertical extent of the body in device pixels (for the sheen).
    pub top: f32,
    pub height: f32,
    /// Optional clip (plate coverage) for everything drawn.
    pub clip: Option<&'a Cov>,
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}
fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}
fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}
fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

/// Whether dark ink on this plate is the glyph (white paper plates: Calendar, Notes…).
pub fn light_plate(plate_col: Option<[f32; 3]>) -> bool {
    plate_col.map(|p| luma(p) > 0.82 && sat(p) < 0.2).unwrap_or(false)
}

fn over(d: &mut [u8], c: [f32; 3], a: f32) {
    if a <= 0.0 {
        return;
    }
    let a = a.min(1.0);
    let inv = 1.0 - a;
    for k in 0..3 {
        d[k] = (c[k].clamp(0.0, 1.0) * a * 255.0 + d[k] as f32 * inv + 0.5).min(255.0) as u8;
    }
    d[3] = (a * 255.0 + d[3] as f32 * inv + 0.5).min(255.0) as u8;
}

/// Composite the merged foreground `fg` (premultiplied, original or Dark-mapped
/// colours) onto `dst` (which already holds the style's plate) as a glass glyph.
pub fn composite(dst: &mut Pixmap, fg: &Pixmap, look: &Look, cx: &Ctx) {
    let (w, h) = (fg.width() as usize, fg.height() as usize);
    let n = w * h;
    let src = fg.data();
    let mut col = vec![[0.0f32; 3]; n];
    let mut cov = Cov::empty(w, h);
    for i in 0..n {
        let a = src[i * 4 + 3] as f32 / 255.0;
        if a <= 0.0 {
            continue;
        }
        cov.a[i] = a;
        col[i] = [
            (src[i * 4] as f32 / 255.0 / a).min(1.0),
            (src[i * 4 + 1] as f32 / 255.0 / a).min(1.0),
            (src[i * 4 + 2] as f32 / 255.0 / a).min(1.0),
        ];
    }
    let Some(_) = cov.bounds() else { return };
    let mono = look.style != Style::Dark;
    let tinted_light = look.style == Style::Tinted && !look.dark;
    let inv = cx.plate && mono && !tinted_light && light_plate(cx.plate_col);

    let mut le: Vec<f32> = col.iter().map(|c| if inv { 1.0 - luma(*c) } else { luma(*c) }).collect();
    if mono && !(tinted_light && cx.plate && light_plate(cx.plate_col)) {
        let mut s: Vec<f32> = (0..n).filter(|&i| cov.a[i] > 0.85).map(|i| le[i]).collect();
        if s.len() >= 16 {
            s.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let q = |p: f32| s[((s.len() - 1) as f32 * p) as usize];
            let (lo, hi) = (q(0.01), q(0.995));
            if hi - lo >= 0.12 {
                let g = 1.0 / (hi - lo).max(0.55);
                for v in le.iter_mut() {
                    *v = (1.0 - (hi - *v) * g).clamp(0.0, 1.0);
                }
            } else {
                le.iter_mut().for_each(|v| *v = 1.0);
            }
        }
    }

    let tint = [look.tint.0, look.tint.1, look.tint.2];
    let white = [1.0f32; 3];
    let material = |t: f32| -> ([f32; 3], f32) {
        match (look.style, look.dark) {
            (Style::Clear, false) => ([1.0, 1.0, 1.0], if cx.plate { 0.42 + 0.58 * t } else { 0.45 + 0.40 * t }),
            (Style::Clear, true) => (
                mix([0.62, 0.64, 0.69], [0.95, 0.955, 0.97], t),
                if cx.plate { 0.26 + 0.68 * t } else { 0.42 + 0.42 * t },
            ),
            (Style::Tinted, true) => (mix(mix(tint, [0.0; 3], 0.25), mix(tint, white, 0.62), t), 0.42 + 0.58 * t),
            (Style::Tinted, false) => {
                if cx.plate {
                    (mix(scale(tint, 0.35), white, 0.12 + 0.88 * t), 0.9 + 0.1 * t)
                } else {
                    (mix(scale(tint, 0.6), mix(tint, white, 0.65), t), 0.92)
                }
            }
            _ => (white, 1.0),
        }
    };

    let u = cx.unit;
    let clip = |i: usize| cx.clip.map(|c| c.a[i]).unwrap_or(1.0);
    let mut dens = Cov::empty(w, h);
    let mut glyph = vec![[0.0f32; 3]; n];
    for i in 0..n {
        if cov.a[i] <= 0.0 {
            continue;
        }
        let (c, o) = if mono { material(le[i]) } else { (col[i], 1.0) };
        glyph[i] = c;
        dens.a[i] = cov.a[i] * o;
    }

    if cx.plate {
        let (k, sc) = match (look.style, look.dark) {
            (Style::Dark, _) => (0.42, [0.0; 3]),
            (Style::Clear, false) => (0.24, [0.04, 0.06, 0.14]),
            (Style::Clear, true) => (0.34, [0.0; 3]),
            (Style::Tinted, false) => (0.14, scale(tint, 0.25)),
            (Style::Tinted, true) => (0.38, [0.0; 3]),
            _ => (0.2, [0.0; 3]),
        };
        let sh = dens.blur((9.0 * u).max(0.8)).shift(0.0, 7.0 * u);
        let d = dst.data_mut();
        for i in 0..n {
            let v = sh.a[i] * k * clip(i) * (1.0 - cov.a[i]);
            if v > 0.002 {
                over(&mut d[i * 4..i * 4 + 4], sc, v);
            }
        }
    }

    let thick = cov.blur((7.0 * u).max(0.8));
    let mass = |i: usize| {
        let t = ((thick.a[i] - 0.45) / 0.4).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let inner = cov.blur((5.0 * u).max(0.6)).shift(-2.0 * u, -5.0 * u);
    let (shade_k, sheen_k) = match look.style {
        Style::Dark => (0.22, 0.10),
        Style::Clear => (0.16, 0.10),
        _ => (0.14, 0.08),
    };
    {
        let d = dst.data_mut();
        for i in 0..n {
            let a = dens.a[i] * clip(i);
            if a <= 0.0 {
                continue;
            }
            let y = (i / w) as f32;
            let t = ((y - cx.top) / cx.height.max(1.0)).clamp(0.0, 1.0);
            let shade = (1.0 - inner.a[i]).clamp(0.0, 1.0) * shade_k * mass(i);
            let k = (1.0 + sheen_k * 0.5 - sheen_k * t) * (1.0 - shade);
            over(&mut d[i * 4..i * 4 + 4], scale(glyph[i], k), a);
        }
    }

    let shape = Cov { a: cov.a.iter().map(|v| ((v - 0.15) / 0.7).clamp(0.0, 1.0)).collect(), ..cov.clone() };
    let spec_k = match (look.style, look.dark) {
        (Style::Clear, false) => 0.75,
        (Style::Clear, true) => 0.6,
        (Style::Tinted, false) => 0.45,
        (Style::Tinted, true) => 0.5,
        (Style::Dark, _) => 0.35,
        _ => 0.4,
    };
    let sp = specular(&shape, (5.0 * u).max(0.75), 1.0, 0.5, 0.06);
    let d = dst.data_mut();
    for i in 0..n {
        let v = sp.a[i] * spec_k * clip(i) * (0.35 + 0.65 * (dens.a[i] / cov.a[i].max(1e-3))) * (0.3 + 0.7 * mass(i));
        if v > 0.002 {
            over(&mut d[i * 4..i * 4 + 4], white, v);
        }
    }
}
