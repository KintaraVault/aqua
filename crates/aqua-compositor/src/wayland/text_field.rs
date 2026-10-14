//! Optional, double-buffered focused-field bounds. Standard text-input only supplies a caret.
use crate::state::Aqua;
use aqua_text::Shape;
use smithay::reexports::wayland_server::{
    protocol::wl_surface::WlSurface, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::wayland::compositor::with_states;
use std::sync::Mutex;

#[allow(non_upper_case_globals, non_camel_case_types, unused_imports, missing_docs, clippy::all)]
pub mod proto {
    use smithay::reexports::wayland_server;
    use smithay::reexports::wayland_server::protocol::*;
    pub mod __interfaces {
        use smithay::reexports::wayland_server::backend as wayland_backend;
        use smithay::reexports::wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("../../protocols/aqua-text-field-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_server_code!("../../protocols/aqua-text-field-v1.xml");
}
use proto::aqua_text_field_manager_v1::{self as manager, AquaTextFieldManagerV1};
use proto::aqua_text_field_v1::{self as field, AquaTextFieldV1};
#[derive(Default)]
struct Geometry {
    owner: Option<AquaTextFieldV1>,
    pending: Option<Option<Shape>>,
    committed: Option<Shape>,
}
#[derive(Default)]
struct State(Mutex<Geometry>);
pub struct FieldObject(WlSurface);
pub fn init(dh: &DisplayHandle) {
    dh.create_global::<Aqua, AquaTextFieldManagerV1, _>(1, ());
}
pub fn commit(surface: &WlSurface) {
    with_states(surface, |s| {
        if let Some(g) = s.data_map.get::<State>() {
            let mut g = g.0.lock().unwrap();
            if let Some(pending) = g.pending.take() {
                g.committed = pending;
            }
        }
    });
}
pub fn geometry(surface: &WlSurface) -> Option<Shape> {
    with_states(surface, |s| s.data_map.get::<State>().and_then(|g| g.0.lock().unwrap().committed))
}
impl GlobalDispatch<AquaTextFieldManagerV1, ()> for Aqua {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        res: New<AquaTextFieldManagerV1>,
        _: &(),
        di: &mut DataInit<'_, Self>,
    ) {
        di.init(res, ());
    }
}
impl Dispatch<AquaTextFieldManagerV1, ()> for Aqua {
    fn request(
        _: &mut Self,
        _: &Client,
        res: &AquaTextFieldManagerV1,
        req: manager::Request,
        _: &(),
        _: &DisplayHandle,
        di: &mut DataInit<'_, Self>,
    ) {
        if let manager::Request::GetField { id, surface } = req {
            with_states(&surface, |s| {
                s.data_map.insert_if_missing_threadsafe(State::default);
                let mut g = s.data_map.get::<State>().unwrap().0.lock().unwrap();
                if g.owner.as_ref().is_some_and(Resource::is_alive) {
                    di.post_error(id, manager::Error::AlreadyExists, "field already exists");
                    return;
                }
                let obj = di.init(id, FieldObject(surface.clone()));
                g.owner = Some(obj);
            });
            let _ = res;
        }
    }
}
impl Dispatch<AquaTextFieldV1, FieldObject> for Aqua {
    fn request(
        st: &mut Self,
        _: &Client,
        _: &AquaTextFieldV1,
        req: field::Request,
        data: &FieldObject,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        with_states(&data.0, |s| {
            let mut g = s.data_map.get::<State>().unwrap().0.lock().unwrap();
            match req {
                field::Request::SetGeometry { x, y, width, height, radius } => {
                    g.pending = Some(
                        Shape { x: x as f32, y: y as f32, w: width as f32, h: height as f32, radius: radius as f32 }
                            .validated(),
                    );
                }
                field::Request::Clear => g.pending = Some(None),
                field::Request::Destroy => {
                    g.pending = Some(None);
                    g.committed = None;
                }
            }
        });
        st.needs_redraw = true;
    }
    fn destroyed(
        st: &mut Self,
        _: smithay::reexports::wayland_server::backend::ClientId,
        obj: &AquaTextFieldV1,
        data: &FieldObject,
    ) {
        if data.0.is_alive() {
            with_states(&data.0, |s| {
                if let Some(g) = s.data_map.get::<State>() {
                    let mut g = g.0.lock().unwrap();
                    if g.owner.as_ref() == Some(obj) {
                        *g = Geometry::default();
                    }
                }
            });
        }
        st.needs_redraw = true;
    }
}
