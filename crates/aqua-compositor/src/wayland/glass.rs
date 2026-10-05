//! `aqua_glass_v1`: Liquid Glass shapes for Aqua's own apps (Finder, System Settings, App
//! Store, their menus). A client places shapes of one of the standard materials on its
//! surface; the renderer draws them with the same glass as the shell (see
//! `aqua_config::material`).
use crate::state::Aqua;
use smithay::reexports::wayland_server::{
    protocol::wl_surface::WlSurface, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::utils::{Logical, Rectangle};
use smithay::wayland::compositor::with_states;
use std::sync::Mutex;

#[allow(non_upper_case_globals, non_camel_case_types, unused_imports, missing_docs, clippy::all)]
pub mod proto {
    use smithay::reexports::wayland_server;
    use smithay::reexports::wayland_server::protocol::*;
    pub mod __interfaces {
        use smithay::reexports::wayland_server::backend as wayland_backend;
        use smithay::reexports::wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("../../protocols/aqua-glass-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_server_code!("../../protocols/aqua-glass-v1.xml");
}

use proto::aqua_glass_manager_v1::{self as mgr, AquaGlassManagerV1};
use proto::aqua_glass_v1::{self as glass, AquaGlassV1};

/// Most shapes honoured per surface.
pub const MAX_SHAPES: usize = 32;

/// One glass shape (surface-local, logical).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    pub rect: Rectangle<f64, Logical>,
    pub radius: f32,
    /// Wire material id, see `aqua_config::material::wire`.
    pub material: u32,
}

#[derive(Default)]
struct Shapes(Mutex<Vec<Shape>>);

/// Per-object state: the surface it decorates.
pub struct GlassObj {
    surface: WlSurface,
}

fn set(surface: &WlSurface, v: Vec<Shape>) {
    with_states(surface, |s| {
        s.data_map.insert_if_missing_threadsafe(Shapes::default);
        *s.data_map.get::<Shapes>().unwrap().0.lock().unwrap() = v;
    });
}

/// Glass shapes the client placed on `surface`.
pub fn shapes(surface: &WlSurface) -> Vec<Shape> {
    with_states(surface, |s| s.data_map.get::<Shapes>().map(|b| b.0.lock().unwrap().clone()).unwrap_or_default())
}

/// Decode a `set_shapes` array (see the protocol XML): invalid records are skipped.
pub fn decode(bytes: &[u8]) -> Vec<Shape> {
    let fixed = |b: &[u8]| i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64 / 256.0;
    let (records, _) = bytes.as_chunks::<24>();
    records
        .iter()
        .take(MAX_SHAPES)
        .filter_map(|r| {
            let (x, y, w, h, rad) =
                (fixed(&r[0..4]), fixed(&r[4..8]), fixed(&r[8..12]), fixed(&r[12..16]), fixed(&r[16..20]));
            let material = u32::from_le_bytes([r[20], r[21], r[22], r[23]]);
            let ok = [x, y, w, h, rad].iter().all(|v| v.is_finite()) && w >= 1.0 && h >= 1.0 && w < 1e5 && h < 1e5;
            ok.then(|| Shape {
                rect: Rectangle::new((x, y).into(), (w, h).into()),
                radius: rad.clamp(0.0, w.min(h) / 2.0) as f32,
                material,
            })
        })
        .collect()
}

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<Aqua, AquaGlassManagerV1, ()>(1, ());
}

impl GlobalDispatch<AquaGlassManagerV1, ()> for Aqua {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        res: New<AquaGlassManagerV1>,
        _: &(),
        di: &mut DataInit<'_, Self>,
    ) {
        di.init(res, ());
    }
}

impl Dispatch<AquaGlassManagerV1, ()> for Aqua {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &AquaGlassManagerV1,
        req: mgr::Request,
        _: &(),
        _: &DisplayHandle,
        di: &mut DataInit<'_, Self>,
    ) {
        if let mgr::Request::GetGlass { id, surface } = req {
            di.init(id, GlassObj { surface });
        }
    }
}

impl Dispatch<AquaGlassV1, GlassObj> for Aqua {
    fn request(
        st: &mut Self,
        _: &Client,
        _: &AquaGlassV1,
        req: glass::Request,
        data: &GlassObj,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if !data.surface.is_alive() {
            return;
        }
        match req {
            glass::Request::SetShapes { shapes } => set(&data.surface, decode(&shapes)),
            glass::Request::Destroy => set(&data.surface, vec![]),
        }
        st.needs_redraw = true;
    }

    fn destroyed(
        st: &mut Self,
        _: smithay::reexports::wayland_server::backend::ClientId,
        _: &AquaGlassV1,
        data: &GlassObj,
    ) {
        if data.surface.is_alive() {
            set(&data.surface, vec![]);
        }
        st.needs_redraw = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(x: f64, y: f64, w: f64, h: f64, r: f64, m: u32) -> Vec<u8> {
        let mut v = vec![];
        for f in [x, y, w, h, r] {
            v.extend(((f * 256.0) as i32).to_le_bytes());
        }
        v.extend(m.to_le_bytes());
        v
    }

    #[test]
    fn shapes_decode() {
        let mut b = rec(8.0, 8.5, 220.0, 600.0, 20.0, 1);
        b.extend(rec(0.0, 0.0, 0.0, 10.0, 0.0, 2)); // empty: skipped
        b.extend(rec(10.0, 10.0, 30.0, 20.0, 99.0, 2)); // radius clamped to a pill
        b.extend([1, 2, 3]); // trailing garbage: ignored
        let s = decode(&b);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].rect, Rectangle::new((8.0, 8.5).into(), (220.0, 600.0).into()));
        assert_eq!((s[0].radius, s[0].material), (20.0, 1));
        assert_eq!(s[1].radius, 10.0);
    }
}
