//! Auto-hiding of the menu bar and the Dock.
use super::*;

impl Shell {
    /// Slide the menu bar up / the Dock down when they hide (full screen, auto-hide)
    /// and back when revealed — eased (~0.3 s).
    pub(super) fn animate_hiding(&mut self, dt: f32) -> bool {
        let instant = self.cfg.reduce_motion;
        let dt = dt / aqua_config::anim_slow();
        let step = |v: &mut f32, target: f32, secs: f32| -> bool {
            if (*v - target).abs() < 1e-4 {
                *v = target;
                return false;
            }
            if instant {
                *v = target;
                return true;
            }
            let d = dt / secs;
            *v = if target > *v { (*v + d).min(target) } else { (*v - d).max(target) };
            true
        };
        let bar_t = if self.bar_shown() { 0.0 } else { 1.0 };
        let dock_t = if self.dock_shown() { 0.0 } else { 1.0 };
        let a = step(&mut self.bar_hide, bar_t, if bar_t > 0.5 { 0.30 } else { 0.22 });
        let b = step(&mut self.dock_hide, dock_t, if dock_t > 0.5 { 0.32 } else { 0.24 });
        a || b
    }

    /// Eased slide amount (logical px) of the Dock below its resting position.
    pub fn dock_offset(&self) -> f32 {
        if self.dock_hide <= 0.0 {
            return 0.0;
        }
        let t = self.dock_hide.clamp(0.0, 1.0);
        let e = t * t * (3.0 - 2.0 * t);
        let travel = self.cfg.dock_icon_size
            + aqua_config::metrics::DOCK_PADDING * 2.0
            + aqua_config::metrics::DOCK_BOTTOM_MARGIN
            + 24.0;
        e * travel
    }

    /// Eased slide amount (logical px) of the menu bar above the screen edge.
    pub fn bar_offset(&self) -> f32 {
        let t = self.bar_hide.clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t) * (self.cfg.menubar_height + 2.0)
    }

    /// Menu bar visible (always, unless a full-screen window hides it).
    pub fn bar_shown(&self) -> bool {
        !self.bar_hides() || (!self.fullscreen && self.fs_reveal_bar) || self.menu.open.is_some()
    }

    /// "Automatically hide and show the menu bar": "fullscreen" (default),
    /// "always" or "never".
    pub(super) fn bar_hides(&self) -> bool {
        match self.cfg.menubar_autohide.as_str() {
            "never" => false,
            "always" => !self.locked,
            _ => self.fullscreen,
        }
    }

    /// Dock visible: always, unless a full-screen window or "Automatically hide and show
    /// the Dock" tucks it away until the pointer reaches the bottom edge.
    pub fn dock_shown(&self) -> bool {
        !self.dock_hides() || (!self.fullscreen && self.fs_reveal_dock)
    }

    pub(super) fn dock_hides(&self) -> bool {
        self.fullscreen || self.cfg.dock_autohide
    }
}
