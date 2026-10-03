//! Offscreen output rendering for screenshots and screencopy clients.
use super::*;

impl Aqua {
    /// Fulfil pending ext-image-copy-capture frames (screen recorders, portals, grim).
    pub fn service_captures(&mut self, renderer: &mut GlesRenderer) {
        use smithay::reexports::wayland_server::protocol::wl_shm::Format;
        use smithay::wayland::image_copy_capture::CaptureFailureReason;
        if self.pending_captures.is_empty() {
            return;
        }
        let caps = std::mem::take(&mut self.pending_captures);
        for (frame, output, _cursor) in caps {
            if self.lock.is_locked() {
                frame.fail(CaptureFailureReason::Stopped);
                continue;
            }
            let Some(mode) = output.current_mode() else {
                frame.fail(CaptureFailureReason::Unknown);
                continue;
            };
            let buffer = frame.buffer();
            let pixels = match render_output_pixels(self, renderer, &output, mode.size) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("capture render failed: {e}");
                    frame.fail(CaptureFailureReason::Unknown);
                    continue;
                }
            };
            let (data, w, h) = pixels;
            let res = smithay::wayland::shm::with_buffer_contents_mut(&buffer, |ptr, len, bd| {
                if bd.width as usize != w || bd.height as usize != h {
                    return false;
                }
                let swap = matches!(bd.format, Format::Argb8888 | Format::Xrgb8888);
                let dst = unsafe { std::slice::from_raw_parts_mut(ptr, len) };
                for y in 0..h {
                    let srow = &data[y * w * 4..(y + 1) * w * 4];
                    let off = bd.offset as usize + y * bd.stride as usize;
                    if off + w * 4 > len {
                        return false;
                    }
                    let drow = &mut dst[off..off + w * 4];
                    if swap {
                        for (d, sp) in drow.as_chunks_mut::<4>().0.iter_mut().zip(srow.as_chunks::<4>().0) {
                            d[0] = sp[2];
                            d[1] = sp[1];
                            d[2] = sp[0];
                            d[3] = 255;
                        }
                    } else {
                        drow.copy_from_slice(srow);
                    }
                }
                true
            });
            match res {
                Ok(true) => {
                    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
                    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
                    frame.success(
                        Transform::Normal,
                        None,
                        std::time::Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32),
                    );
                }
                _ => frame.fail(CaptureFailureReason::BufferConstraints),
            }
        }
    }
}

/// Render the current scene into an offscreen texture and save it as PNG.
pub fn screenshot(
    state: &mut Aqua,
    renderer: &mut GlesRenderer,
    size: Size<i32, Physical>,
    path: &str,
) -> Result<(), String> {
    let (output, path) = match path.split_once(':') {
        Some((name, p)) if !name.contains('/') => (state.outputs.list.iter().find(|o| o.name() == name).cloned(), p),
        _ => (state.output.clone(), path),
    };
    let output = output.ok_or("no such output")?;
    let size = if Some(&output) == state.output.as_ref() {
        size
    } else {
        output.current_mode().map(|m| m.size).ok_or("no mode")?
    };
    let (data, w, h) = render_output_pixels(state, renderer, &output, size)?;
    save_png(data, w, h, path)
}

/// Render one output offscreen and read back RGBA pixels (screenshots, screen capture).
pub fn render_output_pixels(
    state: &mut Aqua,
    renderer: &mut GlesRenderer,
    output: &smithay::output::Output,
    size: Size<i32, Physical>,
) -> Result<(Vec<u8>, usize, usize), String> {
    render_output_with(state, renderer, output, size, false, |d, w, h| (d.to_vec(), w, h))
        .map(|r| r.expect("unchanged frames are read back"))
}

/// A persistent offscreen target per output: repeated captures (screen recording,
/// screencast, screencopy clients) only redraw what changed, and glass keeps its blur
/// cache instead of re-allocating and re-blurring every backdrop on every frame.
pub struct OffscreenTarget {
    tex: GlesTexture,
    dt: smithay::backend::renderer::damage::OutputDamageTracker,
    size: Size<i32, Physical>,
    scale: f64,
    ctx: ContextId<GlesTexture>,
    rendered: bool,
    pub(super) used: Instant,
}

/// Render `output` offscreen and hand the RGBA pixels (top row first as rendered) to
/// `read`. With `skip_unchanged`, a frame without damage is not read back (`Ok(None)`).
pub fn render_output_with<R>(
    state: &mut Aqua,
    renderer: &mut GlesRenderer,
    output: &smithay::output::Output,
    size: Size<i32, Physical>,
    skip_unchanged: bool,
    read: impl FnOnce(&[u8], usize, usize) -> R,
) -> Result<Option<R>, String> {
    use smithay::backend::renderer::{damage::OutputDamageTracker, Bind, Offscreen};
    let scale = output.current_scale().fractional_scale();
    let name = output.name();
    let ctx = renderer.context_id();
    let mut target = match state.render_cache.offscreen.remove(&name) {
        Some(t) if t.size == size && t.scale == scale && t.ctx == ctx => t,
        _ => OffscreenTarget {
            tex: renderer.create_buffer(Fourcc::Abgr8888, (size.w, size.h).into()).map_err(|e| format!("{e:?}"))?,
            dt: OutputDamageTracker::new(size, scale, Transform::Normal),
            size,
            scale,
            ctx,
            rendered: false,
            used: Instant::now(),
        },
    };
    target.used = Instant::now();
    let elements = state.elements_for_output(renderer, output);
    let res = (|| {
        let mut fb = renderer.bind(&mut target.tex).map_err(|e| format!("{e:?}"))?;
        // The texture still holds the previous capture: only damage needs redrawing.
        let age = if target.rendered { 1 } else { 0 };
        let r = target
            .dt
            .render_output(renderer, &mut fb, age, &elements, crate::backend::winit::CLEAR)
            .map_err(|e| format!("{e:?}"))?;
        let changed = !target.rendered || r.damage.is_some_and(|d| !d.is_empty());
        target.rendered = true;
        if skip_unchanged && !changed {
            return Ok(None);
        }
        let map = renderer
            .copy_framebuffer(&fb, Rectangle::from_size((size.w, size.h).into()), Fourcc::Abgr8888)
            .map_err(|e| format!("{e:?}"))?;
        drop(fb);
        let data = renderer.map_texture(&map).map_err(|e| format!("{e:?}"))?;
        Ok(Some(read(data, size.w as usize, size.h as usize)))
    })();
    if res.is_err() {
        target.rendered = false;
    }
    state.render_cache.offscreen.insert(name, target);
    res
}

pub(super) fn save_png(data: Vec<u8>, w: usize, h: usize, path: &str) -> Result<(), String> {
    let flip = std::env::var("AQUA_SHOT_FLIP").map(|v| v == "1").unwrap_or(false);
    let stride = w * 4;
    let mut rows = vec![0u8; stride * h];
    for y in 0..h {
        let sy = if flip { h - 1 - y } else { y };
        rows[y * stride..(y + 1) * stride].copy_from_slice(&data[sy * stride..(sy + 1) * stride]);
    }
    for px in rows.as_chunks_mut::<4>().0 {
        px[3] = 255;
    }
    if let Some(dir) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let img = image::RgbaImage::from_raw(w as u32, h as u32, rows).ok_or("bad size")?;
    img.save(path).map_err(|e| e.to_string())
}
