//! Glass icon material (CPU).
//!
//! icons are built from a background and one or more foreground layers made
//! of "glass": every layer gets a soft per-layer shadow that lifts it off the
//! background, a frosted (slightly translucent) fill and a specular edge highlight that
//! follows its outline — bright where the edge faces the light (top left), a fainter
//! bounce on the opposite side. The plate itself carries the same rim light.
//!
//! Everything here works on float coverage maps ([`Cov`]) so the effects follow any
//! shape exactly. Icons are drawn supersampled ([`Ic`]) and box-filtered down, which
//! gives the smooth edges of hand-drawn artwork even at Dock sizes.
use aqua_gfx::tiny_skia::{self as sk, FillRule, IntSize, Mask, Paint, Path, Pixmap, Stroke, Transform};
use aqua_gfx::{shapes, Color, Rect};

/// Width of the visible plate on the 1024 grid.
pub const B: f32 = 824.0;
/// Continuous-corner radius of the plate (relative to its width).
pub const RADIUS: f32 = 0.275;
/// Direction the light comes from (unit vector, pointing at the light): top, a bit left.
pub const LIGHT: (f32, f32) = (-0.42, -0.91);

/// The plate shape for a body rect.
pub fn plate_path(r: Rect) -> Option<Path> {
    shapes::squircle(r, r.w.min(r.h) * RADIUS)
}

/// Coverage map (0..1 per device pixel).
#[derive(Clone)]
pub struct Cov {
    pub w: usize,
    pub h: usize,
    pub a: Vec<f32>,
}

impl Cov {
    pub fn empty(w: usize, h: usize) -> Self {
        Self { w, h, a: vec![0.0; w * h] }
    }
    pub fn full(w: usize, h: usize) -> Self {
        Self { w, h, a: vec![1.0; w * h] }
    }
    pub fn from_mask(m: &Mask) -> Self {
        Self { w: m.width() as usize, h: m.height() as usize, a: m.data().iter().map(|&v| v as f32 / 255.0).collect() }
    }
    pub fn path(w: usize, h: usize, p: &Path, ts: Transform) -> Self {
        let Some(mut m) = Mask::new(w as u32, h as u32) else { return Self::empty(w, h) };
        m.fill_path(p, FillRule::Winding, true, ts);
        Self::from_mask(&m)
    }
    pub fn path_eo(w: usize, h: usize, p: &Path, ts: Transform) -> Self {
        let Some(mut m) = Mask::new(w as u32, h as u32) else { return Self::empty(w, h) };
        m.fill_path(p, FillRule::EvenOdd, true, ts);
        Self::from_mask(&m)
    }
    /// Alpha channel of a pixmap.
    pub fn alpha(pm: &Pixmap) -> Self {
        Self {
            w: pm.width() as usize,
            h: pm.height() as usize,
            a: pm.data().as_chunks::<4>().0.iter().map(|p| p[3] as f32 / 255.0).collect(),
        }
    }
    pub fn to_mask(&self) -> Option<Mask> {
        let d: Vec<u8> = self.a.iter().map(|&v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8).collect();
        Mask::from_vec(d, IntSize::from_wh(self.w as u32, self.h as u32)?)
    }
    pub fn at(&self, x: isize, y: isize) -> f32 {
        if x < 0 || y < 0 || x >= self.w as isize || y >= self.h as isize {
            0.0
        } else {
            self.a[y as usize * self.w + x as usize]
        }
    }
    pub fn scale(mut self, k: f32) -> Self {
        self.a.iter_mut().for_each(|v| *v *= k);
        self
    }
    /// Union (screen) of two coverages.
    pub fn union(mut self, o: &Cov) -> Self {
        for (a, b) in self.a.iter_mut().zip(&o.a) {
            *a = *a + *b - *a * *b;
        }
        self
    }
    /// Coverage of both `self` and `o` (product).
    pub fn intersect(mut self, o: &Cov) -> Self {
        for (a, b) in self.a.iter_mut().zip(&o.a) {
            *a *= *b;
        }
        self
    }
    /// `self` minus `o` (cut out).
    pub fn minus(mut self, o: &Cov) -> Self {
        for (a, b) in self.a.iter_mut().zip(&o.a) {
            *a *= 1.0 - *b;
        }
        self
    }
    /// Sub-pixel shift (bilinear).
    pub fn shift(&self, dx: f32, dy: f32) -> Self {
        let Some((x0, y0, x1, y1)) = self.bounds() else { return self.clone() };
        let mut out = Self::empty(self.w, self.h);
        let (ix, iy) = (dx.floor(), dy.floor());
        let (fx, fy) = (dx - ix, dy - iy);
        let (ix, iy) = (ix as isize, iy as isize);
        let clampx = |v: isize| v.clamp(0, self.w as isize) as usize;
        let clampy = |v: isize| v.clamp(0, self.h as isize) as usize;
        for y in clampy(y0 as isize + iy)..clampy(y1 as isize + iy + 1) {
            for x in clampx(x0 as isize + ix)..clampx(x1 as isize + ix + 1) {
                let (sx, sy) = (x as isize - ix, y as isize - iy);
                let v = self.at(sx, sy) * (1.0 - fx) * (1.0 - fy)
                    + self.at(sx - 1, sy) * fx * (1.0 - fy)
                    + self.at(sx, sy - 1) * (1.0 - fx) * fy
                    + self.at(sx - 1, sy - 1) * fx * fy;
                out.a[y * self.w + x] = v;
            }
        }
        out
    }
    /// Bounding box of the non-zero coverage (x0, y0, x1, y1 exclusive).
    pub fn bounds(&self) -> Option<(usize, usize, usize, usize)> {
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        for y in 0..self.h {
            let row = &self.a[y * self.w..(y + 1) * self.w];
            let Some(f) = row.iter().position(|v| *v > 0.0) else { continue };
            let l = row.iter().rposition(|v| *v > 0.0).unwrap_or(f);
            x0 = x0.min(f);
            x1 = x1.max(l + 1);
            y0 = y0.min(y);
            y1 = y + 1;
        }
        (x0 < x1).then_some((x0, y0, x1, y1))
    }
    fn crop(&self, x0: usize, y0: usize, x1: usize, y1: usize) -> Self {
        let w = x1 - x0;
        let mut a = Vec::with_capacity(w * (y1 - y0));
        for y in y0..y1 {
            a.extend_from_slice(&self.a[y * self.w + x0..y * self.w + x1]);
        }
        Self { w, h: y1 - y0, a }
    }
    fn paste(&self, w: usize, h: usize, x0: usize, y0: usize) -> Self {
        let mut out = Self::empty(w, h);
        for y in 0..self.h {
            out.a[(y0 + y) * w + x0..(y0 + y) * w + x0 + self.w].copy_from_slice(&self.a[y * self.w..(y + 1) * self.w]);
        }
        out
    }
    /// Run `f` on the part of the map around the shape (padded by `pad`), zero elsewhere.
    fn local(&self, pad: usize, f: impl FnOnce(&Cov) -> Cov) -> Self {
        let Some((x0, y0, x1, y1)) = self.bounds() else { return Self::empty(self.w, self.h) };
        let (x0, y0) = (x0.saturating_sub(pad), y0.saturating_sub(pad));
        let (x1, y1) = ((x1 + pad).min(self.w), (y1 + pad).min(self.h));
        f(&self.crop(x0, y0, x1, y1)).paste(self.w, self.h, x0, y0)
    }
    /// Gaussian blur (exact kernel for small sigma, three box passes above).
    pub fn blur(&self, sigma: f32) -> Self {
        if sigma < 0.3 {
            return self.clone();
        }
        self.local((sigma * 3.2).ceil() as usize + 2, |c| c.blur_full(sigma))
    }
    fn blur_full(&self, sigma: f32) -> Self {
        if sigma < 0.3 {
            return self.clone();
        }
        let mut a = self.a.clone();
        let mut tmp = vec![0.0f32; a.len()];
        if sigma <= 3.0 {
            let r = (sigma * 3.0).ceil() as isize;
            let k: Vec<f32> = (-r..=r).map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp()).collect();
            let ks: f32 = k.iter().sum();
            let k: Vec<f32> = k.iter().map(|v| v / ks).collect();
            conv(&a, &mut tmp, self.w, self.h, &k, true);
            conv(&tmp, &mut a, self.w, self.h, &k, false);
        } else {
            let bw = ((4.0 * sigma * sigma + 1.0).sqrt()).round().max(1.0) as usize;
            let r = (bw / 2).max(1);
            for _ in 0..3 {
                boxp(&a, &mut tmp, self.w, self.h, r, true);
                boxp(&tmp, &mut a, self.w, self.h, r, false);
            }
        }
        Self { w: self.w, h: self.h, a }
    }
    /// Morphological-ish grow / shrink by `d` device px (blur + re-threshold).
    pub fn offset(&self, d: f32) -> Self {
        let sigma = d.abs().max(0.5);
        let b = self.blur(sigma);
        let t = (0.5 - d / (sigma * 2.5066)).clamp(0.02, 0.98);
        let soft = 0.5 / (sigma * 2.5066).max(1.0);
        let mut out = b;
        for v in out.a.iter_mut() {
            *v = ((*v - t) / soft.max(0.05) * 0.5 + 0.5).clamp(0.0, 1.0);
        }
        out
    }
}

fn conv(src: &[f32], dst: &mut [f32], w: usize, h: usize, k: &[f32], horiz: bool) {
    let r = k.len() / 2;
    if horiz {
        for y in 0..h {
            let row = &src[y * w..(y + 1) * w];
            let out = &mut dst[y * w..(y + 1) * w];
            for x in 0..w {
                let lo = x.saturating_sub(r);
                let hi = (x + r + 1).min(w);
                let k0 = lo + r - x;
                let mut acc = 0.0;
                for (j, v) in row[lo..hi].iter().enumerate() {
                    acc += v * k[k0 + j];
                }
                out[x] = acc;
            }
        }
    } else {
        dst.iter_mut().for_each(|v| *v = 0.0);
        for y in 0..h {
            let out = y * w;
            for (i, kv) in k.iter().enumerate() {
                let sy = y as isize + i as isize - r as isize;
                if sy < 0 || sy >= h as isize {
                    continue;
                }
                let srow = &src[sy as usize * w..(sy as usize + 1) * w];
                for (d, s) in dst[out..out + w].iter_mut().zip(srow) {
                    *d += s * kv;
                }
            }
        }
    }
}

fn boxp(src: &[f32], dst: &mut [f32], w: usize, h: usize, r: usize, horiz: bool) {
    let (n, lines) = if horiz { (w, h) } else { (h, w) };
    let idx = |line: usize, i: usize| if horiz { line * w + i } else { i * w + line };
    let div = (2 * r + 1) as f32;
    for line in 0..lines {
        let mut acc = 0.0f32;
        for i in 0..=r.min(n - 1) {
            acc += src[idx(line, i)];
        }
        for i in 0..n {
            dst[idx(line, i)] = acc / div;
            if i + r + 1 < n {
                acc += src[idx(line, i + r + 1)];
            }
            if i >= r {
                acc -= src[idx(line, i - r)];
            }
        }
    }
}

/// Specular rim of a shape: a thin band just inside its outline whose brightness
/// depends on how the edge faces the light — `lit` where it faces it, `back` on the
/// opposite side (light bouncing inside the glass) and `amb` everywhere.
pub fn specular(m: &Cov, width: f32, lit: f32, back: f32, amb: f32) -> Cov {
    m.local((width * 2.0).ceil() as usize + 3, |c| specular_full(c, width, lit, back, amb))
}

fn specular_full(m: &Cov, width: f32, lit: f32, back: f32, amb: f32) -> Cov {
    let sigma = (width * 0.5).max(0.45);
    let g = m.blur(sigma);
    let norm = sigma * 2.5066;
    let mut out = Cov::empty(m.w, m.h);
    let (lx, ly) = LIGHT;
    for y in 0..m.h as isize {
        for x in 0..m.w as isize {
            let i = y as usize * m.w + x as usize;
            let mv = m.a[i];
            if mv <= 0.0 {
                continue;
            }
            let gx = (g.at(x + 1, y) - g.at(x - 1, y)) * 0.5;
            let gy = (g.at(x, y + 1) - g.at(x, y - 1)) * 0.5;
            let mag = (gx * gx + gy * gy).sqrt();
            if mag < 1e-5 {
                continue;
            }
            let e = (mag * norm).min(1.0);
            let (nx, ny) = (-gx / mag, -gy / mag);
            let d = nx * lx + ny * ly;
            let l = d.max(0.0).powf(1.4);
            let bk = (-d).max(0.0).powf(1.4);
            out.a[i] = (mv * e * e * (amb + lit * l + back * bk)).min(1.0);
        }
    }
    out
}

/// A layer's material.
#[derive(Clone, Copy, Debug)]
pub struct Glass {
    /// Drop shadow under the layer (alpha), its blur and offset in grid units.
    pub shadow: f32,
    pub shadow_blur: f32,
    pub shadow_dy: f32,
    pub shadow_color: Color,
    /// Specular rim strength.
    pub spec: f32,
    /// Fill opacity (frosted glass lets the background show through a little).
    pub opacity: f32,
    /// Rim width in grid units.
    pub rim: f32,
}

impl Default for Glass {
    fn default() -> Self {
        Self {
            shadow: 0.22,
            shadow_blur: 16.0,
            shadow_dy: 9.0,
            shadow_color: Color::BLACK,
            spec: 0.85,
            opacity: 1.0,
            rim: 7.0,
        }
    }
}

impl Glass {
    pub fn flat() -> Self {
        Self { shadow: 0.0, spec: 0.0, ..Self::default() }
    }
    pub fn frosted(opacity: f32) -> Self {
        Self { opacity, ..Self::default() }
    }
    pub fn shadow(mut self, a: f32) -> Self {
        self.shadow = a;
        self
    }
    pub fn spec(mut self, s: f32) -> Self {
        self.spec = s;
        self
    }
    pub fn tint(mut self, c: Color) -> Self {
        self.shadow_color = c;
        self
    }
    pub fn lift(mut self, blur: f32, dy: f32) -> Self {
        self.shadow_blur = blur;
        self.shadow_dy = dy;
        self
    }
}

/// An icon being drawn: a (supersampled) 1024-grid canvas plus the plate coverage.
pub struct Ic {
    pub pm: Pixmap,
    /// Device pixels per grid unit.
    pub s: f32,
    /// Supersampling factor.
    pub ss: u32,
    pub px: u32,
    /// Grid offset of the drawing coordinates (100 for plate icons: body coords 0..824).
    pub off: f32,
    /// Plate coverage (or everything, for free-form icons).
    pub body: Cov,
    plate: bool,
    /// Non-default icon style (Dark / Clear / Tinted): drawn natively, layer by layer.
    style: Option<Look>,
    /// The original plate colour (average of the background gradient).
    plate_col: Option<[f32; 3]>,
    /// Scratch canvas for recolouring a layer before it is composited.
    scratch: Option<Pixmap>,
    /// Bypass the style mapping (drawing the style's own plate).
    raw: bool,
    /// Styled looks: the merged foreground, turned into a glass glyph in [`Ic::finish`].
    fg: Option<Pixmap>,
}

use crate::look::{Look, Style};

/// What recolouring a foreground layer needs to know about the icon.
#[derive(Clone, Copy)]
struct GlyphCtx {
    style: Option<Look>,
    plate: bool,
    plate_col: Option<[f32; 3]>,
}

impl GlyphCtx {
    /// Recolour one foreground pixel (un-premultiplied colour, alpha) for the style.
    fn map(&self, c: [f32; 3], a: f32) -> ([f32; 3], f32) {
        let Some(l) = self.style else { return (c, a) };
        let tint = [l.tint.0, l.tint.1, l.tint.2];
        let white = [1.0f32; 3];
        let lu = luma3(c);
        let light_plate = !self.plate || self.plate_col.map(|p| luma3(p) > 0.82 && sat3(p) < 0.2).unwrap_or(false);
        let light_plate = light_plate && self.plate;
        let inv = light_plate && (l.dark || l.style != Style::Tinted);
        let le = if inv { 1.0 - lu } else { lu };
        match (l.style, l.dark) {
            (Style::Dark, _) => {
                let sc = sat3(c);
                if light_plate {
                    if sc > 0.28 {
                        (mix3(c, white, 0.06), a)
                    } else {
                        let g = 0.20 + 0.76 * le;
                        ([g, g, g * 1.02], a)
                    }
                } else if let Some(pl) = self.plate_col.filter(|p| sat3(*p) > 0.22) {
                    let mx = pl[0].max(pl[1]).max(pl[2]).max(0.05);
                    let vivid = mix3([pl[0] / mx, pl[1] / mx, pl[2] / mx], white, 0.1);
                    let k = sstep(0.55, 0.85, lu) * (1.0 - sstep(0.12, 0.32, sc));
                    let v = [vivid[0] * (0.7 + 0.3 * lu), vivid[1] * (0.7 + 0.3 * lu), vivid[2] * (0.7 + 0.3 * lu)];
                    let o = mix3(c, v, k);
                    let o = mix3(o, [0.93, 0.93, 0.95], sstep(0.3, 0.1, lu) * (1.0 - sstep(0.2, 0.4, sc)));
                    (o, a)
                } else {
                    (c, a)
                }
            }
            (Style::Tinted, true) => (
                mix3(
                    [tint[0] * 0.42 + 0.04, tint[1] * 0.42 + 0.04, tint[2] * 0.42 + 0.05],
                    mix3(tint, white, 0.62),
                    sstep(0.0, 1.0, le),
                ),
                a,
            ),
            (Style::Tinted, false) if !self.plate => {
                (mix3([tint[0] * 0.6, tint[1] * 0.6, tint[2] * 0.6], mix3(tint, white, 0.65), le), a)
            }
            (Style::Tinted, false) => {
                (mix3([tint[0] * 0.35, tint[1] * 0.35, tint[2] * 0.35], white, 0.15 + 0.85 * le), a)
            }
            (Style::Clear, true) => {
                let g = 0.42 + 0.56 * le;
                ([g, g, g * 1.01], a * 0.92)
            }
            (Style::Clear, false) => {
                let g = 0.50 + 0.50 * le;
                ([g, g, g * 1.01], a * 0.90)
            }
            (Style::Default, _) => (c, a),
        }
    }
}

thread_local! {
    /// Style the next built-in icons are drawn in (see [`with_look`]).
    static LOOK: std::cell::Cell<Option<Look>> = const { std::cell::Cell::new(None) };
}

/// Draw built-in icons inside `f` in the given look.
pub fn with_look<T>(look: &Look, f: impl FnOnce() -> T) -> T {
    let prev = LOOK.with(|c| c.replace((look.style != Style::Default).then_some(*look)));
    let r = f();
    LOOK.with(|c| c.set(prev));
    r
}

fn luma3(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}
fn sat3(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}
fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}
fn sstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Ic {
    fn with(px: u32, plate: bool) -> Self {
        let ss = if px <= 160 {
            3
        } else if px <= 520 {
            2
        } else {
            1
        };
        let n = px * ss;
        let s = n as f32 / 1024.0;
        let off = if plate { 100.0 } else { 0.0 };
        let pm = Pixmap::new(n, n).expect("icon alloc");
        let ts = Transform::from_scale(s, s).pre_translate(off, off);
        let body = if plate {
            plate_cache(n, |n| {
                plate_path(Rect::new(0.0, 0.0, B, B))
                    .map(|p| Cov::path(n, n, &p, ts))
                    .unwrap_or_else(|| Cov::full(n, n))
            })
            .0
        } else {
            Cov::full(n as usize, n as usize)
        };
        let style = LOOK.with(|c| c.get());
        let fg = style.map(|_| Pixmap::new(n, n).expect("icon alloc"));
        Self { pm, s, ss, px, off, body, plate, style, plate_col: None, scratch: None, raw: false, fg }
    }

    /// Plate colour and alpha of the style at relative height `t` (0 top … 1 bottom).
    fn style_plate(&self, t: f32) -> ([f32; 3], f32) {
        let Some(l) = self.style else { return ([1.0; 3], 1.0) };
        let tint = [l.tint.0, l.tint.1, l.tint.2];
        let white = [1.0f32; 3];
        match (l.style, l.dark) {
            (Style::Dark, _) => (mix3([0.215, 0.215, 0.23], [0.085, 0.085, 0.095], t), 1.0),
            (Style::Tinted, true) => (
                mix3(
                    [tint[0] * 0.30 + 0.05, tint[1] * 0.30 + 0.05, tint[2] * 0.30 + 0.06],
                    [tint[0] * 0.13 + 0.025, tint[1] * 0.13 + 0.025, tint[2] * 0.13 + 0.03],
                    t,
                ),
                1.0,
            ),
            (Style::Tinted, false) => (mix3(mix3(tint, white, 0.30), mix3(tint, [0.0; 3], 0.06), t), 1.0),
            (Style::Clear, true) => (mix3([0.17, 0.18, 0.21], [0.06, 0.065, 0.08], t), 0.52 + 0.14 * t),
            (Style::Clear, false) => (white, 0.17 - 0.07 * t),
            (Style::Default, _) => (white, 1.0),
        }
    }

    /// Icon on the plate; drawing coordinates are body coords (0..824).
    pub fn plate(px: u32) -> Self {
        Self::with(px, true)
    }
    /// Free-form icon (Trash, folders); drawing coordinates are the 1024 grid.
    pub fn free(px: u32) -> Self {
        Self::with(px, false)
    }
    pub fn ts(&self) -> Transform {
        Transform::from_scale(self.s, self.s).pre_translate(self.off, self.off)
    }
    pub fn n(&self) -> usize {
        self.pm.width() as usize
    }
    /// Grid units → device pixels.
    pub fn dev(&self, v: f32) -> f32 {
        v * self.s
    }
    pub fn cov(&self, p: &Path) -> Cov {
        Cov::path(self.n(), self.n(), p, self.ts())
    }
    pub fn cov_eo(&self, p: &Path) -> Cov {
        Cov::path_eo(self.n(), self.n(), p, self.ts())
    }
    pub fn cov_stroke(&self, p: &Path, width: f32) -> Cov {
        let st = Stroke { width, line_cap: sk::LineCap::Round, line_join: sk::LineJoin::Round, ..Default::default() };
        match p.stroke(&st, self.s) {
            Some(sp) => self.cov(&sp),
            None => Cov::empty(self.n(), self.n()),
        }
    }
    pub fn cov_stroke_butt(&self, p: &Path, width: f32) -> Cov {
        let st = Stroke { width, line_cap: sk::LineCap::Butt, line_join: sk::LineJoin::Round, ..Default::default() };
        match p.stroke(&st, self.s) {
            Some(sp) => self.cov(&sp),
            None => Cov::empty(self.n(), self.n()),
        }
    }
    pub fn cov_rrect(&self, r: Rect, radius: f32) -> Cov {
        shapes::rrect(r, radius).map(|p| self.cov(&p)).unwrap_or_else(|| Cov::empty(self.n(), self.n()))
    }
    pub fn cov_squircle(&self, r: Rect, radius: f32) -> Cov {
        shapes::squircle(r, radius).map(|p| self.cov(&p)).unwrap_or_else(|| Cov::empty(self.n(), self.n()))
    }
    pub fn cov_circle(&self, cx: f32, cy: f32, r: f32) -> Cov {
        sk::PathBuilder::from_circle(cx, cy, r).map(|p| self.cov(&p)).unwrap_or_else(|| Cov::empty(self.n(), self.n()))
    }
    pub fn cov_oval(&self, cx: f32, cy: f32, rx: f32, ry: f32, rot_deg: f32) -> Cov {
        let mut pb = sk::PathBuilder::new();
        if let Some(r) = sk::Rect::from_xywh(cx - rx, cy - ry, rx * 2.0, ry * 2.0) {
            pb.push_oval(r);
        }
        match pb.finish().and_then(|p| p.transform(Transform::from_rotate_at(rot_deg, cx, cy))) {
            Some(p) => self.cov(&p),
            None => Cov::empty(self.n(), self.n()),
        }
    }
    pub fn cov_poly(&self, pts: &[(f32, f32)]) -> Cov {
        let mut pb = sk::PathBuilder::new();
        for (i, (x, y)) in pts.iter().enumerate() {
            if i == 0 {
                pb.move_to(*x, *y)
            } else {
                pb.line_to(*x, *y)
            }
        }
        pb.close();
        pb.finish().map(|p| self.cov(&p)).unwrap_or_else(|| Cov::empty(self.n(), self.n()))
    }
    /// Text as coverage (drawing coords; `rect` horizontally aligned by `align` 0..1).
    pub fn cov_text(
        &self,
        fonts: &aqua_gfx::Fonts,
        r: Rect,
        align: f32,
        size: f32,
        w: aqua_gfx::Weight,
        text: &str,
    ) -> Cov {
        let mut c = aqua_gfx::Canvas::from_pixmap(Pixmap::new(self.n() as u32, self.n() as u32).unwrap(), self.s);
        c.text_in(fonts, r.translate(self.off, self.off), align, size, w, Color::WHITE, text);
        Cov::alpha(&c.pm)
    }
    /// Fill `cov` with `paint` (paint coordinates are drawing coords).
    pub fn fill(&mut self, cov: &Cov, paint: &Paint) {
        if self.style.is_none() {
            return self.fill_raw(cov, paint);
        }
        let Some((x0, y0, x1, y1)) = cov.bounds() else { return };
        let Some(m) = cov.to_mask() else { return };
        let n = self.n();
        let mut sc = self.scratch.take().unwrap_or_else(|| Pixmap::new(n as u32, n as u32).expect("icon alloc"));
        let (s, o) = (self.s, self.off);
        let mut p = paint.clone();
        p.blend_mode = sk::BlendMode::SourceOver;
        if let Some(r) = sk::Rect::from_ltrb(x0 as f32 / s - o, y0 as f32 / s - o, x1 as f32 / s - o, y1 as f32 / s - o)
        {
            sc.fill_rect(r, &p, self.ts(), Some(&m));
        }
        let dark = self.style.map(|l| l.style == Style::Dark).unwrap_or(false);
        let mut fgp = self.fg.take().expect("styled icon has a foreground");
        {
            let src = sc.data_mut();
            let dst = fgp.data_mut();
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = (y * n + x) * 4;
                    let a = src[i + 3] as f32 / 255.0;
                    if a <= 0.0 {
                        continue;
                    }
                    let c = [src[i] as f32 / 255.0 / a, src[i + 1] as f32 / 255.0 / a, src[i + 2] as f32 / 255.0 / a];
                    src[i..i + 4].copy_from_slice(&[0, 0, 0, 0]);
                    let c = [c[0].min(1.0), c[1].min(1.0), c[2].min(1.0)];
                    let (c, a) =
                        if dark { Self::glyph_of(self.style, self.plate, self.plate_col, c, a) } else { (c, a) };
                    let inv = 1.0 - a;
                    for k in 0..3 {
                        dst[i + k] = (c[k] * a * 255.0 + dst[i + k] as f32 * inv + 0.5).min(255.0) as u8;
                    }
                    dst[i + 3] = (a * 255.0 + dst[i + 3] as f32 * inv + 0.5).min(255.0) as u8;
                }
            }
        }
        self.scratch = Some(sc);
        self.fg = Some(fgp);
    }

    fn glyph_of(style: Option<Look>, plate: bool, plate_col: Option<[f32; 3]>, c: [f32; 3], a: f32) -> ([f32; 3], f32) {
        GlyphCtx { style, plate, plate_col }.map(c, a)
    }

    /// Forget the plate colour: the foreground is a full-colour picture (a Linux theme
    /// icon on our plate), not ink on paper — its greys must not be inverted.
    pub fn forget_plate(&mut self) {
        self.plate_col = None;
    }
    /// A bitmap as a foreground layer, placed in `r` (drawing coords).
    pub fn layer_pixmap(&mut self, src: &Pixmap, r: Rect) {
        let n = self.n();
        let (dx, dy) = (((r.x + self.off) * self.s).round() as i64, ((r.y + self.off) * self.s).round() as i64);
        let (dw, dh) = ((r.w * self.s).round().max(1.0) as u32, (r.h * self.s).round().max(1.0) as u32);
        let img = aqua_gfx::resize(src, dw, dh);
        let dark = self.style.map(|l| l.style == Style::Dark).unwrap_or(false);
        let (style, plate, plate_col) = (self.style, self.plate, self.plate_col);
        let target = match self.fg.as_mut() {
            Some(f) => f,
            None => &mut self.pm,
        };
        let sd = img.data();
        let d = target.data_mut();
        for y in 0..dh as i64 {
            let ty = dy + y;
            if ty < 0 || ty >= n as i64 {
                continue;
            }
            for x in 0..dw as i64 {
                let tx = dx + x;
                if tx < 0 || tx >= n as i64 {
                    continue;
                }
                let si = ((y as usize) * dw as usize + x as usize) * 4;
                let a = sd[si + 3] as f32 / 255.0;
                if a <= 0.0 {
                    continue;
                }
                let c = [
                    (sd[si] as f32 / 255.0 / a).min(1.0),
                    (sd[si + 1] as f32 / 255.0 / a).min(1.0),
                    (sd[si + 2] as f32 / 255.0 / a).min(1.0),
                ];
                let (c, a) = if dark { GlyphCtx { style, plate, plate_col }.map(c, a) } else { (c, a) };
                let i = (ty as usize * n + tx as usize) * 4;
                let inv = 1.0 - a;
                for k in 0..3 {
                    d[i + k] = (c[k] * a * 255.0 + d[i + k] as f32 * inv + 0.5).min(255.0) as u8;
                }
                d[i + 3] = (a * 255.0 + d[i + 3] as f32 * inv + 0.5).min(255.0) as u8;
            }
        }
    }
    /// Fill without any style mapping (shadows, highlights).
    pub fn fill_raw(&mut self, cov: &Cov, paint: &Paint) {
        let Some((x0, y0, x1, y1)) = cov.bounds() else { return };
        let Some(m) = cov.to_mask() else { return };
        if self.fg.is_some() && !self.raw {
            let n = self.n();
            let mut sc = self.scratch.take().unwrap_or_else(|| Pixmap::new(n as u32, n as u32).expect("icon alloc"));
            let (s, o) = (self.s, self.off);
            if let Some(r) =
                sk::Rect::from_ltrb(x0 as f32 / s - o, y0 as f32 / s - o, x1 as f32 / s - o, y1 as f32 / s - o)
            {
                sc.fill_rect(r, paint, self.ts(), Some(&m));
            }
            let Some(fgp) = self.fg.as_mut() else {
                self.scratch = Some(sc);
                return;
            };
            let src = sc.data_mut();
            let dst = fgp.data_mut();
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = (y * n + x) * 4;
                    let sa = src[i + 3] as f32 / 255.0;
                    if sa <= 0.0 {
                        continue;
                    }
                    let da = dst[i + 3] as f32 / 255.0;
                    for k in 0..3 {
                        dst[i + k] = (src[i + k] as f32 * da + dst[i + k] as f32 * (1.0 - sa) + 0.5).min(255.0) as u8;
                    }
                    src[i..i + 4].copy_from_slice(&[0, 0, 0, 0]);
                }
            }
            self.scratch = Some(sc);
            return;
        }
        let (s, o) = (self.s, self.off);
        if let Some(r) = sk::Rect::from_ltrb(x0 as f32 / s - o, y0 as f32 / s - o, x1 as f32 / s - o, y1 as f32 / s - o)
        {
            let ts = self.ts();
            self.pm.fill_rect(r, paint, ts, Some(&m));
        }
    }
    /// Vertical two-colour gradient between `y0` and `y1` (drawing coords) through `cov`.
    pub fn fill_vgrad(&mut self, cov: &Cov, y0: f32, y1: f32, top: Color, bottom: Color) {
        let n = self.n();
        let Some((bx0, by0, bx1, by1)) = cov.bounds() else { return };
        let styled = self.style.is_some() && !self.raw;
        let dark = self.style.map(|l| l.style == Style::Dark).unwrap_or(false);
        let (style, plate, plate_col) = (self.style, self.plate, self.plate_col);
        let d = if styled { self.fg.as_mut().unwrap().data_mut() } else { self.pm.data_mut() };
        for y in by0..by1 {
            let gy = (y as f32 + 0.5) / self.s - self.off;
            let t = ((gy - y0) / (y1 - y0).max(1e-3)).clamp(0.0, 1.0);
            let c = [
                top.red() + (bottom.red() - top.red()) * t,
                top.green() + (bottom.green() - top.green()) * t,
                top.blue() + (bottom.blue() - top.blue()) * t,
                top.alpha() + (bottom.alpha() - top.alpha()) * t,
            ];
            let c = if styled && dark {
                let (m, a) = GlyphCtx { style, plate, plate_col }.map([c[0], c[1], c[2]], c[3]);
                [m[0], m[1], m[2], a]
            } else {
                c
            };
            for x in bx0..bx1 {
                let i = y * n + x;
                let a = cov.a[i] * c[3];
                if a <= 0.0 {
                    continue;
                }
                let p = &mut d[i * 4..i * 4 + 4];
                let inv = 1.0 - a;
                for k in 0..3 {
                    p[k] = (c[k] * a * 255.0 + p[k] as f32 * inv + 0.5).min(255.0) as u8;
                }
                p[3] = (a * 255.0 + p[3] as f32 * inv + 0.5).min(255.0) as u8;
            }
        }
    }
    pub fn fill_color(&mut self, cov: &Cov, c: Color) {
        self.fill(cov, &solid(c));
    }
    /// Background gradient over the whole plate.
    pub fn bg(&mut self, paint: &Paint) {
        if self.style.is_some() {
            return self.bg_v(Color::WHITE, Color::WHITE);
        }
        let body = self.body.clone();
        self.fill(&body, paint);
    }
    /// Background: vertical gradient over the whole plate (in a styled look: the
    /// style's plate; the original colour is remembered for the foreground mapping).
    pub fn bg_v(&mut self, top: Color, bottom: Color) {
        let body = std::mem::replace(&mut self.body, Cov::empty(0, 0));
        if self.style.is_some() {
            self.plate_col = Some([
                (top.red() + bottom.red()) * 0.5,
                (top.green() + bottom.green()) * 0.5,
                (top.blue() + bottom.blue()) * 0.5,
            ]);
            let (t, ta) = self.style_plate(0.0);
            let (b, ba) = self.style_plate(1.0);
            self.raw = true;
            self.fill_vgrad(
                &body,
                0.0,
                B,
                Color::from_rgba(t[0], t[1], t[2], ta).unwrap_or(Color::WHITE),
                Color::from_rgba(b[0], b[1], b[2], ba).unwrap_or(Color::WHITE),
            );
            self.raw = false;
        } else {
            self.fill_vgrad(&body, 0.0, B, top, bottom);
        }
        self.body = body;
    }
    /// A glass layer.
    pub fn glass(&mut self, cov: &Cov, paint: &Paint, g: Glass) {
        if g.shadow > 0.0 {
            let sh = cov.blur(self.dev(g.shadow_blur) * 0.5).shift(0.0, self.dev(g.shadow_dy)).scale(g.shadow);
            let c = if self.style.is_some() { Color::BLACK } else { g.shadow_color };
            self.fill_raw(&sh, &solid(c));
        }
        let fill = if g.opacity < 1.0 { cov.clone().scale(g.opacity) } else { cov.clone() };
        self.fill(&fill, paint);
        if g.spec > 0.0 {
            let w = self.dev(g.rim).max(0.8 * self.ss as f32);
            let k = match self.style.map(|l| l.style) {
                Some(Style::Clear) => 1.25,
                Some(Style::Dark) => 0.8,
                _ => 1.0,
            };
            let sp = specular(cov, w, 1.0, 0.55, 0.16).scale(g.spec * k);
            self.fill_raw(&sp, &solid(Color::WHITE));
        }
    }
    /// Finish: plate rim light, clip to the plate, downsample, contact shadow.
    pub fn finish(mut self, rim: bool) -> Pixmap {
        if let (Some(fg), Some(look)) = (self.fg.take(), self.style) {
            let (top, height) = if self.plate { (self.off * self.s, B * self.s) } else { (0.0, self.n() as f32) };
            let cx = crate::glyph::Ctx {
                plate: self.plate,
                plate_col: self.plate_col,
                unit: self.s,
                top,
                height,
                clip: None,
            };
            crate::glyph::composite(&mut self.pm, &fg, &look, &cx);
        }
        self.raw = true;
        if self.plate {
            if rim {
                let n = self.n() as u32;
                let cached = plate_cache(n, |_| Cov::empty(0, 0)).1;
                let ov = cached.unwrap_or_else(|| {
                    let w = self.dev(8.0).max(0.9 * self.ss as f32);
                    let mut ov = specular(&self.body, w, 0.85, 0.5, 0.2);
                    let (cx, cy, r) = (B * 0.5, -B * 0.1, B * 0.75);
                    for y in 0..ov.h {
                        for x in 0..ov.w {
                            let (gx, gy) = ((x as f32 + 0.5) / self.s - self.off, (y as f32 + 0.5) / self.s - self.off);
                            let d = ((gx - cx).powi(2) + (gy - cy).powi(2)).sqrt() / r;
                            if d < 1.0 {
                                let i = y * ov.w + x;
                                let g = 0.14 * (1.0 - d) * self.body.a[i];
                                ov.a[i] = ov.a[i] + g - ov.a[i] * g;
                            }
                        }
                    }
                    PLATES.with(|c| c.borrow_mut().get_mut(&n).map(|e| e.1 = Some(ov.clone())));
                    ov
                });
                let ov = match self.style.map(|l| l.style) {
                    Some(Style::Clear) => ov.scale(1.4),
                    _ => ov,
                };
                self.fill_raw(&ov, &solid(Color::WHITE));
            }
            clip(&mut self.pm, &self.body);
        }
        let pm = downsample(&self.pm, self.ss);
        crate::normalize::drop_shadow(&pm, self.plate)
    }
}

thread_local! {
    /// Plate coverage and its rim light per canvas size (identical for every icon).
    static PLATES: std::cell::RefCell<std::collections::HashMap<u32, (Cov, Option<Cov>)>> = Default::default();
}

fn plate_cache(n: u32, make: impl FnOnce(usize) -> Cov) -> (Cov, Option<Cov>) {
    PLATES.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 24 {
            c.clear();
        }
        c.entry(n).or_insert_with(|| (make(n as usize), None)).clone()
    })
}

pub fn solid(c: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(c);
    p.anti_alias = true;
    p
}

/// Multiply a pixmap by a coverage.
pub fn clip(pm: &mut Pixmap, cov: &Cov) {
    for (px, a) in pm.data_mut().as_chunks_mut::<4>().0.iter_mut().zip(&cov.a) {
        if *a >= 1.0 {
            continue;
        }
        for c in px.iter_mut() {
            *c = (*c as f32 * a + 0.5) as u8;
        }
    }
}

/// Box-filter a supersampled pixmap down by an integer factor.
pub fn downsample(pm: &Pixmap, k: u32) -> Pixmap {
    if k <= 1 {
        return pm.clone();
    }
    let (w, h) = (pm.width() / k, pm.height() / k);
    let mut out = Pixmap::new(w.max(1), h.max(1)).unwrap();
    let src = pm.data();
    let sw = pm.width() as usize;
    let n = k * k;
    let od = out.data_mut();
    for y in 0..h as usize {
        for x in 0..w as usize {
            let mut acc = [0u32; 4];
            for yy in 0..k as usize {
                for xx in 0..k as usize {
                    let i = ((y * k as usize + yy) * sw + x * k as usize + xx) * 4;
                    for c in 0..4 {
                        acc[c] += src[i + c] as u32;
                    }
                }
            }
            let o = (y * w as usize + x) * 4;
            for c in 0..4 {
                od[o + c] = ((acc[c] + n / 2) / n) as u8;
            }
        }
    }
    out
}
