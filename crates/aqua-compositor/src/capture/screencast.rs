//! ScreenCast portal streams: the shared monitor is rendered offscreen (with or without the
//! pointer) at up to [`FPS`] whenever the screen changed and pushed into a PipeWire node.
use crate::state::Aqua;
use aqua_notify::portal_impl::CastStream;
use smithay::backend::renderer::gles::GlesRenderer;
use std::time::{Duration, Instant};

pub const FPS: u32 = 30;

pub struct ActiveCast {
    /// Portal session handle.
    pub session: String,
    pub app_id: String,
    pub output: String,
    pub cursor: bool,
    started: Instant,
    last: Option<Instant>,
    /// The screen changed while rate limited: another frame is wanted.
    missed: bool,
    sent: bool,
    #[cfg(feature = "screencast")]
    stream: aqua_screencast::Cast,
}

impl ActiveCast {
    fn interval() -> Duration {
        Duration::from_micros(1_000_000 / FPS as u64)
    }

    #[cfg(feature = "screencast")]
    fn streaming(&self) -> bool {
        self.stream.streaming()
    }
    #[cfg(not(feature = "screencast"))]
    fn streaming(&self) -> bool {
        false
    }

    #[cfg(feature = "screencast")]
    fn ended(&self) -> bool {
        self.stream.ended()
    }
    #[cfg(not(feature = "screencast"))]
    fn ended(&self) -> bool {
        true
    }
}

/// Human name of the sharing app for the notification.
fn app_label(apps: &[aqua_apps::App], app_id: &str) -> String {
    let id = app_id.trim_end_matches(".desktop");
    if id.is_empty() {
        return aqua_shell::tr("An app").to_string();
    }
    aqua_apps::match_app_id(apps, id)
        .map(|a| a.name.clone())
        .unwrap_or_else(|| id.rsplit('.').next().unwrap_or(id).to_string())
}

impl Aqua {
    /// Start sharing the monitor under the pointer for portal session `session`.
    pub fn start_cast(&mut self, session: String, app_id: &str, cursor: bool) -> Result<Vec<CastStream>, String> {
        if self.lock.is_locked() {
            return Err("the screen is locked".into());
        }
        self.stop_cast(&session, false);
        let pos = self.seat.get_pointer().map(|p| p.current_location()).unwrap_or_default();
        let output = self.output_at(pos).or_else(|| self.output.clone()).ok_or("no output")?;
        let geo = self.space.output_geometry(&output).ok_or("output is not mapped")?;
        let mode = output.current_mode().ok_or("output has no mode")?;
        #[cfg(feature = "screencast")]
        let stream = aqua_screencast::Cast::start(
            &format!("aqua-screencast-{}", self.casts.len() + 1),
            mode.size.w as u32,
            mode.size.h as u32,
        )?;
        #[cfg(not(feature = "screencast"))]
        {
            let _ = (mode, cursor);
            return Err("Aqua was built without PipeWire screen casting".into());
        }
        #[cfg(feature = "screencast")]
        {
            let node = stream.node_id();
            tracing::info!("screencast for {app_id:?}: {} as PipeWire node {node}", output.name());
            let who = app_label(&self.shell.apps, app_id);
            self.shell.notify(aqua_shell::notifications::Note {
                app_id: "aqua-screenshot".into(),
                app_name: aqua_shell::tr("Screen Sharing").into(),
                summary: aqua_shell::trf("{app} is sharing your screen", &[("app", &who)]),
                body: output.name(),
                timeout: 6.0,
                ..Default::default()
            });
            self.casts.push(ActiveCast {
                session,
                app_id: app_id.to_string(),
                output: output.name(),
                cursor,
                started: Instant::now(),
                last: None,
                missed: true,
                sent: false,
                stream,
            });
            self.needs_redraw = true;
            Ok(vec![CastStream { node, position: (geo.loc.x, geo.loc.y), size: (geo.size.w, geo.size.h) }])
        }
    }

    /// End a cast; `notify_portal` when the compositor (not the client) ended it.
    pub fn stop_cast(&mut self, session: &str, notify_portal: bool) {
        let before = self.casts.len();
        self.casts.retain(|c| c.session != session);
        if self.casts.len() != before {
            tracing::info!("screencast {session} stopped");
            if notify_portal {
                self.appearance.cast_closed(session);
            }
        }
    }

    /// Stop every cast (e.g. the screen locked).
    pub fn stop_all_casts(&mut self) {
        for s in self.casts.iter().map(|c| c.session.clone()).collect::<Vec<_>>() {
            self.stop_cast(&s, true);
        }
    }

    /// Does a cast need another frame rendered (rate-limited change, first frame)?
    pub fn casts_want_frame(&self) -> bool {
        self.casts.iter().any(|c| {
            c.streaming() && (!c.sent || c.missed) && c.last.is_none_or(|l| l.elapsed() >= ActiveCast::interval())
        })
    }

    /// After every render pass: keep stale glass blurs converging and log profiling data.
    pub fn after_render_stats(&mut self) {
        if self.render_cache.blur_stale() {
            self.needs_redraw = true;
        }
        if aqua_render::stats::profiling() {
            static LAST: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
            let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
            if last.is_none_or(|t| t.elapsed() >= Duration::from_secs(10)) {
                if last.is_some() {
                    for line in aqua_render::stats::report().lines() {
                        tracing::info!("profile: {line}");
                    }
                    aqua_render::stats::reset();
                }
                *last = Some(Instant::now());
            }
        }
    }

    /// Called by the backends after rendering: feed the PipeWire streams.
    pub fn service_casts(&mut self, renderer: &mut GlesRenderer) {
        if self.casts.is_empty() {
            return;
        }
        if self.lock.is_locked() {
            self.stop_all_casts();
            return;
        }
        let dead: Vec<String> = self
            .casts
            .iter()
            .filter(|c| {
                (c.ended() && c.started.elapsed() > Duration::from_millis(500))
                    || !self.outputs.list.iter().chain(self.outputs.virtuals.iter()).any(|o| o.name() == c.output)
            })
            .map(|c| c.session.clone())
            .collect();
        for s in dead {
            self.stop_cast(&s, true);
        }
        for i in 0..self.casts.len() {
            let c = &mut self.casts[i];
            if !c.streaming() {
                continue;
            }
            if c.last.is_some_and(|l| l.elapsed() < ActiveCast::interval()) {
                c.missed = true;
                continue;
            }
            c.last = Some(Instant::now());
            c.missed = false;
            c.sent = true;
            let (name, cursor) = (c.output.clone(), c.cursor);
            let Some(output) =
                self.outputs.list.iter().chain(self.outputs.virtuals.iter()).find(|o| o.name() == name).cloned()
            else {
                continue;
            };
            let Some(mode) = output.current_mode() else { continue };
            let saved = self.draw_cursor;
            self.draw_cursor = cursor;
            let px = crate::render::render_output_pixels(self, renderer, &output, mode.size);
            self.draw_cursor = saved;
            match px {
                #[cfg(feature = "screencast")]
                Ok((data, w, h)) => {
                    let c = &self.casts[i];
                    if (w as u32, h as u32) == c.stream.size() {
                        c.stream.push(data);
                    } else {
                        // Mode changed: the negotiated format no longer fits.
                        let s = c.session.clone();
                        self.stop_cast(&s, true);
                        break;
                    }
                }
                #[cfg(not(feature = "screencast"))]
                Ok(_) => {}
                Err(e) => tracing::warn!("screencast frame failed: {e}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn app_labels() {
        assert_eq!(super::app_label(&[], "org.example.NoSuchApp"), "NoSuchApp");
        assert_eq!(super::app_label(&[], "org.example.NoSuchApp.desktop"), "NoSuchApp");
        assert!(!super::app_label(&[], "").is_empty());
    }
}
