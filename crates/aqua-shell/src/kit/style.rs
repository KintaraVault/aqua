//! Adaptive foreground colours + glass material presets.
use aqua_config::material::Material;
use aqua_config::GlassStyle;
use aqua_gfx::{rgba, Color, Pixmap, Rect};

/// Coarse luminance map of the wallpaper used to pick legible foregrounds.
pub struct Adaptive {
    grid: Vec<f32>,
    gw: usize,
    gh: usize,
    w: f32,
    h: f32,
    /// System-wide Dark Mode (Control Center toggle).
    pub dark: bool,
}

const GW: usize = 48;
const GH: usize = 32;

impl Adaptive {
    pub fn from_wallpaper(wp: &Pixmap, w: f32, h: f32) -> Self {
        let mut grid = vec![0.0; GW * GH];
        let (pw, ph) = (wp.width() as usize, wp.height() as usize);
        for gy in 0..GH {
            for gx in 0..GW {
                let mut acc = 0.0;
                let mut n = 0.0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let x = ((gx * 4 + sx) * pw / (GW * 4)).min(pw - 1);
                        let y = ((gy * 4 + sy) * ph / (GH * 4)).min(ph - 1);
                        let p = wp.pixel(x as u32, y as u32).unwrap();
                        acc += lum(p.red(), p.green(), p.blue());
                        n += 1.0;
                    }
                }
                grid[gy * GW + gx] = acc / n;
            }
        }
        Self { grid, gw: GW, gh: GH, w, h, dark: false }
    }

    /// Mean luminance (0..1) of the wallpaper under logical rect `r`.
    pub fn lum(&self, r: Rect) -> f32 {
        let x0 = ((r.x / self.w) * self.gw as f32).floor().clamp(0.0, self.gw as f32 - 1.0) as usize;
        let x1 = ((r.right() / self.w) * self.gw as f32).ceil().clamp(1.0, self.gw as f32) as usize;
        let y0 = ((r.y / self.h) * self.gh as f32).floor().clamp(0.0, self.gh as f32 - 1.0) as usize;
        let y1 = ((r.bottom() / self.h) * self.gh as f32).ceil().clamp(1.0, self.gh as f32) as usize;
        let mut acc = 0.0;
        let mut n = 0.0;
        for y in y0..y1.max(y0 + 1) {
            for x in x0..x1.max(x0 + 1) {
                acc += self.grid[y * self.gw + x];
                n += 1.0;
            }
        }
        acc / n
    }

    /// Foreground for content placed directly on the wallpaper (menu bar).
    pub fn on_wallpaper(&self, r: Rect) -> Color {
        if !self.dark && self.lum(r) > 0.72 {
            rgba(20, 20, 22, 0.92)
        } else {
            rgba(255, 255, 255, 0.98)
        }
    }

    /// Foreground for content on a regular (tinted) glass panel.
    pub fn is_dark_glass(&self, r: Rect) -> bool {
        self.dark || self.lum(r) < 0.42
    }
}

fn lum(r: u8, g: u8, b: u8) -> f32 {
    (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / 255.0
}

pub fn text_primary(dark: bool) -> Color {
    if dark {
        rgba(255, 255, 255, 0.96)
    } else {
        rgba(29, 29, 31, 0.92)
    }
}
pub fn text_secondary(dark: bool) -> Color {
    if dark {
        rgba(255, 255, 255, 0.62)
    } else {
        rgba(60, 60, 67, 0.62)
    }
}
pub fn separator(dark: bool) -> Color {
    if dark {
        rgba(255, 255, 255, 0.18)
    } else {
        rgba(0, 0, 0, 0.10)
    }
}
pub const ACCENT: (u8, u8, u8) = (0, 122, 255);
pub fn accent(a: f32) -> Color {
    rgba(ACCENT.0, ACCENT.1, ACCENT.2, a)
}

/// Material presets: thin wrappers over the desktop's one glass standard
/// (`aqua_config::material`), kept for the call sites' readability.
pub fn glass_panel(base: &GlassStyle, radius: f32) -> GlassStyle {
    base.material(Material::Regular, false).with_radius(radius)
}
pub fn glass_dock(base: &GlassStyle) -> GlassStyle {
    base.material(Material::Dock, false)
}
pub fn glass_menu(base: &GlassStyle, dark: bool) -> GlassStyle {
    base.material(Material::Menu, dark)
}
pub fn glass_tile(base: &GlassStyle, radius: f32) -> GlassStyle {
    base.material(Material::Tile, false).with_radius(radius)
}
/// Dark-mode variant of a material: smoky graphite tint, dimmer backdrop.
pub fn darken(g: &GlassStyle) -> GlassStyle {
    aqua_config::material::darken(g)
}
/// Glass under a Clear icon plate of width `w` (logical px): a thick lens at the rim
/// (refraction + slight dispersion), a light blur and a whisper of tint.
pub fn glass_icon(base: &GlassStyle, w: f32) -> GlassStyle {
    base.material(Material::Icon(w), false)
}
pub fn glass_clear(base: &GlassStyle, radius: f32) -> GlassStyle {
    base.material(Material::Clear, false).with_radius(radius)
}
/// Lock screen fields and buttons.
pub fn glass_pill(base: &GlassStyle, radius: f32) -> GlassStyle {
    base.material(Material::Pill, false).with_radius(radius)
}
/// Frosted full-screen backdrop without lens (lock screen, Mission Control).
pub fn glass_overlay(base: &GlassStyle) -> GlassStyle {
    base.material(Material::Overlay, false)
}
