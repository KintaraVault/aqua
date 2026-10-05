//! Extra Wayland protocols beyond the core desktop set: layer shell, session lock,
//! idle notify/inhibit, data control (clipboard managers), screen capture,
//! fractional scale, presentation time, text input / input method (IME),
//! pointer constraints / gestures / relative pointer, xdg-foreign (portal dialogs),
//! keyboard-shortcut inhibit, cursor shape, XWayland shell, security context …
use crate::input::focus::KeyboardFocusTarget;
use crate::state::Aqua;
use smithay::{
    desktop::{layer_map_for_output, LayerSurface, PopupKind, PopupManager, WindowSurfaceType},
    input::pointer::PointerHandle,
    output::Output,
    reexports::{
        calloop::LoopHandle,
        wayland_server::{
            protocol::{wl_output::WlOutput, wl_surface::WlSurface},
            DisplayHandle, Resource,
        },
    },
    utils::{Logical, Point, Rectangle},
    wayland::{
        compositor::{get_parent, with_states},
        fractional_scale::{with_fractional_scale, FractionalScaleHandler, FractionalScaleManagerState},
        idle_inhibit::{IdleInhibitHandler, IdleInhibitManagerState},
        idle_notify::{IdleNotifierHandler, IdleNotifierState},
        image_capture_source::{
            ImageCaptureSource, ImageCaptureSourceHandler, ImageCaptureSourceState, OutputCaptureSourceHandler,
            OutputCaptureSourceState,
        },
        image_copy_capture::{
            BufferConstraints, Frame, ImageCopyCaptureHandler, ImageCopyCaptureState, Session, SessionRef,
        },
        input_method::{InputMethodHandler, InputMethodManagerState, PopupSurface as ImPopup},
        keyboard_shortcuts_inhibit::{
            KeyboardShortcutsInhibitHandler, KeyboardShortcutsInhibitState, KeyboardShortcutsInhibitor,
        },
        pointer_constraints::{with_pointer_constraint, PointerConstraintsHandler, PointerConstraintsState},
        pointer_gestures::PointerGesturesState,
        presentation::PresentationState,
        relative_pointer::RelativePointerManagerState,
        seat::WaylandFocus,
        selection::{
            ext_data_control::{DataControlHandler as ExtDataControlHandler, DataControlState as ExtDataControlState},
            primary_selection::PrimarySelectionState,
            wlr_data_control::{DataControlHandler, DataControlState},
        },
        session_lock::{LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker},
        shell::{
            wlr_layer::{Layer as WlrLayer, LayerSurface as WlrLayerSurface, WlrLayerShellHandler, WlrLayerShellState},
            xdg::PopupSurface,
        },
        single_pixel_buffer::SinglePixelBufferState,
        text_input::TextInputManagerState,
        viewporter::ViewporterState,
        virtual_keyboard::VirtualKeyboardManagerState,
        xdg_foreign::{XdgForeignHandler, XdgForeignState},
        xwayland_keyboard_grab::{XWaylandKeyboardGrabHandler, XWaylandKeyboardGrabState},
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
};

pub struct Protocols {
    pub layer_shell: WlrLayerShellState,
    pub session_lock: SessionLockManagerState,
    pub idle_notifier: IdleNotifierState<Aqua>,
    pub idle_inhibit: IdleInhibitManagerState,
    pub inhibitors: Vec<WlSurface>,
    pub wlr_data_control: DataControlState,
    pub ext_data_control: ExtDataControlState,
    pub capture_source: ImageCaptureSourceState,
    pub output_capture: OutputCaptureSourceState,
    pub image_copy: ImageCopyCaptureState,
    pub fractional: FractionalScaleManagerState,
    pub presentation: PresentationState,
    pub xdg_foreign: XdgForeignState,
    pub shortcuts_inhibit: KeyboardShortcutsInhibitState,
    pub xwayland_shell: XWaylandShellState,
    /// Surfaces that currently inhibit compositor shortcuts (games, VMs, remote desktops).
    pub shortcut_inhibitors: Vec<KeyboardShortcutsInhibitor>,
}

impl Protocols {
    pub fn new(dh: &DisplayHandle, lh: &LoopHandle<'static, Aqua>, primary: &PrimarySelectionState) -> Self {
        TextInputManagerState::new::<Aqua>(dh);
        InputMethodManagerState::new::<Aqua, _>(dh, |_| true);
        VirtualKeyboardManagerState::new::<Aqua, _>(dh, |_| true);
        RelativePointerManagerState::new::<Aqua>(dh);
        PointerConstraintsState::new::<Aqua>(dh);
        PointerGesturesState::new::<Aqua>(dh);
        SinglePixelBufferState::new::<Aqua>(dh);
        XWaylandKeyboardGrabState::new::<Aqua>(dh);
        smithay::wayland::cursor_shape::CursorShapeManagerState::new::<Aqua>(dh);
        smithay::wayland::xdg_system_bell::XdgSystemBellState::new::<Aqua>(dh);
        smithay::wayland::content_type::ContentTypeState::new::<Aqua>(dh);
        smithay::wayland::alpha_modifier::AlphaModifierState::new::<Aqua>(dh);
        smithay::wayland::fixes::FixesState::new::<Aqua>(dh);
        smithay::wayland::tablet_manager::TabletManagerState::new::<Aqua>(dh);
        smithay::wayland::xdg_toplevel_icon::XdgToplevelIconManager::new::<Aqua>(dh);
        smithay::wayland::shell::xdg::dialog::XdgDialogState::new::<Aqua>(dh);
        let _ = ViewporterState::new::<Aqua>;
        let trusted = |c: &smithay::reexports::wayland_server::Client| {
            c.get_data::<crate::state::ClientState>().map(|d| !d.sandboxed).unwrap_or(true)
        };
        Self {
            layer_shell: WlrLayerShellState::new_with_filter::<Aqua, _>(dh, trusted),
            session_lock: SessionLockManagerState::new::<Aqua, _>(dh, trusted),
            idle_notifier: IdleNotifierState::new(dh, lh.clone()),
            idle_inhibit: IdleInhibitManagerState::new::<Aqua>(dh),
            inhibitors: vec![],
            wlr_data_control: DataControlState::new::<Aqua, _>(dh, Some(primary), trusted),
            ext_data_control: ExtDataControlState::new::<Aqua, _>(dh, Some(primary), trusted),
            capture_source: ImageCaptureSourceState::new(),
            output_capture: OutputCaptureSourceState::new::<Aqua>(dh),
            image_copy: ImageCopyCaptureState::new_with_filter::<Aqua, _>(dh, trusted),
            fractional: FractionalScaleManagerState::new::<Aqua>(dh),
            presentation: PresentationState::new::<Aqua>(dh, libc_clock_monotonic()),
            xdg_foreign: XdgForeignState::new::<Aqua>(dh),
            shortcuts_inhibit: KeyboardShortcutsInhibitState::new::<Aqua>(dh),
            xwayland_shell: XWaylandShellState::new::<Aqua>(dh),
            shortcut_inhibitors: vec![],
        }
    }
}

fn libc_clock_monotonic() -> u32 {
    1
}

impl WlrLayerShellHandler for Aqua {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.p.layer_shell
    }
    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        wl_output: Option<WlOutput>,
        layer: WlrLayer,
        namespace: String,
    ) {
        let output = wl_output
            .as_ref()
            .and_then(Output::from_resource)
            .or_else(|| self.output.clone())
            .or_else(|| self.space.outputs().next().cloned());
        let Some(output) = output else {
            surface.send_close();
            return;
        };
        tracing::info!("layer surface {namespace:?} on {:?} ({layer:?})", output.name());
        let mut map = layer_map_for_output(&output);
        if let Err(e) = map.map_layer(&LayerSurface::new(surface, namespace)) {
            tracing::warn!("failed to map layer surface: {e}");
        }
        drop(map);
        self.needs_redraw = true;
    }
    fn new_popup(&mut self, parent: WlrLayerSurface, popup: PopupSurface) {
        self.unconstrain_layer_popup(&popup, parent.wl_surface());
        let _ = self.popups.track_popup(PopupKind::Xdg(popup));
    }
    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        for o in self.space.outputs().cloned().collect::<Vec<_>>() {
            let mut map = layer_map_for_output(&o);
            let found = map.layers().find(|l| l.layer_surface() == &surface).cloned();
            if let Some(l) = found {
                map.unmap_layer(&l);
            }
        }
        if let Some(kb) = self.seat.get_keyboard() {
            if let Some(KeyboardFocusTarget::LayerSurface(l)) = kb.current_focus() {
                if l.layer_surface() == &surface {
                    let next = self.space.elements().last().cloned().map(KeyboardFocusTarget::Window);
                    kb.set_focus(self, next, smithay::utils::SERIAL_COUNTER.next_serial());
                }
            }
        }
        self.needs_redraw = true;
    }
}

impl Aqua {
    /// Topmost layer surface under `pos` among `layers` (front to back).
    pub fn layer_under(
        &self,
        pos: Point<f64, Logical>,
        layers: &[WlrLayer],
    ) -> Option<(crate::input::focus::PointerFocusTarget, Point<f64, Logical>)> {
        for o in self.space.outputs() {
            let Some(og) = self.space.output_geometry(o) else { continue };
            if !og.to_f64().contains(pos) {
                continue;
            }
            let map = layer_map_for_output(o);
            for l in layers {
                if let Some(layer) = map.layer_under(*l, pos - og.loc.to_f64()) {
                    let lg = map.layer_geometry(layer).unwrap_or_default();
                    if let Some((s, p)) =
                        layer.surface_under(pos - og.loc.to_f64() - lg.loc.to_f64(), WindowSurfaceType::ALL)
                    {
                        return Some((s.into(), (p + lg.loc + og.loc).to_f64()));
                    }
                }
            }
        }
        None
    }

    /// Give keyboard focus to layer surfaces that ask for exclusive keyboard interactivity.
    pub fn layer_keyboard_focus(&mut self) {
        use smithay::wayland::shell::wlr_layer::KeyboardInteractivity;
        let mut want = None;
        for o in self.space.outputs() {
            let map = layer_map_for_output(o);
            for l in map.layers().rev() {
                let excl = l.cached_state().keyboard_interactivity == KeyboardInteractivity::Exclusive;
                if excl && matches!(l.layer(), WlrLayer::Overlay | WlrLayer::Top) {
                    want = Some(l.clone());
                    break;
                }
            }
        }
        if let (Some(l), Some(kb)) = (want, self.seat.get_keyboard()) {
            let target = KeyboardFocusTarget::LayerSurface(l);
            if kb.current_focus().as_ref() != Some(&target) {
                kb.set_focus(self, Some(target), smithay::utils::SERIAL_COUNTER.next_serial());
            }
        }
    }

    pub fn layer_for_surface(&self, s: &WlSurface) -> Option<LayerSurface> {
        for o in self.space.outputs() {
            let map = layer_map_for_output(o);
            if let Some(l) = map.layer_for_surface(s, WindowSurfaceType::ALL) {
                return Some(l.clone());
            }
        }
        None
    }

    /// Layer surfaces need an initial configure and re-arrangement on every commit.
    pub fn handle_layer_commit(&mut self, surface: &WlSurface) {
        use smithay::wayland::shell::wlr_layer::LayerSurfaceData;
        let mut root = surface.clone();
        while let Some(p) = get_parent(&root) {
            root = p;
        }
        let outputs: Vec<Output> = self.space.outputs().cloned().collect();
        for o in outputs {
            let mut map = layer_map_for_output(&o);
            let Some(layer) = map.layer_for_surface(&root, WindowSurfaceType::TOPLEVEL).cloned() else { continue };
            map.arrange();
            let initial_sent = with_states(&root, |st| {
                st.data_map.get::<LayerSurfaceData>().map(|d| d.lock().unwrap().initial_configure_sent).unwrap_or(true)
            });
            if !initial_sent {
                layer.layer_surface().send_configure();
            }
            drop(map);
            self.layer_keyboard_focus();
            self.needs_redraw = true;
            return;
        }
    }

    /// Keep layer-shell popups (e.g. waybar menus) inside their output.
    pub fn unconstrain_layer_popup(&self, popup: &PopupSurface, root: &WlSurface) {
        for o in self.space.outputs() {
            let map = layer_map_for_output(o);
            let Some(layer) = map.layer_for_surface(root, WindowSurfaceType::TOPLEVEL) else { continue };
            let Some(og) = self.space.output_geometry(o) else { continue };
            let lg = map.layer_geometry(layer).unwrap_or_default();
            let mut target = Rectangle::new((0, 0).into(), og.size);
            target.loc -= lg.loc;
            target.loc -= smithay::desktop::get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
            popup.with_pending_state(|st| st.geometry = st.positioner.get_unconstrained_geometry(target));
            return;
        }
    }

    /// Area of the output not covered by exclusive zones (bars), plus our menu bar / Dock.
    pub fn usable_area(&self, o: &Output) -> Rectangle<i32, Logical> {
        let og = self.space.output_geometry(o).unwrap_or_else(|| Rectangle::new((0, 0).into(), (1440, 900).into()));
        let map = layer_map_for_output(o);
        let mut z = map.non_exclusive_zone();
        drop(map);
        z.loc += og.loc;
        let mb = self.cfg.menubar_height.round() as i32;
        let top = og.loc.y + mb;
        if z.loc.y < top {
            z.size.h -= top - z.loc.y;
            z.loc.y = top;
        }
        let primary = self.output.as_ref() == Some(o) && !self.cfg.dock_autohide;
        if primary {
            let dock = (self.cfg.dock_icon_size + 22.0).round() as i32;
            let bottom = og.loc.y + og.size.h - dock;
            if z.loc.y + z.size.h > bottom {
                z.size.h = bottom - z.loc.y;
            }
        }
        z
    }
}

impl SessionLockHandler for Aqua {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.p.session_lock
    }
    fn lock(&mut self, confirmation: SessionLocker) {
        if self.lock.mode == crate::system::lock::Mode::Internal {
            // Already behind Aqua's own lock screen: confirming would let any client end the
            // lock with unlock_and_destroy — without a password. Dropping the locker sends
            // `finished`.
            tracing::warn!("refusing external session locker: the session is already locked");
            drop(confirmation);
            return;
        }
        tracing::info!("external session locker connected");
        self.lock.external_lock(confirmation);
        self.on_locked();
    }
    fn unlock(&mut self) {
        tracing::info!("external session locker unlocked");
        self.lock.external_unlock();
        self.on_unlocked();
    }
    fn new_surface(&mut self, surface: LockSurface, wl_output: WlOutput) {
        let Some(output) = Output::from_resource(&wl_output) else { return };
        if let Some(g) = self.space.output_geometry(&output) {
            surface.with_pending_state(|s| s.size = Some((g.size.w as u32, g.size.h as u32).into()));
            surface.send_configure();
        }
        let wl = surface.wl_surface().clone();
        self.lock.ext_surfaces.push((output, surface));
        if let Some(kb) = self.seat.get_keyboard() {
            kb.set_focus(self, Some(KeyboardFocusTarget::Surface(wl)), smithay::utils::SERIAL_COUNTER.next_serial());
        }
        self.needs_redraw = true;
    }
}

impl IdleNotifierHandler for Aqua {
    fn idle_notifier_state(&mut self) -> &mut IdleNotifierState<Self> {
        &mut self.p.idle_notifier
    }
}

impl IdleInhibitHandler for Aqua {
    fn inhibit(&mut self, surface: WlSurface) {
        self.p.inhibitors.push(surface);
        self.refresh_idle_inhibit();
    }
    fn uninhibit(&mut self, surface: WlSurface) {
        self.p.inhibitors.retain(|s| *s != surface);
        self.refresh_idle_inhibit();
    }
}

impl Aqua {
    pub fn refresh_idle_inhibit(&mut self) {
        self.p.inhibitors.retain(|s| s.is_alive());
        let inhibited =
            !self.p.inhibitors.is_empty() || self.idle.dbus_inhibited() || aqua_notify::portal_impl::inhibited();
        self.p.idle_notifier.set_is_inhibited(inhibited);
        self.idle.inhibited = inhibited;
    }
}

impl DataControlHandler for Aqua {
    fn data_control_state(&mut self) -> &mut DataControlState {
        &mut self.p.wlr_data_control
    }
}
impl ExtDataControlHandler for Aqua {
    fn data_control_state(&mut self) -> &mut ExtDataControlState {
        &mut self.p.ext_data_control
    }
}

impl ImageCaptureSourceHandler for Aqua {
    fn source_destroyed(&mut self, _source: ImageCaptureSource) {}
}

impl OutputCaptureSourceHandler for Aqua {
    fn output_capture_source_state(&mut self) -> &mut OutputCaptureSourceState {
        &mut self.p.output_capture
    }
    fn output_source_created(&mut self, source: ImageCaptureSource, output: &Output) {
        source.user_data().insert_if_missing(|| output.downgrade());
    }
}

impl ImageCopyCaptureHandler for Aqua {
    fn image_copy_capture_state(&mut self) -> &mut ImageCopyCaptureState {
        &mut self.p.image_copy
    }
    fn capture_constraints(&mut self, source: &ImageCaptureSource) -> Option<BufferConstraints> {
        let output = source.user_data().get::<smithay::output::WeakOutput>()?.upgrade()?;
        let mode = output.current_mode()?;
        use smithay::reexports::wayland_server::protocol::wl_shm::Format;
        Some(BufferConstraints {
            size: (mode.size.w, mode.size.h).into(),
            shm: vec![Format::Argb8888, Format::Xrgb8888, Format::Abgr8888, Format::Xbgr8888],
            dma: None,
        })
    }
    fn new_session(&mut self, _session: Session) {}
    fn frame(&mut self, session: &SessionRef, frame: Frame) {
        let Some(output) = session.source().user_data().get::<smithay::output::WeakOutput>().and_then(|w| w.upgrade())
        else {
            frame.fail(smithay::wayland::image_copy_capture::CaptureFailureReason::Unknown);
            return;
        };
        if self.lock.is_locked() {
            frame.fail(smithay::wayland::image_copy_capture::CaptureFailureReason::Stopped);
            return;
        }
        let cursor = session.draw_cursor();
        self.pending_captures.push((frame, output, cursor));
        self.needs_redraw = true;
    }
}

impl FractionalScaleHandler for Aqua {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        let mut root = surface.clone();
        while let Some(p) = get_parent(&root) {
            root = p;
        }
        let scale = self
            .window_for_surface(&root)
            .and_then(|w| self.space.outputs_for_element(&w).first().map(|o| o.current_scale().fractional_scale()))
            .unwrap_or(self.scale);
        with_states(&surface, |states| with_fractional_scale(states, |f| f.set_preferred_scale(scale)));
    }
}

impl InputMethodHandler for Aqua {
    fn new_popup(&mut self, surface: ImPopup) {
        if let Err(e) = self.popups.track_popup(PopupKind::from(surface)) {
            tracing::warn!("failed to track IME popup: {e}");
        }
    }
    fn popup_repositioned(&mut self, _: ImPopup) {}
    fn dismiss_popup(&mut self, surface: ImPopup) {
        if let Some(parent) = surface.get_parent().map(|p| p.surface.clone()) {
            let _ = PopupManager::dismiss_popup(&parent, &PopupKind::from(surface));
        }
    }
    fn parent_geometry(&self, parent: &WlSurface) -> Rectangle<i32, Logical> {
        self.space
            .elements()
            .find_map(|w| (w.wl_surface().as_deref() == Some(parent)).then(|| w.geometry()))
            .unwrap_or_default()
    }
}

impl KeyboardShortcutsInhibitHandler for Aqua {
    fn keyboard_shortcuts_inhibit_state(&mut self) -> &mut KeyboardShortcutsInhibitState {
        &mut self.p.shortcuts_inhibit
    }
    fn new_inhibitor(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        inhibitor.activate();
        self.p.shortcut_inhibitors.push(inhibitor);
    }
    fn inhibitor_destroyed(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        self.p.shortcut_inhibitors.retain(|i| i != &inhibitor);
    }
}

impl Aqua {
    /// Is the focused surface inhibiting compositor shortcuts?
    pub fn shortcuts_inhibited(&self) -> bool {
        let Some(f) = self.seat.get_keyboard().and_then(|k| k.current_focus()) else { return false };
        let Some(s) = f.wl_surface() else { return false };
        self.p.shortcut_inhibitors.iter().any(|i| i.is_active() && i.wl_surface() == &*s)
    }
}

impl PointerConstraintsHandler for Aqua {
    fn new_constraint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>) {
        if pointer.current_focus().and_then(|f| f.wl_surface().map(|s| s.into_owned())).as_ref() == Some(surface) {
            with_pointer_constraint(surface, pointer, |c| {
                if let Some(c) = c {
                    c.activate()
                }
            });
        }
    }
    fn cursor_position_hint(
        &mut self,
        surface: &WlSurface,
        pointer: &PointerHandle<Self>,
        location: Point<f64, Logical>,
    ) {
        if with_pointer_constraint(surface, pointer, |c| c.is_some_and(|c| c.is_active())) {
            let origin = self
                .space
                .elements()
                .find_map(|w| (w.wl_surface().as_deref() == Some(surface)).then(|| self.space.element_location(w)))
                .flatten()
                .unwrap_or_default()
                .to_f64();
            pointer.set_location(origin + location);
        }
    }
}

impl XdgForeignHandler for Aqua {
    fn xdg_foreign_state(&mut self) -> &mut XdgForeignState {
        &mut self.p.xdg_foreign
    }
}

impl XWaylandShellHandler for Aqua {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.p.xwayland_shell
    }
}

impl XWaylandKeyboardGrabHandler for Aqua {
    fn keyboard_focus_for_xsurface(&self, surface: &WlSurface) -> Option<KeyboardFocusTarget> {
        self.window_for_surface(surface).map(KeyboardFocusTarget::Window)
    }
}

impl smithay::wayland::xdg_system_bell::XdgSystemBellHandler for Aqua {
    fn ring(&mut self, surface: Option<WlSurface>) {
        if let Some(w) = surface.and_then(|s| self.window_for_surface(&s)) {
            let (app, _) = crate::state::title_of(&w);
            self.shell.bounce_app(&app);
        }
        if self.cfg.alert_sound {
            crate::system::sound::play_alert();
        }
        self.needs_redraw = true;
    }
}

impl smithay::input::tablet::TabletSeatHandler for Aqua {
    type ToolFocus = crate::input::focus::PointerFocusTarget;
}

impl smithay::wayland::xdg_toplevel_icon::XdgToplevelIconHandler for Aqua {}

impl smithay::wayland::shell::xdg::dialog::XdgDialogHandler for Aqua {
    fn dialog_hint_changed(
        &mut self,
        toplevel: smithay::wayland::shell::xdg::ToplevelSurface,
        hint: smithay::wayland::shell::xdg::dialog::ToplevelDialogHint,
    ) {
        if let Some(w) = self.window_for_surface(toplevel.wl_surface()) {
            crate::state::meta(&w).borrow_mut().modal =
                matches!(hint, smithay::wayland::shell::xdg::dialog::ToplevelDialogHint::Modal);
        }
    }
}
