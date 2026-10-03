//! Smooth scrolling for shell panels (Applications, Spotlight, Character Viewer).
//!
//! Mouse wheels deliver coarse notches (15 px each): they are amplified and eased
//! toward the target so a notch glides instead of jumping. Touchpads already deliver
//! fine, continuous deltas: they follow the fingers almost 1:1 with light smoothing.

/// Logical px a wheel notch (15 units from libinput) scrolls, as a multiplier.
pub const WHEEL_GAIN: f32 = 5.0;
/// Touchpad / continuous deltas.
pub const FINGER_GAIN: f32 = 1.35;

#[derive(Default, Clone, Copy, Debug)]
pub struct Smooth {
    /// Position drawn this frame.
    pub pos: f32,
    /// Where the scroll is heading.
    pub target: f32,
    pub max: f32,
}

impl Smooth {
    pub fn reset(&mut self) {
        self.pos = 0.0;
        self.target = 0.0;
    }

    /// Jump without animation (keyboard navigation that must be visible at once).
    pub fn jump(&mut self, v: f32) {
        self.target = v.clamp(0.0, self.max);
        self.pos = self.target;
    }

    /// Glide to `v` (keeping a selection visible).
    pub fn glide(&mut self, v: f32) {
        self.target = v.clamp(0.0, self.max);
    }

    pub fn set_max(&mut self, m: f32) {
        self.max = m.max(0.0);
        self.target = self.target.clamp(0.0, self.max);
        self.pos = self.pos.clamp(0.0, self.max);
    }

    /// Apply a scroll delta (logical px, positive = content moves up).
    pub fn scroll(&mut self, dy: f32, wheel: bool) {
        let d = dy * if wheel { WHEEL_GAIN } else { FINGER_GAIN };
        self.target = (self.target + d).clamp(0.0, self.max);
        if !wheel {
            self.pos += (self.target - self.pos) * 0.65;
        }
    }

    /// Advance the easing; true while still moving.
    pub fn animate(&mut self, dt: f32) -> bool {
        let d = self.target - self.pos;
        if d.abs() < 0.3 {
            self.pos = self.target;
            return false;
        }
        self.pos += d * (1.0 - (-16.0 * dt).exp());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wheel_glides_and_clamps() {
        let mut s = Smooth { max: 500.0, ..Default::default() };
        s.scroll(15.0, true);
        assert_eq!(s.target, 75.0);
        assert!(s.pos < 1.0);
        for _ in 0..120 {
            s.animate(1.0 / 60.0);
        }
        assert!((s.pos - 75.0).abs() < 0.01);
        s.scroll(10_000.0, true);
        assert_eq!(s.target, 500.0);
        s.scroll(-10_000.0, false);
        assert_eq!(s.target, 0.0);
    }
}
