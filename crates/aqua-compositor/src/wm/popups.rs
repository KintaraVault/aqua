//! App menus that may extend past their window ("menu popups").
//!
//! winit has no xdg_popup support, so Aqua's own apps open context / toolbar menus as small
//! borderless toplevels titled `aqua-popup:X:Y[:ALT_X]`: (X, Y) is where the menu's top-left
//! corner goes relative to the app's active window (surface coordinates), ALT_X the x to
//! use instead when the menu does not fit to the right (submenus flip to the left).
//! Such windows are placed at once (no open animation), never take the keyboard focus (the
//! owning window stays active), are left out of the Dock / Mission Control / ⌘Tab, and get
//! closed when the user clicks anywhere else.
use crate::state::{meta, title_of, Aqua};
use smithay::{
    desktop::Window,
    reexports::wayland_server::Resource,
    utils::{Logical, Point, Rectangle},
    wayland::seat::WaylandFocus,
};

/// Margin kept between a menu and the screen edge.
const EDGE: i32 = 6;
/// Corner radius the compositor clips menu popups to (matches `GlassMenu`).
pub const MENU_RADIUS: f32 = 12.0;

/// A parsed `aqua-popup:X:Y[:ALT_X][:sub]` title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Req {
    pub x: i32,
    pub y: i32,
    pub alt_x: Option<i32>,
    /// Relative to the client's top-most open menu (a submenu), not its window.
    pub sub: bool,
}

pub fn parse_title(title: &str) -> Option<Req> {
    let rest = title.strip_prefix("aqua-popup:")?;
    let mut parts = rest.split(':');
    let num = |s: Option<&str>| s.and_then(|s| s.trim().parse::<f64>().ok()).map(|v| v.round() as i32);
    let x = num(parts.next())?;
    let y = num(parts.next())?;
    let alt_x = num(parts.next());
    let sub = parts.next() == Some("sub");
    Some(Req { x, y, alt_x, sub })
}

/// Screen position of a `size` menu asked for at `(x, y, alt_x)` relative to `origin`,
/// kept inside `area` (the output minus the menu bar).
pub fn place(
    origin: (i32, i32),
    req: Req,
    size: (i32, i32),
    area: Rectangle<i32, Logical>,
) -> (i32, i32) {
    let Req { x, y, alt_x: alt, .. } = req;
    let (w, h) = size;
    let right = area.loc.x + area.size.w - EDGE;
    let bottom = area.loc.y + area.size.h - EDGE;
    let mut px = origin.0 + x;
    if px + w > right {
        px = match alt {
            Some(a) if origin.0 + a >= area.loc.x + EDGE => origin.0 + a,
            _ => right - w,
        };
    }
    px = px.max(area.loc.x + EDGE);
    let mut py = origin.1 + y;
    if py + h > bottom {
        py = bottom - h;
    }
    py = py.max(area.loc.y + EDGE);
    (px, py)
}

pub fn is_menu_popup(w: &Window) -> bool {
    meta(w).borrow().menu_popup
}

impl Aqua {
    /// Open menu popups, top-most last.
    pub fn menu_popups(&self) -> Vec<Window> {
        self.space.elements().filter(|w| is_menu_popup(w)).cloned().collect()
    }

    /// First commit of a toplevel: if it is a menu popup, place it next to its owner and
    /// return true.
    pub fn try_place_menu_popup(&mut self, window: &Window) -> bool {
        let Some(t) = window.toplevel() else { return false };
        let (_, title) = title_of(window);
        let Some(req) = parse_title(&title) else { return false };
        let client = t.wl_surface().client();
        let same_client = |w: &Window| {
            w.wl_surface().and_then(|s| s.client()).is_some_and(|c| Some(c) == client.clone())
        };
        let focused = self.seat.get_keyboard().and_then(|k| k.current_focus()).and_then(|f| match f {
            crate::input::focus::KeyboardFocusTarget::Window(w) => Some(w),
            _ => None,
        });
        let owner = focused.filter(|w| same_client(w) && !is_menu_popup(w)).or_else(|| {
            self.space
                .elements()
                .rev()
                .find(|w| w != &window && same_client(w) && !is_menu_popup(w) && meta(w).borrow().placed)
                .cloned()
        });
        let menu = if req.sub {
            self.space
                .elements()
                .rev()
                .find(|w| w != &window && same_client(w) && is_menu_popup(w))
                .cloned()
        } else {
            None
        };
        let origin = menu
            .as_ref()
            .or(owner.as_ref())
            .and_then(|o| Some(self.space.element_location(o)? - o.geometry().loc))
            .unwrap_or_else(|| {
                let p = self.seat.get_pointer().map(|p| p.current_location()).unwrap_or_default();
                Point::from((p.x as i32 - req.x, p.y as i32 - req.y))
            });
        let anchor: Point<f64, Logical> = (origin + Point::from((req.x, req.y))).to_f64();
        let out = self.output_at(anchor).or_else(|| self.output.clone());
        let mut area = out
            .as_ref()
            .and_then(|o| self.space.output_geometry(o))
            .unwrap_or_else(|| Rectangle::new((0, 0).into(), (1920, 1080).into()));
        let mb = self.cfg.menubar_height as i32;
        area.loc.y += mb;
        area.size.h -= mb;
        let geo = window.geometry();
        let (x, y) = place((origin.x, origin.y), req, (geo.size.w, geo.size.h), area);
        {
            let (desk, display) = owner
                .as_ref()
                .map(|o| (meta(o).borrow().desk, meta(o).borrow().display.clone()))
                .unwrap_or_else(|| {
                    let (d, c) = self.placement_desk();
                    (c, d)
                });
            let mut m = meta(window).borrow_mut();
            m.placed = true;
            m.menu_popup = true;
            m.override_redirect = true;
            m.desk = desk;
            m.display = display;
            m.mapped_at = None;
        }
        self.space.map_element(window.clone(), (x + geo.loc.x, y + geo.loc.y), false);
        self.space.raise_element(window, false);
        if let Some(o) = owner {
            if let Some(kb) = self.seat.get_keyboard() {
                let cur = kb.current_focus();
                let want = crate::input::focus::KeyboardFocusTarget::Window(o);
                if cur.as_ref() != Some(&want) {
                    kb.set_focus(self, Some(want), smithay::utils::SERIAL_COUNTER.next_serial());
                }
            }
        }
        self.update_activation();
        self.needs_redraw = true;
        true
    }

    /// Ask every open menu popup to close (a click landed elsewhere, or another window took
    /// the focus). `keep` = the popup the pointer is over, if any: clicks inside menus are
    /// the app's business.
    pub fn dismiss_menu_popups(&mut self, pos: Option<Point<f64, Logical>>) {
        let pops = self.menu_popups();
        if pops.is_empty() {
            return;
        }
        if let Some(p) = pos {
            let over = pops.iter().any(|w| {
                self.space.element_geometry(w).is_some_and(|r| r.to_f64().contains(p))
            });
            if over {
                return;
            }
        }
        for w in pops {
            if let Some(t) = w.toplevel() {
                t.send_close();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(x: i32, y: i32, alt_x: Option<i32>) -> Req {
        Req { x, y, alt_x, sub: false }
    }

    fn area() -> Rectangle<i32, Logical> {
        Rectangle::new((0, 24).into(), (1280, 696).into())
    }

    #[test]
    fn parses_titles() {
        let r = |x, y, alt_x, sub| Some(Req { x, y, alt_x, sub });
        assert_eq!(parse_title("aqua-popup:10:20"), r(10, 20, None, false));
        assert_eq!(parse_title("aqua-popup:10.6:-20:300"), r(11, -20, Some(300), false));
        assert_eq!(parse_title("aqua-popup:256:40:-236:sub"), r(256, 40, Some(-236), true));
        assert_eq!(parse_title("aqua-popup:5:6::sub"), r(5, 6, None, true));
        assert_eq!(parse_title("aqua-popup:x:20"), None);
        assert_eq!(parse_title("Finder"), None);
    }

    #[test]
    fn menu_goes_where_asked_when_it_fits() {
        assert_eq!(place((100, 100), q(50, 60, None), (260, 300), area()), (150, 160));
    }

    #[test]
    fn menu_may_extend_past_its_window_but_not_the_screen() {
        // Window at y=500; a 300 px menu at its bottom edge shifts up to stay on screen.
        let (_, y) = place((100, 500), q(20, 180, None), (260, 300), area());
        assert_eq!(y, 24 + 696 - EDGE - 300);
        // Too far right: clamp to the right edge.
        let (x, _) = place((1100, 100), q(50, 0, None), (260, 100), area());
        assert_eq!(x, 1280 - EDGE - 260);
        // Never above the menu bar / left of the screen.
        assert_eq!(place((-400, -50), q(0, 0, None), (260, 100), area()), (EDGE, 24 + EDGE));
    }

    #[test]
    fn submenu_flips_left() {
        let (x, _) = place((900, 100), q(256, 0, Some(-236)), (240, 100), area());
        assert_eq!(x, 900 - 236);
    }
}
