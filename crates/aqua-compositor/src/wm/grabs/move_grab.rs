//! Move grab is the state of a composer during which the client window is being dragged around.
//!
//! eg. Usually whenever a user clicks on the app's titlebar and starts dragging, the compositors
//! enters a MoveSurfaceGrab state.

use crate::state::Aqua;
use smithay::{
    desktop::Window,
    input::pointer::{
        AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
        GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent,
        GestureSwipeUpdateEvent, GrabStartData as PointerGrabStartData, MotionEvent, PointerGrab, PointerInnerHandle,
        RelativeMotionEvent,
    },
    utils::{Logical, Point},
};

pub struct MoveSurfaceGrab {
    pub start_data: PointerGrabStartData<Aqua>,
    pub window: Window,
    pub initial_window_location: Point<i32, Logical>,
}

impl PointerGrab<Aqua> for MoveSurfaceGrab {
    fn motion(
        &mut self,
        data: &mut Aqua,
        handle: &mut PointerInnerHandle<'_, Aqua>,
        _focus: Option<(crate::input::focus::PointerFocusTarget, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        handle.motion(data, None, event);

        let delta = event.location - self.start_data.location;
        // Dragging a tiled window away gives it back its previous size (macOS).
        if delta.x.abs().max(delta.y.abs()) > 6.0 && crate::state::meta(&self.window).borrow().tiled.is_some() {
            if let Some(loc) = data.untile_for_drag(&self.window, event.location) {
                self.initial_window_location = (loc.x - delta.x.round() as i32, loc.y - delta.y.round() as i32).into();
            }
        }
        data.update_tile_preview(&self.window, event.location);
        let new_location = self.initial_window_location.to_f64() + delta;
        let mut loc: Point<i32, Logical> = new_location.to_i32_round();
        loc.y = loc.y.max(data.top_inset(&self.window));
        data.space.map_element(self.window.clone(), loc, true);
        data.request_redraw();
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
            data.window_moved(&self.window);
            if let Some(p) = data.render_cache.tile_preview.take() {
                data.tile_window(&self.window, p.tile);
            }
            handle.unset_grab(self, data, event.serial, event.time, true);
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

    fn unset(&mut self, data: &mut Aqua) {
        if data.render_cache.tile_preview.take().is_some() {
            data.needs_redraw = true;
        }
    }
}
