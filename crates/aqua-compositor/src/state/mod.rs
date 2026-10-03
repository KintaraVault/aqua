//! Compositor state: Smithay protocol states + the Aqua shell.
use std::cell::RefCell;
use std::ffi::OsString;
use std::sync::Arc;
use std::time::{Duration, Instant};

use aqua_config::{metrics, Config};
use aqua_gfx::Pixmap;
use aqua_shell::{Shell, WindowInfo};
use smithay::{
    desktop::{PopupManager, Space, Window, WindowSurfaceType},
    input::{Seat, SeatState},
    output::Output,
    reexports::{
        calloop::{generic::Generic, EventLoop, Interest, LoopHandle, LoopSignal, Mode, PostAction},
        wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecoMode,
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
            Display, DisplayHandle,
        },
    },
    utils::{Logical, Point, Rectangle},
    wayland::{
        compositor::{with_states, CompositorClientState, CompositorState},
        output::OutputManagerState,
        selection::{data_device::DataDeviceState, primary_selection::PrimarySelectionState},
        shell::xdg::{decoration::XdgDecorationState, XdgShellState, XdgToplevelSurfaceData},
        shm::ShmState,
        socket::ListeningSocketSource,
        viewporter::ViewporterState,
        xdg_activation::XdgActivationState,
    },
};

/// Per-window bookkeeping stored in the window's user data.
#[derive(Debug, Default)]
pub struct WinMeta {
    pub id: u64,
    pub placed: bool,
    pub hover_lights: bool,
    /// Geometry before "zoom" (maximise) so it can be restored.
    pub saved: Option<Rectangle<i32, Logical>>,
    /// Open animation progress 0..1 (kept for compatibility).
    pub open_anim: f32,
    /// When the window was first shown (drives the open animation).
    pub mapped_at: Option<std::time::Instant>,
    /// Minimise (false) / restore (true) animation start.
    pub minimizing: Option<(std::time::Instant, bool)>,
    /// Where the window was when it got minimised (restored there).
    pub min_loc: Option<smithay::utils::Point<i32, Logical>>,
    /// Dock icon the window shrinks into (logical).
    pub anim_target: Option<(f32, f32)>,
    /// Desktop (Space) index the window lives on.
    pub desk: usize,
    /// Display (output name) whose Spaces the window belongs to; "" = primary.
    pub display: String,
    /// X11 override-redirect window (menus, tooltips): no decorations, never focused.
    pub override_redirect: bool,
    /// Fullscreen: geometry to restore.
    pub fullscreen: Option<Rectangle<i32, Logical>>,
    /// xdg-dialog modal hint.
    pub modal: bool,
    /// The window grows out of this app icon (Dock / Launchpad / Spotlight launch).
    pub launch_from: Option<aqua_gfx::Rect>,
    /// Zoom / full-screen transition: start, frame before, frame after (logical).
    pub geo_anim: Option<(std::time::Instant, Rectangle<f64, Logical>, Rectangle<f64, Logical>)>,
}

static REDUCE_MOTION: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Accessibility → Reduce motion: animations finish (almost) instantly.
pub fn set_reduce_motion(on: bool) {
    REDUCE_MOTION.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn reduce_motion() -> bool {
    REDUCE_MOTION.load(std::sync::atomic::Ordering::Relaxed)
}

/// Animation time multiplier: `AQUA_ANIM_SLOW=N` slows everything down N times
/// (debugging / screenshots); Reduce motion makes transitions near-instant.
pub fn anim_slow() -> f32 {
    let s = aqua_config::anim_slow();
    if reduce_motion() {
        s * 0.02
    } else {
        s
    }
}

pub const GEO_ANIM_MS: f32 = 340.0;

/// Zoom / full-screen transition: current interpolated frame and the frame the
/// window is heading to, or None when idle.
pub fn geo_anim(w: &Window) -> Option<(Rectangle<f64, Logical>, f32)> {
    let m = meta(w).borrow();
    let (t0, from, to) = m.geo_anim?;
    let x = (t0.elapsed().as_secs_f32() * 1000.0 / (GEO_ANIM_MS * anim_slow())).min(1.0);
    if x >= 1.0 {
        return None;
    }
    let e = (1.0 - (1.0 - x).powi(4)) as f64;
    let lerp = |a: f64, b: f64| a + (b - a) * e;
    let r = Rectangle::new(
        (lerp(from.loc.x, to.loc.x), lerp(from.loc.y, to.loc.y)).into(),
        (lerp(from.size.w, to.size.w), lerp(from.size.h, to.size.h)).into(),
    );
    Some((r, x))
}

/// Eased minimise progress (0 = normal, 1 = in the dock) and target.
pub fn minimize_progress(w: &Window) -> (f32, Option<(f32, f32)>) {
    let m = meta(w).borrow();
    match m.minimizing {
        Some((t, restore)) => {
            let x = (t.elapsed().as_secs_f32() * 1000.0 / (crate::render::MINIMIZE_ANIM_MS * anim_slow())).min(1.0);
            let e = if x < 0.5 { 4.0 * x * x * x } else { 1.0 - (-2.0 * x + 2.0).powi(3) / 2.0 };
            (if restore { 1.0 - e } else { e }, m.anim_target)
        }
        None => (0.0, None),
    }
}

/// Start time for a minimise/restore animation that reverses the running one
/// smoothly (the ease is point-symmetric, so x' = 1 - x keeps the progress).
pub fn reversed_anim_start(m: &WinMeta) -> std::time::Instant {
    let now = std::time::Instant::now();
    let total = crate::render::MINIMIZE_ANIM_MS * anim_slow() / 1000.0;
    match m.minimizing {
        Some((t, _)) => {
            let x = (t.elapsed().as_secs_f32() / total).clamp(0.0, 1.0);
            now.checked_sub(Duration::from_secs_f32((1.0 - x) * total)).unwrap_or(now)
        }
        None => now,
    }
}

pub fn minimize_finished(w: &Window) -> Option<bool> {
    let m = meta(w).borrow();
    let (t, restore) = m.minimizing?;
    (t.elapsed().as_secs_f32() * 1000.0 >= crate::render::MINIMIZE_ANIM_MS * anim_slow()).then_some(restore)
}

pub const OPEN_ANIM_MS: f32 = 260.0;
/// Opening out of a launcher icon takes a little longer.
pub const LAUNCH_ANIM_MS: f32 = 440.0;
pub const DARK_ANIM_MS: f32 = 650.0;

/// Ease-out progress of the open animation, 1.0 when finished.
pub fn open_progress(w: &Window) -> f32 {
    match meta(w).borrow().mapped_at {
        Some(t) => {
            let ms = if meta(w).borrow().launch_from.is_some() { LAUNCH_ANIM_MS } else { OPEN_ANIM_MS };
            (t.elapsed().as_secs_f32() * 1000.0 / (ms * anim_slow())).min(1.0)
        }
        None => 1.0,
    }
}

impl Aqua {
    /// 0 = light wallpaper, 1 = night wallpaper.
    pub fn dark_progress(&self) -> f32 {
        match self.dark_anim {
            None => 0.0,
            Some((t0, d)) => {
                let p = (t0.elapsed().as_secs_f32() * 1000.0 / (DARK_ANIM_MS * anim_slow())).clamp(0.0, 1.0);
                let e = p * p * (3.0 - 2.0 * p);
                if d {
                    e
                } else {
                    1.0 - e
                }
            }
        }
    }

    pub fn windows_animating(&self) -> bool {
        self.mission.animating()
            || self.spaces.has_anim()
            || self
                .dark_anim
                .map(|(t0, _)| t0.elapsed().as_secs_f32() * 1000.0 < DARK_ANIM_MS * anim_slow())
                .unwrap_or(false)
            || self
                .space
                .elements()
                .any(|w| open_progress(w) < 1.0 || meta(w).borrow().minimizing.is_some() || geo_anim(w).is_some())
            || !self.render_cache.ghosts.is_empty()
    }

    /// Move notifications from the D-Bus thread into the shell.
    pub fn poll_notifications(&mut self) -> bool {
        for (id, action) in std::mem::take(&mut self.shell.notes.clicked).into_iter().filter(|(id, _)| *id != 0) {
            match action {
                Some(a) => aqua_notify::invoke(id, &a),
                None => aqua_notify::closed(id, aqua_notify::CloseReason::Dismissed),
            }
        }
        for id in std::mem::take(&mut self.shell.notes.expired).into_iter().filter(|id| *id != 0) {
            aqua_notify::closed(id, aqua_notify::CloseReason::Expired);
        }
        for (id, text) in std::mem::take(&mut self.shell.notes.replied) {
            aqua_notify::replied(id, &text);
        }
        let Some(rx) = &self.notify_rx else { return false };
        let evs: Vec<aqua_notify::Event> = rx.try_iter().collect();
        let any = !evs.is_empty();
        for e in evs {
            match e {
                aqua_notify::Event::Show(n) => {
                    let n = *n;
                    let app_id = n.desktop_entry.clone().unwrap_or_else(|| n.app_name.to_lowercase().replace(' ', "-"));
                    let app_name =
                        if n.app_name.is_empty() { String::new() } else { self.shell.app_display_name(&app_id) };
                    self.shell.notify(aqua_shell::notifications::Note {
                        id: n.id,
                        app_id,
                        app_name: if app_name.is_empty() { n.app_name } else { app_name },
                        icon: n.app_icon,
                        summary: n.summary,
                        body: n.body,
                        time: (0, 0),
                        timeout: if n.expire_timeout > 0 { n.expire_timeout as f32 / 1000.0 } else { 0.0 },
                        persistent: n.expire_timeout == 0,
                        action: n.default_action,
                        actions: n.actions,
                        reply: n.reply,
                        resident: n.resident,
                        critical: n.critical,
                        expired: false,
                    });
                }
                aqua_notify::Event::Close(id) => self.shell.notes.close(id),
            }
        }
        any
    }

    /// Complete finished minimise / restore animations.
    pub fn finish_window_anims(&mut self) {
        let done: Vec<(Window, bool)> =
            self.space.elements().filter_map(|w| minimize_finished(w).map(|r| (w.clone(), r))).collect();
        for (w, restore) in done {
            meta(&w).borrow_mut().minimizing = None;
            if !restore {
                use smithay::desktop::space::SpaceElement;
                let outs = self.space.outputs_for_element(&w);
                self.space.unmap_elem(&w);
                let size = w.geometry().size;
                for o in &outs {
                    w.output_enter(o, Rectangle::new((0, 0).into(), size));
                }
                self.minimized.push(w);
            }
            self.needs_redraw = true;
        }
    }
}

pub fn meta(w: &Window) -> &RefCell<WinMeta> {
    w.user_data().insert_if_missing(|| RefCell::new(WinMeta::default()));
    w.user_data().get::<RefCell<WinMeta>>().unwrap()
}

/// Is the window using server side (Aqua) decorations?
pub fn is_ssd(w: &Window) -> bool {
    if let Some(x) = w.x11_surface() {
        return !x.is_decorated()
            && !crate::wayland::xwayland::is_popup(x)
            && !x.is_fullscreen()
            && meta(w).borrow().fullscreen.is_none();
    }
    if meta(w).borrow().fullscreen.is_some() {
        return false;
    }
    w.toplevel()
        .map(|t| t.with_committed_state(|s| s.and_then(|s| s.decoration_mode) == Some(DecoMode::ServerSide)))
        .unwrap_or(false)
}

pub fn title_of(w: &Window) -> (String, String) {
    if let Some(x) = w.x11_surface() {
        let class = x.class();
        let id = if class.is_empty() { x.instance() } else { class };
        return (id, x.title());
    }
    let Some(t) = w.toplevel() else { return Default::default() };
    with_states(t.wl_surface(), |s| {
        s.data_map
            .get::<XdgToplevelSurfaceData>()
            .map(|d| {
                let d = d.lock().unwrap();
                (d.app_id.clone().unwrap_or_default(), d.title.clone().unwrap_or_default())
            })
            .unwrap_or_default()
    })
}

pub struct Aqua {
    pub start_time: std::time::Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,
    pub loop_handle: LoopHandle<'static, Aqua>,
    pub loop_signal: LoopSignal,

    pub space: Space<Window>,
    pub minimized: Vec<Window>,
    pub popups: PopupManager,
    pub output: Option<Output>,

    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub xdg_decoration_state: XdgDecorationState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<Aqua>,
    pub data_device_state: DataDeviceState,
    pub primary_selection_state: PrimarySelectionState,
    pub viewporter_state: ViewporterState,
    pub xdg_activation_state: XdgActivationState,
    pub seat: Seat<Self>,
    pub p: crate::wayland::protocols::Protocols,
    pub xwm: Option<smithay::xwayland::X11Wm>,
    pub xdisplay: Option<u32>,
    pub xwayland_failures: u32,
    pub lock: crate::system::lock::Lock,
    pub idle: crate::system::idle::Idle,
    pub outputs: crate::wm::outputs::Outputs,
    pub input_cfg: crate::input::config::InputCfg,
    pub cfg_watch: Option<std::time::SystemTime>,
    pub pending_captures: Vec<(smithay::wayland::image_copy_capture::Frame, smithay::output::Output, bool)>,
    pub gesture: crate::input::gestures::GestureState,
    pub sys_rx: Option<std::sync::mpsc::Receiver<crate::system::logind::SysEvent>>,
    pub clip: crate::selection::clipboard::History,

    pub cfg: Config,
    pub shell: Shell,
    pub wallpaper: Arc<Pixmap>,
    pub wallpaper_serial: u64,
    /// Dark Mode cross-fade: (start, target dark?).
    pub dark_anim: Option<(Instant, bool)>,
    pub mission: crate::wm::mission::Mission,
    pub spaces: crate::wm::spaces::Spaces,
    /// Window home locations captured at the start of a desktop slide.
    pub spaces_homes: Vec<(Window, smithay::utils::Point<i32, Logical>)>,
    /// Per-window (current frame, thumbnail frame) while Mission Control is visible.
    /// Borderless window that started covering the whole output, and since when: it only
    /// counts as full screen once that has held for a moment (see `front_is_fullscreen`).
    pub fs_guess: Option<(u64, std::time::Instant)>,
    pub mission_targets: std::collections::HashMap<u64, (Rectangle<f64, Logical>, Rectangle<f64, Logical>)>,
    pub scale: f64,
    pub next_window_id: u64,
    pub needs_redraw: bool,
    pub pointer_in_shell: bool,
    /// Windows appeared/disappeared: re-pick the pointer focus after the next frame.
    pub repick_pointer: bool,
    /// Last reported config problems (to notify only about new ones).
    pub config_issues: Vec<String>,
    pub screenshot_request: Option<String>,
    pub screenshots: crate::capture::screenshot::Shots,
    /// App launched from a shell icon: when, and the icon rectangle (logical).
    pub pending_launch: Option<(std::time::Instant, aqua_gfx::Rect)>,
    pub portal_shots: Vec<crate::portal::PendingShot>,
    /// Running ScreenCast portal streams.
    pub casts: Vec<crate::capture::screencast::ActiveCast>,
    pub render_cache: crate::render::RenderCache,
    pub notify_rx: Option<std::sync::mpsc::Receiver<aqua_notify::Event>>,
    pub appearance: aqua_notify::portal::Appearance,
    pub dmabuf_state: smithay::wayland::dmabuf::DmabufState,
    /// Native backend data (None when nested in another session).
    pub udev: Option<crate::backend::udev::UdevData>,
    /// Draw the pointer ourselves (native backend).
    pub draw_cursor: bool,
}

impl Aqua {
    pub fn new(
        event_loop: &mut EventLoop<'static, Self>,
        display: Display<Self>,
        cfg: Config,
        w: f32,
        h: f32,
        scale: f64,
    ) -> Self {
        let dh = display.handle();
        let cfg_dark = cfg.dark;
        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let xdg_decoration_state = XdgDecorationState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let primary_selection_state = PrimarySelectionState::new::<Self>(&dh);
        let viewporter_state = ViewporterState::new::<Self>(&dh);
        let xdg_activation_state = XdgActivationState::new::<Self>(&dh);
        let mut seat_state = SeatState::new();
        let mut seat: Seat<Self> = seat_state.new_wl_seat(&dh, "seat0");
        let per_output = cfg.spaces_per_output;
        let input_cfg = crate::input::config::InputCfg::from_config(&cfg);
        if seat.add_keyboard(input_cfg.xkb(), input_cfg.repeat_delay, input_cfg.repeat_rate).is_err() {
            tracing::warn!("invalid XKB configuration {:?}, falling back to defaults", input_cfg.layouts);
            seat.add_keyboard(Default::default(), 300, 30).expect("default keymap");
        }
        seat.add_pointer();
        seat.add_touch();
        let p = crate::wayland::protocols::Protocols::new(&dh, &event_loop.handle(), &primary_selection_state);
        crate::wayland::blur::init(&dh);

        let socket_name = Self::init_wayland_listener(display, event_loop);
        let pw = (w as f64 * scale).round() as u32;
        let ph = (h as f64 * scale).round() as u32;
        let wallpaper = aqua_wallpaper::wallpaper(cfg.wallpaper.as_deref(), pw, ph);
        let shell = Shell::new(cfg.clone(), w, h, scale as f32, &wallpaper);
        set_reduce_motion(cfg.reduce_motion);
        aqua_render::set_blur_max_fps(cfg.blur_max_fps);
        let dnd = cfg.do_not_disturb;

        let mut st = Self {
            start_time: std::time::Instant::now(),
            socket_name,
            display_handle: dh,
            loop_handle: event_loop.handle(),
            loop_signal: event_loop.get_signal(),
            space: Space::default(),
            minimized: vec![],
            popups: PopupManager::default(),
            output: None,
            compositor_state,
            xdg_shell_state,
            xdg_decoration_state,
            shm_state,
            output_manager_state,
            seat_state,
            data_device_state,
            primary_selection_state,
            viewporter_state,
            xdg_activation_state,
            seat,
            p,
            xwm: None,
            xdisplay: None,
            xwayland_failures: 0,
            lock: crate::system::lock::Lock::new(cfg.lock_on_start),
            idle: crate::system::idle::Idle::new(&cfg),
            outputs: Default::default(),
            input_cfg,
            cfg_watch: aqua_config::Config::mtime(),
            pending_captures: vec![],
            gesture: Default::default(),
            sys_rx: crate::system::logind::spawn(),
            clip: Default::default(),
            cfg,
            shell,
            wallpaper: Arc::new(wallpaper),
            wallpaper_serial: 1,
            mission: Default::default(),
            fs_guess: None,
            mission_targets: Default::default(),
            spaces: aqua_wm::Workspaces::new(per_output),
            spaces_homes: vec![],
            dark_anim: if cfg_dark { Some((Instant::now() - Duration::from_secs(5), true)) } else { None },
            scale,
            next_window_id: 1,
            needs_redraw: true,
            pointer_in_shell: false,
            repick_pointer: false,
            config_issues: vec![],
            screenshot_request: None,
            screenshots: Default::default(),
            pending_launch: None,
            portal_shots: Vec::new(),
            casts: Vec::new(),
            render_cache: Default::default(),
            notify_rx: Some(aqua_notify::spawn()),
            appearance: aqua_notify::portal::spawn(cfg_dark),
            dmabuf_state: smithay::wayland::dmabuf::DmabufState::new(),
            udev: None,
            draw_cursor: false,
        };
        st.shell.notes.dnd = dnd;
        st.shell.control.focus = dnd;
        st
    }

    fn init_wayland_listener(display: Display<Aqua>, event_loop: &mut EventLoop<Self>) -> OsString {
        let listening_socket = ListeningSocketSource::new_auto().unwrap();
        let socket_name = listening_socket.socket_name().to_os_string();
        let lh = event_loop.handle();
        lh.insert_source(listening_socket, move |client_stream, _, state| {
            state
                .display_handle
                .insert_client(client_stream, Arc::new(ClientState::default()))
                .map_err(|e| tracing::warn!("failed to insert client: {e}"))
                .ok();
        })
        .expect("Failed to init the wayland event source.");
        lh.insert_source(Generic::new(display, Interest::READ, Mode::Level), |_, display, state| {
            unsafe {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    display.get_mut().dispatch_clients(state)
                }));
                match r {
                    Ok(Err(e)) => tracing::error!("dispatch_clients: {e}"),
                    Err(_) => tracing::error!("panic while handling a client request (recovered)"),
                    _ => {}
                }
            }
            Ok(PostAction::Continue)
        })
        .unwrap();
        socket_name
    }

    pub fn request_redraw(&mut self) {
        self.needs_redraw = true;
    }

    /// Logical output size.
    pub fn output_size(&self) -> (i32, i32) {
        self.output
            .as_ref()
            .and_then(|o| self.space.output_geometry(o))
            .map(|g| (g.size.w, g.size.h))
            .unwrap_or((1440, 900))
    }

    /// Minimum y for a window's geometry origin (keeps the titlebar below the menu bar).
    pub fn top_inset(&self, w: &Window) -> i32 {
        let mb = self.cfg.menubar_height.round() as i32;
        if is_ssd(w) {
            mb + metrics::TITLEBAR_HEIGHT as i32
        } else {
            mb
        }
    }

    pub fn titlebar_h(w: &Window) -> i32 {
        if is_ssd(w) {
            metrics::TITLEBAR_HEIGHT as i32
        } else {
            0
        }
    }

    /// Icon the next new top-level window should grow out of (launched < 10 s ago).
    pub fn take_launch_origin(&mut self, w: &Window) -> Option<aqua_gfx::Rect> {
        let transient = w.toplevel().map(|t| t.parent().is_some()).unwrap_or(false)
            || w.x11_surface().map(|x| x.is_transient_for().is_some()).unwrap_or(false);
        if transient || !self.cfg.animate_windows || crate::state::reduce_motion() {
            return None;
        }
        let (t, r) = self.pending_launch?;
        if t.elapsed() > std::time::Duration::from_secs(10) {
            self.pending_launch = None;
            return None;
        }
        self.pending_launch = None;
        Some(r)
    }

    /// Full visual frame (incl. titlebar) of a mapped window in logical coords.
    pub fn frame_rect(&self, w: &Window) -> Option<Rectangle<i32, Logical>> {
        let loc = self.space.element_location(w)?;
        let geo = w.geometry();
        let tb = Self::titlebar_h(w);
        Some(Rectangle::new((loc.x, loc.y - tb).into(), (geo.size.w, geo.size.h + tb).into()))
    }

    pub fn window_for_surface(&self, s: &WlSurface) -> Option<Window> {
        use smithay::wayland::seat::WaylandFocus;
        self.space.elements().chain(self.minimized.iter()).find(|w| w.wl_surface().as_deref() == Some(s)).cloned()
    }

    pub fn window_for_x11(&self, x: &smithay::xwayland::X11Surface) -> Option<Window> {
        self.space.elements().chain(self.minimized.iter()).find(|w| w.x11_surface() == Some(x)).cloned()
    }

    pub fn focused_window(&self) -> Option<Window> {
        let kb = self.seat.get_keyboard()?;
        match kb.current_focus()? {
            crate::input::focus::KeyboardFocusTarget::Window(w) => {
                (self.space.elements().any(|e| *e == w) || self.minimized.contains(&w)).then_some(w)
            }
            _ => None,
        }
    }

    /// Topmost window whose frame (incl. titlebar) contains `pos`.
    pub fn window_frame_under(&self, pos: Point<f64, Logical>) -> Option<Window> {
        self.space
            .elements()
            .rev()
            .find(|w| self.frame_rect(w).map(|r| r.to_f64().contains(pos)).unwrap_or(false))
            .cloned()
    }

    pub fn surface_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(crate::input::focus::PointerFocusTarget, Point<f64, Logical>)> {
        use crate::input::focus::PointerFocusTarget;
        use smithay::wayland::shell::wlr_layer::Layer as WlrLayer;
        if self.lock.is_locked() {
            return self.lock_surface_under(pos);
        }
        if let Some(r) = self.layer_under(pos, &[WlrLayer::Overlay, WlrLayer::Top]) {
            return Some(r);
        }
        let win = self.space.elements().rev().find_map(|window| {
            if !self.on_current_space(window) && !meta(window).borrow().override_redirect {
                return None;
            }
            if window.x11_surface().is_some_and(crate::wayland::xwayland::input_transparent) {
                return None;
            }
            if !self.space.element_bbox(window)?.to_f64().contains(pos) {
                return None;
            }
            let location = self.space.element_location(window)? - window.geometry().loc;
            window.surface_under(pos - location.to_f64(), WindowSurfaceType::ALL).map(|(s, p)| {
                let target = match window.x11_surface() {
                    Some(x) if x.wl_surface().as_ref() == Some(&s) => PointerFocusTarget::X11Surface(x.clone()),
                    _ => PointerFocusTarget::WlSurface(s),
                };
                (target, (p + location).to_f64())
            })
        });
        win.or_else(|| self.layer_under(pos, &[WlrLayer::Bottom, WlrLayer::Background]))
    }

    /// Push the current window list into the shell (dock running dots, menu bar app name).
    pub fn sync_shell_windows(&mut self) {
        let focused = self.focused_window();
        let mut wins = Vec::new();
        for (w, minimized) in self
            .space
            .elements()
            .filter(|w| !meta(w).borrow().override_redirect)
            .map(|w| (w, matches!(meta(w).borrow().minimizing, Some((_, false)))))
            .chain(self.minimized.iter().map(|w| (w, true)))
        {
            let (app_id, title) = title_of(w);
            let id = meta(w).borrow().id;
            wins.push(WindowInfo { id, app_id, title, focused: Some(w) == focused.as_ref(), minimized });
        }
        if wins != self.shell.windows {
            self.shell.set_windows(wins);
            self.needs_redraw = true;
        }
    }
}

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
    /// Connected through a wp-security-context listener (Flatpak/sandbox): no privileged protocols.
    pub sandboxed: bool,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}
