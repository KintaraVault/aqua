//! Software compositor for previews, tests and as reference implementation of
//! the liquid-glass material (the GL shaders in `aqua-render` mirror this).
use crate::Layer;
use aqua_config::GlassStyle;
use aqua_gfx::canvas::{lin_grad, solid};
use aqua_gfx::tiny_skia::{FillRule, Mask, Pixmap, PixmapPaint, Transform};
use aqua_gfx::{blur, shapes, Rect};

/// A client window for the preview: rect (logical) + content pixmap (physical).
pub struct SoftWindow {
    pub rect: Rect,
    pub content: Pixmap,
    pub radius: f32,
}

pub fn drop_shadow(dst: &mut Pixmap, r: Rect, radius: f32, scale: f32, alpha: f32, blur_r: f32, dy: f32) {
    if alpha <= 0.0 {
        return;
    }
    let pad = blur_r * 2.0;
    let mut sh = Pixmap::new(((r.w + 2.0 * pad) * scale) as u32, ((r.h + 2.0 * pad) * scale) as u32).unwrap();
    if let Some(p) = shapes::squircle(Rect::new(pad, pad, r.w, r.h), radius) {
        sh.fill_path(
            &p,
            &solid(aqua_gfx::rgba(0, 0, 0, alpha)),
            FillRule::Winding,
            Transform::from_scale(scale, scale),
            None,
        );
    }
    blur::blur(&mut sh, blur_r * scale);
    dst.draw_pixmap(
        ((r.x - pad) * scale) as i32,
        ((r.y - pad + dy) * scale) as i32,
        sh.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        None,
    );
}

/// Draw glass for rect `r` reading the current contents of `dst` as backdrop.
pub fn glass(dst: &mut Pixmap, r: Rect, g: &GlassStyle, scale: f32, opacity: f32) {
    drop_shadow(dst, r, g.radius, scale, g.shadow * opacity, 18.0, 6.0);
    let (x, y) = ((r.x * scale) as i32, (r.y * scale) as i32);
    let (w, h) = ((r.w * scale).ceil() as u32, (r.h * scale).ceil() as u32);
    let mut back = Pixmap::new(w.max(1), h.max(1)).unwrap();
    back.draw_pixmap(-x, -y, dst.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
    let mut back = blur::blur_fast(&back, g.blur * scale);
    blur::saturate(&mut back, g.saturation);
    let (tr, tg, tb, ta) = (g.tint.0, g.tint.1, g.tint.2, g.tint.3);
    for px in back.data_mut().as_chunks_mut::<4>().0 {
        let a = px[3] as f32 / 255.0;
        px[0] = (px[0] as f32 * (1.0 - ta) + tr * 255.0 * ta * a) as u8;
        px[1] = (px[1] as f32 * (1.0 - ta) + tg * 255.0 * ta * a) as u8;
        px[2] = (px[2] as f32 * (1.0 - ta) + tb * 255.0 * ta * a) as u8;
    }
    if let Some(p) = shapes::squircle(Rect::new(0.0, 0.0, r.w, r.h), g.radius) {
        let mut m = Mask::new(w, h).unwrap();
        m.fill_path(&p, FillRule::Winding, true, Transform::from_scale(scale, scale));
        back.apply_mask(&m);
        let rim = lin_grad(
            0.0,
            0.0,
            r.w,
            r.h,
            &[
                (0.0, aqua_gfx::rgba(255, 255, 255, 0.9 * g.rim)),
                (0.35, aqua_gfx::rgba(255, 255, 255, 0.18 * g.rim)),
                (0.7, aqua_gfx::rgba(255, 255, 255, 0.10 * g.rim)),
                (1.0, aqua_gfx::rgba(255, 255, 255, 0.7 * g.rim)),
            ],
        );
        let stroke = aqua_gfx::tiny_skia::Stroke { width: 1.4, ..Default::default() };
        if let Some(p2) = shapes::squircle(Rect::new(0.7, 0.7, r.w - 1.4, r.h - 1.4), g.radius - 0.7) {
            back.stroke_path(&p2, &rim, &stroke, Transform::from_scale(scale, scale), None);
            let soft = aqua_gfx::tiny_skia::Stroke { width: g.bevel * 0.6, ..Default::default() };
            let glow = lin_grad(
                0.0,
                0.0,
                0.0,
                r.h,
                &[
                    (0.0, aqua_gfx::rgba(255, 255, 255, 0.10 * g.rim)),
                    (1.0, aqua_gfx::rgba(255, 255, 255, 0.04 * g.rim)),
                ],
            );
            let mut tmp = Pixmap::new(w, h).unwrap();
            tmp.stroke_path(&p2, &glow, &soft, Transform::from_scale(scale, scale), None);
            blur::blur(&mut tmp, g.bevel * 0.5 * scale);
            tmp.apply_mask(&m);
            back.draw_pixmap(0, 0, tmp.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
        }
    }
    dst.draw_pixmap(x, y, back.as_ref(), &PixmapPaint { opacity, ..Default::default() }, Transform::identity(), None);
}

/// Composite a full frame: wallpaper → windows → shell layers.
pub fn compose(wallpaper: &Pixmap, windows: &[SoftWindow], layers: &[Layer], scale: f32) -> Pixmap {
    let mut out = wallpaper.clone();
    for l in layers.iter().filter(|l| matches!(l.id, crate::LayerId::Widgets(_))) {
        draw_layer(&mut out, l, scale);
    }
    for w in windows {
        drop_shadow(&mut out, w.rect, w.radius, scale, 0.32, 28.0, 14.0);
        let mut c = w.content.clone();
        if let Some(p) = shapes::rrect(Rect::new(0.0, 0.0, w.rect.w, w.rect.h), w.radius) {
            let mut m = Mask::new(c.width(), c.height()).unwrap();
            m.fill_path(&p, FillRule::Winding, true, Transform::from_scale(scale, scale));
            c.apply_mask(&m);
        }
        out.draw_pixmap(
            (w.rect.x * scale) as i32,
            (w.rect.y * scale) as i32,
            c.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }
    for l in layers.iter().filter(|l| !matches!(l.id, crate::LayerId::Widgets(_))) {
        draw_layer(&mut out, l, scale);
    }
    out
}

pub fn draw_layer(out: &mut Pixmap, l: &Layer, scale: f32) {
    if l.opacity <= 0.001 {
        return;
    }
    if let Some(g) = &l.glass {
        glass(out, l.rect, g, scale, l.opacity);
    }
    for (r, g) in &l.tiles {
        glass(out, *r, g, scale, l.opacity);
    }
    out.draw_pixmap(
        (l.rect.x * scale).round() as i32,
        (l.rect.y * scale).round() as i32,
        l.content.as_ref().as_ref(),
        &PixmapPaint { opacity: l.opacity, ..Default::default() },
        Transform::identity(),
        None,
    );
}
