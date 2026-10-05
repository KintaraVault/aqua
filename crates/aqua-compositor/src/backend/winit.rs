//! Nested backend: runs Aqua inside a window on an existing X11/Wayland session.
use std::{cell::RefCell, rc::Rc};

use crate::state::Aqua;
use smithay::{
    backend::{
        renderer::{damage::OutputDamageTracker, gles::GlesRenderer},
        winit::{self, WinitEvent, WinitGraphicsBackend},
    },
    output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::{
        calloop::EventLoop,
        winit::{dpi::PhysicalSize, window::WindowAttributes},
    },
    utils::Transform,
};

pub fn init(
    event_loop: &mut EventLoop<'static, Aqua>,
    state: &mut Aqua,
    pw: i32,
    ph: i32,
) -> Result<(), Box<dyn std::error::Error>> {
    let attrs = WindowAttributes::default()
        .with_surface_size(PhysicalSize::new(pw as u32, ph as u32))
        .with_title("Aqua")
        .with_visible(true);
    let (backend, winit) = winit::init_from_attributes::<GlesRenderer>(attrs)?;
    if std::env::var_os("AQUA_HOST_CURSOR").is_none() {
        backend.window().set_cursor_visible(false);
        state.draw_cursor = true;
    }
    let backend = Rc::new(RefCell::new(backend));
    let mode = Mode { size: backend.borrow().window_size(), refresh: 60_000 };
    let output = Output::new(
        "winit".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Aqua".into(),
            model: "Winit".into(),
            serial_number: "0".into(),
        },
    );
    state.publish_output(&output);
    output.change_current_state(
        Some(mode),
        Some(Transform::Flipped180),
        Some(Scale::Fractional(state.scale)),
        Some((0, 0).into()),
    );
    output.set_preferred(mode);
    state.add_output(output.clone());

    let damage_tracker = Rc::new(RefCell::new(OutputDamageTracker::from_output(&output)));

    let b2 = backend.clone();
    super::schedule_frames(event_loop, move |_| b2.borrow().window().request_redraw())?;

    let b3 = backend.clone();
    event_loop.handle().insert_source(winit, move |event, _, state| match event {
        WinitEvent::Resized { size, .. } => {
            let mode = Mode { size, refresh: 60_000 };
            output.change_current_state(Some(mode), None, None, None);
            state.on_output_resized();
        }
        WinitEvent::Input(event) => {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| state.process_input_event(event))).is_err() {
                tracing::error!("panic in input handling (recovered)");
            }
        }
        WinitEvent::Redraw => {
            let mut backend = b3.borrow_mut();
            render_frame(state, &mut backend, &output, &mut damage_tracker.borrow_mut());
        }
        WinitEvent::CloseRequested => state.loop_signal.stop(),
        _ => (),
    })?;
    Ok(())
}

fn render_frame(
    state: &mut Aqua,
    backend: &mut WinitGraphicsBackend<GlesRenderer>,
    output: &Output,
    dt: &mut OutputDamageTracker,
) {
    state.needs_redraw = false;
    let size = backend.window_size();
    let age = backend.buffer_age().unwrap_or(0);
    let damage = {
        let Ok((renderer, mut fb)) = backend.bind() else { return };
        let t0 = std::time::Instant::now();
        let elements = state.build_elements(renderer);
        match dt.render_output(renderer, &mut fb, age, &elements, CLEAR) {
            Ok(r) => {
                use aqua_render::stats::{record_frame, Frame};
                let rects: Option<Vec<aqua_render::stats::R>> =
                    r.damage.map(|d| d.iter().map(|r| (r.loc.x, r.loc.y, r.size.w, r.size.h)).collect());
                let frame = match &rects {
                    Some(v) if v.is_empty() => Frame::Empty,
                    Some(v) => Frame::Rendered(Some(v)),
                    None => Frame::Empty,
                };
                record_frame(&output.name(), (size.w, size.h), t0.elapsed(), frame);
                r.damage.cloned()
            }
            Err(e) => {
                tracing::warn!("render error: {e:?}");
                None
            }
        }
    };
    if let Some(d) = damage {
        let _ = backend.submit(Some(&d));
    }
    state.service_captures(backend.renderer());
    state.service_casts(backend.renderer());
    state.after_render_stats();
    state.service_screenshot(backend.renderer(), size);
    if let Some(path) = state.screenshot_request.take() {
        let renderer = backend.renderer();
        match crate::render::screenshot(state, renderer, size, &path) {
            Ok(()) => tracing::info!("screenshot saved to {path}"),
            Err(e) => tracing::error!("screenshot failed: {e}"),
        }
    }
    state.after_frame(output);
}

pub const CLEAR: [f32; 4] = [0.05, 0.05, 0.08, 1.0];
