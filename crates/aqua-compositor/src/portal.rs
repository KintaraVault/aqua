//! xdg-desktop-portal Screenshot / PickColor requests served from rendered frames.
use crate::state::Aqua;

/// A portal Screenshot/PickColor request waiting for the next rendered frame.
pub struct PendingShot {
    path: std::path::PathBuf,
    /// Pointer position (physical px on the primary output) when picking a colour.
    pick: Option<(f64, f64)>,
    started: std::time::Instant,
    reply: std::sync::mpsc::Sender<Result<aqua_notify::portal_impl::Reply, String>>,
}

impl Aqua {
    /// Serve xdg-desktop-portal Screenshot/PickColor requests: queue an offscreen render and
    /// reply once the PNG exists.
    pub fn poll_portal(&mut self) -> bool {
        use aqua_notify::portal_impl::{Reply, RequestKind};
        let mut redraw = false;
        for r in aqua_notify::portal_impl::take_requests() {
            let (path, pick) = match r.kind {
                RequestKind::ScreenCast { session, app_id, cursor } => {
                    let _ = r.reply.send(self.start_cast(session, &app_id, cursor).map(Reply::Cast));
                    redraw = true;
                    continue;
                }
                RequestKind::StopCast(session) => {
                    self.stop_cast(&session, false);
                    let _ = r.reply.send(Ok(Reply::Done));
                    continue;
                }
                RequestKind::Screenshot(p) => (p, None),
                RequestKind::PickColor => {
                    let loc = self.seat.get_pointer().map(|p| p.current_location()).unwrap_or_default();
                    let scale = self.output.as_ref().map(|o| o.current_scale().fractional_scale()).unwrap_or(1.0);
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos())
                        .unwrap_or(0);
                    (
                        aqua_config::paths::runtime_dir().join(format!("aqua-pick-{ts}.png")),
                        Some((loc.x * scale, loc.y * scale)),
                    )
                }
            };
            let _ = std::fs::remove_file(&path);
            self.screenshot_request = Some(path.to_string_lossy().into_owned());
            self.portal_shots.push(PendingShot { path, pick, started: std::time::Instant::now(), reply: r.reply });
            redraw = true;
        }
        let mut keep = vec![];
        for p in std::mem::take(&mut self.portal_shots) {
            let ready = std::fs::metadata(&p.path).map(|m| m.len() > 0).unwrap_or(false);
            if ready {
                if p.started.elapsed() < std::time::Duration::from_millis(150) {
                    keep.push(p);
                    continue;
                }
                let res = match p.pick {
                    None => Ok(Reply::Saved(p.path.clone())),
                    Some((x, y)) => {
                        let r = image::open(&p.path).map_err(|e| e.to_string()).map(|img| {
                            let img = img.to_rgb8();
                            let (x, y) = (
                                (x as u32).min(img.width().saturating_sub(1)),
                                (y as u32).min(img.height().saturating_sub(1)),
                            );
                            let px = img.get_pixel(x, y);
                            Reply::Color(px[0] as f64 / 255.0, px[1] as f64 / 255.0, px[2] as f64 / 255.0)
                        });
                        let _ = std::fs::remove_file(&p.path);
                        r
                    }
                };
                let _ = p.reply.send(res);
            } else if p.started.elapsed() > std::time::Duration::from_secs(8) {
                let _ = p.reply.send(Err("timed out".into()));
            } else {
                if self.screenshot_request.is_none() && p.started.elapsed() > std::time::Duration::from_secs(2) {
                    self.screenshot_request = Some(p.path.to_string_lossy().into_owned());
                }
                redraw = true;
                keep.push(p);
            }
        }
        self.portal_shots = keep;
        redraw | self.casts_want_frame()
    }
}
