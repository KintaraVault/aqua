//! Smithay delegation with a text-input transaction observer.
use crate::state::Aqua;
use smithay::reexports::wayland_protocols::wp::text_input::zv3::server::zwp_text_input_manager_v3::Request as ManagerRequest;
use smithay::reexports::wayland_protocols::wp::text_input::zv3::server::zwp_text_input_v3::{Request, ZwpTextInputV3};
use smithay::reexports::wayland_server::{
    backend::ClientId, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::wayland::{input_method::InputMethodSeat, text_input::TextInputSeat};
use smithay::wayland::{Dispatch2, GlobalDispatch2};
use std::any::Any;

impl<I, U> Dispatch<I, U> for Aqua
where
    I: Resource + 'static,
    I::Request: 'static,
    U: Dispatch2<I, Aqua>,
{
    fn request(
        st: &mut Self,
        client: &Client,
        resource: &I,
        request: I::Request,
        data: &U,
        dh: &DisplayHandle,
        di: &mut DataInit<'_, Self>,
    ) {
        let ti = (resource as &dyn Any).downcast_ref::<ZwpTextInputV3>().cloned();
        let commit = if let (Some(ti), Some(req)) = (ti.as_ref(), (&request as &dyn Any).downcast_ref::<Request>()) {
            st.observe_text_input(ti, req)
        } else {
            false
        };
        let entering = if matches!(
            (&request as &dyn Any).downcast_ref::<ManagerRequest>(),
            Some(ManagerRequest::GetTextInput { .. })
        ) && st.seat.text_input().compositor_input_method()
            && !st.seat.input_method().has_instance()
        {
            let mut known = Vec::new();
            st.seat.text_input().with_focused_text_input(|ti, _| known.push(ti.id()));
            Some(known)
        } else {
            None
        };
        data.request(st, client, resource, request, dh, di);
        if commit {
            if let Some(ti) = ti {
                st.commit_text_input(&ti);
            }
        }
        // Enter only the new instance; re-entering existing instances would reset their fields.
        if let Some(known) = entering {
            st.seat.text_input().with_focused_text_input(|ti, surface| {
                if !known.contains(&ti.id()) {
                    ti.enter(surface);
                }
            });
        }
    }
    fn destroyed(st: &mut Self, client: ClientId, resource: &I, data: &U) {
        if let Some(ti) = (resource as &dyn Any).downcast_ref::<ZwpTextInputV3>() {
            st.destroy_text_input(ti);
        }
        data.destroyed(st, client, resource);
    }
}
impl<I, U> GlobalDispatch<I, U> for Aqua
where
    I: Resource,
    U: GlobalDispatch2<I, Aqua>,
{
    fn bind(
        st: &mut Self,
        dh: &DisplayHandle,
        client: &Client,
        resource: New<I>,
        data: &U,
        di: &mut DataInit<'_, Self>,
    ) {
        data.bind(st, dh, client, resource, di);
    }
    fn can_view(client: Client, data: &U) -> bool {
        data.can_view(&client)
    }
}
