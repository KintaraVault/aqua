//! Touchpad gestures (libinput): system gestures with 3+ fingers,
//! everything else is forwarded to clients through `zwp_pointer_gestures_v1`
//! (pinch-zoom in browsers / image viewers, two-finger swipes …).
//!
//! * 3/4-finger swipe left/right → previous/next Space
//! * 3/4-finger swipe up → Mission Control, down → close it
//! * pinch with 3+ fingers → Launchpad (in) / show desktop (out)
use crate::state::Aqua;
use smithay::backend::input::{
    Event, GestureBeginEvent, GestureEndEvent, GesturePinchUpdateEvent, GestureSwipeUpdateEvent, InputBackend,
};
use smithay::input::pointer::{
    GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent, GesturePinchEndEvent,
    GesturePinchUpdateEvent as PinchUp, GestureSwipeBeginEvent, GestureSwipeEndEvent,
    GestureSwipeUpdateEvent as SwipeUp,
};
use smithay::utils::SERIAL_COUNTER;

#[derive(Default)]
pub struct GestureState {
    /// Some(fingers) while a system swipe is in progress.
    swipe: Option<u32>,
    dx: f64,
    dy: f64,
    pinch: Option<u32>,
    scale: f64,
    forwarded: bool,
}

const SWIPE_THRESHOLD: f64 = 70.0;

impl Aqua {
    fn system_gestures(&self, fingers: u32) -> bool {
        self.cfg.pointer.gestures && fingers >= 3 && !self.lock.is_locked()
    }

    pub fn on_swipe_begin<B: InputBackend>(&mut self, e: B::GestureSwipeBeginEvent) {
        self.notify_activity();
        let fingers = e.fingers();
        if self.system_gestures(fingers) {
            self.gesture = GestureState { swipe: Some(fingers), ..Default::default() };
            return;
        }
        self.gesture.forwarded = true;
        let ptr = self.seat.get_pointer().unwrap();
        ptr.gesture_swipe_begin(
            self,
            &GestureSwipeBeginEvent { serial: SERIAL_COUNTER.next_serial(), time: e.time(), fingers },
        );
    }

    pub fn on_swipe_update<B: InputBackend>(&mut self, e: B::GestureSwipeUpdateEvent) {
        if self.gesture.swipe.is_some() {
            self.gesture.dx += e.delta_x();
            self.gesture.dy += e.delta_y();
            return;
        }
        let ptr = self.seat.get_pointer().unwrap();
        ptr.gesture_swipe_update(self, &SwipeUp { time: e.time(), delta: e.delta() });
    }

    pub fn on_swipe_end<B: InputBackend>(&mut self, e: B::GestureSwipeEndEvent) {
        if self.gesture.swipe.take().is_some() {
            let (dx, dy) = (self.gesture.dx, self.gesture.dy);
            if e.cancelled() {
                return;
            }
            if dx.abs() > dy.abs() && dx.abs() > SWIPE_THRESHOLD {
                let dir = if (dx < 0.0) == self.cfg.pointer.natural_scroll { 1 } else { -1 };
                self.switch_space_rel(dir);
            } else if dy.abs() > SWIPE_THRESHOLD && (dy < 0.0) != self.mission.open {
                self.toggle_mission();
            }
            self.needs_redraw = true;
            return;
        }
        let ptr = self.seat.get_pointer().unwrap();
        ptr.gesture_swipe_end(
            self,
            &GestureSwipeEndEvent { serial: SERIAL_COUNTER.next_serial(), time: e.time(), cancelled: e.cancelled() },
        );
    }

    pub fn on_pinch_begin<B: InputBackend>(&mut self, e: B::GesturePinchBeginEvent) {
        self.notify_activity();
        let fingers = e.fingers();
        if self.system_gestures(fingers) {
            self.gesture.pinch = Some(fingers);
            self.gesture.scale = 1.0;
            return;
        }
        let ptr = self.seat.get_pointer().unwrap();
        ptr.gesture_pinch_begin(
            self,
            &GesturePinchBeginEvent { serial: SERIAL_COUNTER.next_serial(), time: e.time(), fingers },
        );
    }

    pub fn on_pinch_update<B: InputBackend>(&mut self, e: B::GesturePinchUpdateEvent) {
        if self.gesture.pinch.is_some() {
            self.gesture.scale = e.scale();
            return;
        }
        let ptr = self.seat.get_pointer().unwrap();
        ptr.gesture_pinch_update(
            self,
            &PinchUp { time: e.time(), delta: e.delta(), scale: e.scale(), rotation: e.rotation() },
        );
    }

    pub fn on_pinch_end<B: InputBackend>(&mut self, e: B::GesturePinchEndEvent) {
        if self.gesture.pinch.take().is_some() {
            if !e.cancelled() {
                let s = self.gesture.scale;
                if s < 0.75 && !self.shell.launchpad.open {
                    self.shell.toggle_launchpad();
                } else if s > 1.3 {
                    if self.shell.launchpad.open {
                        self.shell.toggle_launchpad();
                    } else {
                        self.show_desktop();
                    }
                }
                self.needs_redraw = true;
            }
            return;
        }
        let ptr = self.seat.get_pointer().unwrap();
        ptr.gesture_pinch_end(
            self,
            &GesturePinchEndEvent { serial: SERIAL_COUNTER.next_serial(), time: e.time(), cancelled: e.cancelled() },
        );
    }

    pub fn on_hold_begin<B: InputBackend>(&mut self, e: B::GestureHoldBeginEvent) {
        let ptr = self.seat.get_pointer().unwrap();
        ptr.gesture_hold_begin(
            self,
            &GestureHoldBeginEvent { serial: SERIAL_COUNTER.next_serial(), time: e.time(), fingers: e.fingers() },
        );
    }

    pub fn on_hold_end<B: InputBackend>(&mut self, e: B::GestureHoldEndEvent) {
        let ptr = self.seat.get_pointer().unwrap();
        ptr.gesture_hold_end(
            self,
            &GestureHoldEndEvent { serial: SERIAL_COUNTER.next_serial(), time: e.time(), cancelled: e.cancelled() },
        );
    }

    /// Minimise everything on the current Space.
    pub fn show_desktop(&mut self) {
        let wins: Vec<_> = self
            .space
            .elements()
            .filter(|w| self.on_current_space(w) && !crate::state::meta(w).borrow().override_redirect)
            .cloned()
            .collect();
        for w in wins {
            self.minimize(&w);
        }
    }
}
