//! aqua-gfx: CPU 2D toolkit used by every shell component.
//!
//! * [`Canvas`] – premultiplied RGBA surface (tiny-skia) with HiDPI scale.
//! * [`text`]   – SF Pro text shaping/rasterisation (ab_glyph).
//! * [`shapes`] – rounded rects and "continuous corner" squircles.
//! * [`blur`]   – fast gaussian-approximating blur + saturation (CPU glass fallback).
//! * [`symbols`] – SF-Symbols-like vector glyphs (wifi, battery, search, …).

pub mod blur;
pub mod canvas;
pub mod shapes;
pub mod symbols;
pub mod text;

pub use canvas::Canvas;
pub use text::{Fonts, Weight};
pub use tiny_skia;
pub use tiny_skia::{Color, Pixmap};

/// Simple rect in logical coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w && py < self.y + self.h
    }
    pub fn inset(&self, d: f32) -> Self {
        Self::new(self.x + d, self.y + d, self.w - 2.0 * d, self.h - 2.0 * d)
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn cx(&self) -> f32 {
        self.x + self.w / 2.0
    }
    pub fn cy(&self) -> f32 {
        self.y + self.h / 2.0
    }
    pub fn translate(&self, dx: f32, dy: f32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.w, self.h)
    }
}

pub fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
    Color::from_rgba8(r, g, b, (a.clamp(0.0, 1.0) * 255.0) as u8)
}

/// Load an image file into a premultiplied pixmap.
pub fn load_image(path: &std::path::Path) -> Option<Pixmap> {
    let img = image::open(path).ok()?.to_rgba8();
    from_rgba(img.width(), img.height(), img.as_raw())
}

pub fn load_image_bytes(bytes: &[u8]) -> Option<Pixmap> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    from_rgba(img.width(), img.height(), img.as_raw())
}

/// Straight-alpha RGBA -> premultiplied pixmap.
pub fn from_rgba(w: u32, h: u32, data: &[u8]) -> Option<Pixmap> {
    let mut pm = Pixmap::new(w, h)?;
    for (d, s) in pm.data_mut().as_chunks_mut::<4>().0.iter_mut().zip(data.as_chunks::<4>().0) {
        let a = s[3] as u32;
        d[0] = ((s[0] as u32 * a + 127) / 255) as u8;
        d[1] = ((s[1] as u32 * a + 127) / 255) as u8;
        d[2] = ((s[2] as u32 * a + 127) / 255) as u8;
        d[3] = s[3];
    }
    Some(pm)
}

/// Save pixmap as PNG.
pub fn save_png(pm: &Pixmap, path: &std::path::Path) -> Result<(), String> {
    pm.save_png(path).map_err(|e| e.to_string())
}

/// Resample a pixmap to the given size with high quality filtering.
pub fn resize(src: &Pixmap, w: u32, h: u32) -> Pixmap {
    let (w, h) = (w.max(1), h.max(1));
    if w <= src.width() && h <= src.height() {
        return resample_with(src, w as f32 / src.width() as f32, h as f32 / src.height() as f32, 0.0, 0.0, w, h, true);
    }
    let mut dst = Pixmap::new(w, h).unwrap();
    let sx = w as f32 / src.width() as f32;
    let sy = h as f32 / src.height() as f32;
    dst.draw_pixmap(
        0,
        0,
        src.as_ref(),
        &tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bicubic, ..Default::default() },
        tiny_skia::Transform::from_scale(sx, sy),
        None,
    );
    dst
}

/// Filter taps of one output row/column: (first source index, weights). With `clamp` the
/// weights are normalised over the source pixels only (no transparent fringe at the borders).
fn taps(n_out: u32, n_src: u32, scale: f32, off: f32, clamp: bool) -> Vec<(isize, Vec<f32>)> {
    let r = (1.0 / scale).max(1.0);
    (0..n_out)
        .map(|i| {
            let c = (i as f32 + 0.5 - off) / scale;
            let lo = (c - r).floor() as isize;
            let hi = (c + r).ceil() as isize;
            let mut ws = Vec::with_capacity((hi - lo + 1) as usize);
            let mut sum = 0.0;
            for j in lo..=hi {
                let d = ((j as f32 + 0.5) - c).abs() / r;
                let inside = j >= 0 && (j as u32) < n_src;
                let wv = (1.0 - d).max(0.0);
                if inside || !clamp {
                    sum += wv;
                }
                ws.push(if inside { wv } else { 0.0 });
            }
            if sum > 0.0 {
                ws.iter_mut().for_each(|v| *v /= sum);
            }
            (lo, ws)
        })
        .collect()
}

/// High-quality resampling of a premultiplied pixmap: the source scaled by `sx`,`sy` (destination
/// px per source px) and shifted by the sub-pixel offset `ox`,`oy`, rendered into a `w`×`h` pixmap.
pub fn resample(src: &Pixmap, sx: f32, sy: f32, ox: f32, oy: f32, w: u32, h: u32) -> Pixmap {
    resample_with(src, sx, sy, ox, oy, w, h, false)
}

#[allow(clippy::too_many_arguments)]
fn resample_with(src: &Pixmap, sx: f32, sy: f32, ox: f32, oy: f32, w: u32, h: u32, clamp: bool) -> Pixmap {
    let (sw, shh) = (src.width(), src.height());
    let mut dst = Pixmap::new(w.max(1), h.max(1)).unwrap();
    if sx <= 0.0 || sy <= 0.0 {
        return dst;
    }
    let tx = taps(w, sw, sx, ox, clamp);
    let ty = taps(h, shh, sy, oy, clamp);
    let s = src.data();
    let mut tmp = vec![0.0f32; (w * shh * 4) as usize];
    for y in 0..shh as usize {
        let row = y * sw as usize * 4;
        for (x, (lo, ws)) in tx.iter().enumerate() {
            let mut acc = [0.0f32; 4];
            for (k, wv) in ws.iter().enumerate() {
                if *wv == 0.0 {
                    continue;
                }
                let j = (*lo + k as isize) as usize;
                let p = row + j * 4;
                for c in 0..4 {
                    acc[c] += s[p + c] as f32 * wv;
                }
            }
            let o = (y * w as usize + x) * 4;
            tmp[o..o + 4].copy_from_slice(&acc);
        }
    }
    let d = dst.data_mut();
    for (y, (lo, ws)) in ty.iter().enumerate() {
        for x in 0..w as usize {
            let mut acc = [0.0f32; 4];
            for (k, wv) in ws.iter().enumerate() {
                if *wv == 0.0 {
                    continue;
                }
                let j = (*lo + k as isize) as usize;
                let p = (j * w as usize + x) * 4;
                for c in 0..4 {
                    acc[c] += tmp[p + c] * wv;
                }
            }
            let o = (y * w as usize + x) * 4;
            let a = acc[3].round().clamp(0.0, 255.0);
            d[o + 3] = a as u8;
            for c in 0..3 {
                d[o + c] = acc[c].round().clamp(0.0, a) as u8;
            }
        }
    }
    dst
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, c: [u8; 4]) -> Pixmap {
        let data: Vec<u8> = (0..w * h).flat_map(|_| c).collect();
        from_rgba(w, h, &data).unwrap()
    }

    #[test]
    fn premultiplies() {
        let pm = from_rgba(1, 1, &[255, 128, 0, 128]).unwrap();
        assert_eq!(pm.data(), &[128, 64, 0, 128]);
        assert!(from_rgba(0, 1, &[]).is_none());
    }

    #[test]
    fn resize_keeps_uniform_colour_up_to_the_edges() {
        let src = solid(37, 23, [10, 200, 30, 255]);
        for (w, h) in [(10, 7), (1, 1), (36, 22), (80, 50)] {
            let out = resize(&src, w, h);
            assert_eq!((out.width(), out.height()), (w, h));
            for px in out.pixels() {
                assert_eq!(px.alpha(), 255, "{w}x{h}");
                assert!((px.green() as i32 - 200).abs() <= 2, "{w}x{h}: {px:?}");
            }
        }
        assert_eq!(resize(&src, 0, 0).width(), 1);
    }

    #[test]
    fn rect_helpers() {
        let r = Rect::new(10.0, 20.0, 30.0, 40.0);
        assert!(r.contains(10.0, 20.0) && !r.contains(40.0, 20.0));
        assert_eq!((r.right(), r.bottom(), r.cx(), r.cy()), (40.0, 60.0, 25.0, 40.0));
        let i = r.inset(5.0);
        assert_eq!((i.x, i.y, i.w, i.h), (15.0, 25.0, 20.0, 30.0));
        let t = r.translate(1.0, -1.0);
        assert_eq!((t.x, t.y), (11.0, 19.0));
        assert_eq!(rgba(255, 0, 0, 2.0).alpha(), 1.0);
    }
}
