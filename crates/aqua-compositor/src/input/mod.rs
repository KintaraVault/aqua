//! Input routing: shell UI first, then window chrome, then clients.
pub mod config;
pub mod cursors;
pub mod focus;
pub mod gestures;

use crate::state::{meta, Aqua};
use crate::wm::grabs::MoveSurfaceGrab;
use aqua_shell::{decor, Key};
use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent, InputTime, KeyState,
        KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
    },
    desktop::Window,
    input::{
        keyboard::{keysyms, FilterResult, ModifiersState},
        pointer::{AxisFrame, ButtonEvent, Focus, GrabStartData, MotionEvent, RelativeMotionEvent},
    },
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER},
};

pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;

#[derive(Debug, Clone)]
enum KeyAction {
    Quit,
    Launchpad,
    Terminal,
    CloseWindow,
    QuitApp,
    Minimize,
    HideOthers,
    Screenshot,
    Shell(Option<Key>, Option<String>),
    ControlCenter,
    Vt(i32),
    Spotlight,
    Switch(bool),
    SwitchCancel,
    Mission,
    /// Switch desktop: relative (±1) or absolute index.
    SpaceRel(i32),
    SpaceAbs(usize),
    /// Carry the focused window one desktop left/right.
    SpaceMove(i32),
    Lock,
    Clipboard,
    CharViewer,
    Fullscreen,
    ForceQuit,
    NextLayout,
    Media(Media),
    Bind(String),
    PowerKey,
    SleepKey,
    /// Swallowed (e.g. key release of an intercepted press).
    Nothing,
}

#[derive(Debug, Clone, Copy)]
pub enum Media {
    VolUp,
    VolDown,
    Mute,
    BrightUp,
    BrightDown,
    MicMute,
}

/// System Settings → Keyboard Shortcuts drops this marker while it records a new
/// chord, so the compositor doesn't swallow it (stale markers expire after a minute).
fn shortcut_capture_active() -> bool {
    std::fs::metadata(aqua_config::paths::runtime_dir().join("aqua-shortcut-capture"))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .map(|d| d.as_secs() < 60)
        .unwrap_or(false)
}

fn cmd_mod(m: &ModifiersState) -> bool {
    m.logo || (m.alt && std::env::var("AQUA_CMD").map(|v| v == "alt").unwrap_or(false))
}

impl Aqua {
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>)
    where
        I::Device: 'static,
    {
        if !matches!(event, InputEvent::DeviceAdded { .. } | InputEvent::DeviceRemoved { .. }) {
            self.notify_activity();
        }
        match event {
            InputEvent::DeviceAdded { device } => {
                let any: &dyn std::any::Any = &device;
                if let Some(d) = any.downcast_ref::<smithay::reexports::input::Device>() {
                    let mut d = d.clone();
                    crate::input::config::configure_device(&mut d, &self.input_cfg.pointer);
                    if let Some(ud) = self.udev.as_mut() {
                        ud.input_devices.push(d);
                    }
                }
            }
            InputEvent::DeviceRemoved { device } => {
                let any: &dyn std::any::Any = &device;
                if let Some(d) = any.downcast_ref::<smithay::reexports::input::Device>() {
                    if let Some(ud) = self.udev.as_mut() {
                        ud.input_devices.retain(|x| x != d);
                    }
                }
            }
            InputEvent::Keyboard { event, .. } => self.on_key(event.key_code(), event.state(), event.time()),
            InputEvent::GestureSwipeBegin { event, .. } => self.on_swipe_begin::<I>(event),
            InputEvent::GestureSwipeUpdate { event, .. } => self.on_swipe_update::<I>(event),
            InputEvent::GestureSwipeEnd { event, .. } => self.on_swipe_end::<I>(event),
            InputEvent::GesturePinchBegin { event, .. } => self.on_pinch_begin::<I>(event),
            InputEvent::GesturePinchUpdate { event, .. } => self.on_pinch_update::<I>(event),
            InputEvent::GesturePinchEnd { event, .. } => self.on_pinch_end::<I>(event),
            InputEvent::GestureHoldBegin { event, .. } => self.on_hold_begin::<I>(event),
            InputEvent::GestureHoldEnd { event, .. } => self.on_hold_end::<I>(event),
            InputEvent::PointerMotion { event, .. } => {
                let pointer = self.seat.get_pointer().unwrap();
                let cur = pointer.current_location();
                if self.pointer_locked(cur) {
                    let under = self.surface_under(cur);
                    pointer.relative_motion(
                        self,
                        under,
                        &RelativeMotionEvent {
                            delta: event.delta(),
                            delta_unaccel: event.delta_unaccel(),
                            time: event.time(),
                        },
                    );
                    pointer.frame(self);
                    return;
                }
                let pos = self.clamp_to_outputs(cur + event.delta());
                self.on_motion(pos, event.time());
                let under = self.surface_under(pos);
                pointer.relative_motion(
                    self,
                    under,
                    &RelativeMotionEvent {
                        delta: event.delta(),
                        delta_unaccel: event.delta_unaccel(),
                        time: event.time(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerMotionAbsolute { event, .. } => {
                let g = self.output.as_ref().and_then(|o| self.space.output_geometry(o));
                let (w, h) = g.map(|g| (g.size.w, g.size.h)).unwrap_or(self.output_size());
                let off = g.map(|g| g.loc.to_f64()).unwrap_or_default();
                let pos = event.position_transformed((w, h).into()) + off;
                self.on_motion(pos, event.time());
            }
            InputEvent::PointerButton { event, .. } => self.on_button(event.button_code(), event.state(), event.time()),
            InputEvent::PointerAxis { event, .. } => {
                let pointer = self.seat.get_pointer().unwrap();
                let pos = pointer.current_location();
                let source = event.source();
                let h = event
                    .amount(Axis::Horizontal)
                    .unwrap_or_else(|| event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.);
                let v = event
                    .amount(Axis::Vertical)
                    .unwrap_or_else(|| event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.);
                let p = &self.cfg.pointer;
                let mut k = p.scroll_factor.clamp(0.1, 5.0);
                if self.udev.is_none()
                    && (if source == AxisSource::Finger { p.natural_scroll } else { p.mouse_natural_scroll })
                {
                    k = -k;
                }
                let (h, v) = (h * k, v * k);
                let v120 = |d: f64| (d * k).round() as i32;
                if self.lock.mode == crate::system::lock::Mode::Internal {
                    return;
                }
                if self.shell.wants_pointer(pos.x as f32, pos.y as f32) {
                    let wheel = matches!(source, AxisSource::Wheel | AxisSource::WheelTilt);
                    if self.shell.scroll(pos.x as f32, pos.y as f32, v as f32, wheel) {
                        self.needs_redraw = true;
                    }
                    return;
                }
                let mut frame = AxisFrame::new(event.time()).source(source);
                if h != 0.0 {
                    frame = frame.value(Axis::Horizontal, h);
                    if let Some(d) = event.amount_v120(Axis::Horizontal) {
                        frame = frame.v120(Axis::Horizontal, v120(d));
                    }
                }
                if v != 0.0 {
                    frame = frame.value(Axis::Vertical, v);
                    if let Some(d) = event.amount_v120(Axis::Vertical) {
                        frame = frame.v120(Axis::Vertical, v120(d));
                    }
                }
                if source == AxisSource::Finger {
                    if event.amount(Axis::Horizontal) == Some(0.0) {
                        frame = frame.stop(Axis::Horizontal);
                    }
                    if event.amount(Axis::Vertical) == Some(0.0) {
                        frame = frame.stop(Axis::Vertical);
                    }
                }
                pointer.axis(self, frame);
                pointer.frame(self);
            }
            _ => {}
        }
    }

    pub(crate) fn on_key(&mut self, code: smithay::input::keyboard::Keycode, state: KeyState, time: InputTime) {
        let serial = SERIAL_COUNTER.next_serial();
        let kb = self.seat.get_keyboard().unwrap();
        let modal = self.shell.has_modal() || self.shell.locked;
        let internal_lock = self.lock.mode == crate::system::lock::Mode::Internal;
        let locked = self.lock.is_locked();
        let inhibited = !locked && self.shortcuts_inhibited();
        let act = kb.input::<KeyAction, _>(self, code, state, serial, time, |st, m, h| {
            let sym = h.modified_sym();
            let raw = sym.raw();
            st.shell.lockscreen.caps = m.caps_lock;
            if state == KeyState::Released {
                if st.render_cache.suppressed.contains(&code) {
                    st.render_cache.suppressed.retain(|c| *c != code);
                    return FilterResult::Intercept(KeyAction::Nothing);
                }
                return FilterResult::Forward;
            }
            let plain = crate::input::config::KeySyms {
                current: h.raw_syms().first().copied().unwrap_or(sym),
                latin: crate::input::config::latin_sym(&h),
            };
            let digit = code.raw().checked_sub(8).filter(|c| (2..=10).contains(c)).map(|c| c - 2);
            let media = match raw {
                keysyms::KEY_XF86AudioRaiseVolume => Some(Media::VolUp),
                keysyms::KEY_XF86AudioLowerVolume => Some(Media::VolDown),
                keysyms::KEY_XF86AudioMute => Some(Media::Mute),
                keysyms::KEY_XF86AudioMicMute => Some(Media::MicMute),
                keysyms::KEY_XF86MonBrightnessUp => Some(Media::BrightUp),
                keysyms::KEY_XF86MonBrightnessDown => Some(Media::BrightDown),
                _ => None,
            };
            let a = if (keysyms::KEY_XF86Switch_VT_1..=keysyms::KEY_XF86Switch_VT_12).contains(&raw) {
                Some(KeyAction::Vt((raw - keysyms::KEY_XF86Switch_VT_1 + 1) as i32))
            } else if let Some(md) = media {
                Some(KeyAction::Media(md))
            } else if raw == keysyms::KEY_XF86PowerOff {
                Some(KeyAction::PowerKey)
            } else if raw == keysyms::KEY_XF86Sleep || raw == keysyms::KEY_XF86Suspend {
                Some(KeyAction::SleepKey)
            } else if locked {
                if internal_lock {
                    let key = match raw {
                        keysyms::KEY_Escape => Some(Key::Escape),
                        keysyms::KEY_Return | keysyms::KEY_KP_Enter => Some(Key::Enter),
                        keysyms::KEY_BackSpace => Some(Key::Backspace),
                        keysyms::KEY_Left => Some(Key::Left),
                        keysyms::KEY_Right => Some(Key::Right),
                        keysyms::KEY_Up => Some(Key::Up),
                        keysyms::KEY_Down => Some(Key::Down),
                        keysyms::KEY_Tab => Some(Key::Tab),
                        _ => None,
                    };
                    let switch = st.input_cfg.switch_chord.as_ref().map(|c| c.matches_key(m, &plain)).unwrap_or(false);
                    if switch {
                        Some(KeyAction::NextLayout)
                    } else {
                        let text = if key.is_none() && !m.ctrl && !m.logo {
                            sym.key_char().filter(|c| !c.is_control()).map(|c| c.to_string())
                        } else {
                            None
                        };
                        Some(KeyAction::Shell(key, text))
                    }
                } else {
                    None
                }
            } else if inhibited || shortcut_capture_active() {
                None
            } else if let Some(cmd) =
                st.input_cfg.bindings.iter().find(|(c, _)| c.matches_key(m, &plain)).map(|(_, c)| c.clone())
            {
                Some(KeyAction::Bind(cmd))
            } else if st.input_cfg.switch_chord.as_ref().map(|c| c.matches_key(m, &plain)).unwrap_or(false) {
                Some(KeyAction::NextLayout)
            } else if st.mission.open && raw == keysyms::KEY_Escape {
                Some(KeyAction::Mission)
            } else if st.shell.switcher.open && raw == keysyms::KEY_Escape {
                Some(KeyAction::SwitchCancel)
            } else if let Some(id) = st.input_cfg.system_match(m, &plain) {
                Some(match id {
                    "spotlight" => KeyAction::Spotlight,
                    "switch-apps" => KeyAction::Switch(m.shift || raw == keysyms::KEY_ISO_Left_Tab),
                    "mission" => KeyAction::Mission,
                    "space-left" => KeyAction::SpaceRel(-1),
                    "space-right" => KeyAction::SpaceRel(1),
                    "move-left" => KeyAction::SpaceMove(-1),
                    "move-right" => KeyAction::SpaceMove(1),
                    "launchpad" => KeyAction::Launchpad,
                    "control-center" => KeyAction::ControlCenter,
                    "close-window" => KeyAction::CloseWindow,
                    "minimize" => KeyAction::Minimize,
                    "fullscreen" => KeyAction::Fullscreen,
                    "quit-app" => KeyAction::QuitApp,
                    "hide-others" => KeyAction::HideOthers,
                    "force-quit" => KeyAction::ForceQuit,
                    "screenshot" => KeyAction::Screenshot,
                    "terminal" => KeyAction::Terminal,
                    "clipboard" => KeyAction::Clipboard,
                    "chars" => KeyAction::CharViewer,
                    "lock" => KeyAction::Lock,
                    "quit-aqua" => KeyAction::Quit,
                    other => KeyAction::Bind(other.to_string()),
                })
            } else if raw == keysyms::KEY_XF86ScreenSaver {
                Some(KeyAction::Lock)
            } else if raw == keysyms::KEY_XF86LaunchA {
                Some(KeyAction::Mission)
            } else if raw == keysyms::KEY_XF86LaunchB {
                Some(KeyAction::Launchpad)
            } else if let (true, Some(d)) = (m.ctrl && !m.alt && !m.shift && !m.logo, digit) {
                Some(KeyAction::SpaceAbs(d as usize))
            } else if modal {
                let key = match raw {
                    keysyms::KEY_Escape => Some(Key::Escape),
                    keysyms::KEY_Return | keysyms::KEY_KP_Enter => Some(Key::Enter),
                    keysyms::KEY_BackSpace => Some(Key::Backspace),
                    keysyms::KEY_Left => Some(Key::Left),
                    keysyms::KEY_Right => Some(Key::Right),
                    keysyms::KEY_Up => Some(Key::Up),
                    keysyms::KEY_Down => Some(Key::Down),
                    keysyms::KEY_Tab => Some(Key::Tab),
                    _ if cmd_mod(m) && !m.ctrl => plain
                        .latin
                        .key_char()
                        .filter(|c| c.is_ascii_alphanumeric())
                        .map(|c| Key::Cmd(c.to_ascii_lowercase())),
                    _ => None,
                };
                let text = if key.is_none() && !m.ctrl && !m.logo {
                    sym.key_char().filter(|c| !c.is_control()).map(|c| c.to_string())
                } else {
                    None
                };
                Some(KeyAction::Shell(key, text))
            } else {
                None
            };
            match a {
                Some(a) => {
                    st.render_cache.suppressed.push(code);
                    FilterResult::Intercept(a)
                }
                None if internal_lock => FilterResult::Intercept(KeyAction::Nothing),
                None => FilterResult::Forward,
            }
        });
        if state == KeyState::Released && self.shell.switcher.open {
            let m = kb.modifier_state();
            if !(cmd_mod(&m) || m.alt) {
                if let Some(app) = self.shell.switcher.commit() {
                    self.shell_actions(vec![aqua_shell::Action::Activate(app)]);
                }
                self.needs_redraw = true;
            }
        }
        let Some(act) = act else { return };
        self.needs_redraw = true;
        match act {
            KeyAction::Nothing => {}
            KeyAction::Lock => self.lock_session(),
            KeyAction::Clipboard => self.shell.toggle_clipboard(),
            KeyAction::CharViewer => self.shell_actions(vec![aqua_shell::Action::ShowChars]),
            KeyAction::Fullscreen => self.shell_actions(vec![aqua_shell::Action::FullscreenFocused]),
            KeyAction::ForceQuit => {
                let app = self.focused_window().map(|w| crate::state::title_of(&w).0).unwrap_or_default();
                self.shell_actions(vec![aqua_shell::Action::ForceQuit(app)]);
            }
            KeyAction::NextLayout => self.next_layout(),
            KeyAction::Media(md) => self.media_key(md),
            KeyAction::Bind(cmd) => {
                if !self.run_named_action(&cmd) {
                    aqua_apps::launch(cmd.strip_prefix("exec:").unwrap_or(&cmd));
                }
            }
            KeyAction::PowerKey => {
                if !self.lock.is_locked() {
                    self.shell_actions(vec![aqua_shell::Action::ShutDown]);
                }
            }
            KeyAction::SleepKey => self.shell_actions(vec![aqua_shell::Action::Sleep]),
            KeyAction::Quit => self.loop_signal.stop(),
            KeyAction::Vt(n) => self.change_vt(n),
            KeyAction::Launchpad => self.shell.toggle_launchpad(),
            KeyAction::Spotlight => self.shell.toggle_spotlight(),
            KeyAction::Switch(back) => {
                if self.shell.switcher.open {
                    self.shell.switcher.step(back);
                } else {
                    let order = self.app_mru();
                    self.shell.switcher.start(order, back);
                }
            }
            KeyAction::SwitchCancel => self.shell.switcher.cancel(),
            KeyAction::Mission => self.toggle_mission(),
            KeyAction::SpaceRel(d) => self.switch_space_rel(d),
            KeyAction::SpaceAbs(i) => self.switch_space(i),
            KeyAction::SpaceMove(d) => {
                if let Some(w) = self.focused_window() {
                    let display = self.win_display(&w);
                    let t = self.spaces.cur(&display) as i32 + d;
                    if t >= 0 && (t as usize) < self.spaces.count(&display) {
                        self.move_to_space(&w, t as usize);
                        self.focus_window(&w);
                    } else {
                        self.switch_space_rel(d);
                    }
                }
            }
            KeyAction::ControlCenter => self.shell.control.toggle(),
            KeyAction::Terminal => {
                let t = aqua_shell::dock::resolve_exec(&self.cfg.terminal);
                aqua_apps::launch(&t);
            }
            KeyAction::CloseWindow => self.shell_actions(vec![aqua_shell::Action::CloseFocused]),
            KeyAction::Minimize => self.shell_actions(vec![aqua_shell::Action::MinimizeFocused]),
            KeyAction::HideOthers => self.shell_actions(vec![aqua_shell::Action::HideOthers]),
            KeyAction::Screenshot => self.shell_actions(vec![aqua_shell::Action::Screenshot]),
            KeyAction::QuitApp => {
                if let Some(w) = self.focused_window() {
                    let (id, _) = crate::state::title_of(&w);
                    if id.is_empty() {
                        self.close(&w)
                    } else {
                        self.quit_app(&id)
                    }
                }
            }
            KeyAction::Shell(k, t) => {
                if k.is_some() || t.is_some() {
                    let (_, acts) = self.shell.key(k, t.as_deref());
                    self.handle_actions(acts);
                }
            }
        }
    }

    pub fn on_motion(&mut self, pos: Point<f64, Logical>, time: InputTime) {
        let serial = SERIAL_COUNTER.next_serial();
        let pointer = self.seat.get_pointer().unwrap();
        let (x, y) = (pos.x as f32, pos.y as f32);
        if self.lock.mode == crate::system::lock::Mode::Internal {
            self.render_cache.cursor_override = Some(smithay::input::pointer::CursorIcon::Default);
            self.shell.pointer_motion(x, y);
            pointer.motion(self, None, &MotionEvent { location: pos, serial, time });
            pointer.frame(self);
            self.needs_redraw = true;
            return;
        }
        if self.lock.mode == crate::system::lock::Mode::External {
            self.render_cache.cursor_override = None;
            let under = self.surface_under(pos);
            pointer.motion(self, under, &MotionEvent { location: pos, serial, time });
            pointer.frame(self);
            self.needs_redraw = true;
            return;
        }
        if self.mission.open {
            self.render_cache.cursor_override = Some(smithay::input::pointer::CursorIcon::Default);
            self.mission_motion(pos);
            pointer.motion(self, None, &MotionEvent { location: pos, serial, time });
            pointer.frame(self);
            self.needs_redraw = true;
            return;
        }
        if !pointer.is_grabbed() {
            self.hot_corner(pos);
        }
        self.shell.pointer_motion(x, y);
        let in_shell = !pointer.is_grabbed() && self.shell.wants_pointer(x, y);
        self.pointer_in_shell = in_shell;
        let mut hover_changed = false;
        let hovered = if in_shell { None } else { self.window_frame_under(pos) };
        let mut on_zoom = None;
        for w in self.space.elements() {
            let mut hv = false;
            if Some(w) == hovered.as_ref() && crate::state::is_ssd(w) {
                if let Some(fr) = self.frame_rect(w) {
                    let lx = (pos.x - fr.loc.x as f64) as f32;
                    let ly = (pos.y - fr.loc.y as f64) as f32;
                    hv = ly < aqua_config::metrics::TITLEBAR_HEIGHT && lx < 80.0;
                    if hv && decor::button_at(lx, ly) == Some(decor::Button::Zoom) {
                        on_zoom = Some(meta(w).borrow().id);
                    }
                }
            }
            let mut m = meta(w).borrow_mut();
            if m.hover_lights != hv {
                m.hover_lights = hv;
                hover_changed = true;
            }
        }
        if hover_changed {
            self.needs_redraw = true;
        }
        if pointer.is_grabbed() {
            on_zoom = None;
        }
        match (on_zoom, self.render_cache.zoom_hover) {
            (Some(id), Some((h, _))) if h == id => {}
            (Some(id), _) => self.render_cache.zoom_hover = Some((id, std::time::Instant::now())),
            (None, _) => self.render_cache.zoom_hover = None,
        }
        let under = if in_shell { None } else { self.surface_under(pos) };
        self.render_cache.cursor_override =
            self.cursor_override_at(pos, in_shell, pointer.is_grabbed(), under.is_some());
        pointer.motion(self, under, &MotionEvent { location: pos, serial, time });
        pointer.frame(self);
        self.needs_redraw = true;
    }

    /// The pointer shape the compositor wants at `pos`, or `None` to let the client decide.
    fn cursor_override_at(
        &self,
        pos: Point<f64, Logical>,
        in_shell: bool,
        grabbed: bool,
        over_client: bool,
    ) -> Option<smithay::input::pointer::CursorIcon> {
        use smithay::input::pointer::CursorIcon;
        if grabbed {
            return self.render_cache.grab_cursor;
        }
        if in_shell {
            return Some(if self.shell.dragging_icon() { CursorIcon::Grabbing } else { CursorIcon::Default });
        }
        if let Some((_, edges)) = self.resize_edge_at(pos) {
            return Some(crate::input::cursors::for_edges(edges));
        }
        if !over_client {
            return Some(CursorIcon::Default);
        }
        if let Some(w) = self.window_frame_under(pos) {
            if let Some(fr) = self.frame_rect(&w) {
                let tb = Aqua::titlebar_h(&w) as f64;
                if tb > 0.0 && pos.y < fr.loc.y as f64 + tb {
                    return Some(CursorIcon::Default);
                }
            }
        }
        None
    }

    pub(crate) fn on_button(&mut self, button: u32, state: ButtonState, time: InputTime) {
        let pointer = self.seat.get_pointer().unwrap();
        let serial = SERIAL_COUNTER.next_serial();
        let pos = pointer.current_location();
        let (x, y) = (pos.x as f32, pos.y as f32);
        self.needs_redraw = true;

        let others_held = self.render_cache.held_buttons.iter().any(|b| *b != button);
        match state {
            ButtonState::Pressed => {
                self.dismiss_menu_popups(Some(pos));
                self.shell.shot.note_click(x, y);
                if !self.render_cache.held_buttons.contains(&button) {
                    self.render_cache.held_buttons.push(button);
                }
            }
            ButtonState::Released => self.render_cache.held_buttons.retain(|b| *b != button),
        }
        if state == ButtonState::Pressed && !others_held && pointer.is_grabbed() {
            let grab_serial = pointer.with_grab(|s, _| s);
            let is_popup = grab_serial.is_some() && grab_serial == self.render_cache.popup_grab;
            if !is_popup {
                tracing::warn!("dropping a pointer grab that outlived its button press");
                pointer.unset_grab(self, serial, time);
                self.render_cache.grab_cursor = None;
            }
        }

        if self.lock.mode == crate::system::lock::Mode::Internal {
            if state == ButtonState::Pressed {
                let acts = self.shell.pointer_button(x, y, true);
                self.handle_actions(acts);
            } else {
                pointer.button(self, &ButtonEvent { button, state, serial, time });
                pointer.frame(self);
            }
            return;
        }
        if self.lock.mode == crate::system::lock::Mode::External {
            pointer.button(self, &ButtonEvent { button, state, serial, time });
            pointer.frame(self);
            return;
        }
        if self.shell.shot.active() && button == BTN_LEFT {
            let acts = self.shell.pointer_button(x, y, state == ButtonState::Pressed);
            self.handle_actions(acts);
            if state == ButtonState::Released {
                pointer.button(self, &ButtonEvent { button, state, serial, time });
                pointer.frame(self);
            }
            self.needs_redraw = true;
            return;
        }
        if state == ButtonState::Released && button == BTN_LEFT && self.shell.pointer_captured() {
            let acts = self.shell.pointer_button(x, y, false);
            self.handle_actions(acts);
            self.render_cache.cursor_override = Some(smithay::input::pointer::CursorIcon::Default);
            return;
        }
        if self.mission.open {
            self.mission_button(pos, state == ButtonState::Pressed);
            if state == ButtonState::Released {
                pointer.button(self, &ButtonEvent { button, state, serial, time });
                pointer.frame(self);
            }
            return;
        }
        if state == ButtonState::Pressed && !pointer.is_grabbed() && button == BTN_RIGHT {
            let (consumed, acts) = self.shell.pointer_secondary(x, y);
            if consumed {
                self.handle_actions(acts);
                return;
            }
            let over_ui_layer = self
                .layer_under(
                    pos,
                    &[
                        smithay::wayland::shell::wlr_layer::Layer::Top,
                        smithay::wayland::shell::wlr_layer::Layer::Overlay,
                    ],
                )
                .is_some();
            if self.window_frame_under(pos).is_none() && !over_ui_layer {
                self.shell.open_desktop_menu(x, y);
                return;
            }
        }
        if state == ButtonState::Pressed && !pointer.is_grabbed() {
            if self.shell.wants_pointer(x, y) {
                let acts = self.shell.pointer_button(x, y, true);
                self.handle_actions(acts);
                return;
            }
            if button == BTN_LEFT {
                if let Some((w, edges)) = self.resize_edge_at(pos) {
                    self.focus_window(&w);
                    if let Some(loc) = self.space.element_location(&w) {
                        let size = w.geometry().size;
                        if let Some(t) = w.toplevel() {
                            t.with_pending_state(|s| {
                                s.states.set(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Resizing);
                            });
                            t.send_pending_configure();
                        }
                        let start_data = GrabStartData { focus: None, button, location: pos };
                        self.render_cache.grab_cursor = Some(crate::input::cursors::for_edges(edges));
                        let grab = crate::wm::grabs::ResizeSurfaceGrab::start(
                            start_data,
                            w.clone(),
                            edges,
                            Rectangle::new(loc, size),
                        );
                        pointer.set_grab(self, grab, serial, Focus::Clear);
                        return;
                    }
                }
            }
            if let Some(w) = self.window_frame_under(pos) {
                self.focus_window(&w);
                let Some(fr) = self.frame_rect(&w) else { return };
                let tb = Aqua::titlebar_h(&w) as f64;
                if tb > 0.0 && pos.y < fr.loc.y as f64 + tb && button == BTN_LEFT {
                    let lx = (pos.x - fr.loc.x as f64) as f32;
                    let ly = (pos.y - fr.loc.y as f64) as f32;
                    match decor::button_at(lx, ly) {
                        Some(decor::Button::Close) => self.close(&w),
                        Some(decor::Button::Minimize) => self.minimize(&w),
                        Some(decor::Button::Zoom) => self.toggle_zoom(&w),
                        None => {
                            let now = std::time::Instant::now();
                            let dbl = self
                                .render_cache
                                .last_title_click
                                .map(|t| now.duration_since(t).as_millis() < 400)
                                .unwrap_or(false);
                            self.render_cache.last_title_click = if dbl { None } else { Some(now) };
                            if dbl {
                                match self.cfg.titlebar_double_click.as_str() {
                                    "minimize" => self.minimize(&w),
                                    "none" => {}
                                    _ => self.toggle_zoom(&w),
                                }
                            } else {
                                let start_data = GrabStartData { focus: None, button, location: pos };
                                let Some(initial_window_location) = self.space.element_location(&w) else { return };
                                self.render_cache.grab_cursor = Some(smithay::input::pointer::CursorIcon::Default);
                                let grab = MoveSurfaceGrab { start_data, window: w.clone(), initial_window_location };
                                pointer.set_grab(self, grab, serial, Focus::Clear);
                            }
                        }
                    }
                    return;
                }
            } else {
                if button == BTN_LEFT && self.stage_click(pos) {
                    return;
                }
                let acts = self.shell.pointer_button(x, y, true);
                self.handle_actions(acts);
                if self
                    .layer_under(
                        pos,
                        &[
                            smithay::wayland::shell::wlr_layer::Layer::Bottom,
                            smithay::wayland::shell::wlr_layer::Layer::Background,
                        ],
                    )
                    .is_none()
                {
                    if let Some(kb) = self.seat.get_keyboard() {
                        kb.set_focus(self, None, serial);
                    }
                    self.update_activation();
                }
            }
            if let Some((crate::input::focus::PointerFocusTarget::WlSurface(s), _)) = self.surface_under(pos) {
                if let Some(l) = self.layer_for_surface(&s) {
                    use smithay::wayland::shell::wlr_layer::KeyboardInteractivity;
                    if l.cached_state().keyboard_interactivity != KeyboardInteractivity::None {
                        if let Some(kb) = self.seat.get_keyboard() {
                            kb.set_focus(self, Some(crate::input::focus::KeyboardFocusTarget::LayerSurface(l)), serial);
                        }
                    }
                }
            }
        }
        pointer.button(self, &ButtonEvent { button, state, serial, time });
        pointer.frame(self);
        if state == ButtonState::Released && !pointer.is_grabbed() && self.render_cache.grab_cursor.take().is_some() {
            let in_shell = self.shell.wants_pointer(x, y);
            let over = !in_shell && self.surface_under(pos).is_some();
            self.render_cache.cursor_override = self.cursor_override_at(pos, in_shell, false, over);
        }
    }
}

impl Aqua {
    /// Synthetic left-button event (automation/testing).
    pub fn inject_button(&mut self, pressed: bool, time: InputTime) {
        self.on_button(BTN_LEFT, if pressed { ButtonState::Pressed } else { ButtonState::Released }, time);
    }
}

impl Aqua {
    /// If `pos` is on the resize border of the topmost window under it (server-decorated,
    /// or a client-decorated Wayland window — see [`edge_resizable`]), return that window
    /// and the edges to drag.
    pub fn resize_edge_at(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(Window, crate::wm::grabs::resize_grab::ResizeEdge)> {
        use crate::wm::grabs::resize_grab::ResizeEdge;
        const OUT: f64 = 7.0;
        const IN: f64 = 3.0;
        const CORNER: f64 = 14.0;
        for w in self.space.elements().rev() {
            let Some(fr) = self.frame_rect(w) else { continue };
            let r = fr.to_f64();
            let (x0, y0, x1, y1) = (r.loc.x, r.loc.y, r.loc.x + r.size.w, r.loc.y + r.size.h);
            let inside_expanded = pos.x >= x0 - OUT && pos.x <= x1 + OUT && pos.y >= y0 - OUT && pos.y <= y1 + OUT;
            if !inside_expanded {
                continue;
            }
            if !edge_resizable(w) {
                return None;
            }
            // A menu or tooltip of this window hanging over its edge wins over the border.
            if self.popup_under(pos) {
                return None;
            }
            let mut e = ResizeEdge::empty();
            let near_l = pos.x < x0 + IN;
            let near_r = pos.x > x1 - IN;
            let near_t = pos.y < y0 + IN;
            let near_b = pos.y > y1 - IN;
            if near_l || (near_t || near_b) && pos.x < x0 + CORNER {
                e |= ResizeEdge::LEFT;
            }
            if near_r || (near_t || near_b) && pos.x > x1 - CORNER {
                e |= ResizeEdge::RIGHT;
            }
            if near_t || (near_l || near_r) && pos.y < y0 + CORNER {
                e |= ResizeEdge::TOP;
            }
            if near_b || (near_l || near_r) && pos.y > y1 - CORNER {
                e |= ResizeEdge::BOTTOM;
            }
            return if e.is_empty() { None } else { Some((w.clone(), e)) };
        }
        None
    }
}

/// Does the compositor provide the resize border for `w`?
///
/// Server-decorated windows obviously. Client-decorated Wayland toplevels too: every
/// toplevel is told it is tiled on all edges (so GTK/Chromium/Qt drop their own shadows
/// and let Aqua draw one), and a client that believes it is tiled also removes its
/// invisible resize handles — Chromium, Telegram and friends would otherwise not be
/// resizable at all. X11 clients that draw their own frame keep doing their own
/// resizing (`_NET_WM_MOVERESIZE`).
pub fn edge_resizable(w: &Window) -> bool {
    if crate::state::is_ssd(w) {
        return true;
    }
    let Some(t) = w.toplevel() else { return false };
    if meta(w).borrow().fullscreen.is_some() || std::env::var_os("AQUA_NO_TILED").is_some() {
        return false;
    }
    use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State;
    let full = t.with_committed_state(|s| s.is_some_and(|s| s.states.contains(State::Fullscreen)));
    if full {
        return false;
    }
    // Fixed-size windows (min == max) cannot be resized anyway.
    let (min, max) = smithay::wayland::compositor::with_states(t.wl_surface(), |st| {
        let c = *st.cached_state.get::<smithay::wayland::shell::xdg::SurfaceCachedState>().current();
        (c.min_size, c.max_size)
    });
    !(min.w > 0 && min.h > 0 && min == max)
}

impl Aqua {
    /// Is an xdg popup (menu, tooltip, completion list) the surface under `pos`?
    fn popup_under(&self, pos: Point<f64, Logical>) -> bool {
        use smithay::wayland::seat::WaylandFocus;
        let Some((target, _)) = self.surface_under(pos) else { return false };
        let Some(s) = target.wl_surface() else { return false };
        self.popups.find_popup(&s).is_some()
    }

    /// Is the pointer locked by a client (pointer-constraints lock)?
    fn pointer_locked(&self, pos: Point<f64, Logical>) -> bool {
        use smithay::wayland::pointer_constraints::{with_pointer_constraint, PointerConstraint};
        use smithay::wayland::seat::WaylandFocus;
        let Some(pointer) = self.seat.get_pointer() else { return false };
        let Some((focus, _)) = self.surface_under(pos) else { return false };
        let Some(surface) = focus.wl_surface() else { return false };
        let mut locked = false;
        with_pointer_constraint(&surface, &pointer, |c| {
            if let Some(c) = c {
                if c.is_active() {
                    locked = matches!(&*c, PointerConstraint::Locked(_));
                }
            }
        });
        locked
    }

    /// Volume / brightness keys with the HUD.
    pub fn media_key(&mut self, m: Media) {
        use aqua_shell::alert::HudKind;
        let snap = aqua_sys::snapshot();
        match m {
            Media::VolUp | Media::VolDown => {
                let step = if matches!(m, Media::VolUp) { 1.0 / 16.0 } else { -1.0 / 16.0 };
                let v = ((snap.audio.volume + step) * 16.0).round().clamp(0.0, 16.0) / 16.0;
                aqua_sys::audio::set_volume(v);
                if snap.audio.muted {
                    aqua_sys::audio::set_muted(false);
                }
                aqua_sys::patch(|s| {
                    s.audio.volume = v;
                    s.audio.muted = false;
                });
                self.shell.control.volume = v;
                self.shell.show_hud(HudKind::Volume, v, "");
                aqua_sys::audio::play_sound("audio-volume-change");
            }
            Media::Mute => {
                let muted = !snap.audio.muted;
                aqua_sys::audio::set_muted(muted);
                aqua_sys::patch(|s| s.audio.muted = muted);
                self.shell.show_hud(
                    if muted { HudKind::Mute } else { HudKind::Volume },
                    if muted { 0.0 } else { snap.audio.volume },
                    "",
                );
            }
            Media::MicMute => {
                let v = if snap.audio.input_volume > 0.0 { 0.0 } else { 0.7 };
                aqua_sys::audio::set_input_volume(v);
                self.shell.show_hud(HudKind::Mute, v, if v == 0.0 { "Microphone off" } else { "Microphone on" });
            }
            Media::BrightUp | Media::BrightDown => {
                let cur = snap.brightness.or_else(aqua_sys::backlight::get).unwrap_or(self.shell.control.brightness);
                let step = if matches!(m, Media::BrightUp) { 1.0 / 16.0 } else { -1.0 / 16.0 };
                let v = ((cur + step) * 16.0).round().clamp(1.0, 16.0) / 16.0;
                aqua_sys::backlight::set(v);
                aqua_sys::patch(|s| s.brightness = Some(v));
                self.shell.control.brightness = v;
                self.shell.show_hud(HudKind::Brightness, v, "");
            }
        }
        self.needs_redraw = true;
    }
}

impl Aqua {
    /// Named actions for `[[bindings]]` and hot corners. Returns false if unknown.
    pub fn run_named_action(&mut self, name: &str) -> bool {
        use aqua_shell::Action as A;
        match name {
            "" => return true,
            "lock" => self.lock_session(),
            "dark" | "dark-mode" => self.shell_actions(vec![A::SetDark(true)]),
            "light" | "light-mode" => self.shell_actions(vec![A::SetDark(false)]),
            "toggle-dark" | "toggle-appearance" => self.shell_actions(vec![A::SetDark(!self.cfg.dark)]),
            "clipboard" => self.shell.toggle_clipboard(),
            "mission" | "mission-control" => self.toggle_mission(),
            "launchpad" => self.shell.toggle_launchpad(),
            "spotlight" => self.shell.toggle_spotlight(),
            "notifications" => self.shell.toggle_notification_center(),
            "control-center" | "control" => self.shell.control.toggle(),
            "desktop" | "show-desktop" => self.show_desktop(),
            "screenshot" | "screenshot-full" => self.start_screenshot("full"),
            "screenshot-area" => self.start_screenshot("area"),
            "screenshot-window" => self.start_screenshot("window"),
            "screenshot-ui" => self.start_screenshot("ui"),
            "record" | "screen-record" => self.start_screenshot("record"),
            "record-stop" => self.stop_recording(),
            "chars" | "emoji" => self.shell_actions(vec![A::ShowChars]),
            "keyboard-viewer" | "keyboard" => self.shell_actions(vec![A::ShowKeyboardViewer]),
            "clipboard-history" => self.shell_actions(vec![A::ShowClipboard]),
            "next-layout" | "input-source" => self.next_layout(),
            "minimize" => self.shell_actions(vec![A::MinimizeFocused]),
            "close-window" => self.shell_actions(vec![A::CloseFocused]),
            "fullscreen" => self.shell_actions(vec![A::FullscreenFocused]),
            "hide-others" => self.shell_actions(vec![A::HideOthers]),
            "stage-manager" | "toggle-stage-manager" => self.shell_actions(vec![A::SetStageManager(!self.stage.on)]),
            "stage-manager-on" => self.shell_actions(vec![A::SetStageManager(true)]),
            "stage-manager-off" => self.shell_actions(vec![A::SetStageManager(false)]),
            "zoom" => {
                if let Some(w) = self.focused_window() {
                    self.toggle_zoom(&w);
                }
            }
            "sleep" => self.shell_actions(vec![A::Sleep]),
            "settings" => self.shell_actions(vec![A::OpenSettings(String::new())]),
            "terminal" => {
                let t = aqua_shell::dock::resolve_exec(&self.cfg.terminal);
                aqua_apps::launch(&t);
            }
            "volume-up" => self.media_key(Media::VolUp),
            "volume-down" => self.media_key(Media::VolDown),
            "mute" => self.media_key(Media::Mute),
            "brightness-up" => self.media_key(Media::BrightUp),
            "brightness-down" => self.media_key(Media::BrightDown),
            n if n.starts_with("tile-") || n.starts_with("arrange-") => {
                if !self.run_tile_action(n) {
                    return false;
                }
            }
            _ => return false,
        }
        self.needs_redraw = true;
        true
    }

    /// Hot corners (Desktop & Dock → Hot Corners).
    fn hot_corner(&mut self, pos: Point<f64, Logical>) {
        let Some(g) = self.output.as_ref().and_then(|o| self.space.output_geometry(o)) else { return };
        let g = g.to_f64();
        let (l, r, t, b) = (
            pos.x <= g.loc.x + 1.0,
            pos.x >= g.loc.x + g.size.w - 2.0,
            pos.y <= g.loc.y + 1.0,
            pos.y >= g.loc.y + g.size.h - 2.0,
        );
        let corner = match (l, r, t, b) {
            (true, _, true, _) => Some(0),
            (_, true, true, _) => Some(1),
            (true, _, _, true) => Some(2),
            (_, true, _, true) => Some(3),
            _ => None,
        };
        if corner == self.render_cache.hot_corner {
            return;
        }
        self.render_cache.hot_corner = corner;
        if let Some(c) = corner {
            let act = self.cfg.hot_corners[c].clone();
            if !act.is_empty() && !self.lock.is_locked() {
                self.run_named_action(&act);
            }
        }
    }
}
