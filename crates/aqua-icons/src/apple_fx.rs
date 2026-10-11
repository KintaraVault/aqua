//! Ports of Apple IconRendering.framework stitchable shaders (macOS 26).
//!
//! Translated from the AIR → GLSL dumps in linuxtosh `shaders/liquid-glass/glsl/icon.*.lib.glsl`
//! (`glassHighlight`, `sdfFill`, `glow`). The Apple shaders sample a distance-field texture
//! (r = distance, g/b = encoded normal) produced by IconRendering; here the field is
//! computed on the CPU from a coverage map with an exact Euclidean distance transform, and the
//! per-pixel math is a 1:1 copy of the shader bodies (same clamps, same `fwidth` antialiasing
//! factor 0.833, same 1/1024 epsilon).
//!
//! The mode constants Apple feeds into these shaders are compiled Swift literals and are not
//! in the dumps; [`Highlight::ICON`] holds values fitted by eye against `icons-modes` renders.
use crate::glass::Cov;

const EPS: f32 = 0.000_976_562_5;
/// `fwidth()` of a distance field measured in device pixels is 1; Apple scales it by 0.833.
const AA: f32 = 0.833_007_8;

/// Signed distance field (device px, positive inside) with outward unit normals.
pub struct Sdf {
    pub w: usize,
    pub h: usize,
    pub d: Vec<f32>,
}

impl Sdf {
    /// Exact EDT (Felzenszwalb–Huttenlocher) of the 0.5 iso-line of `m`, refined by the
    /// coverage value at the edge so antialiased outlines keep sub-pixel accuracy.
    pub fn from_cov(m: &Cov) -> Self {
        let (w, h) = (m.w, m.h);
        let inside: Vec<bool> = m.a.iter().map(|&v| v >= 0.5).collect();
        let dist_to = |want: bool| -> Vec<f32> {
            let inf = 1e20f32;
            let mut f: Vec<f32> = inside.iter().map(|&b| if b == want { 0.0 } else { inf }).collect();
            edt_2d(&mut f, w, h);
            f.iter().map(|v| v.sqrt()).collect()
        };
        let to_in = dist_to(true); // distance (outside px) to nearest inside px
        let to_out = dist_to(false); // distance (inside px) to nearest outside px
        let d = (0..w * h)
            .map(|i| {
                let c = m.a[i];
                if inside[i] {
                    (to_out[i] - 1.0) + c.clamp(0.5, 1.0)
                } else {
                    -((to_in[i] - 1.0) + (1.0 - c).clamp(0.5, 1.0))
                }
            })
            .collect();
        Self { w, h, d }
    }

    fn at(&self, x: isize, y: isize) -> f32 {
        let x = x.clamp(0, self.w as isize - 1) as usize;
        let y = y.clamp(0, self.h as isize - 1) as usize;
        self.d[y * self.w + x]
    }

    /// Inner distance and outward normal at a pixel (the `r`/`gb` texture sample).
    pub fn inside(&self, x: usize, y: usize) -> (f32, (f32, f32)) {
        let (xi, yi) = (x as isize, y as isize);
        let gx = (self.at(xi + 1, yi) - self.at(xi - 1, yi)) * 0.5;
        let gy = (self.at(xi, yi + 1) - self.at(xi, yi - 1)) * 0.5;
        let m = (gx * gx + gy * gy).sqrt().max(1e-6);
        // distance grows inward, so the outward normal is minus the gradient
        (self.at(xi, yi), (-gx / m, -gy / m))
    }
}

fn edt_1d(f: &[f32], d: &mut [f32], v: &mut [usize], z: &mut [f32]) {
    let n = f.len();
    let mut k = 0usize;
    v[0] = 0;
    z[0] = f32::NEG_INFINITY;
    z[1] = f32::INFINITY;
    for q in 1..n {
        loop {
            let p = v[k];
            let s = ((f[q] + (q * q) as f32) - (f[p] + (p * p) as f32)) / (2.0 * q as f32 - 2.0 * p as f32);
            if s <= z[k] && k > 0 {
                k -= 1;
                continue;
            }
            if s <= z[k] {
                // k == 0 and s <= -inf cannot happen; fallthrough guard
            }
            k += 1;
            v[k] = q;
            z[k] = s;
            z[k + 1] = f32::INFINITY;
            break;
        }
    }
    k = 0;
    for q in 0..n {
        while z[k + 1] < q as f32 {
            k += 1;
        }
        let p = v[k];
        let dq = q as f32 - p as f32;
        d[q] = dq * dq + f[p];
    }
}

fn edt_2d(g: &mut [f32], w: usize, h: usize) {
    let n = w.max(h);
    let (mut f, mut d, mut v, mut z) = (vec![0.0; n], vec![0.0; n], vec![0usize; n], vec![0.0; n + 1]);
    for x in 0..w {
        for y in 0..h {
            f[y] = g[y * w + x];
        }
        edt_1d(&f[..h], &mut d[..h], &mut v, &mut z);
        for y in 0..h {
            g[y * w + x] = d[y];
        }
    }
    for y in 0..h {
        f[..w].copy_from_slice(&g[y * w..y * w + w]);
        edt_1d(&f[..w], &mut d[..w], &mut v, &mut z);
        g[y * w..y * w + w].copy_from_slice(&d[..w]);
    }
}

/// `glassHighlight(width, inset, angleThreshold, falloff, gradientAmount, lightDirection, color)`.
#[derive(Clone, Copy, Debug)]
pub struct Highlight {
    pub width: f32,
    pub inset: f32,
    pub angle_threshold: f32,
    pub falloff: f32,
    pub gradient: f32,
}

impl Highlight {
    pub const ICON: Highlight = Highlight { width: 8.0, inset: 0.0, angle_threshold: -0.15, falloff: 3.0, gradient: 1.0 };

    /// Antialiased band `0 < d < width` (shader block %9 with `solid == false`).
    pub fn band(&self, d_in: f32) -> f32 {
        let d = d_in - self.inset;
        let fw = AA;
        let outer = ((self.width - d) / fw + 0.5).clamp(0.0, 1.0);
        outer * (d / fw + 0.5).clamp(0.0, 1.0)
    }

    /// Scalar highlight intensity (multiply by the highlight colour).
    pub fn eval(&self, d_in: f32, n: (f32, f32), light: (f32, f32)) -> f32 {
        let d = d_in - self.inset;
        let a = self.band(d_in);
        let r = (d / self.width.max(EPS)).clamp(0.0, 1.0);
        let grad = 1.0 + ((1.0 - r) - 1.0) * self.gradient; // mix(1, 1-r, g)
        let dl = light.0 * n.0 + light.1 * n.1;
        let e = ((dl - self.angle_threshold) / (1.0 - self.angle_threshold).max(EPS)).clamp(0.0, 1.0);
        let denom = ((1.0 - e) * self.falloff + 1.0).max(EPS);
        grad * a * e / denom
    }
}

/// `sdfFill(scale, offset)`: smoothstep coverage of `d*scale + offset`.
pub fn sdf_fill(d_in: f32, scale: f32, offset: f32) -> f32 {
    let v = d_in * scale + offset;
    let fw = AA * scale.abs().clamp(EPS, 2.0);
    let t = ((v + fw * 0.5) / fw).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `glow(radius, falloff, color, scale, offset)`: gaussian halo outside the shape
/// (`d_out` = outside distance in px, positive outside).
pub fn glow(d_out: f32, radius: f32, falloff: f32) -> f32 {
    let x = d_out / radius.max(EPS);
    let g = (-0.5 * x * x).exp();
    let denom = ((1.0 - g) * falloff + 1.0).max(EPS);
    let fw = (AA / radius.max(EPS)).max(EPS);
    let a = (x / fw + 0.5).clamp(0.0, 1.0);
    a * g / denom
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disc(n: usize, r: f32) -> Cov {
        let mut c = Cov::empty(n, n);
        let o = n as f32 / 2.0;
        for y in 0..n {
            for x in 0..n {
                let d = ((x as f32 + 0.5 - o).powi(2) + (y as f32 + 0.5 - o).powi(2)).sqrt();
                c.a[y * n + x] = (r - d + 0.5).clamp(0.0, 1.0);
            }
        }
        c
    }

    #[test]
    fn sdf_matches_disc() {
        let c = disc(64, 20.0);
        let s = Sdf::from_cov(&c);
        let (d, n) = s.inside(32, 22); // ~10 px inside, top edge
        assert!((d - 10.0).abs() < 1.0, "d={d}");
        assert!(n.1 < -0.9, "normal {n:?}");
    }

    #[test]
    fn highlight_faces_light() {
        let h = Highlight::ICON;
        let top = h.eval(1.0, (0.0, -1.0), (0.0, -1.0));
        let bottom = h.eval(1.0, (0.0, 1.0), (0.0, -1.0));
        assert!(top > 0.5 && bottom == 0.0, "{top} {bottom}");
        assert_eq!(h.eval(20.0, (0.0, -1.0), (0.0, -1.0)), 0.0);
    }
}
