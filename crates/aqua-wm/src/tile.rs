//! Window tiling as in macOS 15/26: drag a window to a screen edge or corner (or pick a
//! layout from the green button's menu / a shortcut) and it fills half, a quarter or all of
//! the desktop. Pure geometry so it is unit-tested; the compositor applies the results.
use crate::geom::Rect;

/// Where a window can be tiled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tile {
    Fill,
    Center,
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Tile {
    pub const ALL: [Tile; 10] = [
        Tile::Fill,
        Tile::Center,
        Tile::Left,
        Tile::Right,
        Tile::Top,
        Tile::Bottom,
        Tile::TopLeft,
        Tile::TopRight,
        Tile::BottomLeft,
        Tile::BottomRight,
    ];

    /// Stable name used by actions, shortcuts and the control socket (`tile left`).
    pub fn name(self) -> &'static str {
        match self {
            Tile::Fill => "fill",
            Tile::Center => "center",
            Tile::Left => "left",
            Tile::Right => "right",
            Tile::Top => "top",
            Tile::Bottom => "bottom",
            Tile::TopLeft => "top-left",
            Tile::TopRight => "top-right",
            Tile::BottomLeft => "bottom-left",
            Tile::BottomRight => "bottom-right",
        }
    }

    pub fn parse(s: &str) -> Option<Tile> {
        Tile::ALL.into_iter().find(|t| t.name() == s)
    }

    /// Menu label (English source string, translated by the shell).
    pub fn label(self) -> &'static str {
        match self {
            Tile::Fill => "Fill",
            Tile::Center => "Center",
            Tile::Left => "Left",
            Tile::Right => "Right",
            Tile::Top => "Top",
            Tile::Bottom => "Bottom",
            Tile::TopLeft => "Top Left",
            Tile::TopRight => "Top Right",
            Tile::BottomLeft => "Bottom Left",
            Tile::BottomRight => "Bottom Right",
        }
    }

    /// Which edges of the desktop the tile touches (left, top, right, bottom) — sent to
    /// clients as xdg "tiled" states so they drop their shadows / rounded corners there.
    pub fn edges(self) -> (bool, bool, bool, bool) {
        match self {
            Tile::Fill => (true, true, true, true),
            Tile::Center => (false, false, false, false),
            Tile::Left => (true, true, false, true),
            Tile::Right => (false, true, true, true),
            Tile::Top => (true, true, true, false),
            Tile::Bottom => (true, false, true, true),
            Tile::TopLeft => (true, true, false, false),
            Tile::TopRight => (false, true, true, false),
            Tile::BottomLeft => (true, false, false, true),
            Tile::BottomRight => (false, false, true, true),
        }
    }
}

/// Gap between tiled windows and around them ("Tiled windows have margins").
pub const MARGIN: i32 = 8;

/// Outer frame (title bar included) of a window tiled at `t` in `usable` (the desktop
/// without menu bar and Dock). `gap` is the margin, 0 for edge-to-edge tiling.
/// `size` is the window's current frame size (only used by `Center`).
pub fn frame(usable: Rect, t: Tile, gap: i32, size: (i32, i32)) -> Rect {
    let g = gap.max(0);
    let (x0, y0) = (usable.x + g, usable.y + g);
    let (w, h) = ((usable.w - 2 * g).max(64), (usable.h - 2 * g).max(64));
    // Halves share the middle gap: each side gets (w - g) / 2, the right one takes the rest.
    let lw = (w - g) / 2;
    let rw = w - g - lw;
    let th = (h - g) / 2;
    let bh = h - g - th;
    let (rx, by) = (x0 + lw + g, y0 + th + g);
    match t {
        Tile::Fill => Rect::new(x0, y0, w, h),
        Tile::Center => {
            let cw = size.0.clamp(64, w);
            let ch = size.1.clamp(64, h);
            Rect::new(usable.x + (usable.w - cw) / 2, usable.y + (usable.h - ch) / 2, cw, ch)
        }
        Tile::Left => Rect::new(x0, y0, lw, h),
        Tile::Right => Rect::new(rx, y0, rw, h),
        Tile::Top => Rect::new(x0, y0, w, th),
        Tile::Bottom => Rect::new(x0, by, w, bh),
        Tile::TopLeft => Rect::new(x0, y0, lw, th),
        Tile::TopRight => Rect::new(rx, y0, rw, th),
        Tile::BottomLeft => Rect::new(x0, by, lw, bh),
        Tile::BottomRight => Rect::new(rx, by, rw, bh),
    }
}

/// Client (content) rect for a frame: below a server-side title bar of `titlebar` px.
pub fn client(frame: Rect, titlebar: i32) -> Rect {
    Rect::new(frame.x, frame.y + titlebar, frame.w, (frame.h - titlebar).max(32))
}

/// How close (logical px) the pointer must be to a display edge to tile there.
pub const EDGE: f64 = 3.0;

/// Tile chosen by dragging a window with the pointer at `p` on `display`:
/// the top edge fills, the left / right edges make halves and the four corners quarters
/// (the bottom edge belongs to the Dock and does nothing).
pub fn edge_target(p: (f64, f64), display: Rect) -> Option<Tile> {
    let (x, y) = p;
    let (l, t) = (display.x as f64, display.y as f64);
    let (r, b) = (display.right() as f64, display.bottom() as f64);
    if x < l - 1.0 || x > r + 1.0 || y < t - 1.0 || y > b + 1.0 {
        return None;
    }
    let corner = (display.h as f64 / 7.0).clamp(48.0, 140.0);
    let at_l = x <= l + EDGE;
    let at_r = x >= r - 1.0 - EDGE;
    let at_t = y <= t + EDGE;
    let at_b = y >= b - 1.0 - EDGE;
    let near_t = y <= t + corner;
    let near_b = y >= b - corner;
    let near_l = x <= l + corner;
    let near_r = x >= r - corner;
    match () {
        _ if (at_l && near_t) || (at_t && near_l) => Some(Tile::TopLeft),
        _ if (at_r && near_t) || (at_t && near_r) => Some(Tile::TopRight),
        _ if (at_l && near_b) || (at_b && near_l) => Some(Tile::BottomLeft),
        _ if (at_r && near_b) || (at_b && near_r) => Some(Tile::BottomRight),
        _ if at_l => Some(Tile::Left),
        _ if at_r => Some(Tile::Right),
        _ if at_t => Some(Tile::Fill),
        _ => None,
    }
}

/// "Fill & Arrange" layouts for the front-most windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrange {
    LeftRight,
    RightLeft,
    TopBottom,
    BottomTop,
    Quarters,
}

impl Arrange {
    pub const ALL: [Arrange; 5] =
        [Arrange::LeftRight, Arrange::RightLeft, Arrange::TopBottom, Arrange::BottomTop, Arrange::Quarters];

    pub fn name(self) -> &'static str {
        match self {
            Arrange::LeftRight => "left-right",
            Arrange::RightLeft => "right-left",
            Arrange::TopBottom => "top-bottom",
            Arrange::BottomTop => "bottom-top",
            Arrange::Quarters => "quarters",
        }
    }

    pub fn parse(s: &str) -> Option<Arrange> {
        Arrange::ALL.into_iter().find(|a| a.name() == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            Arrange::LeftRight => "Left & Right",
            Arrange::RightLeft => "Right & Left",
            Arrange::TopBottom => "Top & Bottom",
            Arrange::BottomTop => "Bottom & Top",
            Arrange::Quarters => "Quarters",
        }
    }

    /// Tiles for the front-most windows, front-most first.
    pub fn tiles(self) -> &'static [Tile] {
        match self {
            Arrange::LeftRight => &[Tile::Left, Tile::Right],
            Arrange::RightLeft => &[Tile::Right, Tile::Left],
            Arrange::TopBottom => &[Tile::Top, Tile::Bottom],
            Arrange::BottomTop => &[Tile::Bottom, Tile::Top],
            Arrange::Quarters => &[Tile::TopLeft, Tile::TopRight, Tile::BottomLeft, Tile::BottomRight],
        }
    }
}

/// Dragging a tiled window away restores its previous size; keep the grabbed point under
/// the pointer at the same relative x (clamped so the title bar stays grabbable).
/// Returns the new frame x for a frame `old` (x, width) grabbed at pointer `px` that becomes
/// `new_w` wide.
pub fn untile_x(old_x: i32, old_w: i32, px: f64, new_w: i32) -> i32 {
    let rel = ((px - old_x as f64) / old_w.max(1) as f64).clamp(0.0, 1.0);
    let x = px - rel * new_w as f64;
    // At least 80 px of the frame stay on either side of the pointer when possible.
    let lo = px - (new_w as f64 - 80.0).max(0.0);
    let hi = px - 80.0_f64.min(new_w as f64);
    x.clamp(lo.min(hi), hi.max(lo)).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: Rect = Rect::new(0, 0, 1440, 900);
    /// Desktop minus a 30 px menu bar and an 80 px Dock.
    const U: Rect = Rect::new(0, 30, 1440, 790);

    #[test]
    fn names_round_trip() {
        for t in Tile::ALL {
            assert_eq!(Tile::parse(t.name()), Some(t));
        }
        for a in Arrange::ALL {
            assert_eq!(Arrange::parse(a.name()), Some(a));
        }
        assert_eq!(Tile::parse("nope"), None);
    }

    #[test]
    fn halves_fill_the_desktop_with_margins() {
        let l = frame(U, Tile::Left, MARGIN, (0, 0));
        let r = frame(U, Tile::Right, MARGIN, (0, 0));
        assert_eq!(l.x, U.x + MARGIN);
        assert_eq!(l.y, U.y + MARGIN);
        assert_eq!(r.right(), U.right() - MARGIN);
        assert_eq!(r.x - l.right(), MARGIN, "one margin between the halves");
        assert_eq!(l.h, U.h - 2 * MARGIN);
        assert!(!l.overlaps(&r));
    }

    #[test]
    fn odd_sizes_lose_no_pixel() {
        let u = Rect::new(3, 31, 1367, 761);
        for g in [0, MARGIN] {
            let l = frame(u, Tile::Left, g, (0, 0));
            let r = frame(u, Tile::Right, g, (0, 0));
            assert_eq!(l.w + r.w + 3 * g, u.w);
            let t = frame(u, Tile::TopLeft, g, (0, 0));
            let b = frame(u, Tile::BottomLeft, g, (0, 0));
            assert_eq!(t.h + b.h + 3 * g, u.h);
            assert_eq!(b.bottom(), u.bottom() - g);
        }
    }

    #[test]
    fn quarters_do_not_overlap_and_cover() {
        let q: Vec<Rect> = Arrange::Quarters.tiles().iter().map(|t| frame(U, *t, 0, (0, 0))).collect();
        for i in 0..4 {
            for j in i + 1..4 {
                assert!(!q[i].overlaps(&q[j]), "{i} {j}");
            }
        }
        let area: i32 = q.iter().map(|r| r.w * r.h).sum();
        assert_eq!(area, U.w * U.h);
    }

    #[test]
    fn fill_without_margin_is_the_usable_area() {
        assert_eq!(frame(U, Tile::Fill, 0, (0, 0)), U);
    }

    #[test]
    fn center_keeps_the_size_but_fits() {
        let c = frame(U, Tile::Center, MARGIN, (800, 600));
        assert_eq!((c.w, c.h), (800, 600));
        assert_eq!(c.x + c.w / 2, U.x + U.w / 2);
        let big = frame(U, Tile::Center, MARGIN, (4000, 3000));
        assert!(big.w <= U.w && big.h <= U.h);
    }

    #[test]
    fn client_rect_is_below_the_title_bar() {
        let f = Rect::new(10, 40, 500, 400);
        assert_eq!(client(f, 28), Rect::new(10, 68, 500, 372));
        assert_eq!(client(f, 0), f);
    }

    #[test]
    fn edges_and_corners() {
        assert_eq!(edge_target((0.0, 450.0), D), Some(Tile::Left));
        assert_eq!(edge_target((1439.0, 450.0), D), Some(Tile::Right));
        assert_eq!(edge_target((720.0, 0.0), D), Some(Tile::Fill));
        assert_eq!(edge_target((0.0, 10.0), D), Some(Tile::TopLeft));
        assert_eq!(edge_target((30.0, 0.0), D), Some(Tile::TopLeft));
        assert_eq!(edge_target((1439.0, 0.0), D), Some(Tile::TopRight));
        assert_eq!(edge_target((0.0, 899.0), D), Some(Tile::BottomLeft));
        assert_eq!(edge_target((1439.0, 880.0), D), Some(Tile::BottomRight));
        // the bottom edge between the corners is the Dock's
        assert_eq!(edge_target((720.0, 899.0), D), None);
        // not at an edge
        assert_eq!(edge_target((720.0, 450.0), D), None);
        assert_eq!(edge_target((10.0, 450.0), D), None);
    }

    #[test]
    fn edges_of_a_second_display() {
        let d2 = Rect::new(1440, 0, 1920, 1080);
        assert_eq!(edge_target((1440.0, 500.0), d2), Some(Tile::Left));
        assert_eq!(edge_target((3359.0, 500.0), d2), Some(Tile::Right));
        assert_eq!(edge_target((100.0, 500.0), d2), None, "pointer on another display");
    }

    #[test]
    fn tiled_edges_match_the_tile() {
        assert_eq!(Tile::Left.edges(), (true, true, false, true));
        assert_eq!(Tile::Center.edges(), (false, false, false, false));
        assert_eq!(Tile::BottomRight.edges(), (false, false, true, true));
    }

    #[test]
    fn untiling_keeps_the_grab_point() {
        // grabbed in the middle of a 700 px half, restored to 400 px: still the middle
        assert_eq!(untile_x(8, 700, 358.0, 400), 158);
        // grabbed at the far right: the restored frame keeps 80 px right of the pointer
        let x = untile_x(0, 700, 699.0, 400);
        assert_eq!(x + 400 - 699, 80);
        // grabbed at the far left
        let x = untile_x(0, 700, 1.0, 400);
        assert_eq!(1 - x, 80);
    }
}
