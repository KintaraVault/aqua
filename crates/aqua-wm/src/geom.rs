//! Integer and float rectangles in logical pixels.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
    /// Shares at least one pixel with `o`.
    pub fn overlaps(&self, o: &Rect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x as f64 && y >= self.y as f64 && x < self.right() as f64 && y < self.bottom() as f64
    }
    pub fn center(&self) -> (f64, f64) {
        (self.x as f64 + self.w as f64 / 2.0, self.y as f64 + self.h as f64 / 2.0)
    }
    /// Fully covers `o`.
    pub fn covers(&self, o: &Rect) -> bool {
        self.x <= o.x && self.y <= o.y && self.right() >= o.right() && self.bottom() >= o.bottom()
    }
    pub fn to_f64(self) -> RectF {
        RectF::new(self.x as f64, self.y as f64, self.w as f64, self.h as f64)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RectF {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl RectF {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
    pub fn cx(&self) -> f64 {
        self.x + self.w / 2.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_and_cover() {
        let a = Rect::new(0, 0, 100, 100);
        assert!(a.overlaps(&Rect::new(99, 99, 10, 10)));
        assert!(!a.overlaps(&Rect::new(100, 0, 10, 10)), "touching edges don't overlap");
        assert!(a.covers(&Rect::new(10, 10, 20, 20)));
        assert!(!Rect::new(10, 10, 20, 20).covers(&a));
        assert!(a.contains(0.0, 99.5));
        assert!(!a.contains(100.0, 5.0));
        assert_eq!(a.center(), (50.0, 50.0));
    }
}
