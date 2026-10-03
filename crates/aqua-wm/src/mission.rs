//! Mission Control: lay windows out in a non-overlapping grid.
use crate::geom::RectF;

/// Thumbnail rects for `frames` inside `area`, in the same order as `frames`. Windows are
/// arranged left-to-right by their current centre; the column count maximises the smallest
/// thumbnail scale (capped at 0.85).
pub fn grid(frames: &[RectF], area: RectF) -> Vec<RectF> {
    let n = frames.len();
    if n == 0 {
        return vec![];
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| frames[a].cx().partial_cmp(&frames[b].cx()).unwrap_or(std::cmp::Ordering::Equal));
    let scale = |r: &RectF, cw: f64, ch: f64| ((cw - 40.0) / r.w).min((ch - 60.0) / r.h).min(0.85);
    let mut best = (1usize, f64::MIN);
    for cols in 1..=n {
        let rows = n.div_ceil(cols);
        let (cw, ch) = (area.w / cols as f64, area.h / rows as f64);
        let s = frames.iter().map(|r| scale(r, cw, ch)).fold(f64::MAX, f64::min);
        if s > best.1 {
            best = (cols, s);
        }
    }
    let cols = best.0;
    let rows = n.div_ceil(cols);
    let (cw, ch) = (area.w / cols as f64, area.h / rows as f64);
    let mut out = vec![RectF::default(); n];
    for (slot, &i) in order.iter().enumerate() {
        let r = &frames[i];
        let (row, col) = (slot / cols, slot % cols);
        let in_row = if row == rows - 1 { n - row * cols } else { cols };
        let x0 = area.x + (area.w - in_row as f64 * cw) / 2.0;
        let s = scale(r, cw, ch);
        let (tw, th) = (r.w * s, r.h * s);
        let cx = x0 + (col as f64 + 0.5) * cw;
        let cy = area.y + (row as f64 + 0.5) * ch - 10.0;
        out[i] = RectF::new(cx - tw / 2.0, cy - th / 2.0, tw, th);
    }
    out
}

/// Thumbnail of a window being dragged: shrunk to at most 200 px wide around the grab point.
pub fn drag_thumb(t: RectF, start: (f64, f64), pos: (f64, f64)) -> RectF {
    let k = (200.0 / t.w).min(1.0);
    let (w, h) = (t.w * k, t.h * k);
    let gx = (start.0 - t.x) / t.w;
    let gy = (start.1 - t.y) / t.h;
    RectF::new(pos.0 - gx * w, pos.1 - gy * h, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlaps(a: &RectF, b: &RectF) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    #[test]
    fn grid_is_non_overlapping_and_inside_the_area() {
        let area = RectF::new(50.0, 96.0, 1340.0, 674.0);
        for n in 1..=12 {
            let frames: Vec<RectF> =
                (0..n).map(|i| RectF::new(i as f64 * 37.0 % 900.0, 40.0, 600.0 + i as f64 * 20.0, 400.0)).collect();
            let g = grid(&frames, area);
            assert_eq!(g.len(), n);
            for (i, a) in g.iter().enumerate() {
                assert!(a.x >= area.x - 1e-6 && a.x + a.w <= area.x + area.w + 1e-6, "n={n} {a:?}");
                assert!(a.y >= area.y - 10.0 && a.y + a.h <= area.y + area.h, "n={n} {a:?}");
                assert!(a.w <= frames[i].w * 0.85 + 1e-6);
                for b in &g[i + 1..] {
                    assert!(!overlaps(a, b), "n={n}");
                }
            }
        }
        assert!(grid(&[], area).is_empty());
    }

    #[test]
    fn grid_keeps_left_to_right_order_and_aspect() {
        let area = RectF::new(0.0, 0.0, 1400.0, 700.0);
        let frames = [RectF::new(900.0, 0.0, 800.0, 400.0), RectF::new(0.0, 0.0, 400.0, 800.0)];
        let g = grid(&frames, area);
        assert!(g[1].x < g[0].x, "the left window stays on the left");
        for (f, t) in frames.iter().zip(&g) {
            assert!((f.w / f.h - t.w / t.h).abs() < 1e-9);
        }
    }

    #[test]
    fn dragged_thumbnail_follows_the_pointer() {
        let t = RectF::new(100.0, 100.0, 400.0, 200.0);
        let d = drag_thumb(t, (300.0, 200.0), (600.0, 500.0));
        assert_eq!((d.w, d.h), (200.0, 100.0));
        assert_eq!((d.x + d.w / 2.0, d.y + d.h / 2.0), (600.0, 500.0));
        let small = drag_thumb(RectF::new(0.0, 0.0, 100.0, 50.0), (0.0, 0.0), (10.0, 10.0));
        assert_eq!((small.x, small.y, small.w), (10.0, 10.0, 100.0));
    }
}
