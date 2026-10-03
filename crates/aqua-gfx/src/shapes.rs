use crate::Rect;
use tiny_skia::{Path, PathBuilder};

/// Plain circular-arc rounded rectangle.
pub fn rrect(r: Rect, radius: f32) -> Option<Path> {
    let rad = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
    let k = 0.552_284_8 * rad;
    let (x0, y0, x1, y1) = (r.x, r.y, r.right(), r.bottom());
    let mut pb = PathBuilder::new();
    pb.move_to(x0 + rad, y0);
    pb.line_to(x1 - rad, y0);
    pb.cubic_to(x1 - rad + k, y0, x1, y0 + rad - k, x1, y0 + rad);
    pb.line_to(x1, y1 - rad);
    pb.cubic_to(x1, y1 - rad + k, x1 - rad + k, y1, x1 - rad, y1);
    pb.line_to(x0 + rad, y1);
    pb.cubic_to(x0 + rad - k, y1, x0, y1 - rad + k, x0, y1 - rad);
    pb.line_to(x0, y0 + rad);
    pb.cubic_to(x0, y0 + rad - k, x0 + rad - k, y0, x0 + rad, y0);
    pb.close();
    pb.finish()
}

/// "continuous corner" squircle (the curvature-continuous rounded rect used for app
/// icons, windows and the dock).
pub fn squircle(r: Rect, radius: f32) -> Option<Path> {
    let limit = (r.w.min(r.h) / 2.0 / 1.528).max(0.0);
    let rad = radius.min(limit);
    let (x0, y0, w, h) = (r.x, r.y, r.w, r.h);
    let mut pb = PathBuilder::new();
    let p = |x: f32, y: f32| (x0 + x, y0 + y);
    let c = rad;
    let (a, b) = p(1.528 * c, 0.0);
    pb.move_to(a, b);
    let (a, b) = p(w - 1.528 * c, 0.0);
    pb.line_to(a, b);
    let (ax, ay) = p(w - 0.63149 * c, 0.07491 * c);
    let (c1x, c1y) = p(w - 1.08849 * c, 0.0);
    let (c2x, c2y) = p(w - 0.86636 * c, 0.0);
    pb.cubic_to(c1x, c1y, c2x, c2y, ax, ay);
    let (ax, ay) = p(w - 0.07491 * c, 0.63149 * c);
    let (c1x, c1y) = p(w - 0.37282 * c, 0.16906 * c);
    let (c2x, c2y) = p(w - 0.16906 * c, 0.37282 * c);
    pb.cubic_to(c1x, c1y, c2x, c2y, ax, ay);
    let (ax, ay) = p(w, 1.528 * c);
    let (c1x, c1y) = p(w, 0.86636 * c);
    let (c2x, c2y) = p(w, 1.08849 * c);
    pb.cubic_to(c1x, c1y, c2x, c2y, ax, ay);
    let (a, b) = p(w, h - 1.528 * c);
    pb.line_to(a, b);
    let (ax, ay) = p(w - 0.07491 * c, h - 0.63149 * c);
    let (c1x, c1y) = p(w, h - 1.08849 * c);
    let (c2x, c2y) = p(w, h - 0.86636 * c);
    pb.cubic_to(c1x, c1y, c2x, c2y, ax, ay);
    let (ax, ay) = p(w - 0.63149 * c, h - 0.07491 * c);
    let (c1x, c1y) = p(w - 0.16906 * c, h - 0.37282 * c);
    let (c2x, c2y) = p(w - 0.37282 * c, h - 0.16906 * c);
    pb.cubic_to(c1x, c1y, c2x, c2y, ax, ay);
    let (ax, ay) = p(w - 1.528 * c, h);
    let (c1x, c1y) = p(w - 0.86636 * c, h);
    let (c2x, c2y) = p(w - 1.08849 * c, h);
    pb.cubic_to(c1x, c1y, c2x, c2y, ax, ay);
    let (a, b) = p(1.528 * c, h);
    pb.line_to(a, b);
    let (ax, ay) = p(0.63149 * c, h - 0.07491 * c);
    let (c1x, c1y) = p(1.08849 * c, h);
    let (c2x, c2y) = p(0.86636 * c, h);
    pb.cubic_to(c1x, c1y, c2x, c2y, ax, ay);
    let (ax, ay) = p(0.07491 * c, h - 0.63149 * c);
    let (c1x, c1y) = p(0.37282 * c, h - 0.16906 * c);
    let (c2x, c2y) = p(0.16906 * c, h - 0.37282 * c);
    pb.cubic_to(c1x, c1y, c2x, c2y, ax, ay);
    let (ax, ay) = p(0.0, h - 1.528 * c);
    let (c1x, c1y) = p(0.0, h - 0.86636 * c);
    let (c2x, c2y) = p(0.0, h - 1.08849 * c);
    pb.cubic_to(c1x, c1y, c2x, c2y, ax, ay);
    let (a, b) = p(0.0, 1.528 * c);
    pb.line_to(a, b);
    let (ax, ay) = p(0.07491 * c, 0.63149 * c);
    let (c1x, c1y) = p(0.0, 1.08849 * c);
    let (c2x, c2y) = p(0.0, 0.86636 * c);
    pb.cubic_to(c1x, c1y, c2x, c2y, ax, ay);
    let (ax, ay) = p(0.63149 * c, 0.07491 * c);
    let (c1x, c1y) = p(0.16906 * c, 0.37282 * c);
    let (c2x, c2y) = p(0.37282 * c, 0.16906 * c);
    pb.cubic_to(c1x, c1y, c2x, c2y, ax, ay);
    let (ax, ay) = p(1.528 * c, 0.0);
    let (c1x, c1y) = p(0.86636 * c, 0.0);
    let (c2x, c2y) = p(1.08849 * c, 0.0);
    pb.cubic_to(c1x, c1y, c2x, c2y, ax, ay);
    pb.close();
    pb.finish()
}

/// Icon squircle for a square icon canvas of `size` px following the icon grid
/// (824/1024 body, radius ≈ 185.4/1024).
pub fn icon_squircle(size: f32) -> Option<Path> {
    let body = size * 824.0 / 1024.0;
    let off = (size - body) / 2.0;
    squircle(Rect::new(off, off, body, body), body * 0.225 / 1.0)
}
