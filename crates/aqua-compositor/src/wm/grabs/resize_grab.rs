//! Resize grab is the state of a composer during which the client window is being resized.
//!
//! eg. Usually whenever a user clicks on the app's border and starts dragging, the compositors
//! enters a ResizeSurfaceGrab state.

use crate::state::Aqua;
use smithay::{
    desktop::{Space, Window},
    input::pointer::{
        AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
        GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent,
        GestureSwipeUpdateEvent, GrabStartData as PointerGrabStartData, MotionEvent, PointerGrab, PointerInnerHandle,
        RelativeMotionEvent,
    },
    reexports::{wayland_protocols::xdg::shell::server::xdg_toplevel, wayland_server::protocol::wl_surface::WlSurface},
    utils::{Logical, Point, Rectangle, Size},
    wayland::{compositor, shell::xdg::SurfaceCachedState},
};
use std::cell::RefCell;

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct ResizeEdge: u32 {
        const TOP          = 0b0001;
        const BOTTOM       = 0b0010;
        const LEFT         = 0b0100;
        const RIGHT        = 0b1000;

        const TOP_LEFT     = Self::TOP.bits() | Self::LEFT.bits();
        const BOTTOM_LEFT  = Self::BOTTOM.bits() | Self::LEFT.bits();

        const TOP_RIGHT    = Self::TOP.bits() | Self::RIGHT.bits();
        const BOTTOM_RIGHT = Self::BOTTOM.bits() | Self::RIGHT.bits();
    }
}

impl ResizeEdge {
    pub fn from_x11(e: smithay::xwayland::xwm::ResizeEdge) -> Self {
        use smithay::xwayland::xwm::ResizeEdge as X;
        match e {
            X::Top => Self::TOP,
            X::Bottom => Self::BOTTOM,
            X::Left => Self::LEFT,
            X::Right => Self::RIGHT,
            X::TopLeft => Self::TOP_LEFT,
            X::TopRight => Self::TOP_RIGHT,
            X::BottomLeft => Self::BOTTOM_LEFT,
            X::BottomRight => Self::BOTTOM_RIGHT,
        }
    }
}

impl From<xdg_toplevel::ResizeEdge> for ResizeEdge {
    #[inline]
    fn from(x: xdg_toplevel::ResizeEdge) -> Self {
        Self::from_bits(x as u32).unwrap()
    }
}

pub struct ResizeSurfaceGrab {
    start_data: PointerGrabStartData<Aqua>,
    window: Window,

    edges: ResizeEdge,

    initial_rect: Rectangle<i32, Logical>,
    last_window_size: Size<i32, Logical>,
}

impl ResizeSurfaceGrab {
    pub fn start(
        start_data: PointerGrabStartData<Aqua>,
        window: Window,
        edges: ResizeEdge,
        initial_window_rect: Rectangle<i32, Logical>,
    ) -> Self {
        let initial_rect = initial_window_rect;

        // X11 windows (XWayland, e.g. Steam) have no xdg toplevel; their geometry is driven
        // directly from `motion`, so only Wayland surfaces track the resize state.
        if let Some(t) = window.toplevel() {
            ResizeSurfaceState::with(t.wl_surface(), |state| {
                *state = ResizeSurfaceState::Resizing { edges, initial_rect };
            });
        }

        Self { start_data, window, edges, initial_rect, last_window_size: initial_rect.size }
    }
}

impl PointerGrab<Aqua> for ResizeSurfaceGrab {
    fn motion(
        &mut self,
        data: &mut Aqua,
        handle: &mut PointerInnerHandle<'_, Aqua>,
        _focus: Option<(crate::input::focus::PointerFocusTarget, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        handle.motion(data, None, event);

        let delta = event.location - self.start_data.location;
        let (min_size, max_size) = match self.window.toplevel() {
            Some(t) => compositor::with_states(t.wl_surface(), |states| {
                let mut guard = states.cached_state.get::<SurfaceCachedState>();
                let data = guard.current();
                (data.min_size, data.max_size)
            }),
            None => {
                let Some(x) = self.window.x11_surface() else { return };
                let (min, max) = x11_visible_hints(x.min_size(), x.max_size(), x.frame_extents());
                (min.unwrap_or((40, 30).into()), max.unwrap_or_default())
            }
        };
        let r = resized_rect(self.initial_rect, self.edges, delta, min_size, max_size);
        self.last_window_size = r.size;

        let Some(xdg) = self.window.toplevel() else {
            let loc = r.loc;
            if let Some(x) = self.window.x11_surface() {
                crate::wm::x11_configure(x, Rectangle::new(loc, self.last_window_size));
            }
            data.space.map_element(self.window.clone(), loc, true);
            data.needs_redraw = true;
            return;
        };
        xdg.with_pending_state(|state| {
            state.states.set(xdg_toplevel::State::Resizing);
            state.size = Some(self.last_window_size);
        });

        xdg.send_pending_configure();
    }

    fn relative_motion(
        &mut self,
        data: &mut Aqua,
        handle: &mut PointerInnerHandle<'_, Aqua>,
        focus: Option<(crate::input::focus::PointerFocusTarget, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }

    fn button(&mut self, data: &mut Aqua, handle: &mut PointerInnerHandle<'_, Aqua>, event: &ButtonEvent) {
        handle.button(data, event);

        const BTN_LEFT: u32 = 0x110;

        if !handle.current_pressed().contains(&BTN_LEFT) {
            handle.unset_grab(self, data, event.serial, event.time, true);
            // Resized by hand: no longer a tiled window.
            {
                let mut m = crate::state::meta(&self.window).borrow_mut();
                m.tiled = None;
                m.pre_tile = None;
            }

            let Some(xdg) = self.window.toplevel() else { return };
            xdg.with_pending_state(|state| {
                state.states.unset(xdg_toplevel::State::Resizing);
                state.size = Some(self.last_window_size);
            });

            xdg.send_pending_configure();

            ResizeSurfaceState::with(xdg.wl_surface(), |state| {
                *state =
                    ResizeSurfaceState::WaitingForLastCommit { edges: self.edges, initial_rect: self.initial_rect };
            });
        }
    }

    fn axis(&mut self, data: &mut Aqua, handle: &mut PointerInnerHandle<'_, Aqua>, details: AxisFrame) {
        handle.axis(data, details)
    }

    fn frame(&mut self, data: &mut Aqua, handle: &mut PointerInnerHandle<'_, Aqua>) {
        handle.frame(data);
    }

    fn gesture_swipe_begin(
        &mut self,
        data: &mut Aqua,
        handle: &mut PointerInnerHandle<'_, Aqua>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(data, event)
    }

    fn gesture_swipe_update(
        &mut self,
        data: &mut Aqua,
        handle: &mut PointerInnerHandle<'_, Aqua>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(data, event)
    }

    fn gesture_swipe_end(
        &mut self,
        data: &mut Aqua,
        handle: &mut PointerInnerHandle<'_, Aqua>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(data, event)
    }

    fn gesture_pinch_begin(
        &mut self,
        data: &mut Aqua,
        handle: &mut PointerInnerHandle<'_, Aqua>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(data, event)
    }

    fn gesture_pinch_update(
        &mut self,
        data: &mut Aqua,
        handle: &mut PointerInnerHandle<'_, Aqua>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(data, event)
    }

    fn gesture_pinch_end(
        &mut self,
        data: &mut Aqua,
        handle: &mut PointerInnerHandle<'_, Aqua>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(data, event)
    }

    fn gesture_hold_begin(
        &mut self,
        data: &mut Aqua,
        handle: &mut PointerInnerHandle<'_, Aqua>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(data, event)
    }

    fn gesture_hold_end(
        &mut self,
        data: &mut Aqua,
        handle: &mut PointerInnerHandle<'_, Aqua>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(data, event)
    }

    fn start_data(&self) -> &PointerGrabStartData<Aqua> {
        &self.start_data
    }

    fn unset(&mut self, _data: &mut Aqua) {}
}

/// State of the resize operation.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
enum ResizeSurfaceState {
    #[default]
    Idle,
    Resizing {
        edges: ResizeEdge,
        /// The initial window size and location.
        initial_rect: Rectangle<i32, Logical>,
    },
    /// Resize is done, we are now waiting for last commit, to do the final move
    WaitingForLastCommit {
        edges: ResizeEdge,
        /// The initial window size and location.
        initial_rect: Rectangle<i32, Logical>,
    },
}

impl ResizeSurfaceState {
    fn with<F, T>(surface: &WlSurface, cb: F) -> T
    where
        F: FnOnce(&mut Self) -> T,
    {
        compositor::with_states(surface, |states| {
            states.data_map.insert_if_missing(RefCell::<Self>::default);
            let state = states.data_map.get::<RefCell<Self>>().unwrap();

            cb(&mut state.borrow_mut())
        })
    }

    fn commit(&mut self) -> Option<(ResizeEdge, Rectangle<i32, Logical>)> {
        match *self {
            Self::Resizing { edges, initial_rect } => Some((edges, initial_rect)),
            Self::WaitingForLastCommit { edges, initial_rect } => {
                *self = Self::Idle;

                Some((edges, initial_rect))
            }
            Self::Idle => None,
        }
    }
}

/// Should be called on `WlSurface::commit`
pub fn handle_commit(space: &mut Space<Window>, surface: &WlSurface) -> Option<()> {
    let window =
        space.elements().find(|w| w.toplevel().map(|t| t.wl_surface() == surface).unwrap_or(false)).cloned()?;

    let mut window_loc = space.element_location(&window)?;
    let geometry = window.geometry();

    let new_loc: Point<Option<i32>, Logical> = ResizeSurfaceState::with(surface, |state| {
        state
            .commit()
            .and_then(|(edges, initial_rect)| {
                edges.intersects(ResizeEdge::TOP_LEFT).then(|| {
                    let new_x = edges
                        .intersects(ResizeEdge::LEFT)
                        .then_some(initial_rect.loc.x + (initial_rect.size.w - geometry.size.w));

                    let new_y = edges
                        .intersects(ResizeEdge::TOP)
                        .then_some(initial_rect.loc.y + (initial_rect.size.h - geometry.size.h));

                    (new_x, new_y).into()
                })
            })
            .unwrap_or_default()
    });

    if let Some(new_x) = new_loc.x {
        window_loc.x = new_x;
    }
    if let Some(new_y) = new_loc.y {
        window_loc.y = new_y;
    }

    if new_loc.x.is_some() || new_loc.y.is_some() {
        space.map_element(window, window_loc, false);
    }

    Some(())
}

/// New rectangle of a window being resized from `edges` by the pointer `delta`, within the
/// size hints (`max` 0 = unbounded). Left / top edges keep the opposite edge in place.
pub fn resized_rect(
    initial: Rectangle<i32, Logical>,
    edges: ResizeEdge,
    delta: Point<f64, Logical>,
    min: Size<i32, Logical>,
    max: Size<i32, Logical>,
) -> Rectangle<i32, Logical> {
    let mut w = initial.size.w;
    let mut h = initial.size.h;
    if edges.intersects(ResizeEdge::LEFT) {
        w = (initial.size.w as f64 - delta.x) as i32;
    } else if edges.intersects(ResizeEdge::RIGHT) {
        w = (initial.size.w as f64 + delta.x) as i32;
    }
    if edges.intersects(ResizeEdge::TOP) {
        h = (initial.size.h as f64 - delta.y) as i32;
    } else if edges.intersects(ResizeEdge::BOTTOM) {
        h = (initial.size.h as f64 + delta.y) as i32;
    }
    let max_w = if max.w <= 0 { i32::MAX } else { max.w };
    let max_h = if max.h <= 0 { i32::MAX } else { max.h };
    let size: Size<i32, Logical> = (w.max(min.w.max(1)).min(max_w), h.max(min.h.max(1)).min(max_h)).into();
    let mut loc = initial.loc;
    if edges.intersects(ResizeEdge::LEFT) {
        loc.x = initial.loc.x + initial.size.w - size.w;
    }
    if edges.intersects(ResizeEdge::TOP) {
        loc.y = initial.loc.y + initial.size.h - size.h;
    }
    Rectangle::new(loc, size)
}

/// X11 `WM_NORMAL_HINTS` describe the whole X window, which with GTK includes its
/// client-side shadow (`_GTK_FRAME_EXTENTS`); the grab works with the visible geometry.
pub fn x11_visible_hints(
    min: Option<Size<i32, Logical>>,
    max: Option<Size<i32, Logical>>,
    fe: smithay::utils::FrameExtents<i32, Logical>,
) -> (Option<Size<i32, Logical>>, Option<Size<i32, Logical>>) {
    let (ew, eh) = (fe.left + fe.right, fe.top + fe.bottom);
    let shrink = |v: i32, e: i32| if v > 0 { (v - e).max(1) } else { 0 };
    (
        min.map(|s| Size::from((shrink(s.w, ew), shrink(s.h, eh)))),
        max.map(|s| Size::from((shrink(s.w, ew), shrink(s.h, eh)))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn right_and_bottom_edges_grow_in_place() {
        let r = resized_rect(rect(100, 50, 400, 300), ResizeEdge::BOTTOM_RIGHT, (60.0, 20.0).into(), (1, 1).into(), (0, 0).into());
        assert_eq!(r, rect(100, 50, 460, 320));
    }

    #[test]
    fn left_and_top_edges_keep_the_opposite_edge() {
        let r = resized_rect(rect(100, 50, 400, 300), ResizeEdge::TOP_LEFT, (-30.0, 10.0).into(), (1, 1).into(), (0, 0).into());
        assert_eq!(r, rect(70, 60, 430, 290));
        assert_eq!(r.loc.x + r.size.w, 500);
        assert_eq!(r.loc.y + r.size.h, 350);
    }

    #[test]
    fn hints_clamp_without_moving_the_anchor() {
        let r = resized_rect(rect(100, 50, 400, 300), ResizeEdge::LEFT, (390.0, 0.0).into(), (200, 100).into(), (0, 0).into());
        assert_eq!(r, rect(300, 50, 200, 300));
        let r = resized_rect(rect(100, 50, 400, 300), ResizeEdge::RIGHT, (900.0, 0.0).into(), (1, 1).into(), (640, 480).into());
        assert_eq!(r.size.w, 640);
        // A size hint of min > max must not panic (clamp would).
        let r = resized_rect(rect(0, 0, 400, 300), ResizeEdge::RIGHT, (5.0, 0.0).into(), (500, 400).into(), (300, 200).into());
        assert_eq!(r.size, (300, 200).into());
    }

    #[test]
    fn x11_hints_exclude_the_gtk_shadow() {
        // gnome-text-editor on X11: min 410x250 for the X window, shadow 25px on each side.
        let fe = smithay::utils::FrameExtents::new(25, 25, 25, 25);
        let (min, max) = x11_visible_hints(Some((410, 250).into()), Some((0, 0).into()), fe);
        assert_eq!(min, Some((360, 200).into()));
        assert_eq!(max, Some((0, 0).into()));
        let (min, _) = x11_visible_hints(Some((410, 250).into()), None, smithay::utils::FrameExtents::new(0, 0, 0, 0));
        assert_eq!(min, Some((410, 250).into()));
    }
}
