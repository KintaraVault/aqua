//! Client side of `aqua_glass_v1` (protocols/aqua-glass-v1.xml): Aqua's own apps hand the
//! compositor the shapes of their glass — sidebar islands, toolbar capsules, menu windows —
//! and Aqua draws them with its one Liquid Glass material, the same as the menu bar, Dock and
//! Control Center.
//!
//! Slint side: `AquaGlass` (common.slint). Every `GlassShape` reports its window-relative
//! geometry when `epoch` changes and calls `dirty()` when it moves; [`link`] bumps the epoch,
//! collects the reports and sends them when they changed. Without the protocol (another
//! compositor, an old Aqua) the window falls back to the KDE blur request as before.
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;
use wayland_client::{
    backend::{Backend, ObjectId},
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_registry::WlRegistry, wl_surface::WlSurface},
    Connection, Dispatch, EventQueue, Proxy, QueueHandle,
};

#[allow(non_upper_case_globals, non_camel_case_types, unused_imports, missing_docs, dead_code, clippy::all)]
mod proto {
    use wayland_client;
    use wayland_client::protocol::*;
    pub mod __interfaces {
        use wayland_client::backend as wayland_backend;
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("../../protocols/aqua-glass-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("../../protocols/aqua-glass-v1.xml");
}
use proto::{aqua_glass_manager_v1::AquaGlassManagerV1, aqua_glass_v1::AquaGlassV1};

/// Wire material ids (see `aqua_config::material::wire`).
pub mod material {
    pub const SIDEBAR: i32 = 1;
    pub const CONTROL: i32 = 2;
    pub const MENU: i32 = 3;
    pub const PANEL: i32 = 4;
}

struct St;

impl Dispatch<WlRegistry, GlobalListContents> for St {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<AquaGlassManagerV1, ()> for St {
    fn event(
        _: &mut Self,
        _: &AquaGlassManagerV1,
        _: <AquaGlassManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<AquaGlassV1, ()> for St {
    fn event(
        _: &mut Self,
        _: &AquaGlassV1,
        _: <AquaGlassV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// The app's connection (winit's display, our own queue) and the bound manager.
struct Shared {
    conn: Connection,
    queue: RefCell<EventQueue<St>>,
    mgr: AquaGlassManagerV1,
}

thread_local! {
    /// `None` until the first window asked; `Some(None)`: the compositor has no aqua_glass_v1.
    static SHARED: RefCell<Option<Option<Rc<Shared>>>> = const { RefCell::new(None) };
}

fn shared(display: *mut std::ffi::c_void) -> Option<Rc<Shared>> {
    SHARED.with(|s| {
        s.borrow_mut()
            .get_or_insert_with(|| {
                // SAFETY: the pointer is winit's live wl_display, which outlives the app's windows.
                let backend = unsafe { Backend::from_foreign_display(display.cast()) };
                let conn = Connection::from_backend(backend);
                let (globals, queue) = registry_queue_init::<St>(&conn).ok()?;
                let mgr = globals.bind::<AquaGlassManagerV1, _, _>(&queue.handle(), 1..=1, ()).ok()?;
                Some(Rc::new(Shared { conn, queue: RefCell::new(queue), mgr }))
            })
            .clone()
    })
}

/// One shape as sent: x, y, w, h, radius (24.8 fixed), material.
type Rec = [i32; 6];

fn fixed(l: f32) -> i32 {
    (l * 256.0).round() as i32
}

struct Link {
    win: Box<dyn Fn() -> Option<Box<dyn WinAccess>>>,
    sh: Rc<Shared>,
    /// Glass object and the surface it belongs to (menus get a new surface when re-shown).
    obj: RefCell<Option<(ObjectId, AquaGlassV1)>>,
    pending: RefCell<Vec<Rec>>,
    sent: RefCell<Option<Vec<Rec>>>,
    collect_due: Cell<bool>,
    timers: RefCell<Vec<slint::Timer>>,
    /// Whole-window KDE blur state (follows `window_glass` for main windows).
    body_blur: Cell<Option<bool>>,
}

/// Object-safe access to the component's window and globals.
trait WinAccess {
    fn win(&self) -> &slint::Window;
    fn on_report(&self, f: Box<dyn Fn(f32, f32, f32, f32, f32, i32)>);
    fn on_dirty(&self, f: Box<dyn Fn()>);
    fn bump_epoch(&self);
    fn set_active(&self, on: bool);
    fn window_glass(&self) -> bool;
}

impl Drop for Link {
    fn drop(&mut self) {
        if let Some((_, o)) = self.obj.borrow_mut().take() {
            o.destroy();
            let _ = self.sh.conn.flush();
        }
    }
}

impl Link {
    /// Bump the epoch (the `GlassShape`s report in this event-loop turn), send shortly after.
    fn collect(self: &Rc<Self>) {
        if self.collect_due.replace(true) {
            return;
        }
        let me = Rc::downgrade(self);
        slint::Timer::single_shot(Duration::from_millis(12), move || {
            let Some(me) = me.upgrade() else { return };
            let Some(w) = (me.win)() else { return };
            me.collect_due.set(false);
            me.pending.borrow_mut().clear();
            w.bump_epoch();
            let me2 = Rc::downgrade(&me);
            slint::Timer::single_shot(Duration::from_millis(4), move || {
                if let Some(me) = me2.upgrade() {
                    me.commit();
                }
            });
        });
    }

    fn surface(&self) -> Option<WlSurface> {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use slint::winit_030::WinitWindowAccessor;
        let w = (self.win)()?;
        let ptr = w.win().with_winit_window(|win| match win.window_handle().ok()?.as_raw() {
            RawWindowHandle::Wayland(h) => Some(h.surface.as_ptr()),
            _ => None,
        })??;
        // SAFETY: winit's live wl_surface proxy of this window.
        let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), ptr.cast()) }.ok()?;
        WlSurface::from_id(&self.sh.conn, id).ok()
    }

    fn commit(&self) {
        let Some(surface) = self.surface() else { return };
        let shapes = self.pending.borrow().clone();
        let mut obj = self.obj.borrow_mut();
        if obj.as_ref().is_none_or(|(id, o)| *id != surface.id() || !o.is_alive()) {
            if let Some((_, o)) = obj.take() {
                o.destroy();
            }
            let qh = self.sh.queue.borrow().handle();
            *obj = Some((surface.id(), self.sh.mgr.get_glass(&surface, &qh, ())));
            *self.sent.borrow_mut() = None;
        }
        if self.sent.borrow().as_ref() == Some(&shapes) {
            return;
        }
        tracing::debug!("aqua_glass_v1: {} shape(s)", shapes.len());
        let bytes: Vec<u8> = shapes.iter().flat_map(|r| r.iter().flat_map(|v| v.to_le_bytes())).collect();
        if let Some((_, o)) = obj.as_ref() {
            o.set_shapes(bytes);
        }
        *self.sent.borrow_mut() = Some(shapes);
        let _ = self.sh.queue.borrow_mut().dispatch_pending(&mut St);
        let _ = self.sh.conn.flush();
    }

    fn sync_body_blur(&self, body: bool) {
        use slint::winit_030::WinitWindowAccessor;
        let Some(w) = (self.win)() else { return };
        let want = body && w.window_glass();
        if self.body_blur.get() != Some(want) && w.win().with_winit_window(|win| win.set_blur(want)).is_some() {
            self.body_blur.set(Some(want));
        }
    }
}

macro_rules! access {
    ($c:ty) => {
        impl WinAccess for $c {
            fn win(&self) -> &slint::Window {
                ComponentHandle::window(self)
            }
            fn on_report(&self, f: Box<dyn Fn(f32, f32, f32, f32, f32, i32)>) {
                self.global::<crate::AquaGlass>().on_report(move |x, y, w, h, r, m| f(x, y, w, h, r, m));
            }
            fn on_dirty(&self, f: Box<dyn Fn()>) {
                self.global::<crate::AquaGlass>().on_dirty(move || f());
            }
            fn bump_epoch(&self) {
                let g = self.global::<crate::AquaGlass>();
                g.set_epoch(g.get_epoch().wrapping_add(1));
            }
            fn set_active(&self, on: bool) {
                self.global::<crate::AquaGlass>().set_active(on);
            }
            fn window_glass(&self) -> bool {
                self.global::<crate::Theme>().get_window_glass()
            }
        }
    };
}
access!(crate::FinderWindow);
access!(crate::FinderMenuWindow);
access!(crate::SettingsWindow);
access!(crate::StoreWindow);

/// Give window `ui` Aqua's Liquid Glass: its `GlassShape`s become compositor glass. `body`:
/// a main window, whose whole body is glass (KDE blur) when `window_glass` is on; menu
/// windows pass false. Falls back to whole-window KDE blur without `aqua_glass_v1`.
/// Lives as long as the component.
pub fn link<C>(ui: &C, body: bool)
where
    C: ComponentHandle + WinAccessPub + 'static,
{
    if !crate::glass_supported() {
        return;
    }
    attempt(ui.as_weak(), body, 0);
}

/// Components [`link`] works with.
#[allow(private_bounds)]
pub trait WinAccessPub: WinAccess {}
impl<T: WinAccess> WinAccessPub for T {}

fn attempt<C: ComponentHandle + WinAccessPub + 'static>(weak: slint::Weak<C>, body: bool, tries: u32) {
    use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
    use slint::winit_030::WinitWindowAccessor;
    let Some(ui) = weak.upgrade() else { return };
    let display = ui.win().with_winit_window(|win| match win.display_handle().ok()?.as_raw() {
        RawDisplayHandle::Wayland(d) => Some(d.display.as_ptr()),
        _ => None,
    });
    let Some(display) = display else {
        // No winit window yet (not shown): try again shortly.
        if tries < 60 {
            slint::Timer::single_shot(Duration::from_millis(50), move || attempt(weak, body, tries + 1));
        }
        return;
    };
    let Some(sh) = display.and_then(shared) else {
        // Not Aqua's glass protocol: whole-window KDE blur, as before.
        ui.win().with_winit_window(|win| win.set_blur(true));
        return;
    };
    let w2 = weak.clone();
    let win: Box<dyn Fn() -> Option<Box<dyn WinAccess>>> =
        Box::new(move || w2.upgrade().map(|c| Box::new(c) as Box<dyn WinAccess>));
    let link = Rc::new(Link {
        win,
        sh,
        obj: RefCell::new(None),
        pending: RefCell::new(vec![]),
        sent: RefCell::new(None),
        collect_due: Cell::new(false),
        timers: RefCell::new(vec![]),
        body_blur: Cell::new(None),
    });
    {
        let l = link.clone();
        ui.on_report(Box::new(move |x, y, w, h, r, m| {
            let mut p = l.pending.borrow_mut();
            if p.len() < 32 {
                p.push([fixed(x), fixed(y), fixed(w), fixed(h), fixed(r), m]);
            }
        }));
    }
    {
        let l = link.clone();
        ui.on_dirty(Box::new(move || l.collect()));
    }
    ui.set_active(true);
    link.sync_body_blur(body);
    // Safety net: conditional elements vanish without a `dirty()`; the config may toggle
    // `window_glass`.
    let t = slint::Timer::default();
    let l = Rc::downgrade(&link);
    t.start(slint::TimerMode::Repeated, Duration::from_millis(500), move || {
        if let Some(l) = l.upgrade() {
            l.sync_body_blur(body);
            l.collect();
        }
    });
    link.timers.borrow_mut().push(t);
    link.collect();
    ui.win().request_redraw();
}
