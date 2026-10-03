//! XWayland: X11 apps (Steam, Discord, older Electron, JetBrains IDEs, Wine …).
//!
//! The X server is spawned lazily-ready at startup; `DISPLAY` is exported to every
//! app launched by Aqua. X11 toplevels become regular Aqua windows (title bar
//! unless the app draws its own), override-redirect windows (menus, tooltips) are
//! placed where the client asks. Clipboard and primary selection are bridged both
//! ways, drag-and-drop goes through the shared DnD focus targets.
use crate::input::focus::KeyboardFocusTarget;
use crate::state::{meta, Aqua};
use smithay::{
    desktop::Window,
    input::pointer::Focus,
    utils::{Logical, Rectangle, SERIAL_COUNTER},
    wayland::selection::{
        data_device::{
            clear_data_device_selection, current_data_device_selection_userdata, request_data_device_client_selection,
            set_data_device_selection,
        },
        primary_selection::{
            clear_primary_selection, current_primary_selection_userdata, request_primary_client_selection,
            set_primary_selection,
        },
        SelectionTarget,
    },
    xwayland::{
        xwm::{Reorder, ResizeEdge as X11ResizeEdge, WmWindowType, XwmId},
        X11Surface, X11Wm, XWayland, XWaylandEvent, XwmHandler,
    },
};
use std::os::unix::io::OwnedFd;

/// Menus, tooltips and other X11 windows that position themselves: override-redirect
/// windows and managed windows typed as menus/tooltips (Steam, Electron, Java). They get no
/// title bar, keep the position the client asks for and never take keyboard focus.
pub fn is_popup(x: &X11Surface) -> bool {
    x.is_override_redirect() || x.window_type().is_some_and(popup_type)
}

fn popup_type(t: WmWindowType) -> bool {
    use WmWindowType::*;
    matches!(t, DropdownMenu | PopupMenu | Menu | Tooltip | Combo | Dnd | Notification)
}

/// Windows that must never receive pointer events: tooltips, drag icons and notification
/// bubbles. Otherwise a tooltip that pops up under the cursor swallows scrolling and clicks
/// until it disappears.
pub fn input_transparent(x: &X11Surface) -> bool {
    use WmWindowType::*;
    matches!(x.window_type(), Some(Tooltip | Dnd | Notification))
}

const XWAYLAND_READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

impl Aqua {
    pub fn start_xwayland(&mut self) {
        if !self.cfg.xwayland || std::env::var_os("AQUA_NO_XWAYLAND").is_some() {
            return;
        }
        if !aqua_sys::have("Xwayland") {
            tracing::warn!("Xwayland is not installed: X11 apps cannot start");
            self.shell.notify(aqua_shell::notifications::Note {
                app_id: "aqua-settings".into(),
                app_name: "Aqua".into(),
                summary: "X11 apps need XWayland".into(),
                body: "Steam, Wine and some older apps need the X server. Install the \"xwayland\" package (xorg-xwayland on Arch).".into(),
                timeout: 10.0,
                ..Default::default()
            });
            return;
        }
        if self.xwayland_failures == 0 {
            aqua_apps::hold_launches();
            let _ = self.loop_handle.insert_source(
                smithay::reexports::calloop::timer::Timer::from_duration(XWAYLAND_READY_TIMEOUT),
                |_, _, st| {
                    if st.xdisplay.is_none() {
                        tracing::warn!(
                            "XWayland is not ready after {}s; releasing held launches",
                            XWAYLAND_READY_TIMEOUT.as_secs()
                        );
                    }
                    aqua_apps::release_launches();
                    smithay::reexports::calloop::timer::TimeoutAction::Drop
                },
            );
        }
        let spawned = XWayland::spawn(
            &self.display_handle,
            None,
            std::iter::empty::<(String, String)>(),
            std::iter::empty::<String>(),
            true,
            std::process::Stdio::null(),
            std::process::Stdio::null(),
            |_| (),
        );
        let (xwayland, client) = match spawned {
            Ok(x) => x,
            Err(e) => {
                tracing::warn!("XWayland unavailable: {e}");
                aqua_apps::release_launches();
                return;
            }
        };
        let dh = self.display_handle.clone();
        let res = self.loop_handle.insert_source(xwayland, move |event, _, st| match event {
            XWaylandEvent::Ready { x11_socket, display_number } => {
                let xs = if st.scale.fract() == 0.0 { st.scale } else { 1.0 };
                use smithay::wayland::compositor::CompositorHandler;
                st.client_compositor_state(&client).set_client_scale(xs);
                match X11Wm::start_wm(st.loop_handle.clone(), &dh, x11_socket, client.clone()) {
                    Ok(mut wm) => {
                        let cur = crate::render::arrow_cursor(1.0);
                        let _ = wm.set_cursor(
                            cur.data(),
                            (cur.width() as u16, cur.height() as u16).into(),
                            (4u16, 4u16).into(),
                        );
                        st.xwm = Some(wm);
                        st.xdisplay = Some(display_number);
                        unsafe { std::env::set_var("DISPLAY", format!(":{display_number}")) };
                        crate::system::env::export_env("DISPLAY", &format!(":{display_number}"));
                        tracing::info!("XWayland ready on DISPLAY=:{display_number}");
                        aqua_tray::xembed::start(display_number);
                        st.xwayland_failures = 0;
                    }
                    Err(e) => tracing::error!("failed to attach the X11 window manager: {e}"),
                }
                aqua_apps::release_launches();
            }
            XWaylandEvent::Error => {
                st.xwayland_failures += 1;
                tracing::warn!("XWayland crashed on startup (attempt {})", st.xwayland_failures);
                if st.xwayland_failures < 3 {
                    st.loop_handle.insert_idle(|st| st.start_xwayland());
                } else {
                    aqua_apps::release_launches();
                }
            }
        });
        if let Err(e) = res {
            tracing::error!("failed to insert XWayland source: {e}");
        }
    }

    /// Map a managed X11 window that places itself (typed as a menu/tooltip).
    fn x11_place_popup(&mut self, window: &X11Surface) {
        let w = Window::new_x11_window(window.clone());
        let (display, desk) = self.placement_desk();
        {
            let mut m = meta(&w).borrow_mut();
            m.id = self.next_window_id;
            m.placed = true;
            m.override_redirect = true;
            m.desk = desk;
            m.display = display;
        }
        self.next_window_id += 1;
        let geo = window.geometry();
        let _ = window.configure(geo);
        self.space.map_element(w, geo.loc, true);
        self.repick_pointer = true;
        self.needs_redraw = true;
    }

    /// Tell X11 clients where their windows are. Menus and tooltips are positioned by the
    /// client from its own window position, so a window moved (dragged, zoomed, placed on
    /// another output) without a configure gets its dropdowns at the old place — e.g. Steam
    /// menus opening in the top-left corner.
    pub fn sync_x11_positions(&mut self) {
        if self.spaces.animating() {
            return;
        }
        for w in self.space.elements() {
            let Some(x) = w.x11_surface() else { continue };
            {
                let m = meta(w).borrow();
                if m.override_redirect || m.minimizing.is_some() || m.geo_anim.is_some() {
                    continue;
                }
            }
            let Some(home) = self.home_loc(w) else { continue };
            let g = x.geometry();
            if g.loc != home {
                let _ = x.configure(Rectangle::new(home, g.size));
            }
        }
    }

    /// Re-evaluate what is under the pointer without it moving (a window under the cursor
    /// appeared or went away), so scrolling keeps going to the right surface.
    pub fn refresh_pointer_focus(&mut self) {
        let Some(pointer) = self.seat.get_pointer() else { return };
        if pointer.is_grabbed() || self.pointer_in_shell || self.mission.open || self.lock.is_locked() {
            return;
        }
        let pos = pointer.current_location();
        let under = self.surface_under(pos);
        let time = smithay::backend::input::InputTime::now();
        pointer.motion(
            self,
            under,
            &smithay::input::pointer::MotionEvent { location: pos, serial: SERIAL_COUNTER.next_serial(), time },
        );
        pointer.frame(self);
    }

    fn x11_place(&mut self, window: &X11Surface) {
        if is_popup(window) {
            return self.x11_place_popup(window);
        }
        let w = Window::new_x11_window(window.clone());
        let (display, desk) = self.placement_desk();
        let n = self.windows_on_desk(&display, desk, None);
        {
            let mut m = meta(&w).borrow_mut();
            m.id = self.next_window_id;
            m.desk = desk;
            m.display = display;
        }
        self.next_window_id += 1;
        let geo = window.geometry();
        let area = self.placement_output_rect();
        let (ow, oh) = (area.size.w, area.size.h);
        let mb = self.cfg.menubar_height as i32;
        let tb = if crate::state::is_ssd(&w) { aqua_config::metrics::TITLEBAR_HEIGHT as i32 } else { 0 };
        let chrome = self.chrome(tb);
        let size =
            (geo.size.w.clamp(1, (ow - 40).max(1)), geo.size.h.clamp(1, (oh - mb - tb - chrome.dock).max(1))).into();
        let centered = aqua_wm::place::cascade(crate::wm::to_rect(area), (geo.size.w, geo.size.h), n, chrome);
        let wants_pos = window.size_hints().map(|h| h.position.is_some()).unwrap_or(false)
            && area.to_f64().contains(geo.loc.to_f64())
            && geo.loc.y > area.loc.y + mb;
        let loc = if wants_pos { (geo.loc.x, geo.loc.y.max(area.loc.y + mb + tb)) } else { centered };
        let rect = Rectangle::<i32, Logical>::new(loc.into(), size);
        let _ = window.configure(rect);
        let from = self.take_launch_origin(&w);
        {
            let mut m = meta(&w).borrow_mut();
            m.placed = true;
            m.mapped_at = Some(std::time::Instant::now());
            m.launch_from = from;
        }
        self.space.map_element(w.clone(), rect.loc, true);
        self.repick_pointer = true;
        self.focus_window(&w);
        if window.is_fullscreen() || (geo.size.w >= ow && geo.size.h >= oh) {
            self.set_fullscreen(&w, true);
        } else if window.is_maximized() {
            if let Some(out) = self.output_at(rect.loc.to_f64()).or_else(|| self.output.clone()) {
                let normal = if geo.size.w > ow * 9 / 10 || geo.size.h > (oh - mb - tb - chrome.dock) * 9 / 10 {
                    self.fallback_rect_pub(&w, &out)
                } else {
                    rect
                };
                meta(&w).borrow_mut().saved = Some(normal);
                self.zoom_in(&w, &out);
                meta(&w).borrow_mut().geo_anim = None;
            }
        }
        self.needs_redraw = true;
    }
}

impl XwmHandler for Aqua {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm.as_mut().expect("xwm event without wm")
    }
    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}
    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Err(e) = window.set_mapped(true) {
            tracing::warn!("x11 map failed: {e}");
            return;
        }
        if self.window_for_x11(&window).is_none() {
            self.x11_place(&window);
        }
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        if window.class() == aqua_tray::xembed::CONTAINER_CLASS {
            // Off-screen holder of a bridged XEmbed tray icon: never shown.
            return;
        }
        let loc = window.geometry().loc;
        let w = Window::new_x11_window(window);
        let (display, desk) = self.placement_desk();
        {
            let mut m = meta(&w).borrow_mut();
            m.id = self.next_window_id;
            m.placed = true;
            m.override_redirect = true;
            m.desk = desk;
            m.display = display;
        }
        self.next_window_id += 1;
        self.space.map_element(w, loc, true);
        self.repick_pointer = true;
        self.needs_redraw = true;
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(w) = self.window_for_x11(&window) {
            if !meta(&w).borrow().override_redirect {
                self.capture_ghost(&w);
            }
            self.space.unmap_elem(&w);
            self.minimized.retain(|m| m != &w);
            if self.focused_window().is_none() || self.focused_window().as_ref() == Some(&w) {
                let next = self
                    .space
                    .elements()
                    .rev()
                    .find(|e| !meta(e).borrow().override_redirect && self.on_current_space(e))
                    .cloned();
                if let Some(kb) = self.seat.get_keyboard() {
                    kb.set_focus(self, next.map(KeyboardFocusTarget::Window), SERIAL_COUNTER.next_serial());
                }
            }
        }
        if !window.is_override_redirect() {
            let _ = window.set_mapped(false);
        }
        self.repick_pointer = true;
        self.needs_redraw = true;
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(w) = self.window_for_x11(&window) {
            self.space.unmap_elem(&w);
            self.minimized.retain(|m| m != &w);
        }
        self.repick_pointer = true;
    }

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        let mut geo = window.geometry();
        let managed = self.window_for_x11(&window);
        if let Some(w) = w {
            geo.size.w = w as i32;
        }
        if let Some(h) = h {
            geo.size.h = h as i32;
        }
        let self_placed = managed.as_ref().map(|w| meta(w).borrow().override_redirect).unwrap_or(true);
        if self_placed {
            if let Some(x) = x {
                geo.loc.x = x;
            }
            if let Some(y) = y {
                geo.loc.y = y;
            }
        } else if let Some(loc) = managed.as_ref().and_then(|win| self.home_loc(win)) {
            geo.loc = loc;
        }
        let _ = window.configure(geo);
        if let Some(win) = managed.filter(|_| self_placed) {
            self.space.map_element(win, geo.loc, false);
            self.needs_redraw = true;
        }
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<u32>,
    ) {
        let Some(w) = self.window_for_x11(&window) else { return };
        if meta(&w).borrow().override_redirect {
            self.space.map_element(w, geometry.loc, true);
            self.needs_redraw = true;
        }
    }

    fn maximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(w) = self.window_for_x11(&window) {
            if !window.is_maximized() {
                self.toggle_zoom(&w);
            }
        }
    }
    fn unmaximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(w) = self.window_for_x11(&window) {
            if window.is_maximized() {
                self.toggle_zoom(&w);
            }
        }
    }
    fn fullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(w) = self.window_for_x11(&window) {
            self.set_fullscreen(&w, true);
        }
    }
    fn unfullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(w) = self.window_for_x11(&window) {
            self.set_fullscreen(&w, false);
        }
    }
    fn minimize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(w) = self.window_for_x11(&window) {
            self.minimize(&w);
        }
    }
    fn unminimize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(w) = self.window_for_x11(&window) {
            self.focus_window(&w);
        }
    }
    fn active_window_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        _timestamp: u32,
        _current: Option<X11Surface>,
    ) {
        if let Some(w) = self.window_for_x11(&window) {
            self.focus_window(&w);
        }
    }

    fn resize_request(&mut self, _xwm: XwmId, window: X11Surface, button: u32, edges: X11ResizeEdge) {
        let Some(w) = self.window_for_x11(&window) else { return };
        let Some(pointer) = self.seat.get_pointer() else { return };
        let Some(loc) = self.space.element_location(&w) else { return };
        let start =
            smithay::input::pointer::GrabStartData { focus: None, button, location: pointer.current_location() };
        let grab = crate::wm::grabs::ResizeSurfaceGrab::start(
            start,
            w.clone(),
            crate::wm::grabs::resize_grab::ResizeEdge::from_x11(edges),
            Rectangle::new(loc, w.geometry().size),
        );
        pointer.set_grab(self, grab, SERIAL_COUNTER.next_serial(), Focus::Clear);
    }

    fn move_request(&mut self, _xwm: XwmId, window: X11Surface, button: u32) {
        let Some(w) = self.window_for_x11(&window) else { return };
        let Some(pointer) = self.seat.get_pointer() else { return };
        let Some(initial_window_location) = self.space.element_location(&w) else { return };
        let start_data =
            smithay::input::pointer::GrabStartData { focus: None, button, location: pointer.current_location() };
        let grab = crate::wm::grabs::MoveSurfaceGrab { start_data, window: w, initial_window_location };
        pointer.set_grab(self, grab, SERIAL_COUNTER.next_serial(), Focus::Clear);
    }

    fn allow_selection_access(&mut self, _xwm: XwmId, _selection: SelectionTarget) -> bool {
        !self.lock.is_locked()
    }

    fn send_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_type: String, fd: OwnedFd) {
        let r = match selection {
            SelectionTarget::Clipboard => {
                if let Some(crate::selection::clipboard::SelData::History(e)) =
                    current_data_device_selection_userdata(&self.seat).as_deref()
                {
                    crate::selection::clipboard::write_entry(e.clone(), &mime_type, fd);
                    return;
                }
                request_data_device_client_selection(&self.seat, mime_type, fd).map_err(|e| format!("{e:?}"))
            }
            SelectionTarget::Primary => {
                request_primary_client_selection(&self.seat, mime_type, fd).map_err(|e| format!("{e:?}"))
            }
        };
        if let Err(e) = r {
            tracing::debug!("x11 paste failed: {e}");
        }
    }

    fn new_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        match selection {
            SelectionTarget::Clipboard => {
                set_data_device_selection(
                    &self.display_handle,
                    &self.seat,
                    mime_types.clone(),
                    crate::selection::clipboard::SelData::Xwm,
                );
                self.clipboard_record_x11(mime_types);
            }
            SelectionTarget::Primary => set_primary_selection(
                &self.display_handle,
                &self.seat,
                mime_types,
                crate::selection::clipboard::SelData::Xwm,
            ),
        }
    }

    fn cleared_selection(&mut self, _xwm: XwmId, selection: SelectionTarget) {
        match selection {
            SelectionTarget::Clipboard => {
                if matches!(
                    current_data_device_selection_userdata(&self.seat).as_deref(),
                    Some(crate::selection::clipboard::SelData::Xwm)
                ) {
                    clear_data_device_selection(&self.display_handle, &self.seat)
                }
            }
            SelectionTarget::Primary => {
                if matches!(
                    current_primary_selection_userdata(&self.seat).as_deref(),
                    Some(crate::selection::clipboard::SelData::Xwm)
                ) {
                    clear_primary_selection(&self.display_handle, &self.seat)
                }
            }
        }
    }

    fn disconnected(&mut self, _xwm: XwmId) {
        tracing::warn!("XWayland disconnected; restarting");
        self.xwm = None;
        self.xdisplay = None;
        let h = self.loop_handle.clone();
        h.insert_idle(|st| st.start_xwayland());
    }
}
