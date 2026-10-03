//! Mission Control: all windows of the desktop zoom out into a non-overlapping grid
//! over a blurred, dimmed wallpaper. Hover highlights, click brings a window forward.
use std::time::Instant;

use smithay::desktop::Window;
use smithay::utils::{Logical, Point, Rectangle};

use crate::state::{anim_slow, meta, Aqua};

pub const MISSION_MS: f32 = 380.0;

#[derive(Default)]
pub struct Mission {
    pub open: bool,
    /// Start of the current transition (linear progress is derived from it).
    pub t0: Option<Instant>,
    pub hover: Option<u64>,
    /// Spaces bar expansion (collapsed names <-> desktop thumbnails).
    pub bar: Tween,
    pub hover_desk: Option<usize>,
    pub hover_close: bool,
    pub hover_plus: bool,
    pub drag: Option<Drag>,
    /// App Exposé: only this app's windows are laid out (Dock click "expose").
    pub app_filter: Option<String>,
}

pub struct Drag {
    pub id: u64,
    pub start: Point<f64, Logical>,
    pub pos: Point<f64, Logical>,
    pub active: bool,
}

/// Reversible eased 0..1 transition.
#[derive(Default)]
pub struct Tween {
    pub on: bool,
    pub t0: Option<Instant>,
}

const BAR_MS: f32 = 260.0;

impl Tween {
    fn linear(&self) -> f32 {
        let Some(t0) = self.t0 else { return if self.on { 1.0 } else { 0.0 } };
        let x = (t0.elapsed().as_secs_f32() * 1000.0 / (BAR_MS * anim_slow())).clamp(0.0, 1.0);
        if self.on {
            x
        } else {
            1.0 - x
        }
    }
    pub fn value(&self) -> f32 {
        let x = self.linear();
        x * x * (3.0 - 2.0 * x)
    }
    pub fn set(&mut self, on: bool) {
        if on == self.on {
            return;
        }
        let x = self.linear();
        let start = if on { x } else { 1.0 - x };
        self.on = on;
        self.t0 = Some(Instant::now() - std::time::Duration::from_secs_f32(start * BAR_MS * anim_slow() / 1000.0));
    }
    pub fn animating(&self) -> bool {
        self.t0.map(|t| t.elapsed().as_secs_f32() * 1000.0 < BAR_MS * anim_slow()).unwrap_or(false)
    }
}

impl Mission {
    fn linear(&self) -> f32 {
        let Some(t0) = self.t0 else { return if self.open { 1.0 } else { 0.0 } };
        let x = (t0.elapsed().as_secs_f32() * 1000.0 / (MISSION_MS * anim_slow())).clamp(0.0, 1.0);
        if self.open {
            x
        } else {
            1.0 - x
        }
    }
    /// Eased progress 0 (desktop) .. 1 (Mission Control).
    pub fn progress(&self) -> f32 {
        let x = self.linear();
        1.0 - (1.0 - x).powi(3)
    }
    pub fn animating(&self) -> bool {
        self.t0.map(|t| t.elapsed().as_secs_f32() * 1000.0 < MISSION_MS * anim_slow()).unwrap_or(false)
            || self.bar.animating()
    }
}

impl Aqua {
    pub fn toggle_mission(&mut self) {
        let x = self.mission.linear();
        let open = !self.mission.open;
        let start_x = if open { x } else { 1.0 - x };
        let d = std::time::Duration::from_secs_f32(start_x * MISSION_MS * anim_slow() / 1000.0);
        self.mission.open = open;
        self.mission.t0 = Some(Instant::now() - d);
        self.mission.hover = None;
        self.mission.drag = None;
        self.mission.hover_desk = None;
        if !open {
            self.mission.bar.set(false);
        } else {
            self.mission.app_filter = None;
        }
        if open {
            self.shell.close_transients();
        }
        self.needs_redraw = true;
    }

    /// App Exposé: Mission Control showing only one app's windows.
    pub fn app_expose(&mut self, app_id: &str) {
        if !self.mission.open {
            self.toggle_mission();
        }
        self.mission.app_filter = Some(app_id.to_string());
        self.needs_redraw = true;
    }

    pub fn mission_visible(&self) -> bool {
        self.mission.open || self.mission.progress() > 0.0
    }

    /// (window, current frame, target thumbnail frame) for all mapped windows.
    pub fn mission_layout(&self) -> Vec<(Window, Rectangle<f64, Logical>, Rectangle<f64, Logical>)> {
        let wins: Vec<(Window, Rectangle<f64, Logical>)> = self
            .space
            .elements()
            .filter(|w| meta(w).borrow().placed && self.on_current_space(w))
            .filter(|w| match &self.mission.app_filter {
                Some(app) => self.windows_of_app(app).contains(w),
                None => true,
            })
            .filter_map(|w| self.frame_rect(w).map(|r| (w.clone(), r.to_f64())))
            .collect();
        let (ow, oh) = self.output_size();
        let collapsed_top = self.cfg.menubar_height as f64 + 70.0;
        let expanded_top = aqua_shell::mission::bar_bottom(&self.shell) as f64 + 26.0;
        let top = collapsed_top + (expanded_top - collapsed_top).max(0.0) * self.mission.bar.value() as f64;
        let bottom = oh as f64 - 130.0;
        let area = aqua_wm::RectF::new(50.0, top, ow as f64 - 100.0, bottom - top);
        let frames: Vec<aqua_wm::RectF> =
            wins.iter().map(|(_, r)| aqua_wm::RectF::new(r.loc.x, r.loc.y, r.size.w, r.size.h)).collect();
        let grid = aqua_wm::mission::grid(&frames, area);
        let rect = |r: aqua_wm::RectF| Rectangle::<f64, Logical>::new((r.x, r.y).into(), (r.w, r.h).into());
        let mut out = Vec::with_capacity(wins.len());
        for ((w, r), g) in wins.into_iter().zip(grid) {
            let mut t = rect(g);
            if let Some(d) = self.mission.drag.as_ref().filter(|d| d.active && d.id == meta(&w).borrow().id) {
                t = rect(aqua_wm::mission::drag_thumb(g, (d.start.x, d.start.y), (d.pos.x, d.pos.y)));
            }
            out.push((w, r, t));
        }
        out
    }

    pub fn mission_pick(&self, pos: Point<f64, Logical>) -> Option<Window> {
        self.mission_layout().into_iter().rev().find(|(_, _, t)| t.contains(pos)).map(|(w, _, _)| w)
    }

    /// Mirror the overlay state into the shell before building layers.
    pub fn sync_mission_shell(&mut self) {
        let p = self.mission.progress();
        if p <= 0.0 && !self.mission.open {
            self.mission.t0 = None;
            self.shell.mission.progress = 0.0;
            self.shell.mission.items.clear();
            return;
        }
        let items = self
            .mission_layout()
            .into_iter()
            .map(|(w, _, t)| {
                let (app_id, title) = crate::state::title_of(&w);
                aqua_shell::mission::Item {
                    id: meta(&w).borrow().id,
                    rect: aqua_gfx::Rect::new(t.loc.x as f32, t.loc.y as f32, t.size.w as f32, t.size.h as f32),
                    title: if title.is_empty() { app_id.clone() } else { title },
                    app_id,
                }
            })
            .collect();
        self.shell.mission.progress = p;
        self.shell.mission.items = items;
        self.shell.mission.hover =
            if self.mission.drag.as_ref().map(|d| d.active).unwrap_or(false) { None } else { self.mission.hover };
        self.sync_spaces_shell();
    }

    pub fn sync_spaces_shell(&mut self) {
        let d = self.active_display();
        let (desks, cur) = (self.spaces.count(&d), self.spaces.cur(&d));
        let m = &mut self.shell.mission;
        m.desks = desks;
        m.cur = cur;
        m.expand = self.mission.bar.value();
        m.hover_desk = self.mission.hover_desk;
        m.hover_close = self.mission.hover_close;
        m.hover_plus = self.mission.hover_plus;
        m.drop_desk = self.mission.drag.as_ref().filter(|d| d.active).and(self.mission.hover_desk);
    }

    fn bar_hit(&self, pos: Point<f64, Logical>) -> (Option<usize>, bool, bool) {
        let (x, y) = (pos.x as f32, pos.y as f32);
        let sh = &self.shell;
        if self.mission.bar.value() > 0.5 {
            let desks = aqua_shell::mission::desk_rects(sh);
            for (i, r) in desks.iter().enumerate() {
                if desks.len() > 1 && aqua_shell::mission::close_rect(*r).contains(x, y) {
                    return (Some(i), true, false);
                }
                if r.contains(x, y) {
                    return (Some(i), false, false);
                }
            }
            if aqua_shell::mission::plus_rect(sh).contains(x, y) {
                return (None, false, true);
            }
        } else {
            for (i, r) in aqua_shell::mission::name_rects(sh).iter().enumerate() {
                if r.contains(x, y) {
                    return (Some(i), false, false);
                }
            }
        }
        (None, false, false)
    }

    pub fn mission_motion(&mut self, pos: Point<f64, Logical>) {
        let mb = self.cfg.menubar_height as f64;
        let dragging = if let Some(d) = self.mission.drag.as_mut() {
            d.pos = pos;
            if !d.active && (pos - d.start).to_f64().x.hypot((pos - d.start).y) > 6.0 {
                d.active = true;
            }
            d.active
        } else {
            false
        };
        let bottom = aqua_shell::mission::bar_bottom(&self.shell) as f64;
        if pos.y < mb + 56.0 || (dragging && pos.y < bottom + 40.0) {
            self.mission.bar.set(true);
        } else if pos.y > bottom + 30.0 {
            self.mission.bar.set(false);
        }
        let (desk, close, plus) = self.bar_hit(pos);
        let h = if desk.is_some() || plus { None } else { self.mission_pick(pos).map(|w| meta(&w).borrow().id) };
        self.mission.hover = h;
        self.mission.hover_desk = desk;
        self.mission.hover_close = close;
        self.mission.hover_plus = plus;
        self.needs_redraw = true;
    }

    pub fn mission_button(&mut self, pos: Point<f64, Logical>, pressed: bool) {
        if pressed {
            let (desk, close, plus) = self.bar_hit(pos);
            if plus {
                self.add_space();
            } else if let (Some(i), true) = (desk, close) {
                self.remove_space(i);
                self.mission.hover_desk = None;
            } else if let Some(i) = desk {
                self.switch_space(i);
                self.toggle_mission();
            } else if let Some(w) = self.mission_pick(pos) {
                self.mission.drag = Some(Drag { id: meta(&w).borrow().id, start: pos, pos, active: false });
            } else {
                self.toggle_mission();
            }
            self.needs_redraw = true;
            return;
        }
        let Some(d) = self.mission.drag.take() else { return };
        let w = self.space.elements().find(|w| meta(w).borrow().id == d.id).cloned();
        let Some(w) = w else { return };
        if d.active {
            if let (Some(i), false) = (self.mission.hover_desk, self.mission.hover_close) {
                self.move_to_space(&w, i);
            }
        } else {
            self.focus_window(&w);
            self.toggle_mission();
        }
        self.needs_redraw = true;
    }
}
