//! Exact focused-field contours for Aqua's own Wayland applications.
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};
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
        wayland_scanner::generate_interfaces!("../../protocols/aqua-text-field-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("../../protocols/aqua-text-field-v1.xml");
}
use proto::{
    aqua_text_field_manager_v1::AquaTextFieldManagerV1 as Manager, aqua_text_field_v1::AquaTextFieldV1 as Field,
};
struct State;
wayland_client::delegate_noop!(State: ignore Manager);
wayland_client::delegate_noop!(State: ignore Field);
impl Dispatch<WlRegistry, GlobalListContents> for State {
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
struct Shared {
    conn: Connection,
    queue: RefCell<EventQueue<State>>,
    manager: Manager,
}
thread_local! { static SHARED: RefCell<Option<Option<Rc<Shared>>>> = const { RefCell::new(None) }; }
fn shared(display: *mut std::ffi::c_void) -> Option<Rc<Shared>> {
    SHARED.with(|slot| {
        slot.borrow_mut()
            .get_or_insert_with(|| {
                // SAFETY: Winit owns the live display and outlives its window components.
                let conn = Connection::from_backend(unsafe { Backend::from_foreign_display(display.cast()) });
                let (globals, queue) = registry_queue_init::<State>(&conn).ok()?;
                let manager = globals.bind::<Manager, _, _>(&queue.handle(), 1..=1, ()).ok()?;
                Some(Rc::new(Shared { conn, queue: RefCell::new(queue), manager }))
            })
            .clone()
    })
}
type Contour = [f32; 5];
struct Link {
    window: Box<dyn Fn() -> Option<Box<dyn Window>>>,
    shared: Rc<Shared>,
    object: RefCell<Option<(ObjectId, Field)>>,
    pending: Cell<Option<Contour>>,
    sent: Cell<Option<Option<Contour>>>,
    scheduled: Cell<bool>,
    timer: slint::Timer,
}
trait Window {
    fn window(&self) -> &slint::Window;
    fn on_contour(&self, callback: Box<dyn Fn(f32, f32, f32, f32, f32)>);
    fn on_dirty(&self, callback: Box<dyn Fn()>);
    fn collect(&self);
}
macro_rules! access {
    ($c:ty) => {
        impl Window for $c {
            fn window(&self) -> &slint::Window {
                ComponentHandle::window(self)
            }
            fn on_contour(&self, callback: Box<dyn Fn(f32, f32, f32, f32, f32)>) {
                self.global::<crate::AquaGlass>().on_report_field(move |x, y, w, h, r| callback(x, y, w, h, r));
            }
            fn on_dirty(&self, callback: Box<dyn Fn()>) {
                self.global::<crate::AquaGlass>().on_field_dirty(callback);
            }
            fn collect(&self) {
                let g = self.global::<crate::AquaGlass>();
                g.set_field_epoch(g.get_field_epoch().wrapping_add(1));
            }
        }
    };
}
access!(crate::FinderWindow);
access!(crate::FinderMenuWindow);
access!(crate::SettingsWindow);
access!(crate::StoreWindow);
#[allow(private_bounds)]
pub trait FieldWindow: Window {}
impl<T: Window> FieldWindow for T {}
pub fn link<C: ComponentHandle + FieldWindow + 'static>(ui: &C) {
    attempt(ui.as_weak(), 0);
}
fn attempt<C: ComponentHandle + FieldWindow + 'static>(weak: slint::Weak<C>, tries: u32) {
    use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
    use slint::winit_030::WinitWindowAccessor;
    let Some(ui) = weak.upgrade() else { return };
    let display = Window::window(&ui).with_winit_window(|window| match window.display_handle().ok()?.as_raw() {
        RawDisplayHandle::Wayland(d) => Some(d.display.as_ptr()),
        _ => None,
    });
    let Some(display) = display else {
        if tries < 60 {
            slint::Timer::single_shot(Duration::from_millis(50), move || attempt(weak, tries + 1));
        }
        return;
    };
    let Some(shared) = display.and_then(shared) else { return };
    let weak_window = weak.clone();
    let link = Rc::new(Link {
        window: Box::new(move || weak_window.upgrade().map(|w| Box::new(w) as Box<dyn Window>)),
        shared,
        object: RefCell::new(None),
        pending: Cell::new(None),
        sent: Cell::new(None),
        scheduled: Cell::new(false),
        timer: slint::Timer::default(),
    });
    let report = link.clone();
    ui.on_contour(Box::new(move |x, y, w, h, r| {
        let contour = [x, y, w, h, r];
        if contour.iter().all(|n| n.is_finite()) && w > 0.0 && h > 0.0 && r >= 0.0 {
            report.pending.set(Some(contour));
        }
    }));
    let dirty = link.clone();
    ui.on_dirty(Box::new(move || dirty.collect()));
    let safety = Rc::downgrade(&link);
    link.timer.start(slint::TimerMode::Repeated, Duration::from_millis(250), move || {
        if let Some(link) = safety.upgrade() {
            link.collect();
        }
    });
    link.collect();
}
impl Link {
    fn collect(self: &Rc<Self>) {
        if self.scheduled.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        slint::Timer::single_shot(Duration::from_millis(4), move || {
            let Some(link) = weak.upgrade() else { return };
            let Some(window) = (link.window)() else { return };
            link.pending.set(None);
            window.collect();
            let weak = Rc::downgrade(&link);
            slint::Timer::single_shot(Duration::from_millis(4), move || {
                if let Some(link) = weak.upgrade() {
                    link.scheduled.set(false);
                    link.commit();
                }
            });
        });
    }
    fn commit(&self) {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use slint::winit_030::WinitWindowAccessor;
        let Some(window) = (self.window)() else { return };
        let Some(Some(ptr)) = window.window().with_winit_window(|w| match w.window_handle().ok()?.as_raw() {
            RawWindowHandle::Wayland(h) => Some(h.surface.as_ptr()),
            _ => None,
        }) else {
            return;
        };
        // SAFETY: this is the window's live Winit-managed wl_surface proxy.
        let Ok(id) = (unsafe { ObjectId::from_ptr(WlSurface::interface(), ptr.cast()) }) else { return };
        let Ok(surface) = WlSurface::from_id(&self.shared.conn, id) else { return };
        let mut object = self.object.borrow_mut();
        if object.as_ref().is_none_or(|(id, field)| *id != surface.id() || !field.is_alive()) {
            if let Some((_, old)) = object.take() {
                old.destroy();
            }
            *object =
                Some((surface.id(), self.shared.manager.get_field(&surface, &self.shared.queue.borrow().handle(), ())));
            self.sent.set(None);
        }
        let contour = self.pending.get();
        if self.sent.get() == Some(contour) {
            return;
        }
        if let Some((_, field)) = object.as_ref() {
            match contour {
                Some([x, y, w, h, r]) => field.set_geometry(x as f64, y as f64, w as f64, h as f64, r as f64),
                None => field.clear(),
            }
        }
        self.sent.set(Some(contour));
        let _ = self.shared.queue.borrow_mut().dispatch_pending(&mut State);
        let _ = self.shared.conn.flush();
        // Let Winit commit its own surface; never commit another renderer's pending buffer.
        window.window().request_redraw();
    }
}
impl Drop for Link {
    fn drop(&mut self) {
        if let Some((_, field)) = self.object.borrow_mut().take() {
            field.destroy();
            let _ = self.shared.conn.flush();
        }
    }
}
