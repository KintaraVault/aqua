//! Stage Manager (macOS 13+): the front app's windows stay on stage, every other app is
//! put aside as a small "stage" in a strip at the left edge; clicking one swaps it in.
use crate::{Rect, RectF};

/// Width of the strip (logical px), including its margins.
pub const STRIP_W: i32 = 168;
/// Gap between the strip and the windows on stage.
pub const STAGE_GAP: i32 = 12;
/// At most this many stages are shown (most recent first).
pub const MAX_STAGES: usize = 6;
/// Height of one stage slot and the gap between slots.
const SLOT_H: f64 = 118.0;
const SLOT_GAP: f64 = 18.0;
/// Seconds a window takes to move into the strip.
pub const ANIM_SECS: f32 = 0.28;

/// Most recently used first: `app` moves to the front.
pub fn touch(order: &mut Vec<String>, app: &str) {
    order.retain(|a| a != app);
    order.insert(0, app.to_string());
}

/// Slots of the strip for `n` stages, vertically centred in `usable` (the display minus
/// menu bar and Dock).
pub fn slots(usable: Rect, n: usize) -> Vec<RectF> {
    let fit = (((usable.h as f64 - 24.0) + SLOT_GAP) / (SLOT_H + SLOT_GAP)).floor().max(0.0) as usize;
    let n = n.min(MAX_STAGES).min(fit);
    if n == 0 {
        return vec![];
    }
    let total = n as f64 * SLOT_H + (n - 1) as f64 * SLOT_GAP;
    let y0 = usable.y as f64 + (usable.h as f64 - total) / 2.0;
    let x = usable.x as f64 + 14.0;
    let w = (STRIP_W - 28) as f64;
    (0..n).map(|i| RectF::new(x, y0 + i as f64 * (SLOT_H + SLOT_GAP), w, SLOT_H)).collect()
}

/// Index of the slot under `p`.
pub fn hit(slots: &[RectF], p: (f64, f64)) -> Option<usize> {
    slots.iter().position(|r| r.contains(p.0, p.1))
}

/// Thumbnails of up to three windows (front first) inside a slot: each keeps its aspect,
/// the ones behind peek out up and to the right, like macOS's stacked stages.
pub fn thumbs(slot: RectF, sizes: &[(i32, i32)]) -> Vec<RectF> {
    let n = sizes.len().min(3);
    let step = 7.0;
    let bw = slot.w - step * (n.saturating_sub(1)) as f64;
    let bh = slot.h - 18.0 - step * (n.saturating_sub(1)) as f64;
    (0..n)
        .map(|i| {
            let (w, h) = (sizes[i].0.max(1) as f64, sizes[i].1.max(1) as f64);
            let k = (bw / w).min(bh / h).min(0.5);
            let (tw, th) = (w * k, h * k);
            let off = i as f64 * step;
            let cx = slot.x + bw / 2.0 + off;
            let bottom = slot.y + slot.h - 18.0 - off;
            RectF::new(cx - tw / 2.0, bottom - th, tw, th)
        })
        .collect()
}

/// Where a window coming back on stage goes: its old place, moved right if it would sit
/// under the strip (as far as the display allows).
pub fn on_stage_x(x: i32, w: i32, usable: Rect) -> i32 {
    let left = usable.x + STRIP_W + STAGE_GAP;
    if x >= left {
        return x;
    }
    let max_x = (usable.right() - w).max(usable.x);
    left.min(max_x)
}

/// Animation progress of a window staged `t` seconds ago (ease-out, 0 → 1).
pub fn progress(t: f32) -> f32 {
    let x = (t / ANIM_SECS).clamp(0.0, 1.0);
    1.0 - (1.0 - x).powi(3)
}

/// Interpolate from the window's place on screen to its thumbnail.
pub fn lerp(from: RectF, to: RectF, k: f64) -> RectF {
    RectF::new(
        from.x + (to.x - from.x) * k,
        from.y + (to.y - from.y) * k,
        from.w + (to.w - from.w) * k,
        from.h + (to.h - from.h) * k,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mru_order() {
        let mut o = vec![];
        touch(&mut o, "a");
        touch(&mut o, "b");
        touch(&mut o, "a");
        assert_eq!(o, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn slots_are_centred_and_capped() {
        let usable = Rect::new(0, 30, 1440, 800);
        let s = slots(usable, 3);
        assert_eq!(s.len(), 3);
        let top = s[0].y - 30.0;
        let bottom = 830.0 - (s[2].y + s[2].h);
        assert!((top - bottom).abs() < 1.0, "centred: {top} vs {bottom}");
        assert!(s.iter().all(|r| r.x >= 0.0 && r.x + r.w <= STRIP_W as f64));
        assert_eq!(slots(usable, 20).len(), 5, "only as many as fit (800px → 5)");
        assert_eq!(slots(Rect::new(0, 0, 1440, 2000), 20).len(), MAX_STAGES);
        assert!(slots(Rect::new(0, 0, 100, 60), 2).is_empty());
        assert_eq!(hit(&s, (s[1].x + 5.0, s[1].y + 5.0)), Some(1));
        assert_eq!(hit(&s, (900.0, 400.0)), None);
    }

    #[test]
    fn thumbs_fit_their_slot() {
        let slot = RectF::new(14.0, 100.0, 140.0, 118.0);
        let t = thumbs(slot, &[(1200, 800), (400, 900), (800, 600), (10, 10)]);
        assert_eq!(t.len(), 3, "at most three windows per stage");
        for r in &t {
            assert!(r.x >= slot.x - 0.01 && r.x + r.w <= slot.x + slot.w + 0.01, "{r:?}");
            assert!(r.y >= slot.y - 0.01 && r.y + r.h <= slot.y + slot.h + 0.01, "{r:?}");
        }
        assert!((t[0].w / t[0].h - 1.5).abs() < 0.01, "aspect kept");
        assert!(t[1].y + t[1].h < t[0].y + t[0].h, "the next window peeks out above");
        assert!(thumbs(slot, &[]).is_empty());
    }

    #[test]
    fn windows_clear_the_strip() {
        let u = Rect::new(0, 30, 1440, 800);
        assert_eq!(on_stage_x(400, 600, u), 400);
        assert_eq!(on_stage_x(20, 600, u), STRIP_W + STAGE_GAP);
        assert_eq!(on_stage_x(0, 1400, u), 40, "a wide window stays on the display");
        assert_eq!(on_stage_x(0, 2000, u), 0);
    }

    #[test]
    fn animation_curve() {
        assert_eq!(progress(0.0), 0.0);
        assert_eq!(progress(10.0), 1.0);
        assert!(progress(ANIM_SECS / 2.0) > 0.5, "ease-out");
        let a = RectF::new(0.0, 0.0, 100.0, 100.0);
        let b = RectF::new(10.0, 20.0, 50.0, 30.0);
        assert_eq!(lerp(a, b, 1.0), b);
        assert_eq!(lerp(a, b, 0.0), a);
    }
}
