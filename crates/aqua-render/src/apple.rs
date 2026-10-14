//! Apple's Liquid Glass, run as is.
//!
//! The fragment shader is QuartzCore's own `glass_background_sdf_lpf` (macOS 26.7.1), translated
//! instruction for instruction from its AIR bitcode (linuxtosh `tools/air2glsl.py`, verified
//! bit-exact against the original on the GPU) and then mechanically to GLSL ES 3.00
//! (`tools/glsl2es.py`). It reads CoreAnimation's `GlassBackgroundUniforms` at Apple's byte
//! offsets; [`GlassInputs`] holds the `glassBackground` filter inputs captured from AppKit and
//! [`pack`] turns them into that block.
//!
//! The backdrop is a mip pyramid (level n ≈ blur radius 2ⁿ px) because Apple's shader picks its
//! blur with `textureLod(log2(radius))`.
use smithay::backend::renderer::gles::{ffi, GlesError, GlesRenderer};
use std::ffi::CString;

/// AppKit `_variant` groups that aqua uses (see linuxtosh `shaders/liquid-glass/parameters.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Variant {
    /// v0 Regular (also Clear 1, Identity 12, Sidebar 16, toolbar items).
    #[default]
    Regular,
    /// v2 Dock (also FaceTime 7, Control Center 8, AppIcons 3).
    Dock,
    /// v8 Control Center modules.
    ControlCenter,
    /// v16 sidebar (NSContainerConcentricGlassEffectView, inset 8).
    Sidebar,
    /// v18 inspector.
    Inspector,
    /// v20 popover.
    Popover,
    /// NSPopupMenuWindow: context menus, pop-up button menus (Regular rules, denser face).
    Menu,
    /// NSAlert panel.
    Alert,
    /// Window / HUD background: `gaussianBlur 30` backdrop, no lens.
    Flat,
    /// Progressive backdrop blur around menu-bar popovers (Control Center, Notification
    /// Center, menu-bar menus): full blur inside, fading to the sharp backdrop over
    /// [`VEIL_FADE`] points at the edge, no tint, no lens, no rim (linuxtosh code 210).
    Veil,
    /// A clear lens over the app's own pixels (toolbar capsules of Aqua's apps): only the
    /// rim bends and catches light, the face passes the glyphs under it untouched.
    Lens,
}

/// Width of the [`Variant::Veil`] fade, points.
pub const VEIL_FADE: f32 = 52.0;

impl Variant {
    pub fn from_code(c: u8) -> Self {
        match c {
            2 | 3 | 7 => Variant::Dock,
            8 => Variant::ControlCenter,
            16 => Variant::Sidebar,
            18 => Variant::Inspector,
            20 => Variant::Popover,
            100 => Variant::Menu,
            101 => Variant::Alert,
            200 => Variant::Flat,
            210 => Variant::Veil,
            220 => Variant::Lens,
            _ => Variant::Regular,
        }
    }
}

/// CoreImage-style colour matrix inputs (`inputXColorMatrix{Black,White,Saturation,FillColor}`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorMatrix {
    pub black: f32,
    pub white: f32,
    pub saturation: f32,
    pub fill: [f32; 4],
}

impl ColorMatrix {
    pub const IDENTITY: Self = Self { black: 0.0, white: 1.0, saturation: 1.0, fill: [0.0; 4] };

    /// Rows `[r, g, b, offset]`: saturate (Rec. 709 luma), map 0..1 to black..white, then
    /// composite the fill colour over it.
    pub fn rows(&self) -> [[f32; 4]; 3] {
        const L: [f32; 3] = [0.2126, 0.7152, 0.0722];
        let s = self.saturation;
        let a = self.fill[3].clamp(0.0, 1.0);
        let k = (1.0 - a) * (self.white - self.black);
        let mut m = [[0.0; 4]; 3];
        for (i, row) in m.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().take(3).enumerate() {
                let sat = (1.0 - s) * L[j] + if i == j { s } else { 0.0 };
                *v = k * sat;
            }
            row[3] = (1.0 - a) * self.black + a * self.fill[i];
        }
        m
    }
}

/// Specular rim (`CASDFKeyFillHighlightEffect`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Highlight {
    pub key_amount: f32,
    pub key_angle: f32,
    pub fill_amount: f32,
    pub fill_angle: f32,
    /// Band height in points.
    pub height: f32,
    pub curvature: f32,
    /// Rim model (linuxtosh reconstruction, see `glass_background_sdf.frag` wrapper):
    /// `Additive` = key/fill light added on top (Dock, menus, windows);
    /// `SoftLight` = Control Center modules: a 1 pt top-left bevel blended in soft light so
    /// the backdrop shows through, a dim penumbra bottom-right.
    pub rim: Rim,
}

/// How [`Highlight`] is composited onto the glass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Rim {
    Additive,
    /// `white`: straight white mixed over the soft-light result (0.15–0.30);
    /// `shade`: darkening of the edge facing away from the light.
    SoftLight { white: f32, shade: f32 },
}

impl Highlight {
    pub const MAC: Self = Self {
        key_amount: 0.5,
        key_angle: -std::f32::consts::FRAC_PI_4,
        fill_amount: 0.5,
        fill_angle: 3.0 * std::f32::consts::FRAC_PI_4,
        height: 1.0,
        curvature: 0.7,
        rim: Rim::Additive,
    };
    pub const NONE: Self = Self { key_amount: 0.0, fill_amount: 0.0, ..Self::MAC };
    /// Control Center modules (fitted to the macOS 26 Control Center capture).
    pub const CONTROL_CENTER: Self =
        Self { fill_amount: 1.0, key_amount: 0.5, rim: Rim::SoftLight { white: 0.22, shade: 0.35 }, ..Self::MAC };
}

/// `glassBackground` filter inputs, lengths in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlassInputs {
    pub blur_radius: f32,
    pub blur_opacity: [f32; 4],
    pub blur_distance: [f32; 4],
    pub inner_refraction_amount: f32,
    pub inner_refraction_height: f32,
    pub outer_refraction_amount: f32,
    pub outer_refraction_height: f32,
    pub refraction_distance: [f32; 2],
    pub refraction_opacity: f32,
    pub bleed_amount: f32,
    pub bleed_height: f32,
    pub bleed_blur_radius: f32,
    pub bleed_opacity: f32,
    pub bleed_distance: [f32; 2],
    pub bleed_darken: bool,
    pub bleed: ColorMatrix,
    pub shadow_amount: f32,
    pub shadow_height: f32,
    pub shadow_offset: [f32; 2],
    pub shadow_blur_radius: f32,
    pub shadow_radius: f32,
    pub shadow_opacity: f32,
    pub shadow_distance_offset: f32,
    pub shadow_contribution: f32,
    pub shadow: ColorMatrix,
    pub face: ColorMatrix,
    pub face_opacity: f32,
    pub clamp: f32,
    pub clamp_preserve_hue: bool,
    pub highlight: Highlight,
}

impl GlassInputs {
    /// The filter inputs AppKit sets for `variant` at `w`×`h` points (size rules fitted to the
    /// captures, exact at the captured sizes).
    pub fn mac(variant: Variant, dark: bool, w: f32, h: f32) -> Self {
        let m = w.min(h).max(1.0);
        let white = |a: f32| [1.0, 1.0, 1.0, a];
        let black = |a: f32| [0.0, 0.0, 0.0, a];
        let regular_face = if dark {
            ColorMatrix { black: 0.2, white: 0.6, saturation: 1.0, fill: black(0.35) }
        } else {
            ColorMatrix { black: 0.5, white: 0.95, saturation: 1.0, fill: white(0.35) }
        };
        let thin_face = if dark {
            ColorMatrix { black: 0.05, white: 0.8, saturation: 1.0, fill: white(0.05) }
        } else {
            ColorMatrix { black: 0.2, white: 0.95, saturation: 1.0, fill: white(0.10) }
        };
        let bleed = if dark {
            ColorMatrix { black: 0.0, white: 0.5, saturation: 1.0, fill: [0.0; 4] }
        } else {
            ColorMatrix { black: 0.9, white: 1.0, saturation: 1.2, fill: [0.0; 4] }
        };
        let shadow = if dark {
            ColorMatrix { black: 0.0, white: 0.5, saturation: 1.0, fill: black(0.12) }
        } else {
            ColorMatrix { black: 0.0, white: 1.0, saturation: 1.8, fill: black(0.12) }
        };
        let mut g = Self {
            blur_radius: (2.0 + (m - 48.0) / 56.0).clamp(2.0, 4.0),
            blur_opacity: [1.0; 4],
            blur_distance: [0.0; 4],
            inner_refraction_amount: -(0.8 * m).min(60.0),
            inner_refraction_height: (0.25 * m).min(20.0),
            outer_refraction_amount: 0.0,
            outer_refraction_height: 0.0,
            refraction_distance: [-1.0, 0.0],
            refraction_opacity: 0.0,
            bleed_amount: 0.35 * m,
            bleed_height: 0.35 * m,
            bleed_blur_radius: 0.0,
            bleed_opacity: 0.0,
            bleed_distance: [1.0, 0.0],
            bleed_darken: !dark,
            bleed,
            shadow_amount: (0.625 * m).min(75.0),
            shadow_height: 0.4 * m,
            shadow_offset: [0.0, 8.0],
            shadow_blur_radius: 0.0,
            shadow_radius: 0.0,
            shadow_opacity: 0.0,
            shadow_distance_offset: 0.0,
            shadow_contribution: 0.0,
            shadow,
            face: regular_face,
            face_opacity: 1.0,
            clamp: 1.0,
            clamp_preserve_hue: false,
            highlight: Highlight::MAC,
        };
        let dock_class = |g: &mut Self| {
            g.blur_radius = (5.0 + (m - 48.0) * 5.0 / 72.0).clamp(5.0, 10.0);
            g.bleed_amount = 0.0;
            g.bleed_height = 0.0;
            g.face = thin_face;
            if !dark {
                g.bleed.black = 0.75;
                g.shadow.fill = black(0.10);
                g.shadow.saturation = 1.2;
            }
        };
        match variant {
            Variant::Regular | Variant::Sidebar => {}
            Variant::Dock | Variant::ControlCenter => {
                dock_class(&mut g);
                g.inner_refraction_height = (0.36 * m).min(20.0);
                g.inner_refraction_amount = -m.min(60.0);
                if variant == Variant::ControlCenter {
                    g.highlight = Highlight::CONTROL_CENTER;
                }
            }
            Variant::Inspector => {
                dock_class(&mut g);
                g.refraction_opacity = 0.3;
                g.shadow_offset = [-1.0, 0.0];
            }
            Variant::Popover => {
                dock_class(&mut g);
                g.refraction_opacity = 0.3;
                g.inner_refraction_height = 5.0;
            }
            Variant::Menu => {
                g.face = if dark {
                    ColorMatrix { black: 0.2, white: 0.6, saturation: 1.0, fill: black(0.40) }
                } else {
                    ColorMatrix { black: 0.5, white: 1.03, saturation: 1.0, fill: white(0.40) }
                };
            }
            Variant::Alert => {}
            Variant::Flat => {
                g.blur_radius = 30.0;
                g.inner_refraction_amount = 0.0;
                g.bleed_amount = 0.0;
                g.bleed_height = 0.0;
                g.face = ColorMatrix::IDENTITY;
                g.highlight = Highlight::NONE;
            }
            Variant::Veil => {
                g.blur_radius = 10.0;
                // blur radius as a piecewise-linear ramp of the distance to the outline: the
                // pyramid level is log2(radius), so a roughly exponential ramp reads as an even
                // fade instead of a blurred block with a sharp rim.
                g.blur_opacity = [1.0, 0.45, 0.15, 0.0];
                g.blur_distance = [-VEIL_FADE, -0.6 * VEIL_FADE, -0.3 * VEIL_FADE, 0.0];
                g.inner_refraction_amount = 0.0;
                g.bleed_amount = 0.0;
                g.bleed_height = 0.0;
                g.shadow_amount = 0.0;
                g.shadow_height = 0.0;
                g.face = ColorMatrix::IDENTITY;
                g.highlight = Highlight::NONE;
            }
            Variant::Lens => {
                g.blur_radius = 1.0;
                g.inner_refraction_amount = -(0.4 * m).min(16.0);
                g.inner_refraction_height = (0.14 * m).min(6.0);
                g.bleed_amount = 0.0;
                g.bleed_height = 0.0;
                g.shadow_amount = 0.0;
                g.shadow_height = 0.0;
                g.face = ColorMatrix::IDENTITY;
            }
        }
        g
    }
}

/// Size of Apple's uniform block (`SdfFragmentUniforms` + `GlassBackgroundUniforms`).
pub const UBO_SIZE: usize = 320;

/// Everything [`pack`] needs besides the filter inputs.
pub struct Frame {
    /// Shape half size and corner radius, physical px.
    pub half: [f32; 2],
    pub radius: f32,
    /// Shape-space normal → backdrop-uv matrix (`displacement_mat`).
    pub displacement: [f32; 4],
    /// Points → physical px.
    pub scale: f32,
}

fn put_f(b: &mut [u8; UBO_SIZE], off: usize, v: f32) {
    b[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

fn put_h(b: &mut [u8; UBO_SIZE], off: usize, v: f32) {
    b[off..off + 2].copy_from_slice(&f32_to_f16(v).to_le_bytes());
}

fn f32_to_f16(v: f32) -> u16 {
    let x = v.to_bits();
    let sign = ((x >> 16) & 0x8000) as u16;
    let exp = ((x >> 23) & 0xff) as i32 - 127 + 15;
    let mant = x & 0x7f_ffff;
    if exp <= 0 {
        if exp < -10 {
            return sign;
        }
        let m = (mant | 0x80_0000) >> (1 - exp);
        return sign | ((m + 0x1000) >> 13) as u16;
    }
    if exp >= 31 {
        return sign | 0x7c00;
    }
    let h = sign as u32 | ((exp as u32) << 10) | (mant >> 13);
    // round to nearest
    (h + ((mant >> 12) & 1)) as u16
}

fn inv(x: f32) -> f32 {
    if x.abs() < 1e-6 {
        1e6
    } else {
        1.0 / x
    }
}

/// Apple's 320-byte uniform block for `glass_background_sdf`.
pub fn pack(g: &GlassInputs, f: &Frame) -> [u8; UBO_SIZE] {
    let mut b = [0u8; UBO_SIZE];
    let s = f.scale.max(0.01);
    // SdfFragmentUniforms: mode 4 = supercircle (continuous corners), identity normal transform.
    // Words 8/9 blend each axis from the continuous corner (0) to a circular arc (1) when the
    // shape is too short for the 1.528 r continuous extent - QuartzCore's own rule (mode 5 code
    // path: clamp((1.528665 - half / r) * 1.8915572)). Leaving them 0 squashed circles and
    // capsules into lumpy squircles with a kinked SDF.
    let t = |h: f32| if f.radius > 0.0 { ((1.528665 - h / f.radius) * 1.891_557_2).clamp(0.0, 1.0) } else { 0.0 };
    let (tx, ty) = (t(f.half[0]), t(f.half[1]));
    for (i, v) in [f.half[0], f.half[1], 4.0, 0.0, 1.0, 0.0, 0.0, 1.0, tx, ty, f.radius, 0.0].iter().enumerate() {
        put_f(&mut b, i * 4, *v);
    }
    let o = 48;
    for (i, v) in f.displacement.iter().enumerate() {
        put_f(&mut b, o + i * 4, *v);
    }
    put_f(&mut b, o + 16, g.inner_refraction_amount * s);
    put_f(&mut b, o + 20, inv(g.inner_refraction_height * s));
    put_f(&mut b, o + 24, g.outer_refraction_amount * s);
    put_f(&mut b, o + 28, inv(g.outer_refraction_height * s));
    let (t0, t1) = (g.refraction_distance[0] * s, g.refraction_distance[1] * s);
    put_f(&mut b, o + 32, t0);
    put_f(&mut b, o + 36, if (t1 - t0).abs() < 1e-4 { t0 + 1e-3 } else { t1 });
    put_f(&mut b, o + 40, g.blur_radius * s);
    put_f(&mut b, o + 44, g.bleed_blur_radius * s);
    put_f(&mut b, o + 48, g.bleed_amount * s);
    put_f(&mut b, o + 52, inv(g.bleed_height * s));
    put_f(&mut b, o + 56, g.shadow_amount * s);
    put_f(&mut b, o + 60, inv(g.shadow_height * s));
    put_f(&mut b, o + 64, g.shadow_offset[0] * s);
    put_f(&mut b, o + 68, g.shadow_offset[1] * s);
    put_f(&mut b, o + 72, g.shadow_blur_radius * s);
    put_f(&mut b, o + 76, if g.shadow_radius > 0.0 { 1.0 / (g.shadow_radius * s) } else { 0.0 });
    for (k, cm) in [(80, &g.face), (104, &g.bleed), (128, &g.shadow)] {
        for (r, row) in cm.rows().iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                put_h(&mut b, o + k + r * 8 + c * 2, *v);
            }
        }
    }
    put_f(&mut b, o + 152, g.shadow_contribution);
    put_f(&mut b, o + 156, g.face_opacity);
    // blur ramp: alpha0 - Σ alpha_i · ramp(d; dist_{i-1}, dist_i)
    put_f(&mut b, o + 160, g.blur_opacity[0]);
    let mut last = f32::MIN;
    for i in 0..4 {
        if i > 0 {
            put_f(&mut b, o + 160 + i * 4, g.blur_opacity[i - 1] - g.blur_opacity[i]);
        }
        let mut d = g.blur_distance[i] * s;
        if d <= last {
            d = last + 1e-3;
        }
        last = d;
        put_f(&mut b, o + 176 + i * 4, d);
    }
    let (b0, b1) = (g.bleed_distance[0] * s, g.bleed_distance[1] * s);
    put_f(&mut b, o + 192, b0);
    put_f(&mut b, o + 196, if (b1 - b0).abs() < 1e-4 { b0 - 1e-3 } else { b1 });
    put_f(&mut b, o + 200, g.bleed_opacity);
    put_f(&mut b, o + 204, g.face_opacity);
    let (dx, dy) = if g.bleed_darken { (-1.0, 1.0) } else { (0.0, 1.0) };
    put_f(&mut b, o + 208, dx);
    put_f(&mut b, o + 212, dy);
    put_f(&mut b, o + 216, g.shadow_distance_offset * s);
    put_f(&mut b, o + 220, g.shadow_opacity);
    put_f(&mut b, o + 224, g.refraction_opacity);
    put_f(&mut b, o + 228, 0.0); // SDR holding tone off (as captured)
    put_f(&mut b, o + 232, 0.0);
    put_f(&mut b, o + 236, 1e-3);
    put_f(&mut b, o + 240, g.clamp);
    put_f(&mut b, o + 244, if g.clamp_preserve_hue { 1.0 } else { 0.0 });
    put_f(&mut b, o + 248, 1.0);
    put_f(&mut b, o + 252, 0.0);
    // Apple's shader evaluates the blur ramp only on its refraction path, so a progressive
    // blur (Veil) must take that path even with zero displacement.
    let ramp = g.blur_opacity.iter().any(|a| (a - g.blur_opacity[0]).abs() > 1e-4);
    let refracts = g.inner_refraction_amount != 0.0 || g.outer_refraction_amount != 0.0 || ramp;
    put_f(&mut b, o + 256, if refracts { 1.0 } else { 0.0 });
    b
}

/// Highest pyramid level Apple's shader can ask for at `radius_px` (`log2`, see the shader).
pub fn levels_for(radius_px: f32) -> usize {
    let r = if radius_px < 2.0 { radius_px * 0.5 + 1.0 } else { radius_px };
    (r.max(1.0).log2().ceil() as usize + 1).clamp(1, 7)
}

pub struct Programs {
    pub glass: u32,
    u_proj: i32,
    u_vp: i32,
    u_fb_rect: i32,
    u_dst: i32,
    u_sdf_size: i32,
    u_alpha: i32,
    u_hl: i32,
    u_hl2: i32,
    u_hl3: i32,
    tex: i32,
    a_pos: i32,
    ubo: u32,
    edr: u32,
    pub pyramid: u32,
    p_tex: i32,
    p_halfpixel: i32,
    p_lod: i32,
    p_pos: i32,
}

unsafe fn compile(gl: &ffi::Gles2, kind: u32, src: &str) -> Result<u32, String> {
    let sh = gl.CreateShader(kind);
    let c = CString::new(src).unwrap();
    gl.ShaderSource(sh, 1, &c.as_ptr(), std::ptr::null());
    gl.CompileShader(sh);
    let mut ok = 0;
    gl.GetShaderiv(sh, ffi::COMPILE_STATUS, &mut ok);
    if ok == 0 {
        let mut buf = vec![0u8; 4096];
        let mut len = 0;
        gl.GetShaderInfoLog(sh, 4096, &mut len, buf.as_mut_ptr() as *mut _);
        gl.DeleteShader(sh);
        return Err(String::from_utf8_lossy(&buf[..len.max(0) as usize]).to_string());
    }
    Ok(sh)
}

unsafe fn link(gl: &ffi::Gles2, vs: &str, fs: &str) -> Result<u32, String> {
    let v = compile(gl, ffi::VERTEX_SHADER, vs)?;
    let f = compile(gl, ffi::FRAGMENT_SHADER, fs)?;
    let p = gl.CreateProgram();
    gl.AttachShader(p, v);
    gl.AttachShader(p, f);
    gl.LinkProgram(p);
    gl.DeleteShader(v);
    gl.DeleteShader(f);
    let mut ok = 0;
    gl.GetProgramiv(p, ffi::LINK_STATUS, &mut ok);
    if ok == 0 {
        let mut buf = vec![0u8; 4096];
        let mut len = 0;
        gl.GetProgramInfoLog(p, 4096, &mut len, buf.as_mut_ptr() as *mut _);
        return Err(format!("link: {}", String::from_utf8_lossy(&buf[..len.max(0) as usize])));
    }
    Ok(p)
}

unsafe fn uloc(gl: &ffi::Gles2, p: u32, n: &str) -> i32 {
    let c = CString::new(n).unwrap();
    gl.GetUniformLocation(p, c.as_ptr())
}

unsafe fn aloc(gl: &ffi::Gles2, p: u32, n: &str) -> i32 {
    let c = CString::new(n).unwrap();
    gl.GetAttribLocation(p, c.as_ptr())
}

impl Programs {
    pub fn new(r: &mut GlesRenderer) -> Result<Self, GlesError> {
        let res = r.with_context(|gl| unsafe {
            let glass = link(gl, include_str!("shaders/apple/glass.vert"), include_str!("shaders/apple/glass_background_sdf.frag"))?;
            let pyramid = link(gl, include_str!("shaders/apple/pyramid.vert"), include_str!("shaders/apple/pyramid.frag"))?;
            for (name, slot) in [("Buf1", 0u32), ("Buf6", 1u32)] {
                let c = CString::new(name).unwrap();
                let idx = gl.GetUniformBlockIndex(glass, c.as_ptr());
                if idx != ffi::INVALID_INDEX {
                    gl.UniformBlockBinding(glass, idx, slot);
                }
            }
            let mut bufs = [0u32; 2];
            gl.GenBuffers(2, bufs.as_mut_ptr());
            gl.BindBuffer(ffi::UNIFORM_BUFFER, bufs[0]);
            gl.BufferData(ffi::UNIFORM_BUFFER, UBO_SIZE as isize, std::ptr::null(), ffi::DYNAMIC_DRAW);
            let edr: [f32; 4] = [1.0, 0.0, 0.0, 0.0];
            gl.BindBuffer(ffi::UNIFORM_BUFFER, bufs[1]);
            gl.BufferData(ffi::UNIFORM_BUFFER, 16, edr.as_ptr() as *const _, ffi::STATIC_DRAW);
            gl.BindBuffer(ffi::UNIFORM_BUFFER, 0);
            Ok::<_, String>(Programs {
                glass,
                u_proj: uloc(gl, glass, "u_proj"),
                u_vp: uloc(gl, glass, "u_vp"),
                u_fb_rect: uloc(gl, glass, "u_fb_rect"),
                u_dst: uloc(gl, glass, "u_dst"),
                u_sdf_size: uloc(gl, glass, "u_sdf_size"),
                u_alpha: uloc(gl, glass, "u_alpha"),
                u_hl: uloc(gl, glass, "u_hl"),
                u_hl2: uloc(gl, glass, "u_hl2"),
                u_hl3: uloc(gl, glass, "u_hl3"),
                tex: uloc(gl, glass, "tex3"),
                a_pos: aloc(gl, glass, "a_pos"),
                ubo: bufs[0],
                edr: bufs[1],
                pyramid,
                p_tex: uloc(gl, pyramid, "tex"),
                p_halfpixel: uloc(gl, pyramid, "halfpixel"),
                p_lod: uloc(gl, pyramid, "lod"),
                p_pos: aloc(gl, pyramid, "pos"),
            })
        })?;
        res.map_err(|e| {
            tracing::error!("aqua: Apple glass shader: {e}");
            GlesError::ShaderCompileError
        })
    }
}

/// The backdrop pyramid of one glass element.
#[derive(Default)]
pub struct Pyramid {
    pub tex: u32,
    pub fbos: Vec<u32>,
    pub w: i32,
    pub h: i32,
    pub levels: usize,
}

impl Pyramid {
    pub fn gl_objects(&self) -> (u32, Vec<u32>) {
        (self.tex, self.fbos.clone())
    }
}

/// (Re)build `p` from the RGB copy of the backdrop in `src_tex` (`w`×`h`).
///
/// # Safety
/// A GL context must be current; caller saves/restores GL state.
pub unsafe fn build_pyramid(gl: &ffi::Gles2, pr: &Programs, p: &mut Pyramid, src_tex: u32, w: i32, h: i32, levels: usize) {
    let max_levels = 1 + (w.min(h).max(1) as f32).log2().floor() as usize;
    let levels = levels.clamp(1, max_levels.max(1));
    if p.tex == 0 || p.w != w || p.h != h || p.levels != levels {
        if p.tex != 0 {
            gl.DeleteTextures(1, &p.tex);
            gl.DeleteFramebuffers(p.fbos.len() as i32, p.fbos.as_ptr());
        }
        gl.GenTextures(1, &mut p.tex);
        gl.BindTexture(ffi::TEXTURE_2D, p.tex);
        gl.TexStorage2D(ffi::TEXTURE_2D, levels as i32, ffi::RGBA8, w, h);
        p.fbos = vec![0; levels];
        gl.GenFramebuffers(levels as i32, p.fbos.as_mut_ptr());
        for (i, fbo) in p.fbos.iter().enumerate() {
            gl.BindFramebuffer(ffi::FRAMEBUFFER, *fbo);
            gl.FramebufferTexture2D(ffi::FRAMEBUFFER, ffi::COLOR_ATTACHMENT0, ffi::TEXTURE_2D, p.tex, i as i32);
        }
        p.w = w;
        p.h = h;
        p.levels = levels;
    }
    let quad: [f32; 12] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0];
    gl.UseProgram(pr.pyramid);
    gl.Uniform1i(pr.p_tex, 0);
    gl.EnableVertexAttribArray(pr.p_pos as u32);
    gl.VertexAttribPointer(pr.p_pos as u32, 2, ffi::FLOAT, ffi::FALSE, 0, quad.as_ptr() as *const _);
    for i in 0..levels {
        let (lw, lh) = ((w >> i).max(1), (h >> i).max(1));
        gl.BindFramebuffer(ffi::FRAMEBUFFER, p.fbos[i]);
        gl.Viewport(0, 0, lw, lh);
        if i == 0 {
            // level 0: the sharp backdrop (alpha forced to 1)
            gl.BindTexture(ffi::TEXTURE_2D, src_tex);
            gl.Uniform2f(pr.p_halfpixel, 0.0, 0.0);
            gl.Uniform1f(pr.p_lod, 0.0);
        } else {
            gl.BindTexture(ffi::TEXTURE_2D, p.tex);
            gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_BASE_LEVEL, (i - 1) as i32);
            gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MAX_LEVEL, (i - 1) as i32);
            gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MIN_FILTER, ffi::LINEAR as i32);
            gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MAG_FILTER, ffi::LINEAR as i32);
            gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_WRAP_S, ffi::CLAMP_TO_EDGE as i32);
            gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_WRAP_T, ffi::CLAMP_TO_EDGE as i32);
            gl.Uniform2f(pr.p_halfpixel, 0.5 / lw as f32, 0.5 / lh as f32);
            gl.Uniform1f(pr.p_lod, (i - 1) as f32);
        }
        gl.DrawArrays(ffi::TRIANGLES, 0, 6);
    }
    gl.DisableVertexAttribArray(pr.p_pos as u32);
    // Apple's constexpr sampler: linear/linear/linear mip, clamp to edge.
    gl.BindTexture(ffi::TEXTURE_2D, p.tex);
    gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_BASE_LEVEL, 0);
    gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MAX_LEVEL, (levels - 1) as i32);
    gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MIN_FILTER, ffi::LINEAR_MIPMAP_LINEAR as i32);
    gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MAG_FILTER, ffi::LINEAR as i32);
    gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_WRAP_S, ffi::CLAMP_TO_EDGE as i32);
    gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_WRAP_T, ffi::CLAMP_TO_EDGE as i32);
}

/// Per-draw state for [`draw`].
pub struct DrawArgs<'a> {
    pub proj: [f32; 9],
    pub viewport: [i32; 4],
    pub fb_rect: [f32; 4],
    pub dst: [f32; 4],
    pub sdf_size: [f32; 2],
    pub alpha: f32,
    pub ubo: &'a [u8; UBO_SIZE],
    pub highlight: Highlight,
    pub scale: f32,
    /// Damage rects relative to `dst` (physical px).
    pub damage: &'a [[f32; 4]],
}

/// Draw the glass from `pyr` into the bound framebuffer.
///
/// # Safety
/// A GL context must be current.
pub unsafe fn draw(gl: &ffi::Gles2, pr: &Programs, pyr: &Pyramid, a: &DrawArgs<'_>) {
    let mut prog = 0;
    gl.GetIntegerv(ffi::CURRENT_PROGRAM, &mut prog);
    let mut active = 0;
    gl.GetIntegerv(ffi::ACTIVE_TEXTURE, &mut active);
    let mut abuf = 0;
    gl.GetIntegerv(ffi::ARRAY_BUFFER_BINDING, &mut abuf);
    gl.ActiveTexture(ffi::TEXTURE0);
    let mut tex0 = 0;
    gl.GetIntegerv(ffi::TEXTURE_BINDING_2D, &mut tex0);

    gl.UseProgram(pr.glass);
    gl.BindBuffer(ffi::UNIFORM_BUFFER, pr.ubo);
    gl.BufferSubData(ffi::UNIFORM_BUFFER, 0, UBO_SIZE as isize, a.ubo.as_ptr() as *const _);
    gl.BindBuffer(ffi::UNIFORM_BUFFER, 0);
    gl.BindBufferBase(ffi::UNIFORM_BUFFER, 0, pr.ubo);
    gl.BindBufferBase(ffi::UNIFORM_BUFFER, 1, pr.edr);
    gl.BindTexture(ffi::TEXTURE_2D, pyr.tex);
    gl.Uniform1i(pr.tex, 0);
    gl.UniformMatrix3fv(pr.u_proj, 1, ffi::FALSE, a.proj.as_ptr());
    let vp = a.viewport;
    gl.Uniform4f(pr.u_vp, vp[0] as f32, vp[1] as f32, vp[2] as f32, vp[3] as f32);
    gl.Uniform4f(pr.u_fb_rect, a.fb_rect[0], a.fb_rect[1], a.fb_rect[2], a.fb_rect[3]);
    gl.Uniform4f(pr.u_dst, a.dst[0], a.dst[1], a.dst[2], a.dst[3]);
    gl.Uniform2f(pr.u_sdf_size, a.sdf_size[0], a.sdf_size[1]);
    gl.Uniform1f(pr.u_alpha, a.alpha);
    let h = a.highlight;
    gl.Uniform4f(pr.u_hl, h.key_amount, h.fill_amount, h.height * a.scale, h.curvature);
    // CoreAnimation angles are y-up; the shape space here is y-down.
    gl.Uniform4f(pr.u_hl2, h.key_angle.cos(), -h.key_angle.sin(), h.fill_angle.cos(), -h.fill_angle.sin());
    let (mode, white, shade) = match h.rim {
        Rim::Additive => (0.0, 0.0, 0.0),
        Rim::SoftLight { white, shade } => (1.0, white, shade),
    };
    gl.Uniform4f(pr.u_hl3, mode, white, shade, 0.0);
    gl.Enable(ffi::BLEND);
    gl.BlendFunc(ffi::ONE, ffi::ONE_MINUS_SRC_ALPHA);

    let mut verts: Vec<f32> = Vec::with_capacity(a.damage.len().max(1) * 12);
    let (dw, dh) = (a.dst[2].max(1.0), a.dst[3].max(1.0));
    let full = [[0.0, 0.0, dw, dh]];
    let rects = if a.damage.is_empty() { &full[..] } else { a.damage };
    for r in rects {
        let (x0, y0) = ((r[0] / dw).clamp(0.0, 1.0), (r[1] / dh).clamp(0.0, 1.0));
        let (x1, y1) = (((r[0] + r[2]) / dw).clamp(0.0, 1.0), ((r[1] + r[3]) / dh).clamp(0.0, 1.0));
        verts.extend_from_slice(&[x0, y0, x1, y0, x0, y1, x0, y1, x1, y0, x1, y1]);
    }
    gl.BindBuffer(ffi::ARRAY_BUFFER, 0);
    gl.EnableVertexAttribArray(pr.a_pos as u32);
    gl.VertexAttribPointer(pr.a_pos as u32, 2, ffi::FLOAT, ffi::FALSE, 0, verts.as_ptr() as *const _);
    gl.DrawArrays(ffi::TRIANGLES, 0, (verts.len() / 2) as i32);
    gl.DisableVertexAttribArray(pr.a_pos as u32);

    gl.BindBufferBase(ffi::UNIFORM_BUFFER, 0, 0);
    gl.BindBufferBase(ffi::UNIFORM_BUFFER, 1, 0);
    gl.BindTexture(ffi::TEXTURE_2D, tex0 as u32);
    gl.ActiveTexture(active as u32);
    gl.BindBuffer(ffi::ARRAY_BUFFER, abuf as u32);
    gl.UseProgram(prog as u32);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_floats() {
        assert_eq!(f32_to_f16(1.0), 0x3c00);
        assert_eq!(f32_to_f16(0.5), 0x3800);
        assert_eq!(f32_to_f16(-2.0), 0xc000);
        assert_eq!(f32_to_f16(0.0), 0);
    }

    #[test]
    fn identity_face_matrix() {
        let m = ColorMatrix::IDENTITY.rows();
        assert_eq!(m[0], [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(m[2], [0.0, 0.0, 1.0, 0.0]);
    }

    #[test]
    fn regular_rule_matches_captures() {
        // Finder sidebar, 200×584 pt: blur 4, bleed 70 (captured)
        let g = GlassInputs::mac(Variant::Sidebar, true, 200.0, 584.0);
        assert!((g.blur_radius - 4.0).abs() < 1e-4 && (g.bleed_amount - 70.0).abs() < 1e-3);
        assert_eq!(g.inner_refraction_amount, -60.0);
        // alert 260×96: blur 2.857, bleed 33.6
        let a = GlassInputs::mac(Variant::Alert, false, 260.0, 96.0);
        assert!((a.blur_radius - 2.857).abs() < 1e-3 && (a.bleed_amount - 33.6).abs() < 1e-3);
    }

    #[test]
    fn block_layout() {
        let g = GlassInputs::mac(Variant::Regular, false, 100.0, 40.0);
        let b = pack(&g, &Frame { half: [50.0, 20.0], radius: 20.0, displacement: [1.0, 0.0, 0.0, 1.0], scale: 1.0 });
        let f = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        assert_eq!(f(8), 4.0, "sdf mode");
        assert_eq!(f(40), 20.0, "radius");
        assert_eq!(f(48 + 16), -32.0, "inner refraction amount");
        assert_eq!(f(48 + 256), 1.0, "complex refraction");
        assert!(levels_for(30.0) >= 6);
    }

    /// Writes the uniform block for the GPU harness (`LTX_UBO_DUMP=path cargo test -- --ignored`).
    #[test]
    #[ignore]
    fn dump_block() {
        let Ok(path) = std::env::var("LTX_UBO_DUMP") else { return };
        let dark = std::env::var("LTX_DARK").is_ok();
        let v = Variant::from_code(std::env::var("LTX_VARIANT").ok().and_then(|v| v.parse().ok()).unwrap_or(0));
        let g = GlassInputs::mac(v, dark, 400.0, 140.0);
        // test harness: 400×140 px at scale 1, fb y-up (displacement y flipped)
        let b = pack(&g, &Frame { half: [200.0, 70.0], radius: 36.0, displacement: [1.0 / 400.0, 0.0, 0.0, -1.0 / 140.0], scale: 1.0 });
        std::fs::write(path, b).unwrap();
    }
}
