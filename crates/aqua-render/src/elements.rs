use crate::blur::{capture_and_blur, BlurCache, BlurGate};
use crate::Shaders;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{ffi, GlesError, GlesFrame, GlesRenderer, Uniform};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Physical, Point, Rectangle, Scale, Transform};
use std::cell::RefCell;

/// Material parameters in *physical* pixels (see `aqua_config::GlassStyle` for meanings).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassParams {
    pub radius: f32,
    pub roundness: f32,
    pub blur: f32,
    pub tint: [f32; 4],
    pub saturation: f32,
    pub max_luma: f32,
    pub thickness: f32,
    pub refraction: f32,
    pub ior: f32,
    pub dispersion: f32,
    /// The rim refracts the sharp backdrop, blurring in toward the flat top.
    pub sharp_rim: bool,
    /// Gain of the edge lights.
    pub rim: f32,
    pub fresnel: f32,
    /// Fresnel band falloff per physical px.
    pub fresnel_k: f32,
    pub fresnel_hardness: f32,
    /// LCh (hue in radians) of the Fresnel light colour.
    pub fresnel_lch: [f32; 3],
    pub glare: f32,
    /// Glare band falloff per physical px.
    pub glare_k: f32,
    pub glare_hardness: f32,
    pub glare_convergence: f32,
    pub glare_opposite: f32,
    /// Light angle in radians.
    pub glare_angle: f32,
}

/// Highlights scale with the length of the shape normal like the reference renderer does on a
/// typical canvas.
const NORMAL_GAIN: f32 = 1.25;

impl GlassParams {
    pub fn from_style(g: &aqua_style::GlassStyleLike, scale: f32) -> Self {
        let s = scale.max(0.01);
        // Band falloff of the reference model, `(500 / range)² / 1500` per logical px.
        let band = |range: f32| (500.0 / range.max(0.5)).powi(2) / 1500.0 / s;
        let a = g.tint[3] * 0.5;
        let f = [1.0 + (g.tint[0] - 1.0) * a, 1.0 + (g.tint[1] - 1.0) * a, 1.0 + (g.tint[2] - 1.0) * a];
        Self {
            radius: g.radius * s,
            roundness: g.roundness,
            blur: g.blur * s,
            tint: g.tint,
            saturation: g.saturation,
            max_luma: g.max_luma,
            thickness: g.thickness * s,
            refraction: g.refraction * s,
            ior: g.ior.max(1.0),
            dispersion: g.dispersion,
            sharp_rim: !g.blur_edge,
            rim: g.rim,
            fresnel: g.fresnel,
            fresnel_k: band(g.fresnel_range),
            fresnel_hardness: g.fresnel_hardness,
            fresnel_lch: color::srgb_to_lch(f),
            glare: g.glare,
            glare_k: band(g.glare_range),
            glare_hardness: g.glare_hardness,
            glare_convergence: g.glare_convergence,
            glare_opposite: g.glare_opposite,
            glare_angle: g.glare_angle.to_radians(),
        }
    }

    /// No refraction, no lights: the rim band can be skipped entirely.
    fn plain(&self) -> bool {
        self.refraction <= 0.0 && (self.rim <= 0.0 || (self.fresnel <= 0.0 && self.glare <= 0.0))
    }
}

/// Plain-data mirror of `aqua_config::GlassStyle` (keeps this crate free of config deps).
pub mod aqua_style {
    pub struct GlassStyleLike {
        pub radius: f32,
        pub roundness: f32,
        pub blur: f32,
        pub tint: [f32; 4],
        pub saturation: f32,
        pub max_luma: f32,
        pub thickness: f32,
        pub refraction: f32,
        pub ior: f32,
        pub dispersion: f32,
        pub blur_edge: bool,
        pub rim: f32,
        pub fresnel: f32,
        pub fresnel_range: f32,
        pub fresnel_hardness: f32,
        pub glare: f32,
        pub glare_range: f32,
        pub glare_hardness: f32,
        pub glare_convergence: f32,
        pub glare_opposite: f32,
        pub glare_angle: f32,
    }
}

/// sRGB → CIE LCh (D65), as in the shader.
pub mod color {
    fn lin(c: f32) -> f32 {
        if c > 0.04045 {
            ((c + 0.055) / 1.055).powf(2.4)
        } else {
            c / 12.92
        }
    }
    fn f(x: f32) -> f32 {
        if x > 0.008_856_452 {
            x.cbrt()
        } else {
            7.787_037 * x + 0.137_931_03
        }
    }
    /// `[L, C, h]` with the hue in radians.
    pub fn srgb_to_lch(c: [f32; 3]) -> [f32; 3] {
        let (r, g, b) = (lin(c[0]), lin(c[1]), lin(c[2]));
        let x = 0.4124 * r + 0.3576 * g + 0.1805 * b;
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let z = 0.0193 * r + 0.1192 * g + 0.9505 * b;
        let (fx, fy, fz) = (f(x / 0.950_455_9), f(y), f(z / 1.089_057_8));
        let (l, a, bb) = (116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz));
        [l, (a * a + bb * bb).sqrt(), bb.atan2(a)]
    }

    #[cfg(test)]
    #[test]
    fn white_and_black() {
        let w = srgb_to_lch([1.0, 1.0, 1.0]);
        assert!((w[0] - 100.0).abs() < 0.1 && w[1] < 0.5, "{w:?}");
        let k = srgb_to_lch([0.0, 0.0, 0.0]);
        assert!(k[0].abs() < 0.1);
    }
}

/// Corner extent and exponent for a `w`×`h` px shape (mirrors `aqua_config::material::corner`).
pub fn corner(w: f32, h: f32, r: f32, n: f32) -> (f32, f32) {
    let half = (w.min(h) * 0.5).max(0.0);
    let r = r.clamp(0.0, half);
    if r <= 0.0 || n <= 2.001 {
        return (r, 2.0);
    }
    const INSET: f32 = 1.0 - std::f32::consts::FRAC_1_SQRT_2;
    let k = INSET / (1.0 - 2f32.powf(-1.0 / n));
    let extent = (k * r).min(half);
    let t = 1.0 - INSET * r / extent;
    let n_eff = if t <= 0.0 { 2.0 } else { (-(2f32.ln()) / t.ln()).clamp(2.0, n) };
    (extent, n_eff)
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
        // clear glass: refract the sharp backdrop, no blur chain at all
        b if b < 0.5 => (0, 1.0),
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
        let (tex, sharp) = (c.result, c.levels.first().map(|l| l.tex).unwrap_or(c.result));
        frame.with_context(|gl| unsafe {
            gl.ActiveTexture(ffi::TEXTURE1);
            gl.BindTexture(ffi::TEXTURE_2D, tex);
            gl.ActiveTexture(ffi::TEXTURE2);
            gl.BindTexture(ffi::TEXTURE_2D, sharp);
            gl.ActiveTexture(ffi::TEXTURE0);
        })?;
        let p = &self.params;
        // `src` is in the element's own (unscaled) space, see `src()`: the shader's
        // texture space must be that size too. Using `dst.size` drew the glass at zoom²
        // inside a RescaleRenderElement (open / minimise / Mission Control animations), so
        // the frame behind server-side title bars shrank away from the window.
        let (w, h) = (self.geo.size.w.max(1), self.geo.size.h.max(1));
        let (extent, n) = corner(w as f32, h as f32, p.radius, p.roundness);
        // A plain frosted pane skips the rim band (zero thickness).
        let thickness = if p.plain() { 0.0 } else { p.thickness.max(0.0) };
        let res = frame.render_pixel_shader_to(
            &self.shaders.glass,
            src,
            dst,
            (w, h).into(),
            Some(damage),
            self.alpha,
            &[
                Uniform::new("blur_tex", 1i32),
                Uniform::new("sharp_tex", 2i32),
                Uniform::new("fb_rect", c.fb_rect),
                Uniform::new("axes", c.axes),
                Uniform::new("shape", [extent, n, thickness, 1.0 / p.ior.max(1.0)]),
                Uniform::new(
                    "refr",
                    [p.refraction, p.dispersion, if p.sharp_rim { 1.0 } else { 0.0 }, p.saturation],
                ),
                Uniform::new("fres", [p.fresnel_k, p.fresnel_hardness, p.fresnel, p.rim]),
                Uniform::new("glare", [p.glare_k, p.glare_hardness, p.glare, p.glare_convergence]),
                Uniform::new("glare2", [p.glare_opposite, p.glare_angle, p.max_luma, NORMAL_GAIN]),
                Uniform::new("tint", p.tint),
                Uniform::new("fres_lch", p.fresnel_lch),
            ],
        );
        frame.with_context(|gl| unsafe {
            gl.ActiveTexture(ffi::TEXTURE2);
            gl.BindTexture(ffi::TEXTURE_2D, 0);
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
    /// Report no opaque regions (the surface sits on compositor glass, which must see the
    /// windows below it: occlusion culling would leave a stale backdrop under the glass).
    see_through: bool,
}

impl<E: Element> RoundedElement<E> {
    pub fn new(inner: E, clip: Rectangle<i32, Physical>, radius: f32, shaders: &Shaders, scale: f64) -> Self {
        let base = inner.geometry(Scale::from(scale));
        Self { inner, clip, radius, shaders: shaders.clone(), base, see_through: false }
    }

    pub fn see_through(mut self, on: bool) -> Self {
        self.see_through = on;
        self
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
        if self.see_through {
            return OpaqueRegions::default();
        }
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
