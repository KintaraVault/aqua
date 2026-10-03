//! Background blur ("glass") for client windows.
//!
//! Two protocols, same effect: `org_kde_kwin_blur_manager` (KDE; used by Qt, winit's
//! `Window::set_blur`, Konsole, kitty, Alacritty, foot …) and the standard
//! `ext_background_effect_manager_v1`. A surface that asks for it gets Aqua's glass
//! material (blurred, saturated backdrop) behind the requested region; whatever the app
//! draws translucently there turns into glass.
use crate::state::Aqua;
use smithay::{
    reexports::wayland_server::{
        protocol::wl_surface::WlSurface, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
    },
    utils::{Logical, Rectangle},
    wayland::compositor::{get_region_attributes, with_states, RectangleKind},
};
use std::sync::Mutex;
use wayland_protocols::ext::background_effect::v1::server::{
    ext_background_effect_manager_v1::{self as ext_mgr, ExtBackgroundEffectManagerV1},
    ext_background_effect_surface_v1::{self as ext_surf, ExtBackgroundEffectSurfaceV1},
};
use wayland_protocols_plasma::blur::server::{
    org_kde_kwin_blur::{self as kde_blur, OrgKdeKwinBlur},
    org_kde_kwin_blur_manager::{self as kde_mgr, OrgKdeKwinBlurManager},
};

/// Blur behind a surface: `None` = off, `Some([])` = whole surface, else these rects
/// (surface-local, logical).
#[derive(Default)]
struct BlurRegion(Mutex<Option<Vec<Rectangle<i32, Logical>>>>);

/// Pending KDE blur object state (applied on `org_kde_kwin_blur.commit`).
pub struct KdeBlur {
    surface: WlSurface,
    pending: Mutex<Option<Vec<Rectangle<i32, Logical>>>>,
}

/// ext-background-effect object state (applied on the next commit; we apply it at once).
pub struct ExtEffect {
    surface: WlSurface,
}

fn set(surface: &WlSurface, v: Option<Vec<Rectangle<i32, Logical>>>) {
    with_states(surface, |s| {
        s.data_map.insert_if_missing_threadsafe(BlurRegion::default);
        *s.data_map.get::<BlurRegion>().unwrap().0.lock().unwrap() = v;
    });
}

/// The blur region requested for `surface`.
pub fn region(surface: &WlSurface) -> Option<Vec<Rectangle<i32, Logical>>> {
    with_states(surface, |s| s.data_map.get::<BlurRegion>().and_then(|b| b.0.lock().unwrap().clone()))
}

/// Additive rectangles of a wl_region (subtractions shrink the result to "whole surface
/// minus nothing" — rare in practice; we keep only the added parts).
fn rects(
    region: Option<&smithay::reexports::wayland_server::protocol::wl_region::WlRegion>,
) -> Vec<Rectangle<i32, Logical>> {
    let Some(r) = region else { return vec![] };
    get_region_attributes(r)
        .rects
        .into_iter()
        .filter(|(k, _)| matches!(k, RectangleKind::Add))
        .map(|(_, r)| r)
        .filter(|r| r.size.w > 0 && r.size.h > 0)
        .collect()
}

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<Aqua, OrgKdeKwinBlurManager, ()>(1, ());
    dh.create_global::<Aqua, ExtBackgroundEffectManagerV1, ()>(1, ());
}

impl GlobalDispatch<OrgKdeKwinBlurManager, ()> for Aqua {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        res: New<OrgKdeKwinBlurManager>,
        _: &(),
        di: &mut DataInit<'_, Self>,
    ) {
        di.init(res, ());
    }
}

impl Dispatch<OrgKdeKwinBlurManager, ()> for Aqua {
    fn request(
        st: &mut Self,
        _: &Client,
        _: &OrgKdeKwinBlurManager,
        req: kde_mgr::Request,
        _: &(),
        _: &DisplayHandle,
        di: &mut DataInit<'_, Self>,
    ) {
        match req {
            kde_mgr::Request::Create { id, surface } => {
                di.init(id, KdeBlur { surface, pending: Mutex::new(Some(vec![])) });
            }
            kde_mgr::Request::Unset { surface } => {
                set(&surface, None);
                st.needs_redraw = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<OrgKdeKwinBlur, KdeBlur> for Aqua {
    fn request(
        st: &mut Self,
        _: &Client,
        _: &OrgKdeKwinBlur,
        req: kde_blur::Request,
        data: &KdeBlur,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match req {
            kde_blur::Request::SetRegion { region } => *data.pending.lock().unwrap() = Some(rects(region.as_ref())),
            kde_blur::Request::Commit => {
                if data.surface.is_alive() {
                    set(&data.surface, data.pending.lock().unwrap().clone());
                    st.needs_redraw = true;
                }
            }
            kde_blur::Request::Release => {}
            _ => {}
        }
    }
}

impl GlobalDispatch<ExtBackgroundEffectManagerV1, ()> for Aqua {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        res: New<ExtBackgroundEffectManagerV1>,
        _: &(),
        di: &mut DataInit<'_, Self>,
    ) {
        let m = di.init(res, ());
        m.capabilities(ext_mgr::Capability::Blur);
    }
}

impl Dispatch<ExtBackgroundEffectManagerV1, ()> for Aqua {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ExtBackgroundEffectManagerV1,
        req: ext_mgr::Request,
        _: &(),
        _: &DisplayHandle,
        di: &mut DataInit<'_, Self>,
    ) {
        if let ext_mgr::Request::GetBackgroundEffect { id, surface } = req {
            di.init(id, ExtEffect { surface });
        }
    }
}

impl Dispatch<ExtBackgroundEffectSurfaceV1, ExtEffect> for Aqua {
    fn request(
        st: &mut Self,
        _: &Client,
        _: &ExtBackgroundEffectSurfaceV1,
        req: ext_surf::Request,
        data: &ExtEffect,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if !data.surface.is_alive() {
            return;
        }
        match req {
            ext_surf::Request::SetBlurRegion { region } => {
                let v = region.as_ref().map(|r| rects(Some(r)));
                set(&data.surface, v.filter(|v| !v.is_empty()));
            }
            ext_surf::Request::Destroy => set(&data.surface, None),
            _ => {}
        }
        st.needs_redraw = true;
    }
}
