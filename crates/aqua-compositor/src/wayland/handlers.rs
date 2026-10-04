//! Wayland protocol handlers (adapted from Smithay's smallvil/anvil).
use crate::state::{meta, Aqua, ClientState};
use crate::wm::grabs::{resize_grab, MoveSurfaceGrab, ResizeSurfaceGrab};
use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    desktop::{find_popup_root_surface, get_popup_toplevel_coords, PopupKind, Window},
    input::{
        dnd::{DnDGrab, DndGrabHandler, GrabType, Source},
        pointer::{Focus, GrabStartData as PointerGrabStartData},
        Seat, SeatHandler, SeatState,
    },
    reexports::{
        wayland_protocols::xdg::{
            decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecoMode, shell::server::xdg_toplevel,
        },
        wayland_server::{
            protocol::{wl_buffer, wl_seat, wl_surface::WlSurface},
            Client, Resource,
        },
    },
    utils::{Rectangle, Serial},
    wayland::{
        buffer::BufferHandler,
        compositor::{
            get_parent, is_sync_subsurface, with_states, CompositorClientState, CompositorHandler, CompositorState,
        },
        output::OutputHandler,
        selection::{
            data_device::{set_data_device_focus, DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler},
            primary_selection::{set_primary_focus, PrimarySelectionHandler, PrimarySelectionState},
            SelectionHandler,
        },
        shell::xdg::{
            decoration::XdgDecorationHandler, PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler,
            XdgShellState, XdgToplevelSurfaceData,
        },
        shm::{ShmHandler, ShmState},
        xdg_activation::{XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData},
    },
};

impl CompositorHandler for Aqua {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }
    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        if let Some(x) = client.get_data::<smithay::xwayland::XWaylandClientData>() {
            return &x.compositor_state;
        }
        if let Some(c) = client.get_data::<ClientState>() {
            return &c.compositor_state;
        }
        panic!("client without compositor state")
    }
    /// Don't latch a client buffer before the GPU finished rendering it: wait for the explicit-sync
    /// acquire point (linux-drm-syncobj, used by NVIDIA ≥ 555, Mesa Vulkan …) or the dmabuf's
    /// implicit fence.
    fn new_surface(&mut self, surface: &WlSurface) {
        use smithay::{
            reexports::calloop::Interest,
            wayland::{
                compositor::{add_blocker, add_pre_commit_hook, BufferAssignment, SurfaceAttributes},
                dmabuf::get_dmabuf,
                drm_syncobj::DrmSyncobjCachedState,
            },
        };
        add_pre_commit_hook::<Self, _>(surface, move |state, _dh, surface| {
            let mut acquire_point = None;
            let maybe_dmabuf = with_states(surface, |data| {
                acquire_point.clone_from(&data.cached_state.get::<DrmSyncobjCachedState>().pending().acquire_point);
                data.cached_state.get::<SurfaceAttributes>().pending().buffer.as_ref().and_then(|a| match a {
                    BufferAssignment::NewBuffer(buffer) => get_dmabuf(buffer).cloned().ok(),
                    _ => None,
                })
            });
            let Some(dmabuf) = maybe_dmabuf else { return };
            let Some(client) = surface.client() else { return };
            if let Some(acquire_point) = acquire_point {
                if let Ok((blocker, source)) = acquire_point.generate_blocker() {
                    let c = client.clone();
                    let res = state.loop_handle.insert_source(source, move |_, _, data| {
                        let dh = data.display_handle.clone();
                        data.client_compositor_state(&c).blocker_cleared(data, &dh);
                        Ok(())
                    });
                    if res.is_ok() {
                        add_blocker(surface, blocker);
                        return;
                    }
                }
            }
            if let Ok((blocker, source)) = dmabuf.generate_blocker(Interest::READ) {
                let res = state.loop_handle.insert_source(source, move |_, _, data| {
                    let dh = data.display_handle.clone();
                    data.client_compositor_state(&client).blocker_cleared(data, &dh);
                    Ok(())
                });
                if res.is_ok() {
                    add_blocker(surface, blocker);
                }
            }
        });
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(window) = self.window_for_surface(&root) {
                window.on_commit();
                if window.x11_surface().is_some() {
                    self.needs_redraw = true;
                }
            }
        }
        self.handle_xdg_commit(surface);
        self.handle_layer_commit(surface);
        resize_grab::handle_commit(&mut self.space, surface);
        if let Some(icon) = self.render_cache.dnd_icon.as_mut().filter(|i| &i.surface == surface) {
            let delta = with_states(surface, |s| {
                s.cached_state
                    .get::<smithay::wayland::compositor::SurfaceAttributes>()
                    .current()
                    .buffer_delta
                    .take()
                    .unwrap_or_default()
            });
            icon.offset += delta;
        }
        self.needs_redraw = true;
    }
}

impl BufferHandler for Aqua {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for Aqua {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl XdgShellHandler for Aqua {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        surface.with_pending_state(|s| {
            s.states.set(xdg_toplevel::State::Activated);
            if std::env::var_os("AQUA_NO_TILED").is_some() {
                return;
            }
            s.states.set(xdg_toplevel::State::TiledLeft);
            s.states.set(xdg_toplevel::State::TiledRight);
            s.states.set(xdg_toplevel::State::TiledTop);
            s.states.set(xdg_toplevel::State::TiledBottom);
        });
        let window = Window::new_wayland_window(surface.clone());
        {
            let mut m = meta(&window).borrow_mut();
            m.id = self.next_window_id;
        }
        self.next_window_id += 1;
        // Keyboard focus moves when the window is placed on its first commit (by then its
        // title tells whether it is a menu popup, which must leave the focus where it is).
        self.space.map_element(window.clone(), (-100000, -100000), true);
        self.update_scale_hints();
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        let w = self.window_for_surface(surface.wl_surface());
        if let Some(w) = &w {
            self.capture_ghost(w);
            self.space.unmap_elem(w);
        }
        self.minimized.retain(|w| w.toplevel().map(|t| t != &surface).unwrap_or(true));
        let staged: Vec<_> = self.stage.staged.iter().filter(|w| w.toplevel() == Some(&surface)).cloned().collect();
        for s in &staged {
            self.forget_staged(s);
        }
        let had_focus = match self.seat.get_keyboard().and_then(|k| k.current_focus()) {
            None => true,
            Some(crate::input::focus::KeyboardFocusTarget::Window(f)) => Some(&f) == w.as_ref(),
            Some(_) => false,
        };
        if had_focus {
            self.focus_next(w.as_ref());
        }
        self.needs_redraw = true;
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        let _ = self.popups.track_popup(PopupKind::Xdg(surface));
    }

    fn reposition_request(&mut self, surface: PopupSurface, positioner: PositionerState, token: u32) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial) {
        tracing::debug!("xdg move request");
        let Some(seat) = Seat::from_resource(&seat) else { return };
        if let Some(start_data) = check_grab(&seat, surface.wl_surface(), serial) {
            let Some(window) = self.window_for_surface(surface.wl_surface()) else { return };
            let Some(initial_window_location) = self.space.element_location(&window) else { return };
            self.render_cache.grab_cursor = Some(smithay::input::pointer::CursorIcon::Default);
            let grab = MoveSurfaceGrab { start_data, window, initial_window_location };
            seat.get_pointer().unwrap().set_grab(self, grab, serial, Focus::Clear);
        }
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        seat: wl_seat::WlSeat,
        serial: Serial,
        edges: xdg_toplevel::ResizeEdge,
    ) {
        tracing::debug!("xdg resize request {edges:?}");
        let Some(seat) = Seat::from_resource(&seat) else { return };
        if let Some(start_data) = check_grab(&seat, surface.wl_surface(), serial) {
            let Some(window) = self.window_for_surface(surface.wl_surface()) else { return };
            let Some(initial_window_location) = self.space.element_location(&window) else { return };
            let initial_window_size = window.geometry().size;
            surface.with_pending_state(|state| {
                state.states.set(xdg_toplevel::State::Resizing);
            });
            surface.send_pending_configure();
            self.render_cache.grab_cursor = Some(crate::input::cursors::for_edges(edges.into()));
            let grab = ResizeSurfaceGrab::start(
                start_data,
                window,
                edges.into(),
                Rectangle::new(initial_window_location, initial_window_size),
            );
            seat.get_pointer().unwrap().set_grab(self, grab, serial, Focus::Clear);
        }
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        let is_max = surface.with_pending_state(|s| s.states.contains(xdg_toplevel::State::Maximized));
        match self.window_for_surface(surface.wl_surface()) {
            Some(w) if !is_max => self.zoom_request(&w, true),
            Some(_) => {
                if surface.is_initial_configure_sent() {
                    surface.send_configure();
                }
            }
            None => {
                surface.with_pending_state(|s| s.states.set(xdg_toplevel::State::Maximized));
            }
        }
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        let is_max = surface.with_pending_state(|s| s.states.contains(xdg_toplevel::State::Maximized));
        match self.window_for_surface(surface.wl_surface()) {
            Some(w) if is_max => self.zoom_request(&w, false),
            Some(_) => {
                if surface.is_initial_configure_sent() {
                    surface.send_configure();
                }
            }
            None => {
                surface.with_pending_state(|s| s.states.unset(xdg_toplevel::State::Maximized));
            }
        }
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        if let Some(w) = self.window_for_surface(surface.wl_surface()) {
            self.minimize(&w);
        }
    }

    fn grab(&mut self, surface: PopupSurface, seat: wl_seat::WlSeat, serial: Serial) {
        use crate::input::focus::KeyboardFocusTarget;
        use smithay::desktop::{PopupKeyboardGrab, PopupPointerGrab, PopupUngrabStrategy};
        let Some(seat) = Seat::<Aqua>::from_resource(&seat) else { return };
        let kind = PopupKind::Xdg(surface);
        let Ok(root_surface) = find_popup_root_surface(&kind) else { return };
        let root: KeyboardFocusTarget = match self.window_for_surface(&root_surface) {
            Some(w) => KeyboardFocusTarget::Window(w),
            None => match self.layer_for_surface(&root_surface) {
                Some(l) => KeyboardFocusTarget::LayerSurface(l),
                None => return,
            },
        };
        let ret = self.popups.grab_popup(root, kind, &seat, serial);
        if let Ok(mut grab) = ret {
            if let Some(keyboard) = seat.get_keyboard() {
                if keyboard.is_grabbed()
                    && !(keyboard.has_grab(serial) || keyboard.has_grab(grab.previous_serial().unwrap_or(serial)))
                {
                    grab.ungrab(PopupUngrabStrategy::All);
                    return;
                }
                keyboard.set_focus(self, grab.current_grab(), serial);
                keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
            }
            if let Some(pointer) = seat.get_pointer() {
                if pointer.is_grabbed()
                    && !(pointer.has_grab(serial)
                        || pointer.has_grab(grab.previous_serial().unwrap_or_else(|| grab.serial())))
                {
                    grab.ungrab(PopupUngrabStrategy::All);
                    return;
                }
                pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
                self.render_cache.popup_grab = Some(serial);
            }
        }
    }

    fn fullscreen_request(
        &mut self,
        surface: ToplevelSurface,
        _output: Option<smithay::reexports::wayland_server::protocol::wl_output::WlOutput>,
    ) {
        if let Some(w) = self.window_for_surface(surface.wl_surface()) {
            self.set_fullscreen(&w, true);
        } else {
            surface.with_pending_state(|s| s.states.set(xdg_toplevel::State::Fullscreen));
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        if let Some(w) = self.window_for_surface(surface.wl_surface()) {
            self.set_fullscreen(&w, false);
        }
    }

    fn title_changed(&mut self, _surface: ToplevelSurface) {
        self.needs_redraw = true;
    }
    fn app_id_changed(&mut self, _surface: ToplevelSurface) {
        self.needs_redraw = true;
    }
}

fn check_grab(seat: &Seat<Aqua>, surface: &WlSurface, serial: Serial) -> Option<PointerGrabStartData<Aqua>> {
    use smithay::wayland::seat::WaylandFocus;
    let pointer = seat.get_pointer()?;
    if !pointer.has_grab(serial) {
        return None;
    }
    let start_data = pointer.grab_start_data()?;
    let (focus, _) = start_data.focus.as_ref()?;
    if !focus.same_client_as(&surface.id()) {
        return None;
    }
    Some(start_data)
}

impl Aqua {
    fn handle_xdg_commit(&mut self, surface: &WlSurface) {
        if let Some(window) = self.window_for_surface(surface) {
            let Some(toplevel) = window.toplevel() else { return };
            let initial_configure_sent = with_states(surface, |states| {
                states
                    .data_map
                    .get::<XdgToplevelSurfaceData>()
                    .map(|d| d.lock().unwrap().initial_configure_sent)
                    .unwrap_or(true)
            });
            if !initial_configure_sent {
                toplevel.send_configure();
            } else {
                let placed = meta(&window).borrow().placed;
                let size = window.geometry().size;
                if !placed && size.w > 0 && size.h > 0 && !self.try_place_menu_popup(&window) {
                    self.place_new_window(&window);
                }
            }
        }
        self.popups.commit(surface);
        if let Some(PopupKind::Xdg(ref xdg)) = self.popups.find_popup(surface) {
            if !xdg.is_initial_configure_sent() {
                if let Err(e) = xdg.send_configure() {
                    tracing::warn!("popup initial configure failed: {e:?}");
                }
            }
        }
    }

    /// Centre a new window (slightly above centre), cascading over existing ones.
    fn place_new_window(&mut self, window: &Window) {
        let area = self.placement_output_rect();
        let mut size = window.geometry().size;
        if let Some(t) = window.toplevel() {
            let (mn, mx) = smithay::wayland::compositor::with_states(t.wl_surface(), |st| {
                let c = *st.cached_state.get::<smithay::wayland::shell::xdg::SurfaceCachedState>().current();
                (c.min_size, c.max_size)
            });
            tracing::debug!("place_new_window initial size {:?} min {:?} max {:?}", size, mn, mx);
        }
        let tb = Aqua::titlebar_h(window);
        let chrome = self.chrome(tb);
        let (max_w, max_h) = aqua_wm::place::max_initial_size(crate::wm::to_rect(area), chrome);
        let (wants_max, wants_fs) = window
            .toplevel()
            .map(|t| {
                t.with_pending_state(|s| {
                    (
                        s.states.contains(xdg_toplevel::State::Maximized),
                        s.states.contains(xdg_toplevel::State::Fullscreen),
                    )
                })
            })
            .unwrap_or((false, false));
        if (size.w > max_w || size.h > max_h) && !wants_max && !wants_fs {
            size.w = size.w.min(max_w);
            size.h = size.h.min(max_h);
            if let Some(t) = window.toplevel() {
                t.with_pending_state(|s| s.size = Some(size));
                t.send_pending_configure();
            }
        }
        let (display, cur) = self.placement_desk();
        let n = self.windows_on_desk(&display, cur, Some(window));
        let (x, y) = aqua_wm::place::cascade(crate::wm::to_rect(area), (size.w, size.h), n, chrome);
        let from = self.take_launch_origin(window);
        {
            let mut m = meta(window).borrow_mut();
            m.placed = true;
            m.desk = cur;
            m.display = display;
            m.open_anim = 0.0;
            m.mapped_at = Some(std::time::Instant::now());
            m.launch_from = from;
        }
        let (x, y) =
            if meta(window).borrow().modal { self.modal_position(window, size).unwrap_or((x, y)) } else { (x, y) };
        self.space.map_element(window.clone(), (x, y), true);
        if !self.lock.is_locked() {
            self.focus_window(window);
        }
        if wants_max {
            if let Some(out) = self.output_at((x as f64, y as f64).into()).or_else(|| self.output.clone()) {
                let normal = Rectangle::new((x, y).into(), (size.w.min(max_w), size.h.min(max_h)).into());
                let normal = if size.w > max_w * 9 / 10 || size.h > max_h * 9 / 10 {
                    self.fallback_rect_pub(window, &out)
                } else {
                    normal
                };
                meta(window).borrow_mut().saved = Some(normal);
                self.zoom_in(window, &out);
                meta(window).borrow_mut().geo_anim = None;
            }
        }
        if wants_fs {
            self.set_fullscreen(window, true);
            if size.w > max_w || size.h > max_h {
                if let Some(out) = self.output.clone() {
                    let r = self.fallback_rect_pub(window, &out);
                    meta(window).borrow_mut().fullscreen = Some(r);
                }
            }
        }
        self.update_scale_hints();
        self.needs_redraw = true;
    }

    /// Sheets: modal dialogs are centred horizontally on their parent, just below its titlebar.
    fn modal_position(
        &self,
        window: &Window,
        size: smithay::utils::Size<i32, smithay::utils::Logical>,
    ) -> Option<(i32, i32)> {
        let parent = window.toplevel()?.parent()?;
        let pw = self.window_for_surface(&parent)?;
        let r = self.space.element_geometry(&pw)?;
        Some((r.loc.x + (r.size.w - size.w) / 2, r.loc.y + 4))
    }

    /// Output the new window opens on: the one under the pointer.
    /// Shell chrome around a window with a `titlebar` px title bar.
    pub fn chrome(&self, titlebar: i32) -> aqua_wm::place::Chrome {
        aqua_wm::place::Chrome {
            menubar: self.cfg.menubar_height as i32,
            titlebar,
            dock: (self.cfg.dock_icon_size + 30.0) as i32,
        }
    }

    /// (display, desk) a new window opens on: the display under the pointer, its current desk.
    pub fn placement_desk(&self) -> (String, usize) {
        let p = self.seat.get_pointer().map(|p| p.current_location()).unwrap_or_default();
        let name = self.output_at(p).or_else(|| self.output.clone()).map(|o| o.name()).unwrap_or_default();
        let cur = self.spaces.cur(self.spaces.key(&name));
        (name, cur)
    }

    /// Placed windows on `desk` of `display` (for cascading), not counting `except`.
    pub fn windows_on_desk(&self, display: &str, desk: usize, except: Option<&Window>) -> usize {
        let key = self.spaces.key(display).to_string();
        self.space
            .elements()
            .filter(|w| Some(*w) != except)
            .filter(|w| {
                let m = meta(w).borrow();
                m.placed && !m.override_redirect && m.desk == desk
            })
            .filter(|w| self.win_display(w) == key)
            .count()
    }

    pub fn placement_output_rect(&self) -> Rectangle<i32, smithay::utils::Logical> {
        let p = self.seat.get_pointer().map(|p| p.current_location()).unwrap_or_default();
        self.output_at(p)
            .and_then(|o| self.space.output_geometry(&o))
            .or_else(|| self.output.as_ref().and_then(|o| self.space.output_geometry(o)))
            .unwrap_or_else(|| Rectangle::new((0, 0).into(), (1440, 900).into()))
    }

    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else { return };
        let Some(window) = self.window_for_surface(&root) else {
            self.unconstrain_layer_popup(popup, &root);
            return;
        };
        let Some(window_geo) = self.space.element_geometry(&window) else { return };
        let Some(output) = self.output_at(window_geo.loc.to_f64()).or_else(|| self.output.clone()) else { return };
        let Some(output_geo) = self.space.output_geometry(&output) else { return };
        let mut target = output_geo;
        target.loc -= get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
        target.loc -= window_geo.loc;
        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }
}

impl XdgDecorationHandler for Aqua {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| state.decoration_mode = Some(DecoMode::ServerSide));
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }
    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: DecoMode) {
        let mode = if mode == DecoMode::ClientSide { DecoMode::ClientSide } else { DecoMode::ServerSide };
        toplevel.with_pending_state(|state| state.decoration_mode = Some(mode));
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }
    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| state.decoration_mode = Some(DecoMode::ServerSide));
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }
}

impl SeatHandler for Aqua {
    type KeyboardFocus = crate::input::focus::KeyboardFocusTarget;
    type PointerFocus = crate::input::focus::PointerFocusTarget;
    type TouchFocus = crate::input::focus::PointerFocusTarget;

    fn seat_state(&mut self) -> &mut SeatState<Aqua> {
        &mut self.seat_state
    }
    fn cursor_image(&mut self, _seat: &Seat<Self>, image: smithay::input::pointer::CursorImageStatus) {
        self.render_cache.cursor_status = image;
    }
    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&crate::input::focus::KeyboardFocusTarget>) {
        use smithay::wayland::seat::WaylandFocus;
        let dh = &self.display_handle;
        let wl = focused.and_then(|f| f.wl_surface().map(|s| s.into_owned()));
        let client = wl.as_ref().and_then(|s| dh.get_client(s.id()).ok());
        set_data_device_focus(dh, seat, client.clone());
        set_primary_focus(dh, seat, client);
        self.update_activation_for(focused.cloned());
    }
}

impl SelectionHandler for Aqua {
    type SelectionUserData = crate::selection::clipboard::SelData;

    fn new_selection(
        &mut self,
        ty: smithay::wayland::selection::SelectionTarget,
        source: Option<smithay::wayland::selection::SelectionSource>,
        _seat: Seat<Self>,
    ) {
        use smithay::wayland::selection::SelectionTarget;
        let mimes = source.as_ref().map(|s| s.mime_types());
        if ty == SelectionTarget::Clipboard {
            self.clip.pending = mimes.clone();
        }
        if let Some(xwm) = self.xwm.as_mut() {
            if let Err(e) = xwm.new_selection(ty, mimes) {
                tracing::warn!("forwarding selection to X11 failed: {e}");
            }
        }
    }

    fn send_selection(
        &mut self,
        ty: smithay::wayland::selection::SelectionTarget,
        mime_type: String,
        fd: std::os::unix::io::OwnedFd,
        _seat: Seat<Self>,
        user_data: &Self::SelectionUserData,
    ) {
        match user_data {
            crate::selection::clipboard::SelData::History(e) => {
                crate::selection::clipboard::write_entry(e.clone(), &mime_type, fd)
            }
            crate::selection::clipboard::SelData::Xwm => {
                if let Some(xwm) = self.xwm.as_mut() {
                    if let Err(e) = xwm.send_selection(ty, mime_type, fd) {
                        tracing::warn!("X11 selection transfer failed: {e}");
                    }
                }
            }
        }
    }
}

impl DataDeviceHandler for Aqua {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
}

impl PrimarySelectionHandler for Aqua {
    fn primary_selection_state(&mut self) -> &mut PrimarySelectionState {
        &mut self.primary_selection_state
    }
}

impl Aqua {
    /// End of a drag-and-drop: a refused drop flies its icon back to where it came from.
    fn dnd_finished(&mut self, accepted: bool, location: smithay::utils::Point<f64, smithay::utils::Logical>) {
        if let Some(icon) = &mut self.render_cache.dnd_icon {
            if accepted || self.cfg.reduce_motion {
                self.render_cache.dnd_icon = None;
            } else {
                icon.snap = Some((location, std::time::Instant::now()));
            }
        }
        if self.render_cache.grab_cursor == Some(smithay::input::pointer::CursorIcon::Grabbing) {
            self.render_cache.grab_cursor = None;
            self.render_cache.cursor_override = None;
        }
        self.needs_redraw = true;
    }
}

impl DndGrabHandler for Aqua {
    fn dropped(
        &mut self,
        _target: Option<smithay::input::dnd::DndTarget<'_, Self>>,
        validated: bool,
        _seat: Seat<Self>,
        location: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) {
        self.dnd_finished(validated, location);
    }
    fn cancelled(&mut self, _seat: Seat<Self>, location: smithay::utils::Point<f64, smithay::utils::Logical>) {
        self.dnd_finished(false, location);
    }
}
impl WaylandDndGrabHandler for Aqua {
    fn dnd_requested<S: Source>(
        &mut self,
        source: S,
        icon: Option<WlSurface>,
        seat: Seat<Self>,
        serial: Serial,
        type_: GrabType,
    ) {
        match type_ {
            GrabType::Pointer => {
                let ptr = seat.get_pointer().unwrap();
                let Some(start_data) = ptr.grab_start_data() else {
                    source.cancel();
                    return;
                };
                self.render_cache.dnd_icon = icon.map(|surface| crate::render::DndIcon {
                    surface,
                    offset: (0, 0).into(),
                    origin: start_data.location,
                    snap: None,
                });
                self.render_cache.grab_cursor = Some(smithay::input::pointer::CursorIcon::Grabbing);
                self.render_cache.cursor_override = self.render_cache.grab_cursor;
                let grab = DnDGrab::new_pointer(&self.display_handle, start_data, source, seat);
                ptr.set_grab(self, grab, serial, Focus::Keep);
            }
            GrabType::Touch => source.cancel(),
        }
    }
}

impl OutputHandler for Aqua {}

impl XdgActivationHandler for Aqua {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.xdg_activation_state
    }
    fn token_created(&mut self, _token: XdgActivationToken, _data: XdgActivationTokenData) -> bool {
        true
    }
    fn request_activation(
        &mut self,
        _token: XdgActivationToken,
        token_data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        if token_data.timestamp.elapsed().as_secs() < 10 {
            if let Some(w) = self.window_for_surface(&surface) {
                self.focus_window(&w);
            }
        }
    }
}

smithay::delegate_dispatch2!(Aqua);

impl smithay::wayland::drm_syncobj::DrmSyncobjHandler for Aqua {
    fn drm_syncobj_state(&mut self) -> Option<&mut smithay::wayland::drm_syncobj::DrmSyncobjState> {
        self.udev.as_mut().and_then(|u| u.syncobj_state.as_mut())
    }
}

impl smithay::wayland::dmabuf::DmabufHandler for Aqua {
    fn dmabuf_state(&mut self) -> &mut smithay::wayland::dmabuf::DmabufState {
        &mut self.dmabuf_state
    }
    fn dmabuf_imported(
        &mut self,
        _global: &smithay::wayland::dmabuf::DmabufGlobal,
        dmabuf: smithay::backend::allocator::dmabuf::Dmabuf,
        notifier: smithay::wayland::dmabuf::ImportNotifier,
    ) {
        if self.import_dmabuf(&dmabuf) {
            let _ = notifier.successful::<Aqua>();
        } else {
            notifier.failed();
        }
    }
}
