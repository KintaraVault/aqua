//! Spaces (virtual desktops). Every window belongs to one desk of one display. All windows
//! stay mapped; windows of other desks are shifted horizontally by `stride` per desk, so a
//! switch is a slide of the windows of that display only.
use std::time::{Duration, Instant};

pub const SPACE_MS: f32 = 520.0;
pub const MAX_DESKS: usize = 8;
/// Extra distance between desks so no window of another desk peeks onto any display.
pub const GAP: i32 = 60;

/// Desks of one display (or of all displays when they are shared).
#[derive(Clone, Debug, PartialEq)]
pub struct Desks {
    pub count: usize,
    pub cur: usize,
    /// (start, view at start) of the running slide towards `cur`.
    pub anim: Option<(Instant, f32)>,
}

impl Default for Desks {
    fn default() -> Self {
        Self { count: 1, cur: 0, anim: None }
    }
}

/// Ease-out with a small overshoot.
pub fn ease(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3) + 0.06 * (std::f32::consts::PI * t).sin() * (1.0 - t)
}

impl Desks {
    fn t(&self, now: Instant, dur: Duration) -> Option<f32> {
        let (t0, _) = self.anim?;
        let el = now.saturating_duration_since(t0).as_secs_f32();
        Some((el / dur.as_secs_f32().max(1e-6)).clamp(0.0, 1.0))
    }
    /// Fractional desk in view.
    pub fn view_at(&self, now: Instant, dur: Duration) -> f32 {
        match (self.anim, self.t(now, dur)) {
            (Some((_, from)), Some(t)) => from + (self.cur as f32 - from) * ease(t),
            _ => self.cur as f32,
        }
    }
    pub fn animating_at(&self, now: Instant, dur: Duration) -> bool {
        self.t(now, dur).map(|t| t < 1.0).unwrap_or(false)
    }
}

/// Spaces of every display.
#[derive(Clone, Debug)]
pub struct Workspaces {
    /// Every display has its own desks; otherwise all share `shared`.
    pub per_output: bool,
    shared: Desks,
    displays: Vec<(String, Desks)>,
    /// Animation slow-down factor (accessibility / debugging).
    pub slow: f32,
}

impl Default for Workspaces {
    fn default() -> Self {
        Self::new(true)
    }
}

impl Workspaces {
    pub fn new(per_output: bool) -> Self {
        Self { per_output, shared: Desks::default(), displays: vec![], slow: 1.0 }
    }

    pub fn duration(&self) -> Duration {
        Duration::from_secs_f32(SPACE_MS * self.slow.max(0.01) / 1000.0)
    }

    /// Desks of `display`.
    pub fn desks(&self, display: &str) -> &Desks {
        if !self.per_output {
            return &self.shared;
        }
        self.displays.iter().find(|(n, _)| n == display).map(|(_, d)| d).unwrap_or(&self.shared)
    }

    fn desks_mut(&mut self, display: &str) -> &mut Desks {
        if !self.per_output {
            return &mut self.shared;
        }
        if let Some(i) = self.displays.iter().position(|(n, _)| n == display) {
            return &mut self.displays[i].1;
        }
        self.displays.push((display.to_string(), Desks::default()));
        &mut self.displays.last_mut().expect("just pushed").1
    }

    pub fn cur(&self, display: &str) -> usize {
        self.desks(display).cur
    }
    pub fn count(&self, display: &str) -> usize {
        self.desks(display).count
    }
    /// Is `desk` of `display` the one in view?
    pub fn is_current(&self, display: &str, desk: usize) -> bool {
        self.cur(display) == desk
    }
    pub fn view_at(&self, display: &str, now: Instant) -> f32 {
        self.desks(display).view_at(now, self.duration())
    }
    pub fn view(&self, display: &str) -> f32 {
        self.view_at(display, Instant::now())
    }

    fn all(&self) -> impl Iterator<Item = &Desks> {
        std::iter::once(&self.shared).chain(self.displays.iter().map(|(_, d)| d))
    }
    /// A slide is still moving on some display.
    pub fn animating_at(&self, now: Instant) -> bool {
        let dur = self.duration();
        self.all().any(|d| d.animating_at(now, dur))
    }
    pub fn animating(&self) -> bool {
        self.animating_at(Instant::now())
    }
    /// A slide was started and not yet finished with [`Workspaces::finish`].
    pub fn has_anim(&self) -> bool {
        self.all().any(|d| d.anim.is_some())
    }
    /// Drop finished slides; returns whether any slide was running.
    pub fn finish(&mut self, now: Instant) -> bool {
        let dur = self.duration();
        let had = self.has_anim();
        for d in std::iter::once(&mut self.shared).chain(self.displays.iter_mut().map(|(_, d)| d)) {
            if d.anim.is_some() && !d.animating_at(now, dur) {
                d.anim = None;
            }
        }
        had
    }

    /// Show `target` on `display`; false if it does not exist or is already shown.
    pub fn switch(&mut self, display: &str, target: usize, now: Instant) -> bool {
        let dur = self.duration();
        let d = self.desks_mut(display);
        if target >= d.count || (target == d.cur && d.anim.is_none()) {
            return false;
        }
        let from = d.view_at(now, dur);
        d.cur = target;
        d.anim = Some((now, from));
        true
    }

    /// Move `delta` desks; at either end the view bounces a little instead. Returns the new
    /// desk when it changed.
    pub fn switch_rel(&mut self, display: &str, delta: i32, now: Instant) -> Option<usize> {
        let d = self.desks(display);
        let t = (d.cur as i32 + delta).clamp(0, d.count as i32 - 1) as usize;
        if t == d.cur {
            let from = d.cur as f32 + 0.04 * delta.signum() as f32;
            self.desks_mut(display).anim = Some((now, from));
            return None;
        }
        self.switch(display, t, now).then_some(t)
    }

    pub fn add(&mut self, display: &str) -> bool {
        let d = self.desks_mut(display);
        if d.count >= MAX_DESKS {
            return false;
        }
        d.count += 1;
        true
    }

    /// Remove desk `i` of `display`; its windows go to the previous desk (see [`remap_removed`]).
    pub fn remove(&mut self, display: &str, i: usize) -> bool {
        let d = self.desks_mut(display);
        if d.count <= 1 || i >= d.count {
            return false;
        }
        d.count -= 1;
        if d.cur > i || d.cur >= d.count {
            d.cur -= 1;
        }
        d.anim = None;
        true
    }

    /// A display appeared.
    pub fn add_display(&mut self, display: &str) {
        if self.per_output {
            self.desks_mut(display);
        }
    }

    /// A display went away: its desks are dropped. Windows on it move to `into`, onto
    /// desk [`fold_desk`] of that display.
    pub fn remove_display(&mut self, display: &str) {
        if self.per_output {
            self.displays.retain(|(n, _)| n != display);
        }
    }

    /// Switch between shared and per-display Spaces. Going per-display, every display starts
    /// with the shared desks; going shared keeps the desks of `primary`.
    pub fn set_per_output(&mut self, on: bool, displays: &[String], primary: &str) {
        if on == self.per_output {
            return;
        }
        if on {
            let base = Desks { anim: None, ..self.shared.clone() };
            self.displays = displays.iter().map(|n| (n.clone(), base.clone())).collect();
        } else {
            self.shared = Desks { anim: None, ..self.desks(primary).clone() };
            self.displays.clear();
        }
        self.per_output = on;
    }

    /// Display key windows use for their desks: the display itself, or "" when shared.
    pub fn key<'a>(&self, display: &'a str) -> &'a str {
        if self.per_output {
            display
        } else {
            ""
        }
    }
}

/// Desk of a window after desk `removed` was deleted.
pub fn remap_removed(desk: usize, removed: usize) -> usize {
    if desk == removed {
        removed.saturating_sub(1)
    } else if desk > removed {
        desk - 1
    } else {
        desk
    }
}

/// Desk a window keeps when it moves to a display with `count` desks.
pub fn fold_desk(desk: usize, count: usize) -> usize {
    desk.min(count.saturating_sub(1))
}

/// Horizontal distance between desks: wider than the whole layout.
pub fn stride(output_w: i32, layout_w: i32) -> i32 {
    output_w.max(layout_w) + GAP
}

/// Horizontal shift of a window on `desk` while `view` is in view.
pub fn desk_offset(desk: usize, view: f32, stride: i32) -> i32 {
    ((desk as f32 - view) * stride as f32).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn later(t: Instant, ms: u64) -> Instant {
        t + Duration::from_millis(ms)
    }

    #[test]
    fn per_display_spaces_are_independent() {
        let now = Instant::now();
        let mut ws = Workspaces::new(true);
        ws.add_display("A");
        ws.add_display("B");
        assert!(ws.add("A"));
        assert!(ws.add("A"));
        assert_eq!((ws.count("A"), ws.count("B")), (3, 1));
        assert!(ws.switch("A", 2, now));
        assert_eq!((ws.cur("A"), ws.cur("B")), (2, 0));
        assert!(!ws.switch("B", 1, now), "B has a single desk");
        assert!(ws.is_current("A", 2) && ws.is_current("B", 0));
        assert!(ws.animating_at(later(now, 10)));
        assert!(!ws.animating_at(later(now, 600)));
    }

    #[test]
    fn shared_spaces_switch_every_display() {
        let now = Instant::now();
        let mut ws = Workspaces::new(false);
        ws.add_display("A");
        ws.add_display("B");
        ws.add("A");
        assert_eq!(ws.count("B"), 2);
        ws.switch("B", 1, now);
        assert_eq!(ws.cur("A"), 1);
        assert_eq!(ws.key("A"), "");
    }

    #[test]
    fn slide_view_moves_from_old_to_new_desk() {
        let now = Instant::now();
        let mut ws = Workspaces::new(true);
        ws.add("A");
        ws.switch("A", 1, now);
        assert_eq!(ws.view_at("A", now), 0.0);
        let mid = ws.view_at("A", later(now, 260));
        assert!(mid > 0.5 && mid < 1.1, "{mid}");
        assert_eq!(ws.view_at("A", later(now, 2000)), 1.0);
        assert!(ws.finish(later(now, 2000)));
        assert!(!ws.has_anim());
        assert!(!ws.finish(later(now, 2001)));
    }

    #[test]
    fn switching_mid_slide_starts_from_the_current_view() {
        let now = Instant::now();
        let mut ws = Workspaces::new(true);
        ws.add("A");
        ws.add("A");
        ws.switch("A", 2, now);
        let t = later(now, 200);
        let v = ws.view_at("A", t);
        ws.switch("A", 0, t);
        assert!((ws.view_at("A", t) - v).abs() < 1e-4);
    }

    #[test]
    fn relative_switch_bounces_at_the_ends() {
        let now = Instant::now();
        let mut ws = Workspaces::new(true);
        ws.add("A");
        assert_eq!(ws.switch_rel("A", -1, now), None);
        assert!(ws.has_anim());
        assert_eq!(ws.cur("A"), 0);
        assert_eq!(ws.switch_rel("A", 1, now), Some(1));
        assert_eq!(ws.switch_rel("A", 5, now), None);
    }

    #[test]
    fn desk_limits_and_removal() {
        let mut ws = Workspaces::new(true);
        for _ in 0..20 {
            ws.add("A");
        }
        assert_eq!(ws.count("A"), MAX_DESKS);
        let now = Instant::now();
        ws.switch("A", 5, now);
        assert!(ws.remove("A", 2));
        assert_eq!(ws.cur("A"), 4, "current desk keeps showing the same windows");
        assert!(!ws.has_anim());
        assert!(!ws.remove("A", 99));
        let mut one = Workspaces::new(true);
        assert!(!one.remove("A", 0), "the last desk stays");
        assert_eq!([0, 1, 2, 3].map(|d| remap_removed(d, 1)), [0, 0, 1, 2]);
        assert_eq!(remap_removed(0, 0), 0);
        let mut last = Workspaces::new(true);
        last.add("A");
        last.switch("A", 1, now);
        last.remove("A", 1);
        assert_eq!(last.cur("A"), 0);
    }

    #[test]
    fn unplugging_and_mode_changes() {
        let mut ws = Workspaces::new(true);
        ws.add_display("A");
        ws.add_display("B");
        ws.add("B");
        ws.add("B");
        ws.remove_display("B");
        assert_eq!(ws.count("B"), 1, "unknown displays fall back to a single desk");
        assert_eq!(fold_desk(2, 1), 0);
        assert_eq!(fold_desk(1, 3), 1);
        ws.add("A");
        ws.set_per_output(false, &["A".into()], "A");
        assert_eq!(ws.count("anything"), 2);
        ws.set_per_output(true, &["A".into(), "C".into()], "A");
        assert_eq!((ws.count("A"), ws.count("C")), (2, 2));
    }

    #[test]
    fn offsets_keep_other_desks_off_screen() {
        let s = stride(1920, 1920 + 2560);
        assert_eq!(s, 4540);
        assert_eq!(desk_offset(0, 0.0, s), 0);
        assert_eq!(desk_offset(2, 0.0, s), 2 * s);
        assert_eq!(desk_offset(0, 1.0, s), -s);
        assert_eq!(desk_offset(1, 0.5, s), s / 2);
        assert!(ease(0.0).abs() < 1e-6 && (ease(1.0) - 1.0).abs() < 1e-6);
        assert!((0..=10).map(|i| ease(i as f32 / 10.0)).all(|v| (-0.01..=1.1).contains(&v)));
    }
}
