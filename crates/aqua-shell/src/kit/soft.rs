//! Software compositor for previews, tests and as reference implementation of
//! the liquid-glass material (the GL shaders in `aqua-render` mirror this).
use crate::Layer;
use aqua_config::GlassStyle;
use aqua_gfx::canvas::solid;
use aqua_gfx::tiny_skia::{FillRule, Mask, Pixmap, PixmapPaint, Transform};
use aqua_gfx::{blur, shapes, Rect};

/// A client window for the preview: rect (logical) + content pixmap (physical).
pub struct SoftWindow {
    pub rect: Rect,
    pub content: Pixmap,
    pub radius: f32,
}

pub fn drop_shadow(dst: &mut Pixmap, r: Rect, radius: f32, scale: f32, alpha: f32, blur_r: f32, dy: f32) {
    if alpha <= 0.0 {
        return;
    }
    let pad = blur_r * 2.0;
    let mut sh = Pixmap::new(((r.w + 2.0 * pad) * scale) as u32, ((r.h + 2.0 * pad) * scale) as u32).unwrap();
    if let Some(p) = shapes::squircle(Rect::new(pad, pad, r.w, r.h), radius) {
        sh.fill_path(
            &p,
            &solid(aqua_gfx::rgba(0, 0, 0, alpha)),
            FillRule::Winding,
            Transform::from_scale(scale, scale),
            None,
        );
    }
    blur::blur(&mut sh, blur_r * scale);
    dst.draw_pixmap(
        ((r.x - pad) * scale) as i32,
        ((r.y - pad + dy) * scale) as i32,
        sh.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        None,
    );
}

/// Draw glass for rect `r` reading the current contents of `dst` as backdrop: a CPU port of
/// the Liquid Glass shader (`aqua-render/src/shaders/glass.frag`), same model and constants.
pub fn glass(dst: &mut Pixmap, r: Rect, g: &GlassStyle, scale: f32, opacity: f32) {
    drop_shadow(dst, r, g.radius, scale, g.shadow * opacity, 18.0, 6.0);
    let (x, y) = ((r.x * scale) as i32, (r.y * scale) as i32);
    let (w, h) = ((r.w * scale).ceil() as u32, (r.h * scale).ceil() as u32);
    let (w, h) = (w.max(1), h.max(1));
    let mut sharp = Pixmap::new(w, h).unwrap();
    sharp.draw_pixmap(-x, -y, dst.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
    let blurred = if g.blur * scale >= 0.5 { blur::blur_fast(&sharp, g.blur * scale) } else { sharp.clone() };
    let mut out = Pixmap::new(w, h).unwrap();
    let m = LensModel::new(g, w as f32, h as f32, scale);
    let fetch = |pm: &Pixmap, fx: f32, fy: f32| -> [f32; 3] {
        // bilinear, clamped to the captured area like the shader's uv clamp
        let sx = (fx * pm.width() as f32 / w as f32 - 0.5).clamp(0.0, pm.width() as f32 - 1.0);
        let sy = (fy * pm.height() as f32 / h as f32 - 0.5).clamp(0.0, pm.height() as f32 - 1.0);
        let (x0, y0) = (sx.floor() as u32, sy.floor() as u32);
        let (x1, y1) = ((x0 + 1).min(pm.width() - 1), (y0 + 1).min(pm.height() - 1));
        let (tx, ty) = (sx - x0 as f32, sy - y0 as f32);
        let px = |x: u32, y: u32| {
            let p = pm.pixel(x, y).unwrap().demultiply();
            [p.red() as f32 / 255.0, p.green() as f32 / 255.0, p.blue() as f32 / 255.0]
        };
        let (a, b, c, d) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
        std::array::from_fn(|i| {
            let top = a[i] + (b[i] - a[i]) * tx;
            let bot = c[i] + (d[i] - c[i]) * tx;
            top + (bot - top) * ty
        })
    };
    let data = out.data_mut();
    for py in 0..h {
        for px in 0..w {
            let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
            let Some((col, cov)) = m.shade(fx, fy, |dx, dy, sharp_tap| {
                if sharp_tap {
                    fetch(&sharp, fx + dx, fy + dy)
                } else {
                    fetch(&blurred, fx + dx, fy + dy)
                }
            }) else {
                continue;
            };
            let a = cov * opacity;
            let i = ((py * w + px) * 4) as usize;
            data[i] = (col[0] * a * 255.0).round() as u8;
            data[i + 1] = (col[1] * a * 255.0).round() as u8;
            data[i + 2] = (col[2] * a * 255.0).round() as u8;
            data[i + 3] = (a * 255.0).round() as u8;
        }
    }
    dst.draw_pixmap(x, y, out.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
}

/// Per-shape constants of the Liquid Glass model (physical px).
struct LensModel {
    hb: (f32, f32),
    extent: f32,
    n: f32,
    thick: f32,
    inv_ior: f32,
    refraction: f32,
    dispersion: f32,
    sharp_rim: bool,
    saturation: f32,
    max_luma: f32,
    tint: [f32; 4],
    rim: f32,
    fresnel: (f32, f32, f32),
    glare: (f32, f32, f32, f32),
    opposite: f32,
    angle: f32,
    fresnel_lch: [f32; 3],
}

const NORMAL_GAIN: f32 = 1.25;

impl LensModel {
    fn new(g: &GlassStyle, w: f32, h: f32, scale: f32) -> Self {
        let (extent, n) = aqua_config::material::corner(w, h, g.radius * scale, g.roundness);
        let band = |range: f32| (500.0 / range.max(0.5)).powi(2) / 1500.0 / scale;
        let a = g.tint.3 * 0.5;
        let f = [1.0 + (g.tint.0 - 1.0) * a, 1.0 + (g.tint.1 - 1.0) * a, 1.0 + (g.tint.2 - 1.0) * a];
        let plain = g.refraction <= 0.0 && (g.rim <= 0.0 || (g.fresnel <= 0.0 && g.glare <= 0.0));
        Self {
            hb: (w * 0.5, h * 0.5),
            extent,
            n,
            thick: if plain { 0.0 } else { g.thickness * scale },
            inv_ior: 1.0 / g.ior.max(1.0),
            refraction: g.refraction * scale,
            dispersion: g.dispersion,
            sharp_rim: !g.blur_edge,
            saturation: g.saturation,
            max_luma: g.max_luma,
            tint: [g.tint.0, g.tint.1, g.tint.2, g.tint.3],
            rim: g.rim,
            fresnel: (band(g.fresnel_range), g.fresnel_hardness, g.fresnel),
            glare: (band(g.glare_range), g.glare_hardness, g.glare, g.glare_convergence),
            opposite: g.glare_opposite,
            angle: g.glare_angle.to_radians(),
            fresnel_lch: lch::from_srgb(f),
        }
    }

    fn sdf(&self, px: f32, py: f32) -> (f32, f32, f32) {
        let (sx, sy) = (if px < 0.0 { -1.0 } else { 1.0 }, if py < 0.0 { -1.0 } else { 1.0 });
        let (ax, ay) = (px.abs(), py.abs());
        let (dx, dy) = (ax - self.hb.0, ay - self.hb.1);
        let cr = self.extent;
        if cr > 0.0 && dx > -cr && dy > -cr {
            let (qx, qy) = ((ax - (self.hb.0 - cr)).max(0.0), (ay - (self.hb.1 - cr)).max(0.0));
            let m = qx.max(qy);
            if m < 1e-4 {
                return (-cr, 0.0, 0.0);
            }
            let v = m * ((qx / m).powf(self.n) + (qy / m).powf(self.n)).powf(1.0 / self.n);
            let gx = (qx / v).powf(self.n - 1.0) * sx;
            let gy = (qy / v).powf(self.n - 1.0) * sy;
            return (v - cr, gx, gy);
        }
        let dist = dx.max(dy).min(0.0) + (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt();
        if dx > dy {
            (dist, sx, 0.0)
        } else {
            (dist, 0.0, sy)
        }
    }

    fn grade(&self, c: [f32; 3]) -> [f32; 3] {
        let luma = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let l = luma(c);
        let mut c = c.map(|v| (l + (v - l) * self.saturation).clamp(0.0, 1.0));
        if self.max_luma < 0.999 {
            let l = luma(c);
            let knee = self.max_luma * 0.7;
            let span = (self.max_luma - knee).max(1e-3);
            let lc = if l < knee { l } else { knee + span * (1.0 - (-(l - knee) / span).exp()) };
            let k = lc / l.max(1e-4);
            c = c.map(|v| v * k);
            let sat = 1.0 + 0.9 * (1.0 - k);
            c = c.map(|v| (lc + (v - lc) * sat).clamp(0.0, 1.0));
        }
        c
    }

    /// Colour (straight alpha) and coverage at pixel centre (`fx`, `fy`); `tap(dx, dy, sharp)`
    /// reads the backdrop displaced by (`dx`, `dy`).
    fn shade(&self, fx: f32, fy: f32, tap: impl Fn(f32, f32, bool) -> [f32; 3]) -> Option<([f32; 3], f32)> {
        let (d, gx, gy) = self.sdf(fx - self.hb.0, fy - self.hb.1);
        let cov = (0.5 - d).clamp(0.0, 1.0);
        if cov <= 0.0 {
            return None;
        }
        let mix = |a: [f32; 3], b: [f32; 3], t: f32| -> [f32; 3] { std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t) };
        let tint = [self.tint[0], self.tint[1], self.tint[2]];
        let depth = (-d).max(0.0);
        if depth >= self.thick {
            let c = self.grade(tap(0.0, 0.0, false));
            return Some((mix(c, tint, self.tint[3]), cov));
        }
        let glen = (gx * gx + gy * gy).sqrt();
        let x = 1.0 - depth / self.thick;
        let ti = (x * x).clamp(0.0, 1.0).asin();
        let tt = (self.inv_ior * ti.sin()).clamp(-1.0, 1.0).asin();
        let edge = -(tt - ti).tan();
        let (dx, dy) = (-gx * edge * self.refraction, -gy * edge * self.refraction);
        let k = 0.02 * self.dispersion;
        let chan = |sharp: bool| -> [f32; 3] {
            [tap(dx * (1.0 + k), dy * (1.0 + k), sharp)[0], tap(dx, dy, sharp)[1], tap(dx * (1.0 - k), dy * (1.0 - k), sharp)[2]]
        };
        let bl = chan(false);
        let back = if self.sharp_rim { mix(chan(true), bl, depth / self.thick) } else { bl };
        let mut col = mix(self.grade(back), tint, self.tint[3]);
        let gain = self.rim * glen * NORMAL_GAIN;
        if gain > 0.0 {
            let (fk, fh, ff) = self.fresnel;
            let f = (1.0 - depth * fk + fh).max(0.0).powi(5).clamp(0.0, 1.0);
            if f * ff > 0.002 {
                let mut l = self.fresnel_lch;
                l[0] = (l[0] + 20.0 * f * ff).clamp(0.0, 100.0);
                col = mix(col, lch::to_srgb(l), (f * ff * 0.7 * gain).clamp(0.0, 1.0));
            }
            let (gk, gh, gf, gc) = self.glare;
            let geo = (1.0 - depth * gk + gh).max(0.0).powi(5).clamp(0.0, 1.0);
            if geo * gf > 0.002 && glen > 1e-4 {
                use std::f32::consts::PI;
                let mut th = (-gy / glen).atan2(gx / glen);
                if th < 0.0 {
                    th += 2.0 * PI;
                }
                let ga = (th - PI * 0.25 + self.angle) * 2.0;
                let far = (ga > PI * 1.5 && ga < PI * 3.5) || ga < -PI * 0.5;
                let a = (0.5 + 0.5 * ga.sin()) * if far { 1.2 * self.opposite } else { 1.2 } * gf;
                let a = a.max(0.0).powf(0.1 + gc * 2.0).clamp(0.0, 1.0);
                let amt = a * geo;
                if amt > 0.002 {
                    let mut l = lch::from_srgb(mix(bl, tint, self.tint[3] * 0.5));
                    l[0] = (l[0] + 150.0 * amt).clamp(0.0, 120.0);
                    l[1] += 30.0 * amt;
                    col = mix(col, lch::to_srgb(l), (amt * gain).clamp(0.0, 1.0));
                }
            }
        }
        Some((col.map(|v| v.clamp(0.0, 1.0)), cov))
    }
}

/// sRGB ↔ CIE LCh (D65, hue in radians), as in the shader.
mod lch {
    const WHITE: [f32; 3] = [0.950_455_9, 1.0, 1.089_057_8];
    fn lin(c: f32) -> f32 {
        if c > 0.04045 {
            ((c + 0.055) / 1.055).powf(2.4)
        } else {
            c / 12.92
        }
    }
    fn gamma(c: f32) -> f32 {
        let c = c.max(0.0);
        if c > 0.003_130_8 {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        } else {
            12.92 * c
        }
    }
    fn f(x: f32) -> f32 {
        if x > 0.008_856_452 {
            x.cbrt()
        } else {
            7.787_037 * x + 0.137_931_03
        }
    }
    fn finv(x: f32) -> f32 {
        if x > 0.206_897 {
            x * x * x
        } else {
            0.128_418_55 * (x - 0.137_931_03)
        }
    }
    pub fn from_srgb(c: [f32; 3]) -> [f32; 3] {
        let (r, g, b) = (lin(c[0]), lin(c[1]), lin(c[2]));
        let x = 0.4124 * r + 0.3576 * g + 0.1805 * b;
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let z = 0.0193 * r + 0.1192 * g + 0.9505 * b;
        let (fx, fy, fz) = (f(x / WHITE[0]), f(y / WHITE[1]), f(z / WHITE[2]));
        let (l, a, bb) = (116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz));
        [l, (a * a + bb * bb).sqrt(), bb.atan2(a)]
    }
    pub fn to_srgb(l: [f32; 3]) -> [f32; 3] {
        let (a, b) = (l[1] * l[2].cos(), l[1] * l[2].sin());
        let w = (l[0] + 16.0) / 116.0;
        let (x, y, z) = (WHITE[0] * finv(w + a / 500.0), WHITE[1] * finv(w), WHITE[2] * finv(w - b / 200.0));
        let r = 3.240_625_5 * x - 1.537_208 * y - 0.498_628_6 * z;
        let g = -0.968_930_7 * x + 1.875_756_1 * y + 0.041_517_5 * z;
        let bl = 0.055_710_1 * x - 0.204_021_1 * y + 1.056_995_9 * z;
        [gamma(r), gamma(g), gamma(bl)]
    }

    #[cfg(test)]
    #[test]
    fn round_trip() {
        for c in [[0.2, 0.5, 0.8], [1.0, 1.0, 1.0], [0.9, 0.1, 0.3]] {
            let back = to_srgb(from_srgb(c));
            for i in 0..3 {
                assert!((back[i] - c[i]).abs() < 2e-3, "{c:?} → {back:?}");
            }
        }
    }
}

#[cfg(test)]
mod lens_tests {
    use super::*;

    #[test]
    fn rim_refracts_and_lights_the_edge() {
        let g = GlassStyle { blur: 0.0, tint: aqua_config::Rgba(1.0, 1.0, 1.0, 0.0), saturation: 1.0, ..Default::default() };
        let m = LensModel::new(&g, 200.0, 100.0, 1.0);
        // flat top: the backdrop straight through
        let (c, cov) = m.shade(100.5, 50.5, |dx, dy, _| [0.5 + dx * 0.0, 0.5 + dy * 0.0, 0.5]).unwrap();
        assert_eq!(cov, 1.0);
        assert!((c[0] - 0.5).abs() < 1e-3);
        // just inside the left rim the backdrop is pulled from further in (+x)
        let probe = std::cell::Cell::new(0.0f32);
        m.shade(1.5, 50.5, |dx, _, _| {
            probe.set(probe.get().max(dx));
            [0.2, 0.2, 0.2]
        });
        assert!(probe.get() > 5.0, "displacement {}", probe.get());
        // the top-left rim catches the light, the left rim gets the Fresnel band
        let (lit, _) = m.shade(100.5, 0.6, |_, _, _| [0.2, 0.2, 0.2]).unwrap();
        assert!(lit[1] > 0.25, "{lit:?}");
        assert!(m.shade(-2.0, 50.0, |_, _, _| [0.0; 3]).is_none(), "outside");
    }
}

/// Composite a full frame: wallpaper → windows → shell layers.
pub fn compose(wallpaper: &Pixmap, windows: &[SoftWindow], layers: &[Layer], scale: f32) -> Pixmap {
    let mut out = wallpaper.clone();
    for l in layers.iter().filter(|l| matches!(l.id, crate::LayerId::Widgets(_))) {
        draw_layer(&mut out, l, scale);
    }
    for w in windows {
        drop_shadow(&mut out, w.rect, w.radius, scale, 0.32, 28.0, 14.0);
        let mut c = w.content.clone();
        if let Some(p) = shapes::rrect(Rect::new(0.0, 0.0, w.rect.w, w.rect.h), w.radius) {
            let mut m = Mask::new(c.width(), c.height()).unwrap();
            m.fill_path(&p, FillRule::Winding, true, Transform::from_scale(scale, scale));
            c.apply_mask(&m);
        }
        out.draw_pixmap(
            (w.rect.x * scale) as i32,
            (w.rect.y * scale) as i32,
            c.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }
    for l in layers.iter().filter(|l| !matches!(l.id, crate::LayerId::Widgets(_))) {
        draw_layer(&mut out, l, scale);
    }
    out
}

pub fn draw_layer(out: &mut Pixmap, l: &Layer, scale: f32) {
    if l.opacity <= 0.001 {
        return;
    }
    if let Some(g) = &l.glass {
        glass(out, l.rect, g, scale, l.opacity);
    }
    for (r, g) in &l.tiles {
        glass(out, *r, g, scale, l.opacity);
    }
    out.draw_pixmap(
        (l.rect.x * scale).round() as i32,
        (l.rect.y * scale).round() as i32,
        l.content.as_ref().as_ref(),
        &PixmapPaint { opacity: l.opacity, ..Default::default() },
        Transform::identity(),
        None,
    );
}
