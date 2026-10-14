//! Backdrop removal for the glass plate: a logo often sits on its own backdrop shape inside
//! the plate (Telegram's paper plane on a blue disc, a glyph on a coloured badge). Such a
//! shape is found as a large, convex, evenly coloured region that encloses the artwork;
//! it is cut away like the plate, so only the artwork inside it stays on the glass. The
//! artwork is matted out of it (its anti-aliased edge loses the backdrop's colour), and the
//! backdrop's rim, glow and the shadow it casts go with it.

/// A foreground being extracted from an icon.
pub struct Layer<'a> {
    pub w: usize,
    pub h: usize,
    /// Un-premultiplied colour of the source.
    pub col: &'a mut [[f32; 3]],
    /// Source alpha.
    pub alpha: &'a [f32],
    /// How much of each pixel belongs to the glyph (0 = backdrop).
    pub keep: &'a mut [f32],
    /// Pixels whose `keep` / `col` were rewritten here.
    pub touched: &'a mut [bool],
}

fn d3(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// Distance of `c` to the segment `a`–`b`.
fn d_seg(c: [f32; 3], a: [f32; 3], b: [f32; 3]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let l2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
    if l2 < 1e-6 {
        return d3(c, a);
    }
    let t = (((c[0] - a[0]) * ab[0] + (c[1] - a[1]) * ab[1] + (c[2] - a[2]) * ab[2]) / l2).clamp(0.0, 1.0);
    d3(c, [a[0] + ab[0] * t, a[1] + ab[1] * t, a[2] + ab[2] * t])
}

/// Distance of `c` to the shades of `p` (a shadow on the plate is the plate, darker).
pub fn d_shade(c: [f32; 3], p: [f32; 3]) -> f32 {
    d_seg(c, [p[0] * 0.45, p[1] * 0.45, p[2] * 0.45], p)
}

/// Distance of `c` to the darker and lighter variants of `p` (an edge stroke or glow).
pub fn d_tone(c: [f32; 3], p: [f32; 3]) -> f32 {
    d_shade(c, p).min(d_seg(c, p, [(p[0] + 1.0) / 2.0, (p[1] + 1.0) / 2.0, (p[2] + 1.0) / 2.0]))
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Neighbouring colours closer than this belong to the same smooth region (on a 256 px
/// icon; finer renders spread every edge over more pixels, so the step shrinks with them).
const STEP: f32 = 0.05;
/// …and the region never drifts further than this from where it started.
const DRIFT: f32 = 0.7;
/// A matte that leaves more of a colour unexplained than this is a different colour.
const RESIDUE: f32 = 0.25;

/// Smooth, opaque glyph regions of at least `min` pixels.
fn regions(l: &Layer, min: usize, unit: f32) -> Vec<Vec<usize>> {
    let (w, h) = (l.w, l.h);
    let step = STEP * (206.0 / unit).clamp(0.25, 1.0);
    let solid = |i: usize| l.alpha[i] > 0.85 && l.keep[i] > 0.5;
    let mut seen = vec![false; w * h];
    let mut out = vec![];
    let mut stack = vec![];
    for s in 0..w * h {
        if seen[s] || !solid(s) {
            continue;
        }
        seen[s] = true;
        stack.push(s);
        let seed = l.col[s];
        let mut px = vec![];
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            px.push(i);
            let nb = [
                (x > 0).then(|| i - 1),
                (x + 1 < w).then(|| i + 1),
                (y > 0).then(|| i - w),
                (y + 1 < h).then(|| i + w),
            ];
            for j in nb.into_iter().flatten() {
                if !seen[j] && solid(j) && d3(l.col[i], l.col[j]) < step && d3(seed, l.col[j]) < DRIFT {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        if px.len() >= min {
            out.push(px);
        }
    }
    out
}

/// The mask with every enclosed hole filled.
fn fill_holes(m: &[bool], w: usize, h: usize) -> Vec<bool> {
    let mut outside = vec![false; w * h];
    let mut stack = vec![];
    for y in 0..h {
        for x in 0..w {
            if (x == 0 || y == 0 || x + 1 == w || y + 1 == h) && !m[y * w + x] {
                outside[y * w + x] = true;
                stack.push(y * w + x);
            }
        }
    }
    while let Some(i) = stack.pop() {
        let (x, y) = (i % w, i / w);
        let nb = [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)];
        for j in nb.into_iter().flatten() {
            if !outside[j] && !m[j] {
                outside[j] = true;
                stack.push(j);
            }
        }
    }
    outside.iter().map(|v| !*v).collect()
}

/// The outline of a shape: closed over notches narrower than `2r` (where artwork
/// crosses its edge) and with its holes filled.
fn outline(m: &[bool], w: usize, h: usize, r: f32) -> Vec<bool> {
    let not: Vec<bool> = m.iter().map(|v| !*v).collect();
    let grown: Vec<bool> = depth(&not, w, h, r + 2.0).iter().map(|d| *d <= r).collect();
    let closed: Vec<bool> = depth(&grown, w, h, r + 2.0).iter().zip(m).map(|(d, v)| *d > r || *v).collect();
    fill_holes(&closed, w, h)
}

/// Area of the convex hull of the mask.
fn hull_area(m: &[bool], w: usize, h: usize) -> f32 {
    let mut pts: Vec<(f32, f32)> = vec![];
    for y in 0..h {
        let row = &m[y * w..(y + 1) * w];
        if let (Some(a), Some(b)) = (row.iter().position(|v| *v), row.iter().rposition(|v| *v)) {
            for x in [a, b + 1] {
                pts.push((x as f32, y as f32));
                pts.push((x as f32, y as f32 + 1.0));
            }
        }
    }
    pts.sort_by(|a, b| a.partial_cmp(b).unwrap());
    pts.dedup();
    if pts.len() < 3 {
        return 0.0;
    }
    let cross = |o: (f32, f32), a: (f32, f32), b: (f32, f32)| (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0);
    let mut hull: Vec<(f32, f32)> = vec![];
    for pass in 0..2 {
        let start = hull.len();
        let it: Box<dyn Iterator<Item = &(f32, f32)>> =
            if pass == 0 { Box::new(pts.iter()) } else { Box::new(pts.iter().rev()) };
        for &p in it {
            while hull.len() >= start + 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0 {
                hull.pop();
            }
            hull.push(p);
        }
        hull.pop();
    }
    let n = hull.len();
    (0..n).map(|i| hull[i].0 * hull[(i + 1) % n].1 - hull[(i + 1) % n].0 * hull[i].1).sum::<f32>().abs() / 2.0
}

/// Chamfer distance (px) to the nearest pixel where `m` is false, capped at `cap`.
pub fn depth(m: &[bool], w: usize, h: usize, cap: f32) -> Vec<f32> {
    let mut d: Vec<f32> = m.iter().map(|v| if *v { cap } else { 0.0 }).collect();
    let (a, b) = (1.0f32, std::f32::consts::SQRT_2);
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if d[i] == 0.0 {
                continue;
            }
            let mut v = d[i];
            if x > 0 {
                v = v.min(d[i - 1] + a);
            }
            if y > 0 {
                v = v.min(d[i - w] + a);
                if x > 0 {
                    v = v.min(d[i - w - 1] + b);
                }
                if x + 1 < w {
                    v = v.min(d[i - w + 1] + b);
                }
            }
            d[i] = v;
        }
    }
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            let i = y * w + x;
            if d[i] == 0.0 {
                continue;
            }
            let mut v = d[i];
            if x + 1 < w {
                v = v.min(d[i + 1] + a);
            }
            if y + 1 < h {
                v = v.min(d[i + w] + a);
                if x + 1 < w {
                    v = v.min(d[i + w + 1] + b);
                }
                if x > 0 {
                    v = v.min(d[i + w - 1] + b);
                }
            }
            d[i] = v;
        }
    }
    d
}

/// Colour of the backdrop around every pixel (its gradient), blurred from the region.
fn backdrop_field(l: &Layer, region: &[usize], mean: [f32; 3], sigma: f32) -> Vec<[f32; 3]> {
    let (w, h) = (l.w, l.h);
    let mut ch = [crate::glass::Cov::empty(w, h), crate::glass::Cov::empty(w, h), crate::glass::Cov::empty(w, h)];
    let mut m = crate::glass::Cov::empty(w, h);
    for &i in region {
        m.a[i] = 1.0;
        for k in 0..3 {
            ch[k].a[i] = l.col[i][k];
        }
    }
    let m = m.blur(sigma);
    let ch = ch.map(|c| c.blur(sigma));
    (0..w * h)
        .map(|i| if m.a[i] > 0.04 { [ch[0].a[i] / m.a[i], ch[1].a[i] / m.a[i], ch[2].a[i] / m.a[i]] } else { mean })
        .collect()
}

/// A few representative colours of the artwork (k-means).
fn palette(px: &[[f32; 3]]) -> Vec<[f32; 3]> {
    if px.is_empty() {
        return vec![[1.0; 3]];
    }
    let px: Vec<[f32; 3]> = px.iter().step_by((px.len() / 4000).max(1)).copied().collect();
    let n = px.len() as f32;
    let mean = px.iter().fold([0.0f32; 3], |a, c| [a[0] + c[0] / n, a[1] + c[1] / n, a[2] + c[2] / n]);
    let mut cs = vec![mean];
    while cs.len() < 4 {
        let (far, d) = px
            .iter()
            .map(|c| (*c, cs.iter().map(|k| d3(*c, *k)).fold(f32::MAX, f32::min)))
            .fold(([0.0; 3], 0.0), |a, b| if b.1 > a.1 { b } else { a });
        if d < 0.15 {
            break;
        }
        cs.push(far);
    }
    for _ in 0..8 {
        let mut acc = vec![([0.0f32; 3], 0.0f32); cs.len()];
        for c in &px {
            let j = (0..cs.len()).min_by(|a, b| d3(*c, cs[*a]).total_cmp(&d3(*c, cs[*b]))).unwrap_or(0);
            for k in 0..3 {
                acc[j].0[k] += c[k];
            }
            acc[j].1 += 1.0;
        }
        for (k, (s, m)) in acc.into_iter().enumerate() {
            if m > 0.0 {
                cs[k] = [s[0] / m, s[1] / m, s[2] / m];
            }
        }
    }
    cs
}

/// How much of `c` is artwork laid over the backdrop `bd` (0…1), and how much of the
/// colour that leaves unexplained.
fn matte(c: [f32; 3], bd: [f32; 3], art: &[[f32; 3]]) -> (f32, f32) {
    let mut best = (0.0, f32::MAX);
    for a in art {
        let ab = [a[0] - bd[0], a[1] - bd[1], a[2] - bd[2]];
        let l2 = (ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2]).max(1e-4);
        let t = (((c[0] - bd[0]) * ab[0] + (c[1] - bd[1]) * ab[1] + (c[2] - bd[2]) * ab[2]) / l2).clamp(0.0, 1.0);
        let r = d3(c, [bd[0] + ab[0] * t, bd[1] + ab[1] * t, bd[2] + ab[2] * t]);
        if r < best.1 {
            best = (t, r);
        }
    }
    best
}

struct Backdrop {
    region: Vec<bool>,
    /// The region's outline: everything it encloses.
    shape: Vec<bool>,
    field: Vec<[f32; 3]>,
    mean: [f32; 3],
    /// A thin ring of another colour framing it (and its colour), part of the backdrop.
    frame: Option<(Vec<bool>, [f32; 3])>,
}

fn mean_of(l: &Layer, px: &[usize]) -> [f32; 3] {
    let n = px.len().max(1) as f32;
    px.iter().fold([0.0f32; 3], |a, &i| [a[0] + l.col[i][0] / n, a[1] + l.col[i][1] / n, a[2] + l.col[i][2] / n])
}

fn mask(px: &[usize], n: usize) -> Vec<bool> {
    let mut m = vec![false; n];
    for &i in px {
        m[i] = true;
    }
    m
}

/// Glyph pixels in `scope` further than `gap` from `shape`.
fn stray(l: &Layer, scope: &[bool], shape: &[bool], gap: f32) -> f32 {
    let not: Vec<bool> = shape.iter().map(|v| !*v).collect();
    let near = depth(&not, l.w, l.h, gap + 1.0);
    (0..l.w * l.h).filter(|&i| scope[i] && near[i] > gap && l.keep[i] > 0.5 && l.alpha[i] > 0.5).count() as f32
}

/// The largest backdrop inside `scope`: a big, convex, evenly coloured region that
/// encloses contrasting artwork and leaves none of it outside (but for a thin frame).
fn find(l: &Layer, scope: &[bool], unit: f32, body: f32, nested: bool) -> Option<Backdrop> {
    let (w, h) = (l.w, l.h);
    let n = w * h;
    let min_area = body * if nested { 0.12 } else { 0.18 };
    let all = regions(l, (body * 0.005) as usize, unit);
    let opaque = |m: Vec<bool>| -> Vec<bool> { m.iter().zip(l.alpha).map(|(s, a)| *s && *a > 0.5).collect() };
    let mut best: Option<(f32, Backdrop)> = None;
    for (k, px) in all.iter().enumerate() {
        if (px.len() as f32) < body * 0.05 {
            continue;
        }
        let mut region = mask(px, n);
        let mut shape = opaque(outline(&region, w, h, unit * 0.08));
        let mut area = shape.iter().filter(|v| **v).count() as f32;
        if area < min_area || best.as_ref().is_some_and(|b| area <= b.0) {
            continue;
        }
        if area / hull_area(&region, w, h).max(1.0) < 0.9 {
            continue;
        }
        let mean = mean_of(l, px);
        let field = backdrop_field(l, px, mean, unit * 0.04);
        let (mut art, mut rest) = (vec![], 0.0f32);
        for i in 0..n {
            if shape[i] && !region[i] {
                let d = d3(l.col[i], field[i]);
                if d > 0.15 {
                    art.push(d);
                } else {
                    rest += 1.0;
                }
            }
        }
        let content = art.len() as f32 / area;
        if !(0.02..=0.6).contains(&content) || px.len() as f32 / (px.len() as f32 + rest) < 0.85 {
            continue;
        }
        art.sort_by(|a, b| a.total_cmp(b));
        if art[(art.len() - 1) * 3 / 4] < 0.3 {
            continue;
        }
        let gap = unit * 0.03;
        if stray(l, scope, &shape, unit * 0.12) > area * 0.03 {
            continue;
        }
        let not: Vec<bool> = shape.iter().map(|v| !*v).collect();
        let near = depth(&not, w, h, unit * 0.12 + 1.0);
        let ring = all.iter().enumerate().find_map(|(j, q)| {
            if j == k || q.iter().any(|&i| near[i] > unit * 0.12) {
                return None;
            }
            let o = opaque(outline(&mask(q, n), w, h, unit * 0.06));
            let covered = (0..n).filter(|&i| shape[i] && o[i]).count() as f32;
            (covered >= area * 0.95).then_some((q, o))
        });
        let mut frame = None;
        if let Some((q, o)) = ring {
            for &i in q {
                region[i] = true;
            }
            shape = shape.iter().zip(&o).map(|(a, b)| *a || *b).collect();
            area = shape.iter().filter(|v| **v).count() as f32;
            frame = Some((mask(q, n), mean_of(l, q)));
        }
        if stray(l, scope, &shape, gap) > area * 0.03 {
            continue;
        }
        best = Some((area, Backdrop { region, shape, field, mean, frame }));
    }
    best.map(|b| b.1)
}

/// Cut a backdrop away: its region, the anti-aliasing and glow along its edge, the
/// shadow it casts; artwork over it is matted out of it.
fn remove(l: &mut Layer, plate: &dyn Fn(usize) -> Option<[f32; 3]>, b: &Backdrop, unit: f32) {
    let (w, h) = (l.w, l.h);
    let band = (unit * 0.035).max(2.0);
    let reach = unit * 0.12;
    let din = depth(&b.shape, w, h, unit * 0.012 + 2.0);
    let not: Vec<bool> = b.shape.iter().map(|v| !*v).collect();
    let dout = depth(&not, w, h, reach + 1.0);
    let not: Vec<bool> = b.region.iter().map(|v| !*v).collect();
    let deep = depth(&not, w, h, unit * 0.1 + 1.0);
    let art: Vec<[f32; 3]> = (0..w * h)
        .filter(|&i| b.shape[i] && !b.region[i] && l.alpha[i] > 0.9 && d3(l.col[i], b.field[i]) > 0.35)
        .map(|i| l.col[i])
        .collect();
    let art = palette(&art);
    let by_frame = b.frame.as_ref().map(|(m, c)| {
        let not: Vec<bool> = m.iter().map(|v| !*v).collect();
        (depth(&not, w, h, band * 2.0), *c)
    });
    let aa = (unit * 0.012).max(1.5);
    let black = [0.0f32; 3];
    let src: Vec<[f32; 3]> = l.col.to_vec();
    for i in 0..w * h {
        if l.alpha[i] <= 0.0 {
            continue;
        }
        if b.region[i] {
            l.keep[i] = 0.0;
            l.touched[i] = true;
            continue;
        }
        let (c, mut bd, pl) = (l.col[i], b.field[i], plate(i / w));
        // between the frame and the backdrop, and along the frame's outer edge
        let reach_frame = if b.shape[i] { aa } else { band * 1.5 };
        let framed = by_frame.as_ref().and_then(|(d, fc)| (d[i] < reach_frame).then_some(*fc));
        let rim = framed.map(|fc| smoothstep(0.06, 0.3, d_seg(c, bd, fc).min(d_tone(c, fc)))).unwrap_or(1.0);
        if let (Some(fc), false) = (framed, b.shape[i]) {
            bd = fc;
        }
        let (t, res) = matte(c, bd, &art);
        let matted = if res > RESIDUE { 1.0 } else { smoothstep(0.12, 0.85, t) };
        let k = if b.shape[i] {
            let mut k = if res > RESIDUE { smoothstep(0.06, 0.3, d3(c, bd).min(d_shade(c, bd))) } else { matted };
            if let (true, Some(p)) = (din[i] < aa, pl) {
                k = k.min(smoothstep(0.06, 0.3, d_seg(c, bd, p)));
            }
            let k = k.min(rim) * smoothstep(0.3, 0.9, l.alpha[i]);
            if res <= RESIDUE && t > 0.05 && k > 0.02 && k < 0.97 {
                for j in 0..3 {
                    l.col[i][j] = (bd[j] + (c[j] - bd[j]) / t).clamp(0.0, 1.0);
                }
            }
            k
        } else if dout[i] < band * 1.5 {
            let d = match pl {
                Some(p) => d_seg(c, bd, p).min(d_shade(c, p)).min(d_tone(c, bd)),
                None => d_tone(c, bd).min(d_seg(c, black, bd)),
            };
            smoothstep(0.08, 0.3, d).min(matted).min(rim).min(l.keep[i])
        } else if dout[i] < reach && c.iter().sum::<f32>() < pl.unwrap_or(bd).iter().sum::<f32>() {
            // the shadow it casts: on the plate, or (free-form) a translucent darkening
            let k = match pl {
                Some(p) => smoothstep(0.06, 0.2, d_shade(c, p)),
                None if l.alpha[i] < 0.7 => smoothstep(0.06, 0.2, d_seg(c, black, bd)),
                None => 1.0,
            };
            k.min(l.keep[i])
        } else {
            continue;
        };
        l.keep[i] = k;
        l.touched[i] = true;
    }
    // backdrop-coloured details deep inside the artwork (eyes in a face) are artwork; a
    // gap in the artwork near the backdrop (the inside of a ring) is not
    let mut seen = vec![false; w * h];
    let mut stack = vec![];
    for s in 0..w * h {
        if seen[s] || !b.shape[s] || b.region[s] || l.keep[s] >= 0.5 {
            continue;
        }
        seen[s] = true;
        stack.push(s);
        let mut comp = vec![];
        while let Some(i) = stack.pop() {
            comp.push(i);
            let (x, y) = (i % w, i / w);
            let nb = [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)];
            for j in nb.into_iter().flatten() {
                if !seen[j] && b.shape[j] && !b.region[j] && l.keep[j] < 0.5 {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        if comp.iter().any(|&i| deep[i] >= unit * 0.1) && !comp.iter().any(|&i| din[i] < aa) {
            for &i in &comp {
                l.keep[i] = smoothstep(0.3, 0.9, l.alpha[i]);
                l.col[i] = src[i];
            }
        }
    }
}

/// Find and remove backdrop shapes, nested ones too; `plate` is the original plate colour
/// of a row (None for free-form icons), `clip` the plate's coverage, `body` the plate
/// rect's area in pixels. Returns the colour of the innermost backdrop removed: the
/// glyph's ink is read against it, no longer the plate.
pub fn strip(
    l: &mut Layer,
    plate: &dyn Fn(usize) -> Option<[f32; 3]>,
    clip: Option<&[f32]>,
    body: f32,
) -> Option<[f32; 3]> {
    let unit = body.sqrt();
    let mut scope: Vec<bool> =
        (0..l.w * l.h).map(|i| l.alpha[i] > 0.5 && clip.is_none_or(|c| c[i] > 0.5)).collect();
    let mut found = None;
    for level in 0..3 {
        let Some(b) = find(l, &scope, unit, body, level > 0) else { break };
        remove(l, plate, &b, unit);
        found = Some(b.mean);
        scope = b.shape;
    }
    found
}

/// [`strip`] for a free-form picture (a theme icon about to be put on our plate); also
/// returns the colour of the backdrop removed.
pub fn strip_pixmap(pm: &aqua_gfx::Pixmap) -> Option<(aqua_gfx::Pixmap, [f32; 3])> {
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    let d = pm.data();
    let alpha: Vec<f32> = (0..w * h).map(|i| d[i * 4 + 3] as f32 / 255.0).collect();
    let mut col: Vec<[f32; 3]> = (0..w * h)
        .map(|i| {
            let a = alpha[i].max(1e-3);
            [d[i * 4] as f32 / 255.0 / a, d[i * 4 + 1] as f32 / 255.0 / a, d[i * 4 + 2] as f32 / 255.0 / a].map(|v| v.min(1.0))
        })
        .collect();
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for i in (0..w * h).filter(|&i| alpha[i] > 0.5) {
        (x0, y0, x1, y1) = (x0.min(i % w), y0.min(i / w), x1.max(i % w + 1), y1.max(i / w + 1));
    }
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let mut keep = vec![1.0f32; w * h];
    let mut touched = vec![false; w * h];
    let mut l = Layer { w, h, col: &mut col, alpha: &alpha, keep: &mut keep, touched: &mut touched };
    // measured like the plate it is going to sit on (at 70 %)
    let bd = strip(&mut l, &|_| None, None, ((x1 - x0) * (y1 - y0)) as f32 / 0.49)?;
    let mut out = aqua_gfx::Pixmap::new(w as u32, h as u32)?;
    let o = out.data_mut();
    for i in 0..w * h {
        let a = (alpha[i] * keep[i]).clamp(0.0, 1.0);
        for k in 0..3 {
            o[i * 4 + k] = (col[i][k] * a * 255.0 + 0.5) as u8;
        }
        o[i * 4 + 3] = (a * 255.0 + 0.5) as u8;
    }
    Some((out, bd))
}
