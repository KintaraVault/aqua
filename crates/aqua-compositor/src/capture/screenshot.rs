//! Screenshot utility (compositor side): renders the primary display offscreen,
//! crops it to the requested target, saves a PNG and puts it on the clipboard.
//! The interface (selection, toolbar, thumbnail) lives in `aqua_shell::screenshot`.
use crate::state::Aqua;
use aqua_shell::screenshot::{Mode, Target};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Size};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// A capture waiting for the next frame (or for its timer).
pub struct Job {
    pub target: Target,
    pub at: Instant,
    /// Start a screen recording instead of taking a picture.
    pub record: bool,
}

/// Encoded result coming back from the PNG worker thread.
pub struct Done {
    pub path: Option<String>,
    pub png: Vec<u8>,
    pub thumb: Option<aqua_gfx::Pixmap>,
}

pub struct Shots {
    pub job: Option<Job>,
    pub rec: Option<crate::capture::recorder::Recorder>,
    tx: mpsc::Sender<Result<Done, String>>,
    rx: mpsc::Receiver<Result<Done, String>>,
}

impl Default for Shots {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { job: None, rec: None, tx, rx }
    }
}

/// Where screenshots go: ~/Pictures/Screenshots (or the Desktop), created on demand.
pub(crate) fn save_dir(save_to: &str) -> Option<std::path::PathBuf> {
    use aqua_config::paths::user_dir;
    let d = match save_to {
        "clipboard" => return None,
        "desktop" => user_dir("DESKTOP", "Desktop"),
        "movies" => user_dir("VIDEOS", "Movies"),
        _ => user_dir("PICTURES", "Pictures").join("Screenshots"),
    };
    let _ = std::fs::create_dir_all(&d);
    Some(d)
}

/// "Screenshot 2026-10-02 at 09.41.07.png", never overwriting.
fn file_name(dir: &std::path::Path) -> String {
    stamped_name(dir, "Screenshot", "png")
}

/// "<prefix> 2026-10-02 at 09.41.07.<ext>", never overwriting an existing file.
pub(crate) fn stamped_name(dir: &std::path::Path, prefix: &str, ext: &str) -> String {
    let now =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&now, &mut tm) };
    let base = format!(
        "{prefix} {:04}-{:02}-{:02} at {:02}.{:02}.{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    );
    let mut p = dir.join(format!("{base}.{ext}"));
    let mut n = 2;
    while p.exists() {
        p = dir.join(format!("{base} ({n}).{ext}"));
        n += 1;
    }
    p.to_string_lossy().into_owned()
}

impl Aqua {
    /// ⌘⇧3 / ⌘⇧4 / ⌘⇧4+Space / ⌘⇧5: start a screenshot.
    pub fn start_screenshot(&mut self, how: &str) {
        if self.lock.is_locked() {
            return;
        }
        self.shell.close_transients();
        self.shell.shot.thumb = None;
        match how {
            "full" | "screen" | "" => self.queue_screenshot(Target::Full, false),
            "area" | "region" | "selection" => {
                self.screenshot_windows();
                self.shell.shot.begin(Mode::Area);
            }
            "window" => {
                self.screenshot_windows();
                self.shell.shot.begin(Mode::Window);
            }
            "record" | "recording" => {
                if self.screenshots.rec.is_some() {
                    self.stop_recording();
                    return;
                }
                if !self.shell.shot.kind.is_record() {
                    self.shell.shot.kind = aqua_shell::screenshot::Kind::RecScreen;
                }
                self.screenshot_windows();
                self.shell.shot.open_toolbar();
            }
            _ => {
                self.screenshot_windows();
                self.shell.shot.open_toolbar();
            }
        }
        self.update_shot_cursor();
        self.needs_redraw = true;
    }

    /// Feed the window frames (output-local, topmost first) to the selection UI.
    fn screenshot_windows(&mut self) {
        let origin =
            self.output.as_ref().and_then(|o| self.space.output_geometry(o)).map(|g| g.loc).unwrap_or_default();
        let mut v: Vec<aqua_gfx::Rect> = self
            .space
            .elements()
            .filter(|w| self.on_current_space(w))
            .filter_map(|w| self.frame_rect(w))
            .map(|r| {
                aqua_gfx::Rect::new(
                    (r.loc.x - origin.x) as f32,
                    (r.loc.y - origin.y) as f32,
                    r.size.w as f32,
                    r.size.h as f32,
                )
            })
            .collect();
        v.reverse();
        self.shell.shot.windows = v;
    }

    /// Crosshair while selecting, normal arrow otherwise.
    pub fn update_shot_cursor(&mut self) {
        use smithay::input::pointer::CursorIcon;
        let want = match self.shell.shot.mode {
            Mode::Area => Some(CursorIcon::Crosshair),
            Mode::Window => Some(CursorIcon::Pointer),
            Mode::Off => None,
        };
        let ours = matches!(self.render_cache.cursor_override, Some(CursorIcon::Crosshair) | Some(CursorIcon::Pointer));
        if (want.is_some() || ours) && self.render_cache.cursor_override != want {
            self.render_cache.cursor_override = want;
            self.needs_redraw = true;
        }
    }

    /// Schedule a capture (after the configured timer when `timer` is set).
    pub fn queue_screenshot(&mut self, target: Target, timer: bool) {
        let secs = if timer { self.cfg.screenshot_timer } else { 0 };
        let at = Instant::now() + Duration::from_secs(secs as u64);
        self.shell.shot.countdown = (secs > 0).then(|| (Instant::now(), secs as f32));
        self.shell.shot.thumb = None;
        self.screenshots.job = Some(Job { target, at, record: false });
        self.update_shot_cursor();
        self.needs_redraw = true;
    }

    /// Schedule the start of a screen recording (after the timer when `timer` is set).
    pub fn queue_recording(&mut self, target: Target, timer: bool) {
        if self.screenshots.rec.is_some() {
            return;
        }
        self.queue_screenshot(target, timer);
        if let Some(j) = &mut self.screenshots.job {
            j.record = true;
        }
    }

    /// Stop the running recording (the file is finalised in the background).
    pub fn stop_recording(&mut self) {
        if let Some(j) = &self.screenshots.job {
            if j.record {
                self.screenshots.job = None;
                self.shell.shot.countdown = None;
            }
        }
        if let Some(r) = self.screenshots.rec.take() {
            r.stop();
        }
        self.shell.shot.recording = None;
        self.shell.shot.clicks.clear();
        self.needs_redraw = true;
    }

    /// Session ends: stop the recording and wait (briefly) for ffmpeg to finalise it.
    pub fn finish_recording_blocking(&mut self) {
        if self.screenshots.rec.is_none() {
            return;
        }
        self.stop_recording();
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(5) {
            if !crate::capture::recorder::finished().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Persist the toolbar options.
    pub fn save_screenshot_options(&mut self) {
        let o = &self.shell.shot;
        let (s, t, th, p) = (o.save_to.clone(), o.timer, o.show_thumb, o.show_pointer);
        let (rs, rm, rc, rp) = (o.rec_save.clone(), o.rec_mic, o.rec_clicks, o.rec_pointer);
        let c = &self.cfg;
        let changed = c.screenshot_save != s
            || c.screenshot_timer != t
            || c.screenshot_thumbnail != th
            || c.screenshot_pointer != p;
        let rchanged = c.record_save != rs || c.record_mic != rm || c.record_clicks != rc || c.record_pointer != rp;
        if changed || rchanged {
            self.cfg.screenshot_save = s;
            self.cfg.screenshot_timer = t;
            self.cfg.screenshot_thumbnail = th;
            self.cfg.screenshot_pointer = p;
            self.cfg.record_save = rs;
            self.cfg.record_mic = rm;
            self.cfg.record_clicks = rc;
            self.cfg.record_pointer = rp;
            self.shell.cfg = self.cfg.clone();
            let _ = self.cfg.save();
            self.cfg_watch = aqua_config::Config::mtime();
        }
    }

    /// Is a capture due? (polled from the timers; keeps frames coming for the countdown)
    pub fn poll_screenshot(&mut self) -> bool {
        self.update_shot_cursor();
        let mut redraw = false;
        if let Some(j) = &self.screenshots.job {
            if j.at <= Instant::now() {
                self.shell.shot.countdown = None;
            }
            redraw = true;
        }
        let done: Vec<_> = self.screenshots.rx.try_iter().collect();
        for d in done {
            match d {
                Ok(d) => {
                    self.clipboard_put_png(d.png);
                    if let Some(t) = d.thumb {
                        if self.cfg.screenshot_thumbnail {
                            self.shell.shot.set_thumb(t, d.path.clone().unwrap_or_default());
                        }
                    }
                    match &d.path {
                        Some(p) => tracing::info!("screenshot saved to {p} and copied to the clipboard"),
                        None => tracing::info!("screenshot copied to the clipboard"),
                    }
                }
                Err(e) => tracing::error!("screenshot failed: {e}"),
            }
            redraw = true;
        }
        if self.shell.shot.thumb.is_some() || !self.shell.shot.clicks.is_empty() {
            redraw = true;
        }
        let mut failed = None;
        if let Some(r) = &mut self.screenshots.rec {
            if r.wants_frame() {
                redraw = true;
            }
            if let Some(e) = r.failed() {
                failed = Some(e);
            }
        }
        if let Some(e) = failed {
            tracing::error!("screen recording failed: {e}");
            self.stop_recording();
            self.shell.notify(aqua_shell::notifications::Note {
                app_id: "aqua-screenshot".into(),
                app_name: "Screenshot".into(),
                summary: "Screen recording failed".into(),
                body: e,
                timeout: 6.0,
                ..Default::default()
            });
            redraw = true;
        }
        for d in crate::capture::recorder::finished() {
            match d {
                Ok((path, thumb)) => {
                    tracing::info!("screen recording saved to {path}");
                    if let (Some(t), true) = (thumb, self.cfg.screenshot_thumbnail) {
                        self.shell.shot.set_thumb(t, path);
                    }
                }
                Err(e) => {
                    tracing::error!("screen recording failed: {e}");
                    self.shell.notify(aqua_shell::notifications::Note {
                        app_id: "aqua-screenshot".into(),
                        app_name: "Screenshot".into(),
                        summary: "Screen recording failed".into(),
                        body: e,
                        timeout: 6.0,
                        ..Default::default()
                    });
                }
            }
            redraw = true;
        }
        redraw
    }

    /// Crop rectangle (physical pixels) of a capture target on a `w`×`h` frame.
    fn crop_of(target: &Target, scale: f64, w: usize, h: usize) -> (usize, usize, usize, usize) {
        match target {
            Target::Full => (0, 0, w, h),
            Target::Area(r) | Target::Window(r) => {
                let x0 = ((r.x as f64 * scale).round().max(0.0) as usize).min(w);
                let y0 = ((r.y as f64 * scale).round().max(0.0) as usize).min(h);
                let x1 = (((r.x + r.w) as f64 * scale).round().max(0.0) as usize).min(w);
                let y1 = (((r.y + r.h) as f64 * scale).round().max(0.0) as usize).min(h);
                (x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0))
            }
        }
    }

    /// Copy the crop out of a full RGBA frame (opaque, top-down).
    fn crop_rgba(data: &[u8], w: usize, h: usize, (cx, cy, cw, ch): (usize, usize, usize, usize)) -> Vec<u8> {
        let flip = std::env::var("AQUA_SHOT_FLIP").map(|v| v == "1").unwrap_or(false);
        let mut rgba = vec![0u8; cw * ch * 4];
        for y in 0..ch {
            let sy = if flip { h - 1 - (cy + y) } else { cy + y };
            let src = &data[(sy * w + cx) * 4..(sy * w + cx + cw) * 4];
            rgba[y * cw * 4..(y + 1) * cw * 4].copy_from_slice(src);
        }
        for p in rgba.as_chunks_mut::<4>().0 {
            p[3] = 255;
        }
        rgba
    }

    /// Recording: grab a frame (rate-limited) after the backend rendered one.
    fn service_recording(&mut self, renderer: &mut GlesRenderer, size: Size<i32, Physical>) {
        let Some(rec) = &mut self.screenshots.rec else { return };
        if !rec.due() {
            return;
        }
        let Some(output) = self.output.clone() else { return };
        let crop = rec.crop;
        let cursor = self.draw_cursor;
        self.draw_cursor = self.cfg.record_pointer;
        // Crop straight out of the mapped readback; frames without damage are skipped
        // (ffmpeg repeats the last one), so idle screens cost no readback at all.
        let px = crate::render::render_output_with(self, renderer, &output, size, true, |data, w, h| {
            (crop.0 + crop.2 <= w && crop.1 + crop.3 <= h).then(|| Self::crop_rgba(data, w, h, crop))
        });
        self.draw_cursor = cursor;
        let Some(rec) = &mut self.screenshots.rec else { return };
        match px {
            Ok(None) => {}
            Ok(Some(Some(frame))) => rec.push(frame),
            Ok(Some(None)) => self.stop_recording(),
            Err(e) => tracing::warn!("recording frame failed: {e}"),
        }
    }

    /// Called by the backends after a frame: run a due capture.
    pub fn service_screenshot(&mut self, renderer: &mut GlesRenderer, size: Size<i32, Physical>) {
        self.service_recording(renderer, size);
        let due = self.screenshots.job.as_ref().is_some_and(|j| j.at <= Instant::now());
        if !due {
            return;
        }
        let Some(job) = self.screenshots.job.take() else { return };
        self.shell.shot.countdown = None;
        let Some(output) = self.output.clone() else { return };
        let scale = output.current_scale().fractional_scale();
        if job.record {
            let (w, h) = (size.w.max(0) as usize, size.h.max(0) as usize);
            let (cx, cy, cw, ch) = Self::crop_of(&job.target, scale, w, h);
            let crop = (cx, cy, cw & !1, ch & !1);
            if crop.2 < 16 || crop.3 < 16 {
                return;
            }
            let dir = save_dir(if self.cfg.record_save == "desktop" { "desktop" } else { "movies" })
                .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
            let path = stamped_name(&dir, "Screen Recording", "mp4");
            match crate::capture::recorder::Recorder::start(path, crop, self.cfg.record_mic) {
                Ok(r) => {
                    self.screenshots.rec = Some(r);
                    self.shell.shot.recording = Some(Instant::now());
                    tracing::info!("screen recording started");
                }
                Err(e) => {
                    tracing::error!("screen recording failed: {e}");
                    self.shell.notify(aqua_shell::notifications::Note {
                        app_id: "aqua-screenshot".into(),
                        app_name: "Screenshot".into(),
                        summary: "Can't record the screen".into(),
                        body: e,
                        timeout: 6.0,
                        ..Default::default()
                    });
                }
            }
            self.needs_redraw = true;
            return;
        }
        let cursor = self.draw_cursor;
        self.draw_cursor = self.cfg.screenshot_pointer;
        let px = crate::render::render_output_pixels(self, renderer, &output, size);
        self.draw_cursor = cursor;
        let (data, w, h) = match px {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("screenshot failed: {e}");
                return;
            }
        };
        let (cx, cy, cw, ch) = Self::crop_of(&job.target, scale, w, h);
        if cw == 0 || ch == 0 {
            return;
        }
        let rgba = Self::crop_rgba(&data, w, h, (cx, cy, cw, ch));
        drop(data);
        let dir = save_dir(&self.cfg.screenshot_save);
        let path = dir.as_deref().map(file_name);
        crate::system::sound::play_screenshot_sound();
        let tx = self.screenshots.tx.clone();
        std::thread::spawn(move || {
            let res = (|| -> Result<Done, String> {
                let img = image::RgbaImage::from_raw(cw as u32, ch as u32, rgba).ok_or("bad size")?;
                let mut png = Vec::new();
                image::DynamicImage::ImageRgba8(img.clone())
                    .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                    .map_err(|e| e.to_string())?;
                if let Some(p) = &path {
                    std::fs::write(p, &png).map_err(|e| format!("{p}: {e}"))?;
                }
                let t = image::DynamicImage::ImageRgba8(img).thumbnail(480, 480).to_rgba8();
                let thumb = aqua_gfx::from_rgba(t.width(), t.height(), t.as_raw());
                Ok(Done { path, png, thumb })
            })();
            let _ = tx.send(res);
        });
        self.needs_redraw = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamped_names_never_overwrite() {
        let d = std::env::temp_dir().join(format!("aqua-shot-test-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let first = stamped_name(&d, "Screenshot", "png");
        let name = std::path::Path::new(&first).file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("Screenshot ") && name.ends_with(".png") && name.contains(" at "), "{name}");
        std::fs::write(&first, b"").unwrap();
        let second = stamped_name(&d, "Screenshot", "png");
        if second.contains(&name[..name.len() - 4]) {
            assert!(second.ends_with(" (2).png"), "{second}");
        }
        assert_ne!(first, second);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn clipboard_has_no_directory() {
        assert!(save_dir("clipboard").is_none());
    }
}
