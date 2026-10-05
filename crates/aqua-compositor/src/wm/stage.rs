//! Stage Manager: the focused app's windows stay on stage; the windows of every other app
//! on the current Space are unmapped and shown as small stages in a strip at the left
//! edge. Clicking a stage (or focusing one of its windows any other way: Dock, ⌘Tab,
//! activation) brings that app back and puts the previous one aside.
//! Layout maths live in `aqua_wm::stage`.
use super::to_rect;
use crate::state::{meta, title_of, Aqua};
use aqua_wm::stage as sm;
use smithay::desktop::Window;
use smithay::utils::{Logical, Point, Rectangle};
use std::time::Instant;

#[derive(Default)]
pub struct Stage {
    pub on: bool,
    /// App on stage.
    pub active: Option<String>,
    /// Apps in the strip, most recent first.
    pub order: Vec<String>,
    /// Windows put aside (unmapped from the space).
    pub staged: Vec<Window>,
}

impl Aqua {
    /// Turn Stage Manager on or off (Control Centre, `aqua msg action stage-manager`).
    pub fn set_stage_manager(&mut self, on: bool) {
        if self.stage.on == on {
            return;
        }
        self.stage.on = on;
        self.shell.control.stage = on;
        if on {
            self.stage.active = self.focused_window().map(|w| title_of(&w).0);
            self.stage_tick();
        } else {
            for w in std::mem::take(&mut self.stage.staged) {
                self.unstage_window(&w);
            }
            self.stage.order.clear();
            self.stage.active = None;
            if let Some(f) = self.focused_window() {
                self.focus_window(&f);
            }
        }
        self.needs_redraw = true;
    }

    pub fn is_staged(&self, w: &Window) -> bool {
        self.stage.staged.contains(w)
    }

    /// A staged window closed or was destroyed.
    pub fn forget_staged(&mut self, w: &Window) {
        self.stage.staged.retain(|s| s != w);
        let apps: Vec<String> = self.stage.staged.iter().map(|w| title_of(w).0).collect();
        self.stage.order.retain(|a| apps.contains(a));
    }

    /// Windows that belong on stage only with their app (dialogs, menus excluded).
    fn stageable(&self, w: &Window) -> bool {
        let m = meta(w).borrow();
        m.placed && !m.override_redirect && !m.menu_popup && m.minimizing.is_none() && m.fullscreen.is_none()
    }

    /// Per frame: follow the focus and put aside windows of other apps. Returns true while
    /// the strip animates.
    pub fn stage_tick(&mut self) -> bool {
        if !self.stage.on || self.mission.open || self.lock.is_locked() {
            return false;
        }
        if let Some(f) = self.focused_window() {
            let app = title_of(&f).0;
            if !app.is_empty() && self.stage.active.as_deref() != Some(app.as_str()) {
                if let Some(old) = self.stage.active.replace(app.clone()) {
                    sm::touch(&mut self.stage.order, &old);
                }
                self.stage.order.retain(|a| a != &app);
            }
        }
        let Some(active) = self.stage.active.clone() else { return false };
        let aside: Vec<Window> = self
            .space
            .elements()
            .filter(|w| self.on_current_space(w) && self.stageable(w))
            .filter(|w| {
                let app = title_of(w).0;
                !app.is_empty() && app != active
            })
            .cloned()
            .collect();
        for w in aside {
            let app = title_of(&w).0;
            if !self.stage.order.contains(&app) {
                sm::touch(&mut self.stage.order, &app);
            }
            self.stage_window(&w);
        }
        self.stage
            .staged
            .iter()
            .any(|w| meta(w).borrow().stage_from.map(|(_, t)| t.elapsed().as_secs_f32() < sm::ANIM_SECS).unwrap_or(false))
    }

    fn stage_window(&mut self, w: &Window) {
        let Some(frame) = self.frame_rect(w) else { return };
        let Some(loc) = self.space.element_location(w) else { return };
        meta(w).borrow_mut().stage_from = Some((frame, Instant::now()));
        meta(w).borrow_mut().min_loc = Some(loc);
        // like a minimised window: unmapped, but still "on" its outputs so the client keeps
        // drawing (the thumbnail stays live) and X11 clients are not told anything
        use smithay::desktop::space::SpaceElement;
        let outs = self.space.outputs_for_element(w);
        self.space.unmap_elem(w);
        let size = w.geometry().size;
        for o in &outs {
            w.output_enter(o, Rectangle::new((0, 0).into(), size));
        }
        self.stage.staged.push(w.clone());
        self.needs_redraw = true;
    }

    fn unstage_window(&mut self, w: &Window) {
        let loc = meta(w).borrow_mut().min_loc.take().unwrap_or((200, 120).into());
        meta(w).borrow_mut().stage_from = None;
        let loc = match self.output_at(loc.to_f64()).or_else(|| self.output.clone()) {
            Some(o) if self.stage.on => {
                let u = to_rect(self.usable_area(&o));
                Point::<i32, Logical>::from((sm::on_stage_x(loc.x, w.geometry().size.w, u), loc.y))
            }
            _ => loc,
        };
        self.space.map_element(w.clone(), loc, false);
    }

    /// Bring an app from the strip on stage (its windows keep their stacking order).
    pub fn stage_bring(&mut self, app: &str) {
        let wins: Vec<Window> = self.stage.staged.iter().filter(|w| title_of(w).0 == app).cloned().collect();
        if wins.is_empty() {
            return;
        }
        self.stage.staged.retain(|w| !wins.contains(w));
        for w in &wins {
            self.unstage_window(w);
        }
        if let Some(last) = wins.last() {
            self.focus_window(last);
        }
        self.stage_tick();
    }

    /// Apps shown in the strip with their slot rects (logical) on the primary display.
    pub fn stage_slots(&self) -> Vec<(String, aqua_wm::RectF)> {
        if !self.stage.on {
            return vec![];
        }
        let Some(o) = self.output.clone() else { return vec![] };
        let apps: Vec<String> = self
            .stage
            .order
            .iter()
            .filter(|a| self.stage.staged.iter().any(|w| &title_of(w).0 == *a))
            .cloned()
            .collect();
        let slots = sm::slots(to_rect(self.usable_area(&o)), apps.len());
        apps.into_iter().zip(slots).collect()
    }

    /// Click on the desktop: a stage under the pointer comes on stage.
    pub fn stage_click(&mut self, pos: Point<f64, Logical>) -> bool {
        let slots = self.stage_slots();
        let rects: Vec<aqua_wm::RectF> = slots.iter().map(|(_, r)| *r).collect();
        let Some(i) = sm::hit(&rects, (pos.x, pos.y)) else { return false };
        let app = slots[i].0.clone();
        self.stage_bring(&app);
        true
    }

    /// Thumbnails to draw: (window, rect, alpha), back to front within each stage.
    pub fn stage_thumbs(&self) -> Vec<(Window, Rectangle<f64, Logical>, f32)> {
        let mut out = vec![];
        for (app, slot) in self.stage_slots() {
            let wins: Vec<&Window> = self.stage.staged.iter().rev().filter(|w| title_of(w).0 == app).take(3).collect();
            let sizes: Vec<(i32, i32)> = wins.iter().map(|w| (w.geometry().size.w, w.geometry().size.h)).collect();
            let rects = sm::thumbs(slot, &sizes);
            for (w, r) in wins.iter().zip(rects).rev() {
                let (k, from) = match meta(w).borrow().stage_from {
                    Some((f, t)) if !crate::state::reduce_motion() => (
                        sm::progress(t.elapsed().as_secs_f32() / crate::state::anim_slow()) as f64,
                        aqua_wm::RectF::new(f.loc.x as f64, f.loc.y as f64, f.size.w as f64, f.size.h as f64),
                    ),
                    _ => (1.0, r),
                };
                let cur = sm::lerp(from, r, k);
                let lr = Rectangle::new((cur.x, cur.y).into(), (cur.w, cur.h).into());
                out.push(((*w).clone(), lr, 1.0));
            }
        }
        out
    }
}
