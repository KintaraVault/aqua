use crate::{shapes, text::Fonts, Rect, Weight};
use tiny_skia::*;

/// A drawing surface in *logical* coordinates; internally rendered at `scale`.
pub struct Canvas {
    pub pm: Pixmap,
    pub scale: f32,
}

impl Canvas {
    pub fn new(logical_w: f32, logical_h: f32, scale: f32) -> Self {
        let w = (logical_w * scale).ceil().max(1.0) as u32;
        let h = (logical_h * scale).ceil().max(1.0) as u32;
        Self { pm: Pixmap::new(w, h).expect("canvas alloc"), scale }
    }
    pub fn from_pixmap(pm: Pixmap, scale: f32) -> Self {
        Self { pm, scale }
    }
    pub fn logical_size(&self) -> (f32, f32) {
        (self.pm.width() as f32 / self.scale, self.pm.height() as f32 / self.scale)
    }
    pub fn ts(&self) -> Transform {
        Transform::from_scale(self.scale, self.scale)
    }
    pub fn clear(&mut self) {
        self.pm.fill(Color::TRANSPARENT);
    }

    pub fn fill_path(&mut self, path: &Path, paint: &Paint) {
        let ts = self.ts();
        self.pm.fill_path(path, paint, FillRule::Winding, ts, None);
    }
    pub fn fill_path_eo(&mut self, path: &Path, paint: &Paint) {
        let ts = self.ts();
        self.pm.fill_path(path, paint, FillRule::EvenOdd, ts, None);
    }
    pub fn stroke_path(&mut self, path: &Path, paint: &Paint, width: f32) {
        let ts = self.ts();
        let stroke = Stroke { width, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
        self.pm.stroke_path(path, paint, &stroke, ts, None);
    }

    pub fn fill_rrect(&mut self, r: Rect, radius: f32, color: Color) {
        if let Some(p) = shapes::rrect(r, radius) {
            self.fill_path(&p, &solid(color));
        }
    }
    pub fn fill_squircle(&mut self, r: Rect, radius: f32, paint: &Paint) {
        if let Some(p) = shapes::squircle(r, radius) {
            self.fill_path(&p, paint);
        }
    }
    pub fn stroke_rrect(&mut self, r: Rect, radius: f32, color: Color, width: f32) {
        if let Some(p) = shapes::rrect(r, radius) {
            self.stroke_path(&p, &solid(color), width);
        }
    }
    pub fn fill_circle(&mut self, cx: f32, cy: f32, rad: f32, color: Color) {
        if let Some(p) = PathBuilder::from_circle(cx, cy, rad) {
            self.fill_path(&p, &solid(color));
        }
    }
    pub fn fill_rect(&mut self, r: Rect, color: Color) {
        if let Some(rr) = tiny_skia::Rect::from_xywh(r.x, r.y, r.w, r.h) {
            let ts = self.ts();
            if r.w < 2.0 || r.h < 2.0 {
                let p = tiny_skia::PathBuilder::from_rect(rr);
                self.pm.fill_path(&p, &solid(color), tiny_skia::FillRule::Winding, ts, None);
            } else {
                self.pm.fill_rect(rr, &solid(color), ts, None);
            }
        }
    }
    /// Vertical linear gradient fill of a rounded rect.
    pub fn fill_rrect_vgrad(&mut self, r: Rect, radius: f32, top: Color, bottom: Color) {
        if let (Some(p), Some(sh)) = (
            shapes::rrect(r, radius),
            LinearGradient::new(
                Point::from_xy(r.x, r.y),
                Point::from_xy(r.x, r.bottom()),
                vec![GradientStop::new(0.0, top), GradientStop::new(1.0, bottom)],
                SpreadMode::Pad,
                Transform::identity(),
            ),
        ) {
            let paint = Paint { shader: sh, anti_alias: true, ..Default::default() };
            self.fill_path(&p, &paint);
        }
    }

    /// Draw a pixmap into logical rect `dst`.
    pub fn draw_pixmap(&mut self, src: &Pixmap, dst: Rect, opacity: f32) {
        let sx = dst.w * self.scale / src.width() as f32;
        let sy = dst.h * self.scale / src.height() as f32;
        if sx < 0.97 && sy < 0.97 && sx > 0.0 && sy > 0.0 && (src.width() as u64 * src.height() as u64) <= 2048 * 2048 {
            let (x, y) = (dst.x * self.scale, dst.y * self.scale);
            let (x0, y0) = (x.floor(), y.floor());
            let (ox, oy) = (x - x0, y - y0);
            let w = (ox + src.width() as f32 * sx).ceil().max(1.0) as u32;
            let h = (oy + src.height() as f32 * sy).ceil().max(1.0) as u32;
            let r = crate::resample(src, sx, sy, ox, oy, w, h);
            self.pm.draw_pixmap(
                x0 as i32,
                y0 as i32,
                r.as_ref(),
                &PixmapPaint { opacity, quality: FilterQuality::Nearest, ..Default::default() },
                Transform::identity(),
                None,
            );
            return;
        }
        let ts = Transform::from_row(sx, 0.0, 0.0, sy, dst.x * self.scale, dst.y * self.scale);
        self.pm.draw_pixmap(
            0,
            0,
            src.as_ref(),
            &PixmapPaint { opacity, quality: FilterQuality::Bicubic, ..Default::default() },
            ts,
            None,
        );
    }
    /// Draw a pixmap whose pixel size is already physical, at logical position.
    pub fn blit(&mut self, src: &Pixmap, x: f32, y: f32) {
        self.pm.draw_pixmap(
            (x * self.scale).round() as i32,
            (y * self.scale).round() as i32,
            src.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }

    pub fn text(&mut self, fonts: &Fonts, x: f32, baseline: f32, size: f32, w: Weight, color: Color, s: &str) -> f32 {
        fonts.draw(&mut self.pm, self.scale, x, baseline, size, w, color, s)
    }
    /// Draw text vertically centred in `r` with given horizontal alignment (0=left, 0.5=centre, 1=right).
    pub fn text_in(&mut self, fonts: &Fonts, r: Rect, align: f32, size: f32, w: Weight, color: Color, s: &str) -> f32 {
        let s = fonts.ellipsize(s, size, w, r.w);
        let tw = fonts.measure(&s, size, w);
        let x = r.x + (r.w - tw) * align;
        let baseline = r.cy() + fonts.cap_height(size, w) / 2.0;
        self.text(fonts, x, baseline, size, w, color, &s)
    }
}

pub fn solid(c: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(c);
    p.anti_alias = true;
    p
}

pub fn lin_grad(x0: f32, y0: f32, x1: f32, y1: f32, stops: &[(f32, Color)]) -> Paint<'static> {
    let sh = LinearGradient::new(
        Point::from_xy(x0, y0),
        Point::from_xy(x1, y1),
        stops.iter().map(|(p, c)| GradientStop::new(*p, *c)).collect(),
        SpreadMode::Pad,
        Transform::identity(),
    )
    .unwrap_or(Shader::SolidColor(stops[0].1));
    Paint { shader: sh, anti_alias: true, ..Default::default() }
}

pub fn rad_grad(cx: f32, cy: f32, r: f32, stops: &[(f32, Color)]) -> Paint<'static> {
    let sh = RadialGradient::new(
        Point::from_xy(cx, cy),
        Point::from_xy(cx, cy),
        r,
        stops.iter().map(|(p, c)| GradientStop::new(*p, *c)).collect(),
        SpreadMode::Pad,
        Transform::identity(),
    )
    .unwrap_or(Shader::SolidColor(stops[0].1));
    Paint { shader: sh, anti_alias: true, ..Default::default() }
}
