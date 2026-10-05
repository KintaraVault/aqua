//! The desktop's one glass standard: every glass surface is a [`Material`] derived from the
//! user's master [`GlassStyle`] (`[glass]` in the config file). Shell layers, window chrome,
//! the glass behind client windows and the glass of Aqua's own apps (sidebar islands,
//! toolbar controls, menus — requested over the `aqua_glass_v1` protocol) all come from
//! here, so changing the master style changes all of them consistently.
use crate::{metrics, GlassStyle, Rgba};

/// A kind of glass surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Material {
    /// Panels: Control Center, Notification Center, Spotlight, Launchpad, switcher, widgets.
    Regular,
    /// The Dock plate.
    Dock,
    /// Menus (menu bar, context and app menus, alerts, popovers).
    Menu,
    /// Modules inside a panel (Control Center tiles, notification cards, widgets).
    Tile,
    /// Clear glass over the wallpaper (Mission Control buttons, Spaces pills).
    Clear,
    /// Lens under a Clear-style app icon plate of the given width.
    Icon(f32),
    /// Lock screen fields and buttons.
    Pill,
    /// Full-screen backdrop (Mission Control, lock screen): frosted, no lens.
    Overlay,
    /// Server-side window chrome (title bar + frame).
    WindowFrame { focused: bool },
    /// Behind blur regions requested by third-party clients (KDE blur /
    /// ext-background-effect): plain frosted backdrop, the app draws its own tint.
    Backdrop,
    /// Window tiling preview.
    TilePreview,
    /// Sidebar island of Aqua's apps (Finder, System Settings, App Store).
    Sidebar,
    /// Toolbar controls of Aqua's apps (navigation arrows, capsules, search): clear lens
    /// drawn *over* the app's own pixels.
    Control,
    /// A menu window of Aqua's apps (whole surface).
    AppMenu,
}

/// Wire ids of the materials an `aqua_glass_v1` client may request.
pub mod wire {
    pub const SIDEBAR: u32 = 1;
    pub const CONTROL: u32 = 2;
    pub const MENU: u32 = 3;
    pub const PANEL: u32 = 4;
}

impl Material {
    /// The material behind wire id `id` (unknown ids fall back to the sidebar glass).
    pub fn from_wire(id: u32) -> Self {
        match id {
            wire::CONTROL => Material::Control,
            wire::MENU => Material::AppMenu,
            wire::PANEL => Material::Regular,
            _ => Material::Sidebar,
        }
    }

    /// Drawn above the client's pixels (a lens over the app's own control) rather than as a
    /// backdrop behind a transparent part of the surface.
    pub fn over_content(self) -> bool {
        matches!(self, Material::Control)
    }
}

/// Dark-mode variant of a material: smoky graphite tint, dimmer backdrop, softer lights.
pub fn darken(g: &GlassStyle) -> GlassStyle {
    if g.tint.3 <= 0.001 && g.blur <= 0.5 {
        // clear lenses over app content keep their (absent) tint
        return GlassStyle { rim: g.rim * 0.8, ..*g };
    }
    let a = (g.tint.3 * 1.2 + 0.24).min(0.74);
    GlassStyle { tint: Rgba(0.09, 0.09, 0.11, a), max_luma: g.max_luma.min(0.45), rim: g.rim * 0.75, ..*g }
}

/// Surface type `m` of the master style `base` (light appearance unless `dark`).
pub fn style(base: &GlassStyle, m: Material, dark: bool) -> GlassStyle {
    let b = *base;
    let g = match m {
        Material::Regular => b,
        Material::Dock => GlassStyle { radius: metrics::DOCK_RADIUS, tint: Rgba(1.0, 1.0, 1.0, b.tint.3 * 0.9), ..b },
        // Shell menus are the same glass as the apps' menu windows (plus their own shadow).
        Material::Menu => GlassStyle { radius: 13.0, shadow: 0.28, ..style(base, Material::AppMenu, dark) },
        Material::Tile => GlassStyle {
            tint: Rgba(1.0, 1.0, 1.0, 0.07),
            saturation: b.saturation * 1.2,
            shadow: 0.10,
            max_luma: 0.54,
            ..b
        }
        .with_thickness(b.thickness * 0.8),
        Material::Clear => GlassStyle { tint: Rgba(1.0, 1.0, 1.0, 0.06), shadow: 0.0, ..b },
        Material::Icon(w) => GlassStyle {
            radius: w * 0.275 * 1.1,
            blur: (w * 0.07).clamp(2.0, 7.0),
            tint: Rgba(1.0, 1.0, 1.0, 0.04),
            saturation: 1.35,
            thickness: w * 0.26,
            refraction: w * 0.2 * ratio(&b),
            rim: 0.0,
            shadow: 0.0,
            ..b
        },
        Material::Pill => GlassStyle {
            blur: 18.0,
            tint: Rgba(1.0, 1.0, 1.0, 0.16),
            saturation: 1.4,
            rim: b.rim * 0.6,
            shadow: 0.0,
            max_luma: 0.7,
            ..b
        }
        .with_thickness(8.0),
        Material::Overlay => GlassStyle {
            blur: 30.0,
            tint: Rgba(0.04, 0.05, 0.10, 0.38),
            saturation: 1.3,
            radius: 0.0,
            shadow: 0.0,
            max_luma: 1.0,
            ..b
        }
        .flat(),
        Material::WindowFrame { focused } => GlassStyle {
            blur: 34.0,
            tint: match (dark, focused) {
                (true, true) => Rgba(0.16, 0.16, 0.18, 0.88),
                (true, false) => Rgba(0.14, 0.14, 0.15, 0.92),
                (false, true) => Rgba(0.965, 0.965, 0.975, 0.86),
                (false, false) => Rgba(0.94, 0.94, 0.95, 0.90),
            },
            saturation: 1.8,
            refraction: 0.0,
            thickness: 6.0,
            rim: b.rim * 0.35,
            roundness: 2.0,
            shadow: 0.0,
            max_luma: 1.0,
            ..b
        },
        Material::Backdrop => GlassStyle {
            blur: 40.0,
            tint: if dark { Rgba(0.10, 0.10, 0.12, 0.10) } else { Rgba(1.0, 1.0, 1.0, 0.10) },
            saturation: 1.9,
            roundness: 2.0,
            shadow: 0.0,
            max_luma: 1.0,
            ..b
        }
        .flat(),
        Material::TilePreview => GlassStyle {
            blur: 18.0,
            tint: if dark { Rgba(0.55, 0.58, 0.66, 0.22) } else { Rgba(1.0, 1.0, 1.0, 0.30) },
            saturation: 1.25,
            refraction: 0.0,
            thickness: 8.0,
            rim: b.rim * 0.9,
            radius: 16.0,
            shadow: 0.0,
            max_luma: 1.0,
            ..b
        },
        Material::Sidebar => GlassStyle {
            blur: b.blur * 1.35,
            tint: if dark { Rgba(0.19, 0.19, 0.21, 0.50) } else { Rgba(1.0, 1.0, 1.0, 0.34) },
            saturation: b.saturation * 1.1,
            roundness: 2.0,
            shadow: 0.0,
            max_luma: 1.0,
            rim: b.rim * 0.9,
            ..b
        }
        .with_thickness(b.thickness * 0.85),
        // A 36 px toolbar lens: a gentler bend and almost no colour split, so glyphs and the
        // selection pill under the rim stay crisp.
        Material::Control => GlassStyle {
            refraction: b.refraction * 0.5,
            dispersion: b.dispersion * 0.3,
            fresnel: b.fresnel * 0.45,
            fresnel_range: b.fresnel_range * 1.6,
            glare: b.glare * 0.4,
            glare_range: b.glare_range * 1.6,
            blur: 0.0,
            tint: Rgba(1.0, 1.0, 1.0, 0.0),
            saturation: 1.0,
            roundness: 2.0,
            blur_edge: false,
            shadow: 0.0,
            max_luma: 1.0,
            rim: b.rim,
            ..b
        }
        .with_thickness(b.thickness * 0.5),
        // Menu windows of Aqua's apps: heavily frosted, a soft 2 px edge light. Dark: a light
        // grey veil over a luminance-capped backdrop (macOS 26 dark menus are lighter than the
        // content under them, yet stay legible over white).
        Material::AppMenu => {
            let (tint, max_luma) =
                if dark { (Rgba(0.55, 0.55, 0.56, 0.30), 0.5) } else { (Rgba(0.97, 0.97, 0.98, 0.62), 1.0) };
            GlassStyle {
                tint,
                max_luma,
                blur: b.blur * 2.2,
                roundness: 2.0,
                shadow: 0.0,
                fresnel: b.fresnel * 0.45,
                fresnel_range: b.fresnel_range * 1.5,
                glare: b.glare * 0.5,
                glare_range: b.glare_range * 1.5,
                ..b
            }
            .with_thickness(b.thickness * 0.6)
        }
    };
    if dark
        && !matches!(
            m,
            Material::Menu
                | Material::WindowFrame { .. }
                | Material::Backdrop
                | Material::Sidebar
                | Material::AppMenu
                | Material::Control
                | Material::TilePreview
                | Material::Overlay
        )
    {
        darken(&g)
    } else {
        g
    }
}

/// Lens strength of the master style relative to the standard (refraction per thickness).
fn ratio(b: &GlassStyle) -> f32 {
    let d = GlassStyle::default();
    if b.thickness > 0.0 {
        (b.refraction / b.thickness) / (d.refraction / d.thickness)
    } else {
        0.0
    }
}

/// Corner geometry the shaders use for a `w`×`h` shape with corner radius `r` and corner
/// shape `n` (see [`GlassStyle::roundness`]): `(extent, exponent)`. The corner is a
/// superellipse of exponent `n` spanning `extent` px, sized so that its 45° point lies where a
/// circular corner of radius `r` would put it — every roundness reads as "the same radius".
/// Pills keep exact semicircles.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_keep_the_visual_radius() {
        let (e, n) = corner(200.0, 100.0, 20.0, 2.0);
        assert_eq!((e, n), (20.0, 2.0));
        let (e, n) = corner(200.0, 100.0, 20.0, 3.0);
        assert!((n - 3.0).abs() < 1e-3 && e > 27.0 && e < 30.0, "{e} {n}");
        // pill: semicircle ends
        let (e, n) = corner(200.0, 40.0, 20.0, 5.0);
        assert!((e - 20.0).abs() < 1e-3 && (n - 2.0).abs() < 1e-3, "{e} {n}");
        // in between: the exponent eases toward a circle as the corner runs out of room
        let (_, n) = corner(200.0, 40.0, 15.0, 5.0);
        assert!(n > 2.0 && n < 5.0);
    }

    #[test]
    fn materials_derive_from_the_master_style() {
        let mut b = GlassStyle::default();
        let m = style(&b, Material::Menu, false);
        assert!(m.thickness < b.thickness && m.refraction < b.refraction);
        b.rim = 0.0;
        assert_eq!(style(&b, Material::Sidebar, false).rim, 0.0, "edge lights follow the master knob");
        assert_eq!(style(&b, Material::Backdrop, false).refraction, 0.0);
        assert!(Material::from_wire(wire::CONTROL).over_content());
        assert!(!Material::from_wire(wire::SIDEBAR).over_content());
    }
}
