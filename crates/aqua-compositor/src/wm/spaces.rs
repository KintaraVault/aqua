//! Spaces (multiple desktops), per display by default (`spaces_per_output`). Every window
//! belongs to one desk of one display (`WinMeta::{display, desk}`). All windows stay mapped
//! in the smithay `Space`; windows of other desks are placed one stride per desk to the
//! left/right, so switching is a horizontal slide of that display's windows. The rules live
//! in `aqua_wm::spaces`; this module applies them to Smithay windows.
use std::time::Instant;

use smithay::desktop::Window;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::utils::{Logical, Point, SERIAL_COUNTER};

use crate::state::{anim_slow, meta, Aqua};
use aqua_wm::spaces::{desk_offset, fold_desk, remap_removed};

/// Spaces of every display (model in `aqua-wm`).
pub type Spaces = aqua_wm::Workspaces;

impl Aqua {
    /// Name of the primary display ("" before any output exists).
    pub fn primary_display(&self) -> String {
        self.output.as_ref().map(|o| o.name()).unwrap_or_default()
    }

    /// Display whose Spaces a window belongs to (its desks key).
    pub fn win_display(&self, w: &Window) -> String {
        let d = meta(w).borrow().display.clone();
        let d = if d.is_empty() { self.primary_display() } else { d };
        self.spaces.key(&d).to_string()
    }

    /// The display the user works on: the one under the pointer, else the primary.
    pub fn active_display(&self) -> String {
        let p = self.seat.get_pointer().map(|p| p.current_location());
        let d = p.and_then(|p| self.output_at(p)).map(|o| o.name()).unwrap_or_else(|| self.primary_display());
        self.spaces.key(&d).to_string()
    }

    /// Desk in view on the active display.
    pub fn cur_desk(&self) -> usize {
        self.spaces.cur(&self.active_display())
    }

    /// Horizontal distance between Spaces: wider than the whole multi-monitor layout so
    /// windows of other Spaces never show up on any display.
    pub fn desk_stride_px(&self) -> i32 {
        aqua_wm::spaces::stride(self.output_size().0, self.layout_width())
    }

    /// Location a window has when its desktop is the one in view.
    pub fn home_loc(&self, w: &Window) -> Option<Point<i32, Logical>> {
        let loc = self.space.element_location(w)?;
        let view = self.spaces.view(&self.win_display(w));
        let d = desk_offset(meta(w).borrow().desk, view, self.desk_stride_px());
        Some((loc.x - d, loc.y).into())
    }

    /// Re-place every window for the current (possibly fractional) views, keeping z-order.
    fn place_all(&mut self, homes: &[(Window, Point<i32, Logical>)]) {
        let stride = self.desk_stride_px();
        for (w, home) in homes {
            if self.minimized.contains(w) || !self.space.elements().any(|e| e == w) {
                continue;
            }
            let view = self.spaces.view(&self.win_display(w));
            let loc = (home.x + desk_offset(meta(w).borrow().desk, view, stride), home.y);
            if self.space.element_location(w).map(|l| (l.x, l.y)) != Some(loc) {
                self.space.map_element(w.clone(), loc, false);
            }
        }
    }

    fn homes(&self) -> Vec<(Window, Point<i32, Logical>)> {
        self.space.elements().filter_map(|w| self.home_loc(w).map(|h| (w.clone(), h))).collect()
    }

    /// Remember home positions before a slide starts (none running yet).
    fn snapshot_homes(&mut self) {
        if !self.spaces.has_anim() {
            self.spaces_homes = self.homes();
        }
    }

    fn current_homes(&self) -> Vec<(Window, Point<i32, Logical>)> {
        if self.spaces.has_anim() {
            self.spaces_homes.clone()
        } else {
            self.homes()
        }
    }

    /// Advance the slide animations (called once per frame).
    pub fn tick_spaces(&mut self) {
        if !self.spaces.has_anim() {
            return;
        }
        self.spaces.slow = anim_slow();
        let homes = self.spaces_homes.clone();
        self.spaces.finish(Instant::now());
        self.place_all(&homes);
        self.needs_redraw = true;
    }

    /// Focus the front window of the desk now shown on `display`.
    fn focus_desk(&mut self, display: &str) {
        let cur = self.spaces.cur(display);
        let next = self
            .space
            .elements()
            .rfind(|w| {
                let m = meta(w).borrow();
                m.desk == cur && m.minimizing.is_none() && !m.override_redirect
            })
            .filter(|w| self.win_display(w) == display)
            .cloned();
        let serial = SERIAL_COUNTER.next_serial();
        if let Some(kb) = self.seat.get_keyboard() {
            kb.set_focus(self, next.map(Into::into), serial);
        }
        self.update_activation();
    }

    pub fn switch_space(&mut self, target: usize) {
        let d = self.active_display();
        self.switch_space_on(&d, target);
    }

    pub fn switch_space_on(&mut self, display: &str, target: usize) {
        self.spaces.slow = anim_slow();
        self.snapshot_homes();
        if !self.spaces.switch(display, target, Instant::now()) {
            return;
        }
        self.focus_desk(display);
        self.shell.close_transients();
        self.needs_redraw = true;
    }

    pub fn switch_space_rel(&mut self, delta: i32) {
        let d = self.active_display();
        self.spaces.slow = anim_slow();
        self.snapshot_homes();
        if self.spaces.switch_rel(&d, delta, Instant::now()).is_some() {
            self.focus_desk(&d);
            self.shell.close_transients();
        }
        self.needs_redraw = true;
    }

    pub fn add_space(&mut self) {
        let d = self.active_display();
        if self.spaces.add(&d) {
            self.needs_redraw = true;
        }
    }

    /// Remove a desktop of the active display; its windows move to the previous one.
    pub fn remove_space(&mut self, i: usize) {
        let d = self.active_display();
        let homes = self.current_homes();
        if !self.spaces.remove(&d, i) {
            return;
        }
        for w in self.space.elements().chain(self.minimized.iter()) {
            if self.win_display(w) == d {
                let mut m = meta(w).borrow_mut();
                m.desk = remap_removed(m.desk, i);
            }
        }
        self.place_all(&homes);
        self.needs_redraw = true;
    }

    /// Move a window to another desktop of its display (Mission Control drag & drop, ⌃⇧←/→).
    pub fn move_to_space(&mut self, w: &Window, desk: usize) {
        let display = self.win_display(w);
        if desk >= self.spaces.count(&display) || meta(w).borrow().desk == desk {
            return;
        }
        let homes = self.current_homes();
        meta(w).borrow_mut().desk = desk;
        self.place_all(&homes);
        if self.spaces.has_anim() {
            self.spaces_homes = homes;
        }
        if self.focused_window().as_ref() == Some(w) {
            self.focus_desk(&display);
        }
        self.needs_redraw = true;
    }

    /// A window was dragged: when it now sits on another display it joins that display's
    /// current desk.
    pub fn window_moved(&mut self, w: &Window) {
        let Some(g) = self.space.element_geometry(w) else { return };
        let outs: Vec<smithay::output::Output> = self.outputs.list.clone();
        let rects: Vec<aqua_wm::Rect> =
            outs.iter().map(|o| self.space.output_geometry(o).map(crate::wm::to_rect).unwrap_or_default()).collect();
        let Some(i) = aqua_wm::place::display_of(crate::wm::to_rect(g), &rects) else { return };
        let name = outs[i].name();
        if self.spaces.key(&name) == self.win_display(w) {
            return;
        }
        let cur = self.spaces.cur(self.spaces.key(&name));
        let mut m = meta(w).borrow_mut();
        m.display = name;
        m.desk = cur;
    }

    /// Displays came or went: drop Spaces of vanished displays, their windows join the
    /// primary display (keeping their desk when it exists there).
    pub fn sync_display_spaces(&mut self) {
        let names: Vec<String> =
            self.outputs.list.iter().chain(self.outputs.virtuals.iter()).map(|o| o.name()).collect();
        for n in &names {
            self.spaces.add_display(n);
        }
        let primary = self.primary_display();
        let count = self.spaces.count(self.spaces.key(&primary));
        for w in self.space.elements().chain(self.minimized.iter()) {
            let mut m = meta(w).borrow_mut();
            if !m.display.is_empty() && !names.contains(&m.display) {
                m.display = primary.clone();
                m.desk = fold_desk(m.desk, count);
            }
        }
    }

    /// Config switch between shared and per-display Spaces.
    pub fn set_spaces_per_output(&mut self, on: bool) {
        if self.spaces.per_output == on {
            return;
        }
        let homes = self.current_homes();
        let names: Vec<String> = self.outputs.list.iter().map(|o| o.name()).collect();
        let primary = self.primary_display();
        self.spaces.set_per_output(on, &names, &primary);
        for w in self.space.elements().chain(self.minimized.iter()) {
            let count = self.spaces.count(&self.win_display(w));
            let mut m = meta(w).borrow_mut();
            m.desk = fold_desk(m.desk, count);
        }
        self.place_all(&homes);
        self.needs_redraw = true;
    }

    pub fn on_current_space(&self, w: &Window) -> bool {
        self.spaces.is_current(&self.win_display(w), meta(w).borrow().desk)
    }

    /// Is the front-most window on the current Space full-screen? Counts xdg/X11
    /// full-screen state and borderless windows covering the whole output (games, video
    /// players); transient dialogs above a full-screen window keep it full-screen.
    pub fn front_is_fullscreen(&mut self) -> bool {
        let mut guess = None;
        let mut res = false;
        for w in self.space.elements().rev() {
            let (or, leaving, id) = {
                let m = meta(w).borrow();
                (m.override_redirect, matches!(m.minimizing, Some((_, false))), m.id)
            };
            // A window flying into the Dock no longer owns the screen.
            if or || leaving || !self.on_current_space(w) {
                continue;
            }
            if Self::wants_fullscreen(w) {
                res = true;
                break;
            }
            if self.covers_output(w) && Self::settled(w) {
                guess = Some(id);
                break;
            }
            let transient = w.toplevel().map(|t| t.parent().is_some()).unwrap_or(false)
                || w.x11_surface().map(|x| x.is_transient_for().is_some()).unwrap_or(false);
            if transient {
                continue;
            }
            break;
        }
        // "Covers the output" is a guess (games, players, F11 in browsers). Clients briefly
        // commit oversized frames while they maximise/restore (shadows, the old size), so
        // only a window that keeps covering the output for a moment counts — otherwise the
        // menu bar and the Dock blink away and back.
        match guess {
            None => self.fs_guess = None,
            Some(id) => match self.fs_guess {
                Some((g, since)) if g == id => {
                    res = since.elapsed() >= FS_GUESS_DELAY;
                    if !res {
                        self.needs_redraw = true;
                    }
                }
                _ => {
                    self.fs_guess = Some((id, std::time::Instant::now()));
                    self.needs_redraw = true;
                }
            },
        }
        res
    }

    /// The client asked for full screen (xdg / X11 state).
    fn wants_fullscreen(w: &Window) -> bool {
        meta(w).borrow().fullscreen.is_some() || w.x11_surface().map(|x| x.is_fullscreen()).unwrap_or(false)
    }

    /// Not maximised and not animating (opening, zooming, minimising/restoring): only then
    /// can covering the output mean "full screen".
    fn settled(w: &Window) -> bool {
        let maximized = w
            .toplevel()
            .map(|t| t.with_pending_state(|s| s.states.contains(xdg_toplevel::State::Maximized)))
            .or_else(|| w.x11_surface().map(|x| x.is_maximized()))
            .unwrap_or(false);
        let animating = {
            let m = meta(w).borrow();
            m.geo_anim.is_some() || m.minimizing.is_some()
        };
        !maximized && !animating && crate::state::open_progress(w) >= 1.0
    }
}

/// How long a borderless window must cover the output before it counts as full screen.
const FS_GUESS_DELAY: std::time::Duration = std::time::Duration::from_millis(250);

impl Aqua {
    /// Full-screen window: xdg/X11 full-screen state, or a borderless window covering the
    /// whole output.
    pub fn covers_output(&self, w: &Window) -> bool {
        if meta(w).borrow().fullscreen.is_some() || w.x11_surface().map(|x| x.is_fullscreen()).unwrap_or(false) {
            return true;
        }
        let out = self.output.as_ref().and_then(|o| self.space.output_geometry(o));
        if let (Some(o), Some(g)) = (out, self.space.element_geometry(w)) {
            return aqua_wm::place::covers_display(
                crate::wm::to_rect(g),
                crate::wm::to_rect(o),
                crate::state::is_ssd(w),
            );
        }
        false
    }
}
