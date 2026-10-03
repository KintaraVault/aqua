//! Icon appearance: Default, Dark, Clear and Tinted,
//! plus the glass edge highlight.
//!
//! Third-party icons are flat bitmaps, so the styles are derived per pixel: the
//! *plate* (the icon's background) is estimated row by row from the body's left and
//! right edges, every pixel is weighted by how close it is to that plate, and plate
//! and glyph are recoloured separately — the glyph keeps its colours on a dark plate
//! (Dark), becomes a monochrome tint (Tinted) or frosted white on a translucent plate
//! (Clear). Full-bleed artwork without a plate simply keeps its pixels.
use aqua_gfx::canvas::lin_grad;
use aqua_gfx::tiny_skia::{Mask, Transform};
use aqua_gfx::{Pixmap, Rect};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Style {
    #[default]
    Default,
    Dark,
    Clear,
    Tinted,
}

impl Style {
    /// Config value: "default", "dark", "clear", "tinted" or "auto" (dark in Dark Mode).
    pub fn from_config(s: &str, dark_mode: bool) -> Self {
        match s {
            "dark" => Style::Dark,
            "clear" => Style::Clear,
            "tinted" => Style::Tinted,
            "auto" | "automatic" if dark_mode => Style::Dark,
            _ => Style::Default,
        }
    }
}

/// Everything that changes how icons look.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Look {
    pub style: Style,
    pub dark: bool,
    /// Tint colour (accent), 0…1.
    pub tint: (f32, f32, f32),
    /// Glass specular rim on bitmap icons.
    pub glass: bool,
}

impl Look {
    pub fn key(&self) -> u64 {
        let t = |v: f32| (v * 255.0) as u64;
        (self.style as u64)
            | (self.dark as u64) << 3
            | (self.glass as u64) << 4
            | t(self.tint.0) << 8
            | t(self.tint.1) << 16
            | t(self.tint.2) << 24
    }
}

/// Body of an icon-grid icon on a `px` canvas (824/1024, centred).
fn grid_body(px: u32) -> Rect {
    let s = px as f32 / 1024.0;
    Rect::new(100.0 * s, 100.0 * s, 824.0 * s, 824.0 * s)
}

/// The body as it is actually drawn.
fn body(pm: &Pixmap) -> Rect {
    let g = grid_body(pm.width());
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    if w < 16 || h < 16 {
        return g;
    }
    let d = pm.data();
    let a = |x: usize, y: usize| d[(y * w + x) * 4 + 3] as f32 / 255.0;
    let edge = |n: usize, at: &dyn Fn(usize) -> f32| -> Option<(f32, f32)> {
        let first = (0..n).find(|&i| at(i) >= 0.5)?;
        let last = (0..n).rev().find(|&i| at(i) >= 0.5)?;
        let outer =
            |k: Option<usize>, inner: f32| if inner > 0.97 { k.map(|k| at(k).min(0.5)).unwrap_or(0.0) } else { 0.0 };
        let lo = first as f32 + 1.0 - at(first) - outer(first.checked_sub(1), at(first));
        let hi = last as f32 + at(last) + outer((last + 1 < n).then_some(last + 1), at(last));
        Some((lo, hi))
    };
    let (cy, cx) = (h / 2, w / 2);
    let Some((x0, x1)) = edge(w, &|x| a(x, cy)) else { return g };
    let Some((y0, y1)) = edge(h, &|y| a(cx, y)) else { return g };
    let r = Rect::new(x0, y0, x1 - x0, y1 - y0);
    let ok = (r.w - g.w).abs() < g.w * 0.08 && (r.h - g.h).abs() < g.h * 0.12 && (r.x - g.x).abs() < g.w * 0.06;
    if ok {
        r
    } else {
        g
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Apply the look to an icon (`builtin` icons already carry their own glass finish).
pub fn apply(pm: &mut Pixmap, look: &Look, builtin: bool) {
    if builtin {
        return;
    }
    let shape = if look.style != Style::Default { restyle(pm, look) } else { None };
    if look.glass {
        rim(pm, look, shape.as_ref());
    }
}

fn restyle(pm: &mut Pixmap, look: &Look) -> Option<crate::glass::Cov> {
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    if w < 8 || h < 8 {
        return None;
    }
    let b = body(pm);
    let grow = 1.0;
    let path = crate::glass::plate_path(Rect::new(b.x - grow, b.y - grow, b.w + 2.0 * grow, b.h + 2.0 * grow))?;
    let mut mask = Mask::new(pm.width(), pm.height())?;
    mask.fill_path(&path, aqua_gfx::tiny_skia::FillRule::Winding, true, Transform::identity());
    let m = mask.data();
    let data = pm.data_mut();
    let px = |data: &[u8], x: usize, y: usize| -> Option<[f32; 3]> {
        let i = (y * w + x) * 4;
        let a = data[i + 3] as f32 / 255.0;
        (a > 0.6).then(|| [data[i] as f32 / 255.0 / a, data[i + 1] as f32 / 255.0 / a, data[i + 2] as f32 / 255.0 / a])
    };
    let d3 = |a: [f32; 3], b: [f32; 3]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
    let (y0, y1) = (b.y.ceil() as usize, (b.bottom().floor() as usize).min(h - 1));
    let inset = (b.w * 0.05).max(2.0);
    let mut plate: Vec<Option<[f32; 3]>> = vec![None; h];
    let (mut rows, mut agree) = (0, 0);
    for y in y0..=y1 {
        let rel = (y as f32 - b.y) / b.h;
        if !(0.15..=0.85).contains(&rel) {
            continue;
        }
        rows += 1;
        let xl = (b.x + inset) as usize;
        let xr = (b.right() - inset) as usize;
        let side = |xs: [usize; 2]| -> Option<[f32; 3]> {
            let v: Vec<[f32; 3]> = xs.iter().filter_map(|&x| px(data, x.min(w - 1), y)).collect();
            (!v.is_empty()).then(|| {
                let n = v.len() as f32;
                [
                    v.iter().map(|c| c[0]).sum::<f32>() / n,
                    v.iter().map(|c| c[1]).sum::<f32>() / n,
                    v.iter().map(|c| c[2]).sum::<f32>() / n,
                ]
            })
        };
        if let (Some(l), Some(r)) = (side([xl, xl + 2]), side([xr - 2, xr])) {
            if d3(l, r) < 0.16 {
                agree += 1;
                plate[y] = Some(mix(l, r, 0.5));
            }
        }
    }
    let has_plate = rows > 0 && agree * 10 >= rows * 7;
    if has_plate {
        let first = plate.iter().position(|p| p.is_some()).unwrap_or(0);
        let last = plate.iter().rposition(|p| p.is_some()).unwrap_or(h - 1);
        for y in 0..h {
            if plate[y].is_none() {
                plate[y] = if y < first {
                    plate[first]
                } else if y > last {
                    plate[last]
                } else {
                    plate[y - 1]
                };
            }
        }
        let win = ((b.h * 0.1) as usize).max(2);
        let src = plate.clone();
        for y in 0..h {
            let (a0, a1) = (y.saturating_sub(win), (y + win + 1).min(h));
            let mut ch = [vec![], vec![], vec![]];
            for p in src[a0..a1].iter().flatten() {
                for k in 0..3 {
                    ch[k].push(p[k]);
                }
            }
            if ch[0].is_empty() {
                continue;
            }
            let med = |v: &mut Vec<f32>| {
                v.sort_by(|a, b| a.partial_cmp(b).unwrap());
                v[v.len() / 2]
            };
            plate[y] = Some([med(&mut ch[0]), med(&mut ch[1]), med(&mut ch[2])]);
        }
    }
    let mut col = vec![[0.0f32; 3]; w * h];
    let mut sig = vec![0.0f32; w * h];
    let mut alpha = vec![0.0f32; w * h];
    let mut samples: Vec<f32> = vec![];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let a = data[i * 4 + 3] as f32 / 255.0;
            if a <= 0.0 {
                continue;
            }
            let c = [
                (data[i * 4] as f32 / 255.0 / a).min(1.0),
                (data[i * 4 + 1] as f32 / 255.0 / a).min(1.0),
                (data[i * 4 + 2] as f32 / 255.0 / a).min(1.0),
            ];
            col[i] = c;
            alpha[i] = a;
            sig[i] = match plate[y] {
                Some(pl) if has_plate => d3(c, pl),
                _ => luma(c),
            };
            if m[i] > 200 && a > 0.95 {
                samples.push(sig[i]);
            }
        }
    }
    if samples.is_empty() {
        return None;
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pct = |q: f32| samples[((samples.len() - 1) as f32 * q) as usize];
    let (lo, hi) = if has_plate {
        let p97 = pct(0.97);
        (0.07, (p97 * 0.4).max(0.18))
    } else {
        let (a, z) = (pct(0.05), pct(0.95));
        let r = (z - a).max(0.15);
        (a + r * 0.3, a + r * 0.75)
    };
    let tint = [look.tint.0, look.tint.1, look.tint.2];
    let white = [1.0f32, 1.0, 1.0];
    let plate_of = |ty: f32| -> ([f32; 3], f32) {
        match look.style {
            Style::Dark => (mix([0.215, 0.215, 0.23], [0.085, 0.085, 0.095], ty), 1.0),
            Style::Tinted if look.dark => (
                mix(
                    [tint[0] * 0.30 + 0.05, tint[1] * 0.30 + 0.05, tint[2] * 0.30 + 0.06],
                    [tint[0] * 0.13 + 0.025, tint[1] * 0.13 + 0.025, tint[2] * 0.13 + 0.03],
                    ty,
                ),
                1.0,
            ),
            Style::Tinted => (mix(mix(tint, white, 0.30), mix(tint, [0.0; 3], 0.06), ty), 1.0),
            Style::Clear if look.dark => (mix([0.17, 0.18, 0.21], [0.06, 0.065, 0.08], ty), 0.52 + 0.14 * ty),
            Style::Clear => (white, 0.17 - 0.07 * ty),
            Style::Default => (white, 1.0),
        }
    };
    let clipc = crate::glass::Cov { w, h, a: m.iter().map(|v| *v as f32 / 255.0).collect() };
    let mut fg = Pixmap::new(w as u32, h as u32)?;
    let r = ((w as f32 / 64.0).round() as usize).clamp(1, 3);
    let sat = |c: [f32; 3]| c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2]);
    let mut avg = [0.0f32; 3];
    let mut cnt = 0.0f32;
    for p in plate.iter().flatten() {
        avg = [avg[0] + p[0], avg[1] + p[1], avg[2] + p[2]];
        cnt += 1.0;
    }
    let avg = if cnt > 0.0 { [avg[0] / cnt, avg[1] / cnt, avg[2] / cnt] } else { white };
    {
        let fd = fg.data_mut();
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let cov = if has_plate { clipc.a[i] } else { 1.0 };
                let a = alpha[i];
                if cov <= 0.0 || a <= 0.0 {
                    continue;
                }
                let s = if has_plate { smoothstep(lo, hi, sig[i]).min(smoothstep(0.55, 0.97, a)) } else { 1.0 };
                if s <= 0.0 {
                    continue;
                }
                let mut g = col[i];
                if has_plate && s < 0.98 {
                    let mut best = sig[i];
                    for yy in y.saturating_sub(r)..(y + r + 1).min(h) {
                        for xx in x.saturating_sub(r)..(x + r + 1).min(w) {
                            let j = yy * w + xx;
                            if alpha[j] > 0.9 && sig[j] > best {
                                best = sig[j];
                                g = col[j];
                            }
                        }
                    }
                }
                if look.style == Style::Dark && has_plate {
                    let pl = plate[y].unwrap_or(white);
                    if sat(pl) > 0.22 {
                        let mx = pl[0].max(pl[1]).max(pl[2]).max(0.05);
                        let vivid = mix([pl[0] / mx, pl[1] / mx, pl[2] / mx], white, 0.08);
                        g = mix(g, vivid, smoothstep(0.55, 0.85, luma(g)) * (1.0 - smoothstep(0.1, 0.3, sat(g))));
                    }
                    g = mix(
                        g,
                        [0.95, 0.95, 0.97],
                        smoothstep(0.5, 0.15, luma(g)) * (1.0 - smoothstep(0.2, 0.4, sat(g))),
                    );
                }
                let fa = (a * s * cov).min(1.0);
                for k in 0..3 {
                    fd[i * 4 + k] = (g[k].clamp(0.0, 1.0) * fa * 255.0 + 0.5) as u8;
                }
                fd[i * 4 + 3] = (fa * 255.0 + 0.5) as u8;
            }
        }
    }
    {
        let d = pm.data_mut();
        for y in 0..h {
            let ty = ((y as f32 - b.y) / b.h).clamp(0.0, 1.0);
            let (pc, pa) = plate_of(ty);
            for x in 0..w {
                let i = y * w + x;
                let cov = if has_plate { clipc.a[i] } else { 1.0 };
                if cov <= 0.0 {
                    continue;
                }
                let pa = if has_plate { pa * (alpha[i] / cov.max(1e-3)).min(1.0) } else { 0.0 };
                for k in 0..3 {
                    let v = d[i * 4 + k] as f32 / 255.0;
                    d[i * 4 + k] = ((v * (1.0 - cov) + pc[k] * pa * cov) * 255.0 + 0.5).min(255.0) as u8;
                }
                let v = d[i * 4 + 3] as f32 / 255.0;
                d[i * 4 + 3] = ((v * (1.0 - cov) + pa * cov) * 255.0 + 0.5).min(255.0) as u8;
            }
        }
    }
    let cx = crate::glyph::Ctx {
        plate: has_plate,
        plate_col: has_plate.then_some(avg),
        unit: b.w / 824.0,
        top: b.y,
        height: b.h,
        clip: has_plate.then_some(&clipc),
    };
    crate::glyph::composite(pm, &fg, look, &cx);
    has_plate.then_some(clipc)
}

fn rim(pm: &mut Pixmap, look: &Look, plate: Option<&crate::glass::Cov>) {
    let b = body(pm);
    let s = b.w / 824.0;
    let strong = if look.style == Style::Clear { 1.35 } else { 1.0 };
    let shape = match plate {
        Some(p) => p.clone(),
        None => {
            let a = crate::glass::Cov::alpha(pm);
            crate::glass::Cov { a: a.a.iter().map(|v| smoothstep(0.45, 0.9, *v)).collect(), ..a }
        }
    };
    let sp = crate::glass::specular(&shape, (8.0 * s).max(1.0), 0.85 * strong, 0.5 * strong, 0.2 * strong);
    let t = Transform::identity();
    let full = aqua_gfx::tiny_skia::Rect::from_xywh(0.0, 0.0, pm.width() as f32, pm.height() as f32);
    if let (Some(m), Some(r)) = (sp.to_mask(), full) {
        pm.fill_rect(r, &crate::glass::solid(aqua_gfx::rgba(255, 255, 255, 1.0)), t, Some(&m));
    }
    if let (Some(m), Some(r)) = (shape.to_mask(), full) {
        let w = aqua_gfx::rgba(255, 255, 255, 0.0);
        let sheen =
            lin_grad(0.0, b.y, 0.0, b.y + b.h * 0.45, &[(0.0, aqua_gfx::rgba(255, 255, 255, 0.10 * strong)), (1.0, w)]);
        pm.fill_rect(r, &sheen, t, Some(&m));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Renders every style of a few icons to /tmp/aqua-looks.png for visual review
    /// (`cargo test -p aqua-icons render_looks -- --ignored`).
    #[test]
    #[ignore]
    fn render_looks() {
        let fonts =
            aqua_gfx::Fonts::load(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts")));
        let mut srcs: Vec<(Pixmap, bool)> = vec![];
        let names = [
            "finder",
            "launchpad",
            "settings",
            "safari",
            "messages",
            "music",
            "calendar",
            "notes",
            "calculator",
            "photos",
        ];
        for n in [
            "finder",
            "launchpad",
            "settings",
            "safari",
            "messages",
            "music",
            "calendar",
            "notes",
            "calculator",
            "photos",
        ] {
            srcs.push((crate::builtin::draw(n, 108, &fonts).unwrap(), true));
        }
        for k in 0..2 {
            let mut glyph = Pixmap::new(256, 256).unwrap();
            let mut pb = aqua_gfx::tiny_skia::PathBuilder::new();
            pb.push_circle(128.0, 128.0, 120.0);
            let paint = aqua_gfx::canvas::lin_grad(
                0.0,
                0.0,
                0.0,
                256.0,
                &[(0.0, aqua_gfx::rgba(60, 170, 230, 1.0)), (1.0, aqua_gfx::rgba(30, 140, 210, 1.0))],
            );
            glyph.fill_path(
                &pb.finish().unwrap(),
                &paint,
                aqua_gfx::tiny_skia::FillRule::Winding,
                Transform::identity(),
                None,
            );
            if k == 0 {
                let mut pb = aqua_gfx::tiny_skia::PathBuilder::new();
                pb.move_to(60.0, 125.0);
                pb.line_to(190.0, 70.0);
                pb.line_to(165.0, 190.0);
                pb.line_to(120.0, 155.0);
                pb.close();
                glyph.fill_path(
                    &pb.finish().unwrap(),
                    &aqua_gfx::canvas::solid(aqua_gfx::rgba(255, 255, 255, 1.0)),
                    aqua_gfx::tiny_skia::FillRule::Winding,
                    Transform::identity(),
                    None,
                );
            }
            srcs.push((crate::normalize::plate(&glyph, 108), false));
        }
        let looks = [
            (Style::Default, false),
            (Style::Dark, true),
            (Style::Clear, false),
            (Style::Clear, true),
            (Style::Tinted, false),
            (Style::Tinted, true),
        ];
        let cell = 110u32;
        let mut out = Pixmap::new(cell * srcs.len() as u32, cell * looks.len() as u32).unwrap();
        out.fill(aqua_gfx::rgba(50, 100, 135, 1.0));
        for (c, (src, builtin)) in srcs.iter().enumerate() {
            for (r, (st, dark)) in looks.iter().enumerate() {
                let look = Look { style: *st, dark: *dark, tint: (0.55, 0.45, 0.85), glass: true };
                let mut pm = if *builtin {
                    crate::builtin::draw_look(names[c], 108, &fonts, &look).unwrap()
                } else {
                    src.clone()
                };
                apply(&mut pm, &look, *builtin);
                out.draw_pixmap(
                    (c as u32 * cell) as i32,
                    (r as u32 * cell) as i32,
                    pm.as_ref(),
                    &Default::default(),
                    Transform::identity(),
                    None,
                );
            }
        }
        out.save_png("/tmp/aqua-looks.png").unwrap();
    }
}
