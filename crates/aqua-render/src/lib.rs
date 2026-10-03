//! aqua-render: GPU effects for the Smithay GLES renderer.
//!
//! * [`GlassElement`] – real-time glass: captures the framebuffer behind
//!   itself (Smithay framebuffer-effect API, so it is only re-captured when the
//!   content behind changes), dual-kawase blurs it, and composites it with an
//!   edge-refraction lens, dispersion, saturation, tint and specular rim.
//! * [`RoundedElement`] – clips client surfaces to continuous rounded corners.
//! * [`shadow`] – SDF soft shadows for windows and panels.
//! * [`stats`] – frame/damage/blur profiling counters.

mod blur;
mod elements;
pub mod stats;

pub use blur::{dropped_blurs, free_dropped_blurs, set_blur_max_fps, set_blur_unthrottled, BlurGate};
pub use elements::aqua_style;
pub use elements::{GlassElement, GlassParams, RoundedElement};

use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::{
    element::PixelShaderElement, GlesError, GlesPixelProgram, GlesRenderer, GlesTexProgram, Uniform, UniformName,
    UniformType,
};
use smithay::utils::{Logical, Rectangle};
use std::rc::Rc;

/// All compiled programs (create once per renderer).
#[derive(Clone)]
pub struct Shaders {
    pub glass: GlesPixelProgram,
    pub shadow: GlesPixelProgram,
    pub rounded: GlesTexProgram,
    pub genie: GlesTexProgram,
    pub blur: Rc<blur::BlurPrograms>,
}

impl Shaders {
    pub fn new(renderer: &mut GlesRenderer) -> Result<Self, GlesError> {
        let glass = renderer.compile_custom_pixel_shader(
            include_str!("shaders/glass.frag"),
            &[
                UniformName::new("blur_tex", UniformType::_1i),
                UniformName::new("fb_rect", UniformType::_4f),
                UniformName::new("axes", UniformType::_4f),
                UniformName::new("radius", UniformType::_1f),
                UniformName::new("tint", UniformType::_4f),
                UniformName::new("saturation", UniformType::_1f),
                UniformName::new("refraction", UniformType::_1f),
                UniformName::new("bevel", UniformType::_1f),
                UniformName::new("rim", UniformType::_1f),
                UniformName::new("max_luma", UniformType::_1f),
            ],
        )?;
        let shadow = renderer.compile_custom_pixel_shader(
            include_str!("shaders/shadow.frag"),
            &[
                UniformName::new("rect", UniformType::_4f),
                UniformName::new("hole", UniformType::_4f),
                UniformName::new("radius", UniformType::_1f),
                UniformName::new("sigma", UniformType::_1f),
                UniformName::new("strength", UniformType::_1f),
            ],
        )?;
        let rounded = renderer.compile_custom_texture_shader(
            include_str!("shaders/rounded.frag"),
            &[
                UniformName::new("elem_size", UniformType::_2f),
                UniformName::new("clip_rect", UniformType::_4f),
                UniformName::new("radius", UniformType::_1f),
            ],
        )?;
        let genie = renderer.compile_custom_texture_shader(
            include_str!("shaders/genie.frag"),
            &[
                UniformName::new("box", UniformType::_2f),
                UniformName::new("win", UniformType::_4f),
                UniformName::new("tgt", UniformType::_4f),
                UniformName::new("clip_rect", UniformType::_4f),
                UniformName::new("radius", UniformType::_1f),
                UniformName::new("progress", UniformType::_1f),
            ],
        )?;
        let blur = Rc::new(blur::BlurPrograms::new(renderer)?);
        Ok(Self { glass, shadow, rounded, genie, blur })
    }

    /// Drop shadow around a rounded logical rect `r`.
    pub fn shadow(
        &self,
        r: Rectangle<i32, Logical>,
        radius: f32,
        sigma: f32,
        dy: f32,
        strength: f32,
        scale: f64,
    ) -> PixelShaderElement {
        let pad = (sigma * 2.5 + dy.abs()).ceil() as i32;
        let area =
            Rectangle::new((r.loc.x - pad, r.loc.y - pad).into(), (r.size.w + 2 * pad, r.size.h + 2 * pad).into());
        let s = scale as f32;
        let p = pad as f32 * s;
        let (w, h) = (r.size.w as f32 * s, r.size.h as f32 * s);
        PixelShaderElement::new(
            self.shadow.clone(),
            area,
            None,
            1.0,
            vec![
                Uniform::new("rect", [p, p + dy * s, w, h]),
                Uniform::new("hole", [p, p, w, h]),
                Uniform::new("radius", radius * s),
                Uniform::new("sigma", sigma * s),
                Uniform::new("strength", strength),
            ],
            Kind::Unspecified,
        )
    }
}

impl Shaders {
    /// Genie-effect element.
    #[allow(clippy::too_many_arguments)]
    pub fn genie(
        &self,
        inner: smithay::backend::renderer::element::texture::TextureRenderElement<
            smithay::backend::renderer::gles::GlesTexture,
        >,
        box_px: [f32; 2],
        win: [f32; 4],
        tgt: [f32; 4],
        clip: [f32; 4],
        radius: f32,
        progress: f32,
    ) -> smithay::backend::renderer::gles::element::TextureShaderElement {
        smithay::backend::renderer::gles::element::TextureShaderElement::new(
            inner,
            self.genie.clone(),
            vec![
                Uniform::new("box", box_px),
                Uniform::new("win", win),
                Uniform::new("tgt", tgt),
                Uniform::new("clip_rect", clip),
                Uniform::new("radius", radius),
                Uniform::new("progress", progress),
            ],
        )
    }
}
