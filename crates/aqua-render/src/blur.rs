//! Raw GLES2 dual-kawase blur on private textures/FBOs.
use smithay::backend::renderer::gles::{ffi, GlesError, GlesRenderer};
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Minimum time between two blurs of the same glass backdrop (0 = no limit).
static BLUR_MIN_MS: AtomicU32 = AtomicU32::new(0);

/// Limit how often a glass backdrop that keeps changing (video, animation behind the dock)
/// is re-blurred; in between the previous blur is reused. 0 disables the limit.
pub fn set_blur_max_fps(fps: u32) {
    BLUR_MIN_MS.store(if fps == 0 { 0 } else { 1000 / fps.max(1) }, Ordering::Relaxed);
}

/// Lift the limit while the compositor itself animates (windows zooming, flying into
/// the Dock, opening): glass over them must follow every frame, or it alternates between
/// fresh and stale blur and visibly flickers.
static UNTHROTTLED: AtomicBool = AtomicBool::new(false);

pub fn set_blur_unthrottled(on: bool) {
    UNTHROTTLED.store(on, Ordering::Relaxed);
}

fn min_interval() -> Duration {
    if UNTHROTTLED.load(Ordering::Relaxed) {
        return Duration::ZERO;
    }
    Duration::from_millis(BLUR_MIN_MS.load(Ordering::Relaxed) as u64)
}

/// Shared between a glass slot (kept across frames by the compositor) and its elements:
/// remembers when the backdrop was last blurred and whether a capture was skipped.
#[derive(Default, Debug)]
pub struct BlurGate {
    stale: AtomicBool,
    last: Mutex<Option<(Instant, [i32; 4])>>,
}

impl BlurGate {
    /// May the backdrop at `rect` be blurred now? (Otherwise it is marked stale.)
    pub fn admit(&self, rect: [i32; 4], valid: bool, now: Instant) -> bool {
        let min = min_interval();
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        let recent = last.is_some_and(|(t, r)| r == rect && now.saturating_duration_since(t) < min);
        if valid && !min.is_zero() && recent {
            self.stale.store(true, Ordering::Relaxed);
            return false;
        }
        *last = Some((now, rect));
        self.stale.store(false, Ordering::Relaxed);
        true
    }

    /// A capture was skipped: is it time to blur again? (The compositor then bumps the
    /// element's commit so the framebuffer is captured once more.)
    pub fn recapture_due(&self, now: Instant) -> bool {
        if !self.stale.load(Ordering::Relaxed) {
            return false;
        }
        let last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        last.is_none_or(|(t, _)| now.saturating_duration_since(t) >= min_interval())
    }

    /// The shown blur is older than the backdrop.
    pub fn is_stale(&self) -> bool {
        self.stale.load(Ordering::Relaxed)
    }
}

pub struct BlurPrograms {
    pub down: Prog,
    pub up: Prog,
}

pub struct Prog {
    pub id: u32,
    pub pos: i32,
    pub tex: i32,
    pub halfpixel: i32,
    pub offset: i32,
}

unsafe fn compile(gl: &ffi::Gles2, kind: u32, src: &str) -> Result<u32, String> {
    let sh = gl.CreateShader(kind);
    let c = CString::new(src).unwrap();
    gl.ShaderSource(sh, 1, &c.as_ptr(), std::ptr::null());
    gl.CompileShader(sh);
    let mut ok = 0;
    gl.GetShaderiv(sh, ffi::COMPILE_STATUS, &mut ok);
    if ok == 0 {
        let mut buf = vec![0u8; 2048];
        let mut len = 0;
        gl.GetShaderInfoLog(sh, 2048, &mut len, buf.as_mut_ptr() as *mut _);
        return Err(String::from_utf8_lossy(&buf[..len as usize]).to_string());
    }
    Ok(sh)
}

unsafe fn link(gl: &ffi::Gles2, vs: &str, fs: &str) -> Result<Prog, String> {
    let v = compile(gl, ffi::VERTEX_SHADER, vs)?;
    let f = compile(gl, ffi::FRAGMENT_SHADER, fs)?;
    let p = gl.CreateProgram();
    gl.AttachShader(p, v);
    gl.AttachShader(p, f);
    gl.LinkProgram(p);
    let mut ok = 0;
    gl.GetProgramiv(p, ffi::LINK_STATUS, &mut ok);
    gl.DeleteShader(v);
    gl.DeleteShader(f);
    if ok == 0 {
        return Err("blur program link failed".into());
    }
    let loc = |n: &str| {
        let c = CString::new(n).unwrap();
        gl.GetUniformLocation(p, c.as_ptr())
    };
    let pos = {
        let c = CString::new("pos").unwrap();
        gl.GetAttribLocation(p, c.as_ptr())
    };
    Ok(Prog { id: p, pos, tex: loc("tex"), halfpixel: loc("halfpixel"), offset: loc("offset") })
}

impl BlurPrograms {
    pub fn new(r: &mut GlesRenderer) -> Result<Self, GlesError> {
        let res = r.with_context(|gl| unsafe {
            let vs = include_str!("shaders/blur.vert");
            let down = link(gl, vs, include_str!("shaders/blur_down.frag"));
            let up = link(gl, vs, include_str!("shaders/blur_up.frag"));
            (down, up)
        })?;
        match res {
            (Ok(down), Ok(up)) => Ok(Self { down, up }),
            (Err(e), _) | (_, Err(e)) => {
                tracing::error!("aqua blur shader: {e}");
                Err(GlesError::ShaderCompileError)
            }
        }
    }
}

/// One mip level of the blur chain.
#[derive(Default, Clone, Copy)]
pub struct Level {
    pub tex: u32,
    pub fbo: u32,
    pub w: i32,
    pub h: i32,
}

#[derive(Default)]
pub struct BlurCache {
    pub levels: Vec<Level>,
    /// GL framebuffer-space rect of the capture (x, y, w, h).
    pub fb_rect: [f32; 4],
    pub axes: [f32; 4],
    pub result: u32,
    pub valid: bool,
}

/// GL objects of blur caches that were dropped (an element vanished, an offscreen
/// damage tracker went away). `Drop` has no GL context, so they are deleted the next
/// time one is current.
static DROPPED: Mutex<Vec<(u32, u32)>> = Mutex::new(Vec::new());

impl Drop for BlurCache {
    fn drop(&mut self) {
        let objs: Vec<(u32, u32)> = self.levels.iter().filter(|l| l.tex != 0).map(|l| (l.tex, l.fbo)).collect();
        if !objs.is_empty() {
            DROPPED.lock().unwrap_or_else(|e| e.into_inner()).extend(objs);
        }
    }
}

/// Number of dropped blur textures still waiting for deletion.
pub fn dropped_blurs() -> usize {
    DROPPED.lock().unwrap_or_else(|e| e.into_inner()).len()
}

unsafe fn delete_dropped(gl: &ffi::Gles2) {
    let objs = std::mem::take(&mut *DROPPED.lock().unwrap_or_else(|e| e.into_inner()));
    for (tex, fbo) in objs {
        gl.DeleteFramebuffers(1, &fbo);
        gl.DeleteTextures(1, &tex);
    }
}

/// Delete the GL objects of dropped blur caches (call with the renderer that made them).
pub fn free_dropped_blurs(r: &mut GlesRenderer) {
    if dropped_blurs() == 0 {
        return;
    }
    let _ = r.with_context(|gl| unsafe { delete_dropped(gl) });
}

unsafe fn alloc_level(gl: &ffi::Gles2, l: &mut Level, w: i32, h: i32) {
    if l.tex == 0 {
        gl.GenTextures(1, &mut l.tex);
        gl.GenFramebuffers(1, &mut l.fbo);
    }
    if l.w != w || l.h != h {
        gl.BindTexture(ffi::TEXTURE_2D, l.tex);
        gl.TexImage2D(ffi::TEXTURE_2D, 0, ffi::RGBA as i32, w, h, 0, ffi::RGBA, ffi::UNSIGNED_BYTE, std::ptr::null());
        set_params(gl);
        gl.BindFramebuffer(ffi::FRAMEBUFFER, l.fbo);
        gl.FramebufferTexture2D(ffi::FRAMEBUFFER, ffi::COLOR_ATTACHMENT0, ffi::TEXTURE_2D, l.tex, 0);
        l.w = w;
        l.h = h;
    }
}

unsafe fn set_params(gl: &ffi::Gles2) {
    gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MIN_FILTER, ffi::LINEAR as i32);
    gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MAG_FILTER, ffi::LINEAR as i32);
    gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_WRAP_S, ffi::CLAMP_TO_EDGE as i32);
    gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_WRAP_T, ffi::CLAMP_TO_EDGE as i32);
}

/// Capture `fb_rect` (GL coords) of the currently bound framebuffer and blur it.
pub unsafe fn capture_and_blur(
    gl: &ffi::Gles2,
    p: &BlurPrograms,
    cache: &mut BlurCache,
    rect: [i32; 4],
    passes: usize,
    offset: f32,
) {
    let mut fbo = 0;
    gl.GetIntegerv(ffi::FRAMEBUFFER_BINDING, &mut fbo);
    let mut vp = [0i32; 4];
    gl.GetIntegerv(ffi::VIEWPORT, vp.as_mut_ptr());
    let mut prog = 0;
    gl.GetIntegerv(ffi::CURRENT_PROGRAM, &mut prog);
    let mut active = 0;
    gl.GetIntegerv(ffi::ACTIVE_TEXTURE, &mut active);
    gl.ActiveTexture(ffi::TEXTURE0);
    let mut tex0 = 0;
    gl.GetIntegerv(ffi::TEXTURE_BINDING_2D, &mut tex0);
    let mut abuf = 0;
    gl.GetIntegerv(ffi::ARRAY_BUFFER_BINDING, &mut abuf);
    let blend = gl.IsEnabled(ffi::BLEND) != 0;
    let scissor = gl.IsEnabled(ffi::SCISSOR_TEST) != 0;

    delete_dropped(gl);
    let [rx, ry, rw, rh] = rect;
    if cache.levels.len() < passes + 1 {
        cache.levels.resize(passes + 1, Level::default());
    }
    let l0 = &mut cache.levels[0];
    if l0.tex == 0 {
        gl.GenTextures(1, &mut l0.tex);
        gl.GenFramebuffers(1, &mut l0.fbo);
    }
    gl.BindTexture(ffi::TEXTURE_2D, l0.tex);
    gl.CopyTexImage2D(ffi::TEXTURE_2D, 0, ffi::RGB, rx, ry, rw, rh, 0);
    set_params(gl);
    l0.w = rw;
    l0.h = rh;

    gl.Disable(ffi::BLEND);
    gl.Disable(ffi::SCISSOR_TEST);
    gl.BindBuffer(ffi::ARRAY_BUFFER, 0);
    let quad: [f32; 12] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0];

    let draw = |gl: &ffi::Gles2, pr: &Prog, src: Level, dst: Level| {
        gl.BindFramebuffer(ffi::FRAMEBUFFER, dst.fbo);
        gl.Viewport(0, 0, dst.w, dst.h);
        gl.UseProgram(pr.id);
        gl.BindTexture(ffi::TEXTURE_2D, src.tex);
        gl.Uniform1i(pr.tex, 0);
        gl.Uniform2f(pr.halfpixel, 0.5 / dst.w as f32, 0.5 / dst.h as f32);
        gl.Uniform1f(pr.offset, offset);
        gl.EnableVertexAttribArray(pr.pos as u32);
        gl.VertexAttribPointer(pr.pos as u32, 2, ffi::FLOAT, ffi::FALSE, 0, quad.as_ptr() as *const _);
        gl.DrawArrays(ffi::TRIANGLES, 0, 6);
        gl.DisableVertexAttribArray(pr.pos as u32);
    };
    for i in 1..=passes {
        let (w, h) = ((rw >> i).max(1), (rh >> i).max(1));
        let mut l = cache.levels[i];
        alloc_level(gl, &mut l, w, h);
        cache.levels[i] = l;
        draw(gl, &p.down, cache.levels[i - 1], cache.levels[i]);
    }
    for i in (1..passes).rev() {
        draw(gl, &p.up, cache.levels[i + 1], cache.levels[i]);
    }
    cache.result = cache.levels[passes.min(1)].tex;
    cache.valid = true;

    gl.BindFramebuffer(ffi::FRAMEBUFFER, fbo as u32);
    gl.Viewport(vp[0], vp[1], vp[2], vp[3]);
    gl.UseProgram(prog as u32);
    gl.BindTexture(ffi::TEXTURE_2D, tex0 as u32);
    gl.ActiveTexture(active as u32);
    gl.BindBuffer(ffi::ARRAY_BUFFER, abuf as u32);
    if blend {
        gl.Enable(ffi::BLEND);
    }
    if scissor {
        gl.Enable(ffi::SCISSOR_TEST);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_limits_reblurs_of_a_changing_backdrop() {
        let g = BlurGate::default();
        let t0 = Instant::now();
        let r = [0, 0, 100, 40];
        set_blur_max_fps(0);
        assert!(g.admit(r, true, t0));
        assert!(g.admit(r, true, t0), "no limit");
        set_blur_max_fps(20); // 50 ms
        assert!(!g.admit(r, true, t0 + Duration::from_millis(10)), "too soon: reuse the blur");
        assert!(g.is_stale());
        assert!(!g.recapture_due(t0 + Duration::from_millis(20)));
        assert!(g.recapture_due(t0 + Duration::from_millis(60)));
        assert!(g.admit(r, true, t0 + Duration::from_millis(60)));
        assert!(!g.is_stale() && !g.recapture_due(t0 + Duration::from_secs(1)));
        // A moved/resized glass or an empty cache is always blurred.
        assert!(g.admit([0, 0, 120, 40], true, t0 + Duration::from_millis(61)));
        assert!(g.admit([0, 0, 120, 40], false, t0 + Duration::from_millis(62)));
        set_blur_max_fps(0);
    }

    #[test]
    fn dropped_caches_queue_their_gl_objects() {
        let before = dropped_blurs();
        let mut c = BlurCache::default();
        c.levels = vec![Level { tex: 7, fbo: 9, w: 4, h: 4 }, Level::default()];
        drop(c);
        assert_eq!(dropped_blurs(), before + 1, "only allocated levels are queued");
        DROPPED.lock().unwrap().retain(|&(t, f)| (t, f) != (7, 9));
    }
}
