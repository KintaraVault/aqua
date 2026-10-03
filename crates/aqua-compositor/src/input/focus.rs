use std::{borrow::Cow, sync::Arc};

use smithay::xwayland::xwm::XwmOfferData;
use smithay::xwayland::X11Surface;
pub use smithay::{
    backend::input::{InputTime, KeyState},
    desktop::{LayerSurface, PopupKind},
    input::{
        keyboard::{KeyboardTarget, KeysymHandle, ModifiersState},
        pointer::{AxisFrame, ButtonEvent, MotionEvent, PointerTarget, RelativeMotionEvent},
        Seat,
    },
    reexports::wayland_server::{backend::ObjectId, protocol::wl_surface::WlSurface},
    utils::{IsAlive, Serial},
    wayland::seat::WaylandFocus,
};
use smithay::{
    desktop::{Window, WindowSurface},
    input::{
        dnd::{DndFocus, OfferData, Source},
        pointer::{
            GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent, GesturePinchEndEvent,
            GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent,
        },
        tablet::tool::TabletToolTarget,
        touch::{FrameMarker, TouchTarget},
    },
    reexports::wayland_server::DisplayHandle,
    utils::{Logical, Point},
    wayland::selection::data_device::WlOfferData,
};

use crate::state::Aqua;

#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum KeyboardFocusTarget {
    Window(Window),
    LayerSurface(LayerSurface),
    Popup(PopupKind),
    Surface(WlSurface),
}

impl IsAlive for KeyboardFocusTarget {
    #[inline]
    fn alive(&self) -> bool {
        match self {
            KeyboardFocusTarget::Window(w) => w.alive(),
            KeyboardFocusTarget::LayerSurface(l) => l.alive(),
            KeyboardFocusTarget::Popup(p) => p.alive(),
            KeyboardFocusTarget::Surface(p) => p.alive(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum PointerFocusTarget {
    WlSurface(WlSurface),
    X11Surface(X11Surface),
}

impl IsAlive for PointerFocusTarget {
    #[inline]
    fn alive(&self) -> bool {
        match self {
            PointerFocusTarget::WlSurface(w) => w.alive(),
            PointerFocusTarget::X11Surface(w) => w.alive(),
        }
    }
}

impl From<PointerFocusTarget> for WlSurface {
    #[inline]
    fn from(target: PointerFocusTarget) -> Self {
        target.wl_surface().unwrap().into_owned()
    }
}

impl KeyboardFocusTarget {
    fn inner_keyboard_target(&self) -> &dyn KeyboardTarget<Aqua> {
        match self {
            Self::Window(w) => match w.underlying_surface() {
                WindowSurface::Wayland(w) => w.wl_surface(),
                WindowSurface::X11(s) => s,
            },
            Self::LayerSurface(l) => l.wl_surface(),
            Self::Popup(p) => p.wl_surface(),
            Self::Surface(p) => p,
        }
    }
}

impl PointerFocusTarget {
    fn inner_pointer_target(&self) -> &dyn PointerTarget<Aqua> {
        match self {
            Self::WlSurface(w) => w,
            Self::X11Surface(w) => w,
        }
    }

    fn inner_touch_target(&self) -> &dyn TouchTarget<Aqua> {
        match self {
            Self::WlSurface(w) => w,
            Self::X11Surface(w) => w,
        }
    }

    fn inner_tablet_tool_target(&self) -> &dyn TabletToolTarget<Aqua> {
        match self {
            Self::WlSurface(w) => w,
            Self::X11Surface(w) => w,
        }
    }
}

impl PointerTarget<Aqua> for PointerFocusTarget {
    fn enter(&self, seat: &Seat<Aqua>, data: &mut Aqua, event: &MotionEvent) {
        self.inner_pointer_target().enter(seat, data, event)
    }
    fn motion(&self, seat: &Seat<Aqua>, data: &mut Aqua, event: &MotionEvent) {
        self.inner_pointer_target().motion(seat, data, event)
    }
    fn relative_motion(&self, seat: &Seat<Aqua>, data: &mut Aqua, event: &RelativeMotionEvent) {
        self.inner_pointer_target().relative_motion(seat, data, event)
    }
    fn button(&self, seat: &Seat<Aqua>, data: &mut Aqua, event: &ButtonEvent) {
        self.inner_pointer_target().button(seat, data, event)
    }
    fn axis(&self, seat: &Seat<Aqua>, data: &mut Aqua, frame: AxisFrame) {
        self.inner_pointer_target().axis(seat, data, frame)
    }
    fn frame(&self, seat: &Seat<Aqua>, data: &mut Aqua) {
        self.inner_pointer_target().frame(seat, data)
    }
    fn leave(&self, seat: &Seat<Aqua>, data: &mut Aqua, serial: Serial, time: InputTime) {
        self.inner_pointer_target().leave(seat, data, serial, time)
    }
    fn gesture_swipe_begin(&self, seat: &Seat<Aqua>, data: &mut Aqua, event: &GestureSwipeBeginEvent) {
        self.inner_pointer_target().gesture_swipe_begin(seat, data, event)
    }
    fn gesture_swipe_update(&self, seat: &Seat<Aqua>, data: &mut Aqua, event: &GestureSwipeUpdateEvent) {
        self.inner_pointer_target().gesture_swipe_update(seat, data, event)
    }
    fn gesture_swipe_end(&self, seat: &Seat<Aqua>, data: &mut Aqua, event: &GestureSwipeEndEvent) {
        self.inner_pointer_target().gesture_swipe_end(seat, data, event)
    }
    fn gesture_pinch_begin(&self, seat: &Seat<Aqua>, data: &mut Aqua, event: &GesturePinchBeginEvent) {
        self.inner_pointer_target().gesture_pinch_begin(seat, data, event)
    }
    fn gesture_pinch_update(&self, seat: &Seat<Aqua>, data: &mut Aqua, event: &GesturePinchUpdateEvent) {
        self.inner_pointer_target().gesture_pinch_update(seat, data, event)
    }
    fn gesture_pinch_end(&self, seat: &Seat<Aqua>, data: &mut Aqua, event: &GesturePinchEndEvent) {
        self.inner_pointer_target().gesture_pinch_end(seat, data, event)
    }
    fn gesture_hold_begin(&self, seat: &Seat<Aqua>, data: &mut Aqua, event: &GestureHoldBeginEvent) {
        self.inner_pointer_target().gesture_hold_begin(seat, data, event)
    }
    fn gesture_hold_end(&self, seat: &Seat<Aqua>, data: &mut Aqua, event: &GestureHoldEndEvent) {
        self.inner_pointer_target().gesture_hold_end(seat, data, event)
    }
}

impl KeyboardTarget<Aqua> for KeyboardFocusTarget {
    fn enter(&self, seat: &Seat<Aqua>, data: &mut Aqua, keys: Vec<KeysymHandle<'_>>, serial: Serial) {
        self.inner_keyboard_target().enter(seat, data, keys, serial)
    }
    fn leave(&self, seat: &Seat<Aqua>, data: &mut Aqua, serial: Serial) {
        self.inner_keyboard_target().leave(seat, data, serial)
    }
    fn key(
        &self,
        seat: &Seat<Aqua>,
        data: &mut Aqua,
        key: KeysymHandle<'_>,
        state: KeyState,
        serial: Serial,
        time: InputTime,
    ) {
        self.inner_keyboard_target().key(seat, data, key, state, serial, time)
    }
    fn modifiers(&self, seat: &Seat<Aqua>, data: &mut Aqua, modifiers: ModifiersState, serial: Serial) {
        self.inner_keyboard_target().modifiers(seat, data, modifiers, serial)
    }
}

impl TouchTarget<Aqua> for PointerFocusTarget {
    fn down(&self, seat: &Seat<Aqua>, data: &mut Aqua, event: &smithay::input::touch::DownEvent) {
        self.inner_touch_target().down(seat, data, event)
    }

    fn up(&self, seat: &Seat<Aqua>, data: &mut Aqua, event: &smithay::input::touch::UpEvent) {
        self.inner_touch_target().up(seat, data, event)
    }

    fn motion(&self, seat: &Seat<Aqua>, data: &mut Aqua, event: &smithay::input::touch::MotionEvent) {
        self.inner_touch_target().motion(seat, data, event)
    }

    fn frame(&self, seat: &Seat<Aqua>, data: &mut Aqua, marker: FrameMarker) {
        self.inner_touch_target().frame(seat, data, marker)
    }

    fn cancel(&self, seat: &Seat<Aqua>, data: &mut Aqua, marker: FrameMarker) {
        self.inner_touch_target().cancel(seat, data, marker)
    }

    fn shape(&self, seat: &Seat<Aqua>, data: &mut Aqua, event: &smithay::input::touch::ShapeEvent) {
        self.inner_touch_target().shape(seat, data, event)
    }

    fn orientation(&self, seat: &Seat<Aqua>, data: &mut Aqua, event: &smithay::input::touch::OrientationEvent) {
        self.inner_touch_target().orientation(seat, data, event)
    }

    fn last_frame(&self, seat: &Seat<Aqua>, data: &mut Aqua) -> Option<FrameMarker> {
        self.inner_touch_target().last_frame(seat, data)
    }
}

impl TabletToolTarget<Aqua> for PointerFocusTarget {
    fn proximity_in(
        &self,
        seat: &Seat<Aqua>,
        data: &mut Aqua,
        tool_descriptor: &smithay::backend::input::TabletToolDescriptor,
        tablet: &smithay::input::tablet::Tablet,
        serial: Serial,
    ) {
        self.inner_tablet_tool_target().proximity_in(seat, data, tool_descriptor, tablet, serial);
    }

    fn proximity_out(
        &self,
        seat: &Seat<Aqua>,
        data: &mut Aqua,
        tool_descriptor: &smithay::backend::input::TabletToolDescriptor,
    ) {
        self.inner_tablet_tool_target().proximity_out(seat, data, tool_descriptor);
    }

    fn down(
        &self,
        seat: &Seat<Aqua>,
        data: &mut Aqua,
        tool_descriptor: &smithay::backend::input::TabletToolDescriptor,
        event: &smithay::input::tablet::tool::DownEvent,
    ) {
        self.inner_tablet_tool_target().down(seat, data, tool_descriptor, event);
    }

    fn up(
        &self,
        seat: &Seat<Aqua>,
        data: &mut Aqua,
        tool_descriptor: &smithay::backend::input::TabletToolDescriptor,
        event: &smithay::input::tablet::tool::UpEvent,
    ) {
        self.inner_tablet_tool_target().up(seat, data, tool_descriptor, event);
    }

    fn motion(
        &self,
        seat: &Seat<Aqua>,
        data: &mut Aqua,
        tool_descriptor: &smithay::backend::input::TabletToolDescriptor,
        event: &smithay::input::tablet::tool::MotionEvent,
    ) {
        self.inner_tablet_tool_target().motion(seat, data, tool_descriptor, event);
    }

    fn button(
        &self,
        seat: &Seat<Aqua>,
        data: &mut Aqua,
        tool_descriptor: &smithay::backend::input::TabletToolDescriptor,
        event: &smithay::input::tablet::tool::ButtonEvent,
    ) {
        self.inner_tablet_tool_target().button(seat, data, tool_descriptor, event);
    }

    fn axis(
        &self,
        seat: &Seat<Aqua>,
        data: &mut Aqua,
        tool_descriptor: &smithay::backend::input::TabletToolDescriptor,
        frame: smithay::input::tablet::tool::AxisFrame,
    ) {
        self.inner_tablet_tool_target().axis(seat, data, tool_descriptor, frame);
    }

    fn frame(
        &self,
        seat: &Seat<Aqua>,
        data: &mut Aqua,
        tool_descriptor: &smithay::backend::input::TabletToolDescriptor,
        time: InputTime,
    ) {
        self.inner_tablet_tool_target().frame(seat, data, tool_descriptor, time);
    }
}

impl WaylandFocus for PointerFocusTarget {
    #[inline]
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        match self {
            PointerFocusTarget::WlSurface(w) => w.wl_surface(),
            PointerFocusTarget::X11Surface(w) => w.wl_surface().map(Cow::Owned),
        }
    }
    #[inline]
    fn same_client_as(&self, object_id: &ObjectId) -> bool {
        match self {
            PointerFocusTarget::WlSurface(w) => w.same_client_as(object_id),
            PointerFocusTarget::X11Surface(w) => w.same_client_as(object_id),
        }
    }
}

impl WaylandFocus for KeyboardFocusTarget {
    #[inline]
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        match self {
            KeyboardFocusTarget::Window(w) => w.wl_surface(),
            KeyboardFocusTarget::LayerSurface(l) => Some(Cow::Borrowed(l.wl_surface())),
            KeyboardFocusTarget::Popup(p) => Some(Cow::Borrowed(p.wl_surface())),
            KeyboardFocusTarget::Surface(p) => Some(Cow::Borrowed(p)),
        }
    }
}

pub enum AnvilOfferData<S: Source> {
    Wayland(WlOfferData<S>),
    X11(XwmOfferData<S>),
}

impl<S: Source> OfferData for AnvilOfferData<S> {
    fn disable(&self) {
        match self {
            AnvilOfferData::Wayland(data) => data.disable(),
            AnvilOfferData::X11(data) => data.disable(),
        }
    }

    fn drop(&self) {
        match self {
            AnvilOfferData::Wayland(data) => data.drop(),
            AnvilOfferData::X11(data) => data.drop(),
        }
    }

    fn validated(&self) -> bool {
        match self {
            AnvilOfferData::Wayland(data) => data.validated(),
            AnvilOfferData::X11(data) => data.validated(),
        }
    }
}

#[allow(unreachable_patterns)]
impl DndFocus<Aqua> for PointerFocusTarget {
    type OfferData<S>
        = AnvilOfferData<S>
    where
        S: Source;

    fn enter<S: Source>(
        &self,
        data: &mut Aqua,
        dh: &DisplayHandle,
        source: Arc<S>,
        seat: &Seat<Aqua>,
        location: Point<f64, Logical>,
        serial: &Serial,
    ) -> Option<AnvilOfferData<S>> {
        match self {
            PointerFocusTarget::WlSurface(surface) => {
                DndFocus::enter(surface, data, dh, source, seat, location, serial).map(AnvilOfferData::Wayland)
            }
            PointerFocusTarget::X11Surface(surface) => {
                DndFocus::enter(surface, data, dh, source, seat, location, serial).map(AnvilOfferData::X11)
            }
            _ => None,
        }
    }

    fn motion<S: Source>(
        &self,
        data: &mut Aqua,
        offer: Option<&mut AnvilOfferData<S>>,
        seat: &Seat<Aqua>,
        location: Point<f64, Logical>,
        time: InputTime,
    ) {
        match self {
            PointerFocusTarget::WlSurface(surface) => {
                let offer = match offer {
                    Some(AnvilOfferData::Wayland(offer)) => Some(offer),
                    None => None,
                    _ => return,
                };
                DndFocus::motion(surface, data, offer, seat, location, time)
            }
            PointerFocusTarget::X11Surface(surface) => {
                let offer = match offer {
                    Some(AnvilOfferData::X11(offer)) => Some(offer),
                    None => None,
                    _ => return,
                };
                DndFocus::motion(surface, data, offer, seat, location, time)
            }
            _ => {}
        }
    }

    fn leave<S: Source>(&self, data: &mut Aqua, offer: Option<&mut AnvilOfferData<S>>, seat: &Seat<Aqua>) {
        match self {
            PointerFocusTarget::WlSurface(surface) => {
                let offer = match offer {
                    Some(AnvilOfferData::Wayland(offer)) => Some(offer),
                    None => None,
                    _ => return,
                };
                DndFocus::leave(surface, data, offer, seat)
            }
            PointerFocusTarget::X11Surface(surface) => {
                let offer = match offer {
                    Some(AnvilOfferData::X11(offer)) => Some(offer),
                    None => None,
                    _ => return,
                };
                DndFocus::leave(surface, data, offer, seat)
            }
            _ => {}
        }
    }

    fn drop<S: Source>(&self, data: &mut Aqua, offer: Option<&mut AnvilOfferData<S>>, seat: &Seat<Aqua>) {
        match self {
            PointerFocusTarget::WlSurface(surface) => {
                let offer = match offer {
                    Some(AnvilOfferData::Wayland(offer)) => Some(offer),
                    None => None,
                    _ => return,
                };
                DndFocus::drop(surface, data, offer, seat)
            }
            PointerFocusTarget::X11Surface(surface) => {
                let offer = match offer {
                    Some(AnvilOfferData::X11(offer)) => Some(offer),
                    None => None,
                    _ => return,
                };
                DndFocus::drop(surface, data, offer, seat)
            }
            _ => {}
        }
    }
}

impl From<WlSurface> for PointerFocusTarget {
    #[inline]
    fn from(value: WlSurface) -> Self {
        PointerFocusTarget::WlSurface(value)
    }
}

impl From<&WlSurface> for PointerFocusTarget {
    #[inline]
    fn from(value: &WlSurface) -> Self {
        PointerFocusTarget::from(value.clone())
    }
}

impl From<PopupKind> for PointerFocusTarget {
    #[inline]
    fn from(value: PopupKind) -> Self {
        PointerFocusTarget::from(value.wl_surface())
    }
}

impl From<X11Surface> for PointerFocusTarget {
    #[inline]
    fn from(value: X11Surface) -> Self {
        PointerFocusTarget::X11Surface(value)
    }
}

impl From<&X11Surface> for PointerFocusTarget {
    #[inline]
    fn from(value: &X11Surface) -> Self {
        PointerFocusTarget::from(value.clone())
    }
}

impl From<LayerSurface> for KeyboardFocusTarget {
    #[inline]
    fn from(l: LayerSurface) -> Self {
        KeyboardFocusTarget::LayerSurface(l)
    }
}

impl From<PopupKind> for KeyboardFocusTarget {
    #[inline]
    fn from(p: PopupKind) -> Self {
        KeyboardFocusTarget::Popup(p)
    }
}

impl From<KeyboardFocusTarget> for PointerFocusTarget {
    #[inline]
    fn from(value: KeyboardFocusTarget) -> Self {
        match value {
            KeyboardFocusTarget::Window(w) => match w.underlying_surface() {
                WindowSurface::Wayland(w) => PointerFocusTarget::from(w.wl_surface()),
                WindowSurface::X11(s) => PointerFocusTarget::from(s),
            },
            KeyboardFocusTarget::LayerSurface(surface) => PointerFocusTarget::from(surface.wl_surface()),
            KeyboardFocusTarget::Popup(popup) => PointerFocusTarget::from(popup.wl_surface()),
            KeyboardFocusTarget::Surface(s) => PointerFocusTarget::from(s),
        }
    }
}

impl From<WlSurface> for KeyboardFocusTarget {
    fn from(s: WlSurface) -> Self {
        KeyboardFocusTarget::Surface(s)
    }
}
impl From<Window> for KeyboardFocusTarget {
    fn from(w: Window) -> Self {
        KeyboardFocusTarget::Window(w)
    }
}
