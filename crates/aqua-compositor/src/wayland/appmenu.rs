//! `org_kde_kwin_appmenu`: Wayland apps (Qt with the KDE platform theme) announce where
//! their dbusmenu menu bar lives; X11 apps use the D-Bus registrar in `aqua_tray`.
//! The focused window's address goes to `aqua_tray::appmenu`, which feeds the menu bar.
use crate::state::Aqua;
use smithay::{
    desktop::Window,
    reexports::wayland_server::{
        protocol::wl_surface::WlSurface, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
    },
    wayland::compositor::with_states,
};
use std::sync::Mutex;
use wayland_protocols_plasma::appmenu::server::{
    org_kde_kwin_appmenu::{self as menu, OrgKdeKwinAppmenu},
    org_kde_kwin_appmenu_manager::{self as mgr, OrgKdeKwinAppmenuManager},
};

#[derive(Default)]
struct MenuAddress(Mutex<Option<aqua_tray::appmenu::Address>>);

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<Aqua, OrgKdeKwinAppmenuManager, ()>(2, ());
}

/// Address a Wayland surface announced.
pub fn address(surface: &WlSurface) -> Option<aqua_tray::appmenu::Address> {
    with_states(surface, |s| s.data_map.get::<MenuAddress>().and_then(|a| a.0.lock().unwrap().clone()))
}

/// Address of a window's global menu, Wayland or X11.
pub fn window_address(w: &Window) -> Option<aqua_tray::appmenu::Address> {
    if let Some(x) = w.x11_surface() {
        return aqua_tray::appmenu::for_x11_window(x.window_id());
    }
    address(w.toplevel()?.wl_surface())
}

impl Aqua {
    /// Tell the menu worker which menu the focused window has (cheap; called per frame).
    pub fn update_app_menu(&mut self) {
        let addr = self.focused_window().and_then(|w| self.app_menu_address(&w));
        aqua_tray::appmenu::set_active(addr);
    }

    /// Menu of a window; dialogs and other menu-less windows show their app's menu (the
    /// most recently raised window of the same app that has one), as on macOS.
    pub fn app_menu_address(&self, w: &Window) -> Option<aqua_tray::appmenu::Address> {
        if let Some(a) = window_address(w) {
            return Some(a);
        }
        let app = crate::state::title_of(w).0;
        if app.is_empty() {
            return None;
        }
        let same_client = |o: &Window| match (w.toplevel(), o.toplevel()) {
            (Some(a), Some(b)) => a.wl_surface().client() == b.wl_surface().client(),
            _ => w.x11_surface().is_some() && o.x11_surface().is_some(),
        };
        self.space
            .elements()
            .rev()
            .filter(|o| *o != w && crate::state::title_of(o).0 == app && same_client(o))
            .find_map(window_address)
    }
}

impl GlobalDispatch<OrgKdeKwinAppmenuManager, ()> for Aqua {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        res: New<OrgKdeKwinAppmenuManager>,
        _: &(),
        di: &mut DataInit<'_, Self>,
    ) {
        di.init(res, ());
    }
}

impl Dispatch<OrgKdeKwinAppmenuManager, ()> for Aqua {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &OrgKdeKwinAppmenuManager,
        req: mgr::Request,
        _: &(),
        _: &DisplayHandle,
        di: &mut DataInit<'_, Self>,
    ) {
        if let mgr::Request::Create { id, surface } = req {
            di.init(id, surface);
        }
    }
}

impl Dispatch<OrgKdeKwinAppmenu, WlSurface> for Aqua {
    fn request(
        st: &mut Self,
        _: &Client,
        _: &OrgKdeKwinAppmenu,
        req: menu::Request,
        surface: &WlSurface,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        let v = match req {
            menu::Request::SetAddress { service_name, object_path } => {
                (!service_name.is_empty() && !object_path.is_empty() && object_path != "/")
                    .then_some((service_name, object_path))
            }
            menu::Request::Release => None,
            _ => return,
        };
        tracing::debug!("appmenu (wayland): {v:?}");
        with_states(surface, |s| {
            s.data_map.insert_if_missing_threadsafe(MenuAddress::default);
            *s.data_map.get::<MenuAddress>().unwrap().0.lock().unwrap() = v;
        });
        st.update_app_menu();
    }
}
