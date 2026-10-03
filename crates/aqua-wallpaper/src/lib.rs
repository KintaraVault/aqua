//! aqua-wallpaper: wallpaper loading (any PNG/JPEG, cover-fit) and a procedural
//! generator reproducing the Aqua style (soft 3D glass columns)
//! so the shell looks right out of the box without shipping branded artwork.

use aqua_gfx::{blur, resize, Pixmap};
use std::path::Path;

#[derive(Clone, Copy, Debug)]
pub enum Palette {
    /// Light blue columns.
    Tahoe,
    /// Warm sand silk.
    Sand,
}

/// Load `path` scaled with "cover" semantics to w×h (physical px).
pub fn load_cover(path: &Path, w: u32, h: u32) -> Option<Pixmap> {
    let src = aqua_gfx::load_image(path)?;
    Some(cover(&src, w, h))
}

pub fn cover(src: &Pixmap, w: u32, h: u32) -> Pixmap {
    let s = (w as f32 / src.width() as f32).max(h as f32 / src.height() as f32);
    let sw = (src.width() as f32 * s).ceil() as u32;
    let sh = (src.height() as f32 * s).ceil() as u32;
    let scaled = resize(src, sw, sh);
    let mut out = Pixmap::new(w, h).unwrap();
    out.draw_pixmap(
        -(((sw - w) / 2) as i32),
        -(((sh - h) / 2) as i32),
        scaled.as_ref(),
        &Default::default(),
        aqua_gfx::tiny_skia::Transform::identity(),
        None,
    );
    out
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}
fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
fn hex(c: u32) -> [f32; 3] {
    [((c >> 16) & 255) as f32 / 255.0, ((c >> 8) & 255) as f32 / 255.0, (c & 255) as f32 / 255.0]
}

/// Generate the procedural wallpaper at w×h physical pixels.
pub fn generate(w: u32, h: u32, palette: Palette) -> Pixmap {
    let f = 3u32;
    let (lw, lh) = ((w / f).max(16), (h / f).max(16));
    let mut pm = Pixmap::new(lw, lh).unwrap();
    let (bg_top, bg_bot, dark, mid, light, hi) = match palette {
        Palette::Tahoe => (hex(0xc9dcf7), hex(0x9bbcf0), hex(0x2f5fd6), hex(0x6f9cf0), hex(0xb9d3fb), hex(0xeaf3ff)),
        Palette::Sand => (hex(0xd9c6ae), hex(0x6e5a4a), hex(0x5a4536), hex(0xb79a7c), hex(0xe9dccb), hex(0xfbf6ef)),
    };
    let aspect = lw as f32 / lh as f32;
    let cols: [(f32, f32, f32); 7] = [
        (0.02, 0.085, 0.30),
        (0.20, 0.090, 0.18),
        (0.40, 0.095, 0.02),
        (0.60, 0.090, 0.28),
        (0.77, 0.085, 0.05),
        (0.93, 0.080, 0.40),
        (1.10, 0.085, 0.10),
    ];
    let tilt = 0.20f32;
    let data = pm.data_mut();
    for py in 0..lh {
        let v = py as f32 / lh as f32;
        for px in 0..lw {
            let u0 = px as f32 / lw as f32;
            let mut col = mix(bg_top, bg_bot, v);
            let blob = (-(((u0 - 0.15) * aspect).powi(2) + (v - 0.15).powi(2)) * 5.0).exp();
            col = mix(col, hi, blob * 0.35);
            for &(cu, hw, top) in cols.iter() {
                let u = u0 + (1.0 - v) * tilt * 0.55 - 0.08;
                let du = (u - cu) / hw;
                let cap_v = top + hw * aspect * 0.9;
                let mut inside = 1.0 - smooth(0.96, 1.04, du.abs());
                if v < cap_v {
                    let dv = (cap_v - v) / (hw * aspect * 0.9);
                    let r = (du * du + dv * dv).sqrt();
                    inside = inside.min(1.0 - smooth(0.96, 1.04, r));
                }
                let sd = ((u - cu - hw * 0.9) / (hw * 0.9)).clamp(-1.0, 3.0);
                let shadow = if sd > 0.0 { (-sd * 1.6).exp() * smooth(top - 0.05, top + 0.25, v) } else { 0.0 };
                col = mix(col, mix(col, dark, 0.55), shadow * 0.35);
                if inside > 0.0 {
                    let t = du.clamp(-1.0, 1.0);
                    let lambert = (1.0 - t * t).max(0.0).sqrt() * 0.75 + (-t) * 0.25;
                    let base = if lambert > 0.5 {
                        mix(mid, light, (lambert - 0.5) * 2.0)
                    } else {
                        mix(dark, mid, lambert * 2.0)
                    };
                    let spec = (-((t + 0.55) * 4.0).powi(2)).exp() * 0.45;
                    let mut c = mix(base, hi, spec);
                    c = mix(c, dark, (v * 0.25) * (1.0 - lambert));
                    col = mix(col, c, inside);
                }
            }
            let idx = ((py * lw + px) * 4) as usize;
            data[idx] = (col[0].clamp(0.0, 1.0) * 255.0) as u8;
            data[idx + 1] = (col[1].clamp(0.0, 1.0) * 255.0) as u8;
            data[idx + 2] = (col[2].clamp(0.0, 1.0) * 255.0) as u8;
            data[idx + 3] = 255;
        }
    }
    blur::blur(&mut pm, 2.5);
    resize(&pm, w, h)
}

/// Night variant used in Dark Mode: deepen tones with a
/// gamma curve while keeping the hue, so highlights still glow against a navy field.
pub fn night(src: &Pixmap) -> Pixmap {
    let mut out = src.clone();
    for px in out.pixels_mut() {
        let a = px.alpha() as f32 / 255.0;
        if a <= 0.0 {
            continue;
        }
        let g = |v: u8| (v as f32 / 255.0 / a).powf(2.0);
        let (mut r, mut gg, mut b) = (g(px.red()) * 0.42, g(px.green()) * 0.47, g(px.blue()) * 0.70);
        let l = 0.2126 * r + 0.7152 * gg + 0.0722 * b;
        r += (l - r) * 0.3;
        gg += (l - gg) * 0.3;
        b += (l - b) * 0.3;
        let q = |c: f32| -> u8 { (c * a * 255.0).clamp(0.0, 255.0) as u8 };
        let (r, g, b) = (q(r), q(gg), q(b));
        *px = aqua_gfx::tiny_skia::PremultipliedColorU8::from_rgba(
            r.min(px.alpha()),
            g.min(px.alpha()),
            b.min(px.alpha()),
            px.alpha(),
        )
        .unwrap();
    }
    out
}

/// Load configured wallpaper or fall back to the procedural one.
pub fn wallpaper(path: Option<&Path>, w: u32, h: u32) -> Pixmap {
    if let Some(p) = path {
        if let Some(pm) = load_cover(p, w, h) {
            return pm;
        }
        eprintln!("aqua: cannot load wallpaper {}", p.display());
    }
    let pal = match std::env::var("AQUA_WALLPAPER_STYLE").as_deref() {
        Ok("sand") => Palette::Sand,
        _ => Palette::Tahoe,
    };
    generate(w, h, pal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_wallpaper_is_opaque_and_sized() {
        for pal in [Palette::Tahoe, Palette::Sand] {
            let pm = generate(64, 40, pal);
            assert_eq!((pm.width(), pm.height()), (64, 40));
            assert!(pm.pixels().iter().all(|p| p.alpha() == 255));
        }
    }

    #[test]
    fn cover_fills_target() {
        let src = generate(100, 50, Palette::Sand);
        for (w, h) in [(40, 40), (200, 50), (30, 90)] {
            let out = cover(&src, w, h);
            assert_eq!((out.width(), out.height()), (w, h));
            assert!(out.pixels().iter().all(|p| p.alpha() == 255), "{w}x{h} has transparent pixels");
        }
    }

    #[test]
    fn missing_file_falls_back_to_generated() {
        let pm = wallpaper(Some(Path::new("/nonexistent/aqua-wallpaper.png")), 32, 20);
        assert_eq!((pm.width(), pm.height()), (32, 20));
        assert!(load_cover(Path::new("/nonexistent.png"), 10, 10).is_none());
    }
}
