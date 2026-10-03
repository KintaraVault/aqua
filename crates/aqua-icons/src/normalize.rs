//! Normalising foreign icons to the icon grid (1024 canvas, 824 px plate).
use crate::glass::{self, Cov};
use aqua_gfx::canvas::lin_grad;
use aqua_gfx::tiny_skia::{Mask, Transform};
use aqua_gfx::{resize, Canvas, Pixmap, Rect};

/// Content already designed for it: just resample to `px`.
pub fn fit_macos(src: &Pixmap, px: u32) -> Pixmap {
    let corner_alpha = src.pixel(2, 2).map(|p| p.alpha()).unwrap_or(0);
    if corner_alpha > 200 {
        let body = (px as f32 * 824.0 / 1024.0).round() as u32;
        let inner = resize(src, body, body);
        return with_shadow(&mask_squircle(&inner), px);
    }
    resize(src, px, px)
}

/// Mask a square image with the plate shape.
pub fn mask_squircle(src: &Pixmap) -> Pixmap {
    let w = src.width() as f32;
    let mut out = src.clone();
    if let Some(path) = glass::plate_path(Rect::new(0.0, 0.0, w, src.height() as f32)) {
        let mut m = Mask::new(src.width(), src.height()).unwrap();
        m.fill_path(&path, aqua_gfx::tiny_skia::FillRule::Winding, true, Transform::identity());
        out.apply_mask(&m);
    }
    out
}

/// Place a body (sized 824/1024 of px) centred on a px canvas with the standard contact shadow.
pub fn with_shadow(body: &Pixmap, px: u32) -> Pixmap {
    let mut canvas = Pixmap::new(px, px).unwrap();
    let off = ((px.saturating_sub(body.width())) / 2) as i32;
    canvas.draw_pixmap(off, off, body.as_ref(), &Default::default(), Transform::identity(), None);
    drop_shadow(&canvas, true)
}

/// Add the soft contact shadow under whatever is drawn on the canvas.
pub fn drop_shadow(pm: &Pixmap, plate: bool) -> Pixmap {
    let px = pm.width();
    let s = px as f32 / 1024.0;
    let a = Cov::alpha(pm);
    let shape =
        if plate { a.clone() } else { Cov { a: a.a.iter().map(|v| (v * 2.5).min(1.0)).collect(), ..a.clone() } };
    let near = shape.blur((6.0 * s).max(0.6)).shift(0.0, 4.0 * s).scale(0.20);
    let far = shape.blur((18.0 * s).max(1.0)).shift(0.0, 12.0 * s).scale(0.22);
    let mut out = Pixmap::new(px, px).unwrap();
    {
        let d = out.data_mut();
        for i in 0..near.a.len() {
            let mut v = (near.a[i] + far.a[i] - near.a[i] * far.a[i]).min(1.0);
            if plate {
                v *= 1.0 - (a.a[i] * 4.0).min(1.0);
            }
            d[i * 4 + 3] = (v * 255.0 + 0.5) as u8;
        }
    }
    out.draw_pixmap(0, 0, pm.as_ref(), &Default::default(), Transform::identity(), None);
    out
}

/// Linux icon (arbitrary shape) -> put on a light plate.
pub fn plate(src: &Pixmap, px: u32) -> Pixmap {
    let body = (px as f32 * 824.0 / 1024.0).round();
    let mut c = Canvas::new(body, body, 1.0);
    let r = Rect::new(0.0, 0.0, body, body);
    if let Some(p) = glass::plate_path(r) {
        c.fill_path(
            &p,
            &lin_grad(
                0.0,
                0.0,
                0.0,
                body,
                &[(0.0, aqua_gfx::rgba(255, 255, 255, 1.0)), (1.0, aqua_gfx::rgba(232, 234, 239, 1.0))],
            ),
        );
    }
    let inner = (body * 0.70).round();
    let icon = resize(src, inner as u32, inner as u32);
    let mut lifted = Pixmap::new(body as u32, body as u32).unwrap();
    let o = ((body - inner) / 2.0) as i32;
    lifted.draw_pixmap(o, o, icon.as_ref(), &Default::default(), Transform::identity(), None);
    let sh = Cov::alpha(&lifted).blur(body * 0.012).shift(0.0, body * 0.012).scale(0.18);
    if let Some(m) = sh.to_mask() {
        if let Some(rr) = aqua_gfx::tiny_skia::Rect::from_xywh(0.0, 0.0, body, body) {
            c.pm.fill_rect(rr, &glass::solid(aqua_gfx::rgba(0, 0, 0, 1.0)), Transform::identity(), Some(&m));
        }
    }
    c.pm.draw_pixmap(0, 0, lifted.as_ref(), &Default::default(), Transform::identity(), None);
    with_shadow(&c.pm, px)
}

/// [`plate`] in a non-default icon style: drawn natively like our own icons (the
/// style's plate, the picture as one glass glyph), not filtered afterwards.
pub fn plate_look(src: &Pixmap, px: u32, look: &crate::look::Look) -> Pixmap {
    glass::with_look(look, || {
        let mut ic = glass::Ic::plate(px);
        ic.bg_v(aqua_gfx::rgba(255, 255, 255, 1.0), aqua_gfx::rgba(232, 234, 239, 1.0));
        ic.forget_plate();
        let inner = glass::B * 0.70;
        let o = (glass::B - inner) / 2.0;
        ic.layer_pixmap(src, Rect::new(o, o, inner, inner));
        ic.finish(true)
    })
}
