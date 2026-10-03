use crate::blur::{capture_and_blur, BlurCache, BlurGate};
use crate::Shaders;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{ffi, GlesError, GlesFrame, GlesRenderer, Uniform};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Physical, Point, Rectangle, Scale, Transform};
use std::cell::RefCell;

/// Material parameters in *physical* pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassParams {
    pub radius: f32,
    pub blur: f32,
    pub tint: [f32; 4],
    pub saturation: f32,
    pub refraction: f32,
    pub bevel: f32,
    pub rim: f32,
    pub max_luma: f32,
}

impl GlassParams {
    pub fn from_style(g: &aqua_style::GlassStyleLike, scale: f32) -> Self {
        Self {
            radius: g.radius * scale,
            blur: g.blur * scale,
            tint: g.tint,
            saturation: g.saturation,
            refraction: g.refraction * scale,
            bevel: g.bevel * scale,
            rim: g.rim,
            max_luma: g.max_luma,
        }
    }
}

/// Plain-data mirror of `aqua_config::GlassStyle` (keeps this crate free of config deps).
pub mod aqua_style {
    pub struct GlassStyleLike {
        pub radius: f32,
        pub blur: f32,
        pub tint: [f32; 4],
        pub saturation: f32,
        pub refraction: f32,
        pub bevel: f32,
        pub rim: f32,
        pub max_luma: f32,
    }
}

pub struct GlassElement {
    id: Id,
    geo: Rectangle<i32, Physical>,
    params: GlassParams,
    alpha: f32,
    shaders: Shaders,
    commit: CommitCounter,
    gate: Option<std::sync::Arc<BlurGate>>,
}

impl GlassElement {
    /// `id` must be stable across frames for the backdrop cache to work.
    pub fn new(
        id: Id,
        geo: Rectangle<i32, Physical>,
        params: GlassParams,
        alpha: f32,
        shaders: &Shaders,
        commit: CommitCounter,
    ) -> Self {
        Self { id, geo, params, alpha, shaders: shaders.clone(), commit, gate: None }
    }

    /// Rate-limit re-blurring through `gate` (see [`crate::set_blur_max_fps`]).
    pub fn with_gate(mut self, gate: std::sync::Arc<BlurGate>) -> Self {
        self.gate = Some(gate);
        self
    }
}

impl Element for GlassElement {
    fn id(&self) -> &Id {
        &self.id
    }
    fn current_commit(&self) -> CommitCounter {
        self.commit
    }
    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::from_size((self.geo.size.w as f64, self.geo.size.h as f64).into())
    }
    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geo
    }
    fn alpha(&self) -> f32 {
        self.alpha
    }
    fn is_framebuffer_effect(&self) -> bool {
        true
    }
}

fn fb_mapping(proj: &[f32; 9], vp: [i32; 4], dst: Rectangle<i32, Physical>) -> ([i32; 4], [f32; 4]) {
    let m = proj;
    let map = |x: f32, y: f32| {
        let nx = m[0] * x + m[3] * y + m[6];
        let ny = m[1] * x + m[4] * y + m[7];
        (vp[0] as f32 + (nx + 1.0) * 0.5 * vp[2] as f32, vp[1] as f32 + (ny + 1.0) * 0.5 * vp[3] as f32)
    };
    let (x0, y0) = (dst.loc.x as f32, dst.loc.y as f32);
    let (x1, y1) = (x0 + dst.size.w as f32, y0 + dst.size.h as f32);
    let pts = [map(x0, y0), map(x1, y0), map(x0, y1), map(x1, y1)];
    let minx = pts.iter().map(|p| p.0).fold(f32::MAX, f32::min).max(vp[0] as f32);
    let maxx = pts.iter().map(|p| p.0).fold(f32::MIN, f32::max).min((vp[0] + vp[2]) as f32);
    let miny = pts.iter().map(|p| p.1).fold(f32::MAX, f32::min).max(vp[1] as f32);
    let maxy = pts.iter().map(|p| p.1).fold(f32::MIN, f32::max).min((vp[1] + vp[3]) as f32);
    let rect = [
        minx.round() as i32,
        miny.round() as i32,
        ((maxx - minx).round() as i32).max(1),
        ((maxy - miny).round() as i32).max(1),
    ];
    let axes =
        [m[0] * vp[2] as f32 * 0.5, m[1] * vp[3] as f32 * 0.5, m[3] * vp[2] as f32 * 0.5, m[4] * vp[3] as f32 * 0.5];
    (rect, axes)
}

fn passes_for(blur: f32) -> (usize, f32) {
    match blur {
        b if b < 4.0 => (1, 1.0),
        b if b < 12.0 => (2, 1.5),
        b if b < 30.0 => (3, 2.0),
        b if b < 70.0 => (4, 2.5),
        _ => (5, 3.0),
    }
}

impl RenderElement<GlesRenderer> for GlassElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        let Some(c) = cache.and_then(|c| c.get::<RefCell<BlurCache>>()) else { return Ok(()) };
        let c = c.borrow();
        if !c.valid {
            return Ok(());
        }
        let tex = c.result;
        frame.with_context(|gl| unsafe {
            gl.ActiveTexture(ffi::TEXTURE1);
            gl.BindTexture(ffi::TEXTURE_2D, tex);
            gl.ActiveTexture(ffi::TEXTURE0);
        })?;
        let p = &self.params;
        let size = (dst.size.w, dst.size.h).into();
        let res = frame.render_pixel_shader_to(
            &self.shaders.glass,
            src,
            dst,
            size,
            Some(damage),
            self.alpha,
            &[
                Uniform::new("blur_tex", 1i32),
                Uniform::new("fb_rect", c.fb_rect),
                Uniform::new("axes", c.axes),
                Uniform::new("radius", p.radius),
                Uniform::new("tint", p.tint),
                Uniform::new("saturation", p.saturation),
                Uniform::new("refraction", p.refraction),
                Uniform::new("bevel", p.bevel),
                Uniform::new("rim", p.rim),
                Uniform::new("max_luma", p.max_luma),
            ],
        );
        frame.with_context(|gl| unsafe {
            gl.ActiveTexture(ffi::TEXTURE1);
            gl.BindTexture(ffi::TEXTURE_2D, 0);
            gl.ActiveTexture(ffi::TEXTURE0);
        })?;
        res
    }

    fn capture_framebuffer(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        cache: &UserDataMap,
    ) -> Result<(), GlesError> {
        cache.insert_if_missing(|| RefCell::new(BlurCache::default()));
        let cell = cache.get::<RefCell<BlurCache>>().unwrap();
        let proj = *frame.projection();
        let blur = self.shaders.blur.clone();
        let (passes, offset) = passes_for(self.params.blur);
        frame.with_context(|gl| unsafe {
            let mut vp = [0i32; 4];
            gl.GetIntegerv(ffi::VIEWPORT, vp.as_mut_ptr());
            let (rect, axes) = fb_mapping(&proj, vp, dst);
            let mut c = cell.borrow_mut();
            let px = rect[2].max(0) as u64 * rect[3].max(0) as u64;
            let same = c.fb_rect == [rect[0] as f32, rect[1] as f32, rect[2] as f32, rect[3] as f32];
            if let Some(g) = &self.gate {
                if !g.admit(rect, c.valid && same, std::time::Instant::now()) {
                    crate::stats::record_blur(px, true);
                    return;
                }
            }
            crate::stats::record_blur(px, false);
            c.fb_rect = [rect[0] as f32, rect[1] as f32, rect[2] as f32, rect[3] as f32];
            c.axes = axes;
            capture_and_blur(gl, &blur, &mut c, rect, passes, offset);
        })
    }
}

/// Client surface clipped to a rounded rectangle (window corners).
pub struct RoundedElement<E = WaylandSurfaceRenderElement<GlesRenderer>> {
    inner: E,
    /// Clip rect in output physical coordinates.
    clip: Rectangle<i32, Physical>,
    radius: f32,
    shaders: Shaders,
    /// Unscaled element geometry, so the clip follows when wrapped in a rescale element.
    base: Rectangle<i32, Physical>,
}

impl<E: Element> RoundedElement<E> {
    pub fn new(inner: E, clip: Rectangle<i32, Physical>, radius: f32, shaders: &Shaders, scale: f64) -> Self {
        let base = inner.geometry(Scale::from(scale));
        Self { inner, clip, radius, shaders: shaders.clone(), base }
    }
}

impl<E: Element> Element for RoundedElement<E> {
    fn id(&self) -> &Id {
        self.inner.id()
    }
    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }
    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        self.inner.location(scale)
    }
    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }
    fn transform(&self) -> Transform {
        self.inner.transform()
    }
    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }
    fn damage_since(&self, scale: Scale<f64>, commit: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        self.inner.damage_since(scale, commit)
    }
    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        let g = self.inner.geometry(scale);
        let r = self.radius.ceil() as i32;
        let local_clip = Rectangle::new(self.clip.loc - g.loc, self.clip.size);
        let inner_rect = Rectangle::new(
            (local_clip.loc.x, local_clip.loc.y + r).into(),
            (local_clip.size.w, (local_clip.size.h - 2 * r).max(0)).into(),
        );
        self.inner.opaque_regions(scale).into_iter().filter_map(|o| o.intersection(inner_rect)).collect()
    }
    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }
    fn kind(&self) -> Kind {
        self.inner.kind()
    }
}

impl<E: RenderElement<GlesRenderer>> RenderElement<GlesRenderer> for RoundedElement<E> {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        let kx = if self.base.size.w > 0 { dst.size.w as f32 / self.base.size.w as f32 } else { 1.0 };
        let ky = if self.base.size.h > 0 { dst.size.h as f32 / self.base.size.h as f32 } else { 1.0 };
        let rel = self.clip.loc - self.base.loc;
        frame.override_default_tex_program(
            self.shaders.rounded.clone(),
            vec![
                Uniform::new("elem_size", (dst.size.w as f32, dst.size.h as f32)),
                Uniform::new(
                    "clip_rect",
                    [rel.x as f32 * kx, rel.y as f32 * ky, self.clip.size.w as f32 * kx, self.clip.size.h as f32 * ky],
                ),
                Uniform::new("radius", self.radius * kx.min(ky)),
            ],
        );
        let r = self.inner.draw(frame, src, dst, damage, opaque_regions, cache);
        frame.clear_tex_program_override();
        r
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}
