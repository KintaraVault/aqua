//! Output backends (DRM/KMS on a TTY, nested winit window) and the event loop they share.
pub mod udev;
pub mod winit;

use crate::cli::Args;
use crate::state::Aqua;
use smithay::output::Output;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;
use std::time::Duration;

pub const FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// Create the compositor on the selected backend, start the session and run it until
/// it quits.
pub fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let cfg = aqua_config::Config::load();
    crate::system::theming::apply(&cfg);
    let mut event_loop: EventLoop<'static, Aqua> = EventLoop::try_new()?;
    let display: Display<Aqua> = Display::new()?;
    // Apps must not inherit the host's display; the nested (winit) backend still needs
    // it to open its window, so it is dropped only once the backend is up.
    let forget_host_display = || unsafe {
        std::env::remove_var("DISPLAY");
        std::env::remove_var("WAYLAND_SOCKET");
    };
    if args.tty {
        forget_host_display();
    }
    let mut state = if args.tty {
        udev::init(&mut event_loop, display, cfg)?
    } else {
        let (pw, ph) = args.size;
        let (lw, lh) = ((pw as f64 / args.scale) as f32, (ph as f64 / args.scale) as f32);
        let mut state = Aqua::new(&mut event_loop, display, cfg, lw, lh, args.scale);
        winit::init(&mut event_loop, &mut state, pw, ph)?;
        forget_host_display();
        state
    };
    state.draw_cursor |= std::env::var_os("AQUA_DRAW_CURSOR").is_some();
    unsafe { std::env::set_var("WAYLAND_DISPLAY", &state.socket_name) };
    tracing::info!("Aqua running on WAYLAND_DISPLAY={:?} (scale {})", state.socket_name, state.scale);

    state.start_session(&args.commands);
    crate::session::schedule_system_tick(&mut event_loop);
    crate::control::schedule_test_hooks(&mut event_loop, &args);
    crate::ipc::listen(&mut event_loop);
    if state.udev.is_some() {
        state.render_udev();
    }
    event_loop.run(None, &mut state, |state| {
        let _ = state.display_handle.flush_clients();
    })?;
    state.finish_recording_blocking();
    Ok(())
}

/// The periodic frame tick shared by the backends: advances animations and returns
/// whether a frame has to be drawn.
pub fn schedule_frames(
    event_loop: &mut EventLoop<'static, Aqua>,
    mut draw: impl FnMut(&mut Aqua) + 'static,
) -> Result<(), Box<dyn std::error::Error>> {
    event_loop.handle().insert_source(Timer::from_duration(FRAME_INTERVAL), move |_, _, state| {
        state.render_cache.frames += 1;
        if state.shell.tick() | state.windows_animating() | state.poll_notifications() {
            state.needs_redraw = true;
        }
        if state.needs_redraw {
            draw(state);
        }
        TimeoutAction::ToDuration(FRAME_INTERVAL)
    })?;
    Ok(())
}

impl Aqua {
    /// Called after each rendered frame.
    pub fn after_frame(&mut self, output: &Output) {
        self.finish_window_anims();
        self.tick_spaces();
        let time = self.start_time.elapsed();
        self.send_frame_callbacks(output, time);
        self.space.refresh();
        self.sync_x11_positions();
        if std::mem::take(&mut self.repick_pointer) {
            self.refresh_pointer_focus();
        }
        self.popups.cleanup();
        self.prune_render_cache();
        let _ = self.display_handle.flush_clients();
    }

    /// Deliver `wl_surface.frame` callbacks to every client surface that is (or may be) shown on
    /// `output`: toplevels + popups, layer-shell surfaces, lock surfaces and the cursor surface.
    pub fn send_frame_callbacks(&mut self, output: &Output, time: Duration) {
        let o = output.clone();
        self.space.elements().for_each(|window| {
            window.send_frame(output, time, Some(Duration::ZERO), |_, _| Some(o.clone()));
        });
        for w in &self.minimized {
            w.send_frame(output, time, Some(Duration::from_millis(250)), |_, _| Some(o.clone()));
        }
        {
            let map = smithay::desktop::layer_map_for_output(output);
            for layer in map.layers() {
                layer.send_frame(output, time, Some(Duration::ZERO), |_, _| Some(o.clone()));
            }
        }
        for (lo, ls) in &self.lock.ext_surfaces {
            if lo == output {
                smithay::desktop::utils::send_frames_surface_tree(
                    ls.wl_surface(),
                    output,
                    time,
                    Some(Duration::ZERO),
                    |_, _| Some(o.clone()),
                );
            }
        }
        if let smithay::input::pointer::CursorImageStatus::Surface(s) = &self.render_cache.cursor_status {
            smithay::desktop::utils::send_frames_surface_tree(s, output, time, Some(Duration::ZERO), |_, _| {
                Some(o.clone())
            });
        }
        if let Some(icon) = &self.render_cache.dnd_icon {
            smithay::desktop::utils::send_frames_surface_tree(
                &icon.surface,
                output,
                time,
                Some(Duration::ZERO),
                |_, _| Some(o.clone()),
            );
        }
        for p in self.popups_for_layers() {
            smithay::desktop::utils::send_frames_surface_tree(&p, output, time, Some(Duration::ZERO), |_, _| {
                Some(o.clone())
            });
        }
    }

    fn popups_for_layers(&self) -> Vec<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface> {
        let mut out = vec![];
        for o in self.space.outputs() {
            let map = smithay::desktop::layer_map_for_output(o);
            for layer in map.layers() {
                for (p, _) in smithay::desktop::PopupManager::popups_for_surface(layer.wl_surface()) {
                    out.push(p.wl_surface().clone());
                }
            }
        }
        out
    }
}
