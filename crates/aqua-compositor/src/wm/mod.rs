//! Window management: focus, minimise, zoom, fullscreen, close, app activation,
//! and the compositor side of every shell action. Works for Wayland (xdg) and
//! X11 (XWayland) windows alike.
pub mod grabs;
pub mod mission;
pub mod outputs;
pub mod popups;
pub mod spaces;
pub mod stage;
pub mod tile;

use crate::input::focus::KeyboardFocusTarget;
use crate::state::{meta, title_of, Aqua};
use smithay::{
    desktop::Window,
    input::keyboard::{FilterResult, Keycode},
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel,
    utils::{Logical, Rectangle, SERIAL_COUNTER},
    wayland::seat::WaylandFocus,
};

/// Smithay rectangle → model rectangle.
/// Configure an X11 window from its *visible* rectangle (space coordinates). Toolkits that draw
/// client-side shadows (GTK: `_GTK_FRAME_EXTENTS`) have an X window larger than what is shown;
/// the X server must get the full frame or the window shrinks by its shadow on every configure.
pub(crate) fn x11_configure(x: &smithay::xwayland::X11Surface, r: Rectangle<i32, Logical>) {
    let _ = x.configure(r + x.frame_extents());
}

pub fn to_rect(r: Rectangle<i32, Logical>) -> aqua_wm::Rect {
    aqua_wm::Rect::new(r.loc.x, r.loc.y, r.size.w, r.size.h)
}

/// Model rectangle → Smithay rectangle.
pub fn from_rect(r: aqua_wm::Rect) -> Rectangle<i32, Logical> {
    Rectangle::new((r.x, r.y).into(), (r.w, r.h).into())
}

impl Aqua {
    pub fn focus_window(&mut self, w: &Window) {
        if meta(w).borrow().override_redirect {
            // X11 menus / tooltips never take the focus: clicking a Steam (CEF) or Electron
            // menu item must not deactivate the window that owns the menu, or the client
            // closes the menu before the click lands.
            return;
        }
        self.dismiss_menu_popups(None);
        if self.stage.staged.contains(w) {
            let app = title_of(w).0;
            self.stage_bring(&app);
        }
        let desk = meta(w).borrow().desk;
        let display = self.win_display(w);
        let cur = self.spaces.cur(&display);
        if self.minimized.contains(w) {
            meta(w).borrow_mut().desk = cur;
        } else if desk != cur {
            self.switch_space_on(&display, desk);
        }
        if let Some(pos) = self.minimized.iter().position(|m| m == w) {
            let target = self.dock_target(&self.minimized[pos].clone());
            let w = self.minimized.remove(pos);
            let loc = {
                let m = meta(&w).borrow();
                m.min_loc.or(m.saved.map(|r| r.loc)).unwrap_or((200, 120).into())
            };
            {
                let mut m = meta(&w).borrow_mut();
                m.minimizing = Some((std::time::Instant::now(), true));
                m.anim_target = target;
            }
            self.space.map_element(w.clone(), loc, true);
            if let Some(x) = w.x11_surface() {
                let _ = x.set_mapped(true);
            }
        } else if matches!(meta(w).borrow().minimizing, Some((_, false))) {
            let mut m = meta(w).borrow_mut();
            m.minimizing = Some((crate::state::reversed_anim_start(&m), true));
        }
        self.space.raise_element(w, true);
        if let (Some(x), Some(xwm)) = (w.x11_surface(), self.xwm.as_mut()) {
            let _ = xwm.raise_window(x);
        }
        if self.lock.is_locked() {
            return;
        }
        let serial = SERIAL_COUNTER.next_serial();
        if let Some(kb) = self.seat.get_keyboard() {
            kb.set_focus(self, Some(KeyboardFocusTarget::from(w.clone())), serial);
        }
        self.update_activation();
    }

    /// Mark exactly the keyboard-focused window as activated.
    pub fn update_activation(&mut self) {
        let focused = self.seat.get_keyboard().and_then(|k| k.current_focus());
        self.update_activation_for(focused);
    }

    pub fn update_activation_for(&mut self, focused: Option<KeyboardFocusTarget>) {
        let fw = match &focused {
            Some(KeyboardFocusTarget::Window(w)) => Some(w.clone()),
            Some(KeyboardFocusTarget::Popup(p)) => {
                smithay::desktop::find_popup_root_surface(p).ok().and_then(|s| self.window_for_surface(&s))
            }
            _ => None,
        };
        for w in self.space.elements() {
            let act = Some(w) == fw.as_ref();
            w.set_activated(act);
            if let Some(t) = w.toplevel() {
                if t.is_initial_configure_sent() {
                    t.send_pending_configure();
                }
            }
        }
        self.needs_redraw = true;
    }

    /// Move keyboard focus to the topmost remaining window on this Space.
    pub fn focus_next(&mut self, except: Option<&Window>) {
        let next = self
            .space
            .elements()
            .rev()
            .find(|w| {
                Some(*w) != except
                    && self.on_current_space(w)
                    && meta(w).borrow().minimizing.is_none()
                    && !meta(w).borrow().override_redirect
            })
            .cloned();
        let serial = SERIAL_COUNTER.next_serial();
        if let Some(kb) = self.seat.get_keyboard() {
            kb.set_focus(self, next.map(Into::into), serial);
        }
        self.update_activation();
    }

    pub fn minimize(&mut self, w: &Window) {
        if self.minimized.contains(w) || !self.space.elements().any(|e| e == w) {
            return;
        }
        let state = meta(w).borrow().minimizing;
        match state {
            Some((_, false)) => return,
            Some((_, true)) => {
                let mut m = meta(w).borrow_mut();
                m.minimizing = Some((crate::state::reversed_anim_start(&m), false));
                drop(m);
                self.needs_redraw = true;
                self.focus_next(Some(w));
                return;
            }
            None => {}
        }
        if let Some(loc) = self.space.element_location(w) {
            meta(w).borrow_mut().min_loc = Some(self.home_loc(w).unwrap_or(loc));
        }
        meta(w).borrow_mut().minimizing = Some((std::time::Instant::now(), false));
        let target = self.dock_target(w);
        meta(w).borrow_mut().anim_target = target;
        self.needs_redraw = true;
        self.focus_next(Some(w));
    }

    /// Running app ids, most recently focused first (⌘Tab order).
    pub fn app_mru(&self) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        for w in self.space.elements().rev().chain(self.minimized.iter()) {
            if meta(w).borrow().override_redirect {
                continue;
            }
            let (id, _) = crate::state::title_of(w);
            if !id.is_empty() && !out.contains(&id) {
                out.push(id);
            }
        }
        out
    }

    fn dock_target(&mut self, w: &Window) -> Option<(f32, f32)> {
        self.sync_shell_windows();
        let id = meta(w).borrow().id;
        if let Some(c) = aqua_shell::dock::minimized_center(&self.shell, id) {
            return Some(c);
        }
        let (app, _) = crate::state::title_of(w);
        aqua_shell::dock::icon_center_for(&self.shell, &app)
    }

    /// Ask a window to take a new size (xdg configure / X11 ConfigureWindow).
    pub fn configure_size(
        &self,
        w: &Window,
        loc: smithay::utils::Point<i32, Logical>,
        size: smithay::utils::Size<i32, Logical>,
    ) {
        if let Some(t) = w.toplevel() {
            t.with_pending_state(|s| s.size = Some(size));
            t.send_pending_configure();
        } else if let Some(x) = w.x11_surface() {
            x11_configure(x, Rectangle::new(loc, size));
        }
    }

    fn is_maximized(w: &Window) -> bool {
        if let Some(t) = w.toplevel() {
            t.with_pending_state(|s| s.states.contains(xdg_toplevel::State::Maximized))
        } else if let Some(x) = w.x11_surface() {
            x.is_maximized()
        } else {
            false
        }
    }

    /// Zoom (maximise) or un-zoom on the client's request; answers with a configure
    /// even when nothing changes so the client's state never drifts from ours.
    pub fn zoom_request(&mut self, w: &Window, on: bool) {
        if Self::is_maximized(w) != on {
            self.toggle_zoom(w);
        } else if let Some(t) = w.toplevel() {
            if t.is_initial_configure_sent() {
                t.send_configure();
            }
        }
    }

    /// Output a window is on (by its frame), falling back to the primary one.
    fn output_of(&self, w: &Window) -> Option<smithay::output::Output> {
        let placed = meta(w).borrow().placed;
        let g = if placed { self.space.element_geometry(w) } else { None };
        g.and_then(|g| {
            self.output_at(g.loc.to_f64() + g.size.to_f64().downscale(2.0).to_point())
                .or_else(|| self.output_at(g.loc.to_f64()))
        })
        .or_else(|| self.output.clone())
    }

    /// Frame of a zoomed window: the usable area minus a small margin (client rect,
    /// i.e. below the title bar).
    fn zoom_rect(&self, w: &Window, out: &smithay::output::Output) -> Rectangle<i32, Logical> {
        from_rect(aqua_wm::place::zoom_rect(to_rect(self.usable_area(out)), Aqua::titlebar_h(w)))
    }

    /// Is a remembered (client) rect usable for restoring: sane size and still on a display?
    fn restorable(&self, r: Rectangle<i32, Logical>) -> bool {
        let displays: Vec<aqua_wm::Rect> =
            self.space.outputs().filter_map(|o| self.space.output_geometry(o)).map(to_rect).collect();
        aqua_wm::place::restorable(to_rect(r), &displays)
    }

    /// Where an un-zoomed window goes when there is no usable remembered geometry
    /// (zoomed before it was first shown, display unplugged, …): two thirds of the
    /// desktop, centred.
    fn fallback_rect(&self, w: &Window, out: &smithay::output::Output) -> Rectangle<i32, Logical> {
        let (mut mw, mut mh) = (0, 0);
        if let Some(t) = w.toplevel() {
            let mn = smithay::wayland::compositor::with_states(t.wl_surface(), |st| {
                st.cached_state.get::<smithay::wayland::shell::xdg::SurfaceCachedState>().current().min_size
            });
            mw = mn.w;
            mh = mn.h;
        }
        from_rect(aqua_wm::place::fallback_rect(to_rect(self.usable_area(out)), Aqua::titlebar_h(w), (mw, mh)))
    }

    pub(crate) fn fallback_rect_pub(&self, w: &Window, out: &smithay::output::Output) -> Rectangle<i32, Logical> {
        self.fallback_rect(w, out)
    }

    /// Zoom: fill the usable desktop area, toggling back to the old size.
    pub fn toggle_zoom(&mut self, w: &Window) {
        if self.minimized.contains(w) {
            return;
        }
        if meta(w).borrow().fullscreen.is_some() {
            self.set_fullscreen(w, false);
            return;
        }
        let Some(out) = self.output_of(w) else { return };
        if !meta(w).borrow().placed {
            let on = !Self::is_maximized(w);
            let z = self.zoom_rect(w, &out);
            if let Some(t) = w.toplevel() {
                t.with_pending_state(|s| {
                    if on {
                        s.states.set(xdg_toplevel::State::Maximized);
                        s.size = Some(z.size);
                    } else {
                        s.states.unset(xdg_toplevel::State::Maximized);
                        s.size = None;
                    }
                });
                if t.is_initial_configure_sent() {
                    t.send_pending_configure();
                }
            } else if let Some(x) = w.x11_surface() {
                let _ = x.set_maximized(on);
            }
            return;
        }
        if Self::is_maximized(w) {
            self.zoom_out(w, &out);
        } else {
            if let Some(loc) = self.space.element_location(w) {
                meta(w).borrow_mut().saved = Some(Rectangle::new(loc, w.geometry().size));
            }
            self.zoom_in(w, &out);
        }
        self.needs_redraw = true;
    }

    /// Put a (placed) window into the zoomed state and geometry.
    pub(crate) fn zoom_in(&mut self, w: &Window, out: &smithay::output::Output) {
        let z = self.zoom_rect(w, out);
        let tb = Aqua::titlebar_h(w);
        self.start_geo_anim(w, Rectangle::new((z.loc.x, z.loc.y - tb).into(), (z.size.w, z.size.h + tb).into()));
        if let Some(t) = w.toplevel() {
            t.with_pending_state(|s| {
                s.states.set(xdg_toplevel::State::Maximized);
                s.size = Some(z.size);
            });
            t.send_pending_configure();
        } else if let Some(x) = w.x11_surface() {
            let _ = x.set_maximized(true);
            x11_configure(x, z);
        }
        self.space.map_element(w.clone(), z.loc, true);
        self.needs_redraw = true;
    }

    fn zoom_out(&mut self, w: &Window, out: &smithay::output::Output) {
        let saved = meta(w).borrow_mut().saved.take();
        let r = match saved {
            Some(r) if self.restorable(r) => r,
            _ => self.fallback_rect(w, out),
        };
        let tb = Aqua::titlebar_h(w);
        if let Some(t) = w.toplevel() {
            t.with_pending_state(|s| {
                s.states.unset(xdg_toplevel::State::Maximized);
                s.size = Some(r.size);
            });
            t.send_pending_configure();
        } else if let Some(x) = w.x11_surface() {
            let _ = x.set_maximized(false);
            x11_configure(x, r);
        }
        self.start_geo_anim(w, Rectangle::new((r.loc.x, r.loc.y - tb).into(), (r.size.w, r.size.h + tb).into()));
        self.space.map_element(w.clone(), r.loc, true);
        self.needs_redraw = true;
    }

    /// Enter/leave fullscreen on the output the window is on.
    pub fn set_fullscreen(&mut self, w: &Window, on: bool) {
        if !meta(w).borrow().placed {
            if let Some(t) = w.toplevel() {
                t.with_pending_state(|s| {
                    if on {
                        s.states.set(xdg_toplevel::State::Fullscreen);
                    } else {
                        s.states.unset(xdg_toplevel::State::Fullscreen);
                    }
                });
                if t.is_initial_configure_sent() {
                    t.send_pending_configure();
                }
            } else if let Some(x) = w.x11_surface() {
                let _ = x.set_fullscreen(on);
            }
            return;
        }
        let cur = meta(w).borrow().fullscreen;
        if on == cur.is_some() {
            if let Some(t) = w.toplevel() {
                if t.is_initial_configure_sent() {
                    t.send_configure();
                }
            }
            return;
        }
        let out = self.output_of(w);
        if on {
            let Some(geo) = out.as_ref().and_then(|o| self.space.output_geometry(o)) else { return };
            let loc = self.space.element_location(w).unwrap_or_default();
            self.start_geo_anim(w, geo);
            meta(w).borrow_mut().fullscreen = Some(Rectangle::new(loc, w.geometry().size));
            if let Some(t) = w.toplevel() {
                t.with_pending_state(|s| {
                    s.states.set(xdg_toplevel::State::Fullscreen);
                    s.size = Some(geo.size);
                    s.fullscreen_output = out.as_ref().and_then(|o| {
                        let client = smithay::reexports::wayland_server::Resource::client(t.wl_surface())?;
                        o.client_outputs(&client).next()
                    });
                });
                t.send_pending_configure();
            } else if let Some(x) = w.x11_surface() {
                let _ = x.set_fullscreen(true);
                x11_configure(x, geo);
            }
            self.space.map_element(w.clone(), geo.loc, true);
            self.focus_window(w);
        } else {
            let saved = meta(w).borrow_mut().fullscreen.take();
            let zoomed = Self::is_maximized(w);
            let r = match (saved, out.as_ref()) {
                (_, Some(o)) if zoomed => self.zoom_rect(w, o),
                (Some(r), _) if self.restorable(r) => r,
                (_, Some(o)) => self.fallback_rect(w, o),
                (Some(r), None) => r,
                (None, None) => return,
            };
            if let Some(t) = w.toplevel() {
                t.with_pending_state(|s| {
                    s.states.unset(xdg_toplevel::State::Fullscreen);
                    s.size = Some(r.size);
                    s.fullscreen_output = None;
                });
                t.send_pending_configure();
            } else if let Some(x) = w.x11_surface() {
                let _ = x.set_fullscreen(false);
                x11_configure(x, r);
            }
            let tb = Aqua::titlebar_h(w);
            self.start_geo_anim(w, Rectangle::new((r.loc.x, r.loc.y - tb).into(), (r.size.w, r.size.h + tb).into()));
            self.space.map_element(w.clone(), r.loc, true);
        }
        self.needs_redraw = true;
    }

    /// Animate the window frame from where it is now to `to` (logical frame incl. title bar).
    pub fn start_geo_anim(&mut self, w: &Window, to: Rectangle<i32, Logical>) {
        if !self.cfg.animate_windows || crate::state::reduce_motion() {
            return;
        }
        let Some(from) = self.frame_rect(w) else { return };
        if from.size.w <= 0 || from.size.h <= 0 {
            return;
        }
        meta(w).borrow_mut().geo_anim = Some((std::time::Instant::now(), from.to_f64(), to.to_f64()));
        self.needs_redraw = true;
    }

    pub fn close(&mut self, w: &Window) {
        if let Some(t) = w.toplevel() {
            t.send_close();
        } else if let Some(x) = w.x11_surface() {
            let _ = x.close();
        }
    }

    /// Kill the client process (Force Quit).
    pub fn force_quit(&mut self, w: &Window) {
        let mut pid = None;
        if let Some(x) = w.x11_surface() {
            pid = x.pid().map(|p| p as i32);
        } else if let Some(s) = w.wl_surface() {
            if let Some(client) = smithay::reexports::wayland_server::Resource::client(&*s) {
                pid = client.get_credentials(&self.display_handle).ok().map(|c| c.pid);
                if pid.map(|p| p == std::process::id() as i32).unwrap_or(false) {
                    pid = None;
                }
                if pid.is_none() {
                    self.display_handle.backend_handle().kill_client(
                        client.id(),
                        smithay::reexports::wayland_server::backend::DisconnectReason::ConnectionClosed,
                    );
                }
            }
        }
        if let Some(p) = pid.filter(|p| *p > 1) {
            tracing::info!("force quitting pid {p}");
            unsafe {
                libc::kill(p, libc::SIGKILL);
            }
        }
    }

    /// Any managed window (mapped or minimised) by its Aqua id.
    pub fn window_by_id(&self, id: u64) -> Option<Window> {
        self.space
            .elements()
            .chain(self.minimized.iter())
            .chain(self.stage.staged.iter())
            .find(|w| meta(w).borrow().id == id)
            .cloned()
    }

    pub fn windows_of_app(&self, app_id: &str) -> Vec<Window> {
        let want = app_id.to_lowercase();
        self.space
            .elements()
            .chain(self.minimized.iter())
            .chain(self.stage.staged.iter())
            .filter(|w| !meta(w).borrow().override_redirect)
            .filter(|w| {
                let (id, _) = title_of(w);
                let id = id.to_lowercase();
                id == want
                    || id.ends_with(&format!(".{want}"))
                    || want.ends_with(&format!(".{id}"))
                    || (!id.is_empty() && want.contains(&id))
            })
            .cloned()
            .collect()
    }

    pub fn activate_app(&mut self, app_id: &str) {
        let wins = self.windows_of_app(app_id);
        for w in &wins {
            if self.minimized.contains(w) {
                self.focus_window(w);
            } else {
                self.space.raise_element(w, false);
            }
        }
        if let Some(last) = wins.last() {
            self.focus_window(last);
        }
    }

    /// Click on the Dock icon of a running app, per Desktop & Dock settings.
    pub fn dock_click(&mut self, app_id: &str, exec: &str) {
        let wins = self.windows_of_app(app_id);
        let focused = self.focused_window();
        let frontmost = focused.as_ref().map(|f| wins.contains(f)).unwrap_or(false);
        let visible: Vec<Window> = wins.iter().filter(|w| !self.minimized.contains(w)).cloned().collect();
        match self.cfg.dock_click.as_str() {
            "minimize" if frontmost && !visible.is_empty() => {
                let here: Vec<Window> = visible.iter().filter(|w| self.on_current_space(w)).cloned().collect();
                for w in if here.is_empty() { visible } else { here } {
                    self.minimize(&w);
                }
            }
            "cycle" if frontmost && wins.len() > 1 => {
                let cur = focused.as_ref().and_then(|f| wins.iter().position(|w| w == f)).unwrap_or(0);
                let next = wins[(cur + 1) % wins.len()].clone();
                if self.minimized.contains(&next) {
                    self.focus_window(&next);
                } else {
                    self.space.raise_element(&next, true);
                    self.focus_window(&next);
                }
            }
            "expose" if wins.len() > 1 => self.app_expose(app_id),
            "new" if !exec.is_empty() => {
                let _ = aqua_apps::launch(exec);
            }
            _ => self.activate_app(app_id),
        }
        self.needs_redraw = true;
    }

    pub fn quit_app(&mut self, app_id: &str) {
        for w in self.windows_of_app(app_id) {
            self.close(&w);
        }
    }

    pub fn hide_others(&mut self) {
        let f = self.focused_window();
        let others: Vec<_> = self
            .space
            .elements()
            .filter(|w| Some(*w) != f.as_ref() && !meta(w).borrow().override_redirect && self.on_current_space(w))
            .cloned()
            .collect();
        for w in others {
            self.minimize(&w);
        }
        if let Some(f) = f {
            self.focus_window(&f);
        }
    }

    pub fn bring_all_to_front(&mut self) {
        let f = self.focused_window();
        for w in self.minimized.clone() {
            self.focus_window(&w);
        }
        let wins: Vec<_> = self
            .space
            .elements()
            .filter(|w| self.on_current_space(w) && !meta(w).borrow().override_redirect)
            .cloned()
            .collect();
        for w in wins {
            self.space.raise_element(&w, false);
        }
        if let Some(f) = f {
            self.focus_window(&f);
        }
    }

    pub(crate) fn keycode_for(&mut self, name: &str) -> Option<u32> {
        if let Some(n) = name.strip_prefix('#') {
            return n.parse().ok();
        }
        let named = match name {
            "comma" => Some(51),
            "equal" | "plus" => Some(13),
            "minus" => Some(12),
            "0" => Some(11),
            "space" => Some(57),
            "enter" | "return" => Some(28),
            "tab" => Some(15),
            "esc" | "escape" => Some(1),
            "insert" => Some(110),
            "backspace" => Some(14),
            _ => None,
        };
        if named.is_some() {
            return named;
        }
        let kb = self.seat.get_keyboard()?;
        let want = name.chars().next()?.to_ascii_lowercase();
        kb.with_xkb_state(self, |ctx| {
            let xkb = ctx.xkb().lock().unwrap();
            for code in 2u32..=58 {
                for layout in xkb.layouts() {
                    let syms = xkb.raw_syms_for_key_in_layout(Keycode::new(code + 8), layout);
                    if syms.first().and_then(|s| s.key_char()) == Some(want) {
                        return Some(code);
                    }
                }
            }
            None
        })
    }

    fn inject_key(&mut self, code: u32, pressed: bool) {
        let Some(kb) = self.seat.get_keyboard() else { return };
        let serial = SERIAL_COUNTER.next_serial();
        let st = if pressed {
            smithay::backend::input::KeyState::Pressed
        } else {
            smithay::backend::input::KeyState::Released
        };
        kb.input::<(), _>(
            self,
            Keycode::new(code + 8),
            st,
            serial,
            smithay::backend::input::InputTime::now(),
            |_, _, _| FilterResult::Forward,
        );
    }

    /// Like `send_keys`, but through Aqua's own shortcut handling (automation/tests).
    pub fn press_chord(&mut self, chord: &str) {
        let mut codes = vec![];
        for part in chord.split('+') {
            let c = match part {
                "ctrl" => Some(29),
                "shift" => Some(42),
                "alt" => Some(56),
                "super" | "cmd" => Some(125),
                "volup" => Some(115),
                "voldown" => Some(114),
                "mute" => Some(113),
                "brightup" => Some(225),
                "brightdown" => Some(224),
                "left" => Some(105),
                "right" => Some(106),
                "up" => Some(103),
                "down" => Some(108),
                "f3" => Some(61),
                "f4" => Some(62),
                k => self.keycode_for(k),
            };
            let Some(c) = c else {
                tracing::warn!("press_chord: unknown key {part:?}");
                return;
            };
            codes.push(c);
        }
        let t = smithay::backend::input::InputTime::now();
        for &c in &codes {
            self.on_key(Keycode::new(c + 8), smithay::backend::input::KeyState::Pressed, t);
        }
        for &c in codes.iter().rev() {
            self.on_key(Keycode::new(c + 8), smithay::backend::input::KeyState::Released, t);
        }
    }

    /// "ctrl+shift+z" → press modifiers, key, release in reverse.
    pub fn send_keys(&mut self, chord: &str) {
        let mut codes = vec![];
        for part in chord.split('+') {
            let c = match part {
                "ctrl" => Some(29),
                "shift" => Some(42),
                "alt" => Some(56),
                "super" => Some(125),
                k => self.keycode_for(k),
            };
            match c {
                Some(c) => codes.push(c),
                None => {
                    tracing::warn!("send_keys: unknown key {part:?}");
                    return;
                }
            }
        }
        for &c in &codes {
            self.inject_key(c, true);
        }
        for &c in codes.iter().rev() {
            self.inject_key(c, false);
        }
    }

    /// Type text into the focused client: text-input-v3 when the client uses it,
    /// otherwise through the clipboard (+ paste chord).
    pub fn type_text(&mut self, text: &str) {
        use smithay::wayland::text_input::TextInputSeat;
        let mut done = false;
        let ti = self.seat.text_input().clone();
        ti.with_active_text_input(|t, _| {
            t.commit_string(Some(text.to_string()));
            done = true;
        });
        if done {
            ti.done(false);
            return;
        }
        self.clipboard_put_text(text);
        let term = self.focused_window().map(|w| {
            let (id, _) = title_of(&w);
            let id = id.to_lowercase();
            ["term", "xterm", "kitty", "alacritty", "foot", "konsole", "wezterm", "ptyxis"]
                .iter()
                .any(|t| id.contains(t))
        });
        self.send_keys(if term == Some(true) { "ctrl+shift+v" } else { "ctrl+v" });
    }

    /// Run actions that may be shell-internal (Emoji & Symbols, Keyboard Viewer,
    /// clipboard history, confirmation alerts…) — used by shortcuts and `aqua msg`.
    pub fn shell_actions(&mut self, acts: Vec<aqua_shell::Action>) {
        let acts = self.shell.intercept(acts);
        self.needs_redraw = true;
        self.handle_actions(acts);
    }

    pub fn handle_actions(&mut self, acts: Vec<aqua_shell::Action>) {
        use aqua_shell::Action::*;
        for a in acts {
            match a {
                Launch(cmd) => {
                    self.pending_launch = self.shell.launch_origin.take().map(|r| (std::time::Instant::now(), r));
                    aqua_apps::launch(&cmd);
                }
                Activate(id) => self.activate_app(&id),
                DockClick(id, exec) => self.dock_click(&id, &exec),
                Restore(id) => {
                    if let Some(w) = self.minimized.iter().find(|w| meta(w).borrow().id == id).cloned() {
                        self.focus_window(&w);
                    }
                }
                CloseFocused => {
                    if let Some(w) = self.focused_window() {
                        self.close(&w)
                    }
                }
                MinimizeFocused => {
                    if let Some(w) = self.focused_window() {
                        self.minimize(&w)
                    }
                }
                ZoomFocused => {
                    if let Some(w) = self.focused_window() {
                        self.toggle_zoom(&w)
                    }
                }
                FullscreenFocused => {
                    if let Some(w) = self.focused_window() {
                        let on = meta(&w).borrow().fullscreen.is_none();
                        self.set_fullscreen(&w, on)
                    }
                }
                TileWindow(id, what) => {
                    if let Some(w) = self.window_by_id(id) {
                        self.focus_window(&w);
                        if what == "fullscreen" {
                            self.set_fullscreen(&w, true);
                        } else {
                            self.run_tile_action(&what);
                        }
                    }
                }
                QuitApp(id) => self.quit_app(&id),
                HideOthers => self.hide_others(),
                BringAllToFront => self.bring_all_to_front(),
                LogOut | LogOutNow => self.end_session(EndKind::LogOut),
                Restart | RestartNow => self.end_session(EndKind::Restart),
                ShutDown | ShutDownNow => self.end_session(EndKind::ShutDown),
                Sleep => {
                    if self.cfg.idle.lock_on_sleep {
                        self.lock_session();
                    }
                    std::thread::spawn(|| {
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        if let Err(e) = aqua_sys::session::suspend() {
                            tracing::warn!("suspend failed: {e}");
                        }
                    });
                }
                Lock => self.lock_session(),
                Unlock(pw) => self.lock_try_password(pw),
                SetDark(d) => {
                    crate::system::theming::set_dark(d);
                    self.appearance.set_dark(d);
                    self.dark_anim = Some((std::time::Instant::now(), d));
                    if self.cfg.dark != d {
                        self.cfg.dark = d;
                        self.cfg.appearance = if d { "dark".into() } else { "light".into() };
                        let _ = self.cfg.save();
                        self.cfg_watch = aqua_config::Config::mtime();
                    }
                }
                Screenshot => self.start_screenshot("full"),
                ScreenshotTake(t) => {
                    let timer = self.shell.shot.from_toolbar;
                    self.queue_screenshot(t, timer);
                }
                ScreenshotOptions => self.save_screenshot_options(),
                ScreenshotUi(how) => self.start_screenshot(&how),
                RecordStart(t) => {
                    let timer = self.shell.shot.from_toolbar;
                    self.queue_recording(t, timer);
                }
                RecordStop => self.stop_recording(),
                CopyText(t) => self.clipboard_put_text(&t),
                ClipboardUse(id, paste) => {
                    self.clipboard_activate(id);
                    if paste {
                        self.send_keys("ctrl+v");
                    }
                }
                ClipboardClear => self.clipboard_clear(),
                OpenSettings(pane) => {
                    let bin = crate::system::env::own_bin("aqua-settings");
                    aqua_apps::launch(&format!("{bin} --pane '{}' || gnome-control-center", pane.replace('\'', "")));
                }
                SendKeys(chord) => self.send_keys(&chord),
                TypeText(t) => self.type_text(&t),
                TypeKey(code) => {
                    self.inject_key(code, true);
                    self.inject_key(code, false);
                }
                SwitchLayout(i) => self.set_layout(i),
                NewWindow(app_id) => {
                    let cmd = aqua_apps::match_app_id(&self.shell.apps, &app_id)
                        .map(|a| a.launch_command())
                        .unwrap_or_else(|| app_id.rsplit('.').next().unwrap_or(&app_id).to_lowercase());
                    let cmd = if cmd.contains("chrom") || cmd.contains("firefox") {
                        format!("{cmd} --new-window")
                    } else {
                        cmd
                    };
                    aqua_apps::launch(&cmd);
                }
                AppSettings => self.send_keys("ctrl+comma"),
                Beep => crate::system::sound::play_alert(),
                SetStageManager(on) => {
                    self.set_stage_manager(on);
                    let mut c = aqua_config::Config::load();
                    c.stage_manager = on;
                    if c.save().is_ok() {
                        self.cfg.stage_manager = on;
                    }
                }
                SetFocusMode(on) => {
                    self.shell.notes.dnd = on;
                    let mut c = aqua_config::Config::load();
                    c.do_not_disturb = on;
                    if c.save().is_ok() {
                        self.cfg.do_not_disturb = on;
                    }
                }
                ForceQuitConfirmed(id) => {
                    for w in self.windows_of_app(&id) {
                        self.force_quit(&w);
                    }
                }
                Help(id, name) => {
                    let base = id.rsplit('.').next().unwrap_or(&id).to_lowercase();
                    aqua_apps::launch(&format!(
                        "yelp help:{base} || xdg-open 'https://duckduckgo.com/?q={}+help'",
                        name.replace(['\'', ' '], "+")
                    ));
                }
                FocusWindow(id) => {
                    if let Some(w) = self.window_by_id(id) {
                        self.focus_window(&w);
                    }
                }
                CloseWindow(id) => {
                    if let Some(w) = self.window_by_id(id) {
                        self.close(&w);
                    }
                }
                HideApp(app) => {
                    for w in self.windows_of_app(&app) {
                        if !self.minimized.contains(&w) {
                            self.minimize(&w);
                        }
                    }
                }
                OpenTerminal => {
                    let t = aqua_shell::dock::resolve_exec(&self.cfg.terminal);
                    aqua_apps::launch(&t);
                }
                MissionControl => self.toggle_mission(),
                Redraw => {}
                _ => {}
            }
        }
        self.needs_redraw = true;
    }

    /// Log out / restart / shut down: ask every app to quit, wait briefly, then act.
    pub fn end_session(&mut self, kind: EndKind) {
        tracing::info!("ending session: {kind:?}");
        let wins: Vec<_> = self.space.elements().chain(self.minimized.iter()).cloned().collect();
        for w in &wins {
            self.close(w);
        }
        let _ = std::fs::remove_file(crate::system::lock::marker_path());
        let delay = if wins.is_empty() { 50 } else { 1200 };
        use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
        let _ = self.loop_handle.insert_source(
            Timer::from_duration(std::time::Duration::from_millis(delay)),
            move |_, _, st| {
                match kind {
                    EndKind::LogOut => {}
                    EndKind::Restart => {
                        if let Err(e) = aqua_sys::session::reboot() {
                            tracing::error!("reboot failed: {e}");
                        }
                    }
                    EndKind::ShutDown => {
                        if let Err(e) = aqua_sys::session::power_off() {
                            tracing::error!("power off failed: {e}");
                        }
                    }
                }
                st.loop_signal.stop();
                TimeoutAction::Drop
            },
        );
    }
}

#[derive(Debug, Clone, Copy)]
pub enum EndKind {
    LogOut,
    Restart,
    ShutDown,
}
