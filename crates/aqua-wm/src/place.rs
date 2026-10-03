//! Window geometry rules: where new windows open, zoom and restore rectangles, rescuing
//! windows from displays that went away, full-screen detection.
use crate::geom::Rect;

/// Space taken by the shell around windows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chrome {
    pub menubar: i32,
    /// Title bar height of the window (0 for client-side decorations).
    pub titlebar: i32,
    /// Height reserved for the Dock.
    pub dock: i32,
}

/// Largest size a new window may open with on `display`.
pub fn max_initial_size(display: Rect, c: Chrome) -> (i32, i32) {
    (display.w - 80, display.h - c.menubar - c.dock - c.titlebar - 40)
}

/// Position of a new window of `size`: centred (slightly above centre) on `display`,
/// cascading by 28 px for each of the `n` windows already on that desk.
pub fn cascade(display: Rect, size: (i32, i32), n: usize, c: Chrome) -> (i32, i32) {
    let (w, h) = size;
    let off = (n as i32 % 6) * 28;
    let avail_h = display.h - c.menubar - c.dock;
    let top = c.menubar + c.titlebar;
    let x = display.x + ((display.w - w) / 2 + off).clamp(0, (display.w - w).max(0));
    let y = display.y + (top + ((avail_h - h - c.titlebar) / 2).max(8) + off).clamp(top, (display.h - h).max(top));
    (x, y)
}

/// Client rect of a zoomed window: the usable area minus a small gap, below the title bar.
pub fn zoom_rect(usable: Rect, titlebar: i32) -> Rect {
    let gap = 6;
    Rect::new(
        usable.x + gap,
        usable.y + titlebar + gap,
        (usable.w - 2 * gap).max(64),
        (usable.h - titlebar - 2 * gap).max(64),
    )
}

/// Where an un-zoomed window goes without usable remembered geometry: two thirds of the
/// usable area (at least `min`), centred.
pub fn fallback_rect(usable: Rect, titlebar: i32, min: (i32, i32)) -> Rect {
    let w = (usable.w * 2 / 3).max(min.0).min(usable.w);
    let h = (usable.h * 2 / 3).max(min.1).min((usable.h - titlebar).max(32));
    Rect::new(usable.x + (usable.w - w) / 2, usable.y + titlebar + (usable.h - titlebar - h) / 2, w, h)
}

/// A remembered rect is worth restoring: sane size and still on some display.
pub fn restorable(r: Rect, displays: &[Rect]) -> bool {
    r.w >= 48 && r.h >= 32 && displays.iter().any(|d| d.overlaps(&r))
}

/// New home position for a window whose home rect lies outside every display (display
/// unplugged): centred on `primary`, below the menu bar. `None` = still visible.
pub fn rescue(home: Rect, displays: &[Rect], primary: Rect, menubar: i32, titlebar: i32) -> Option<(i32, i32)> {
    if displays.is_empty() || displays.iter().any(|d| d.overlaps(&home)) {
        return None;
    }
    let x = primary.x + ((primary.w - home.w) / 2).max(0);
    let y = primary.y + ((primary.h - home.h) / 2).max(menubar + titlebar);
    Some((x, y))
}

/// Index of the display a window belongs to: the one under its centre, else under its
/// top-left corner.
pub fn display_of(frame: Rect, displays: &[Rect]) -> Option<usize> {
    let (cx, cy) = frame.center();
    displays
        .iter()
        .position(|d| d.contains(cx, cy))
        .or_else(|| displays.iter().position(|d| d.contains(frame.x as f64, frame.y as f64)))
}

/// A borderless window covering the whole display counts as full-screen (games, players).
pub fn covers_display(frame: Rect, display: Rect, decorated: bool) -> bool {
    !decorated && frame.covers(&display)
}

#[cfg(test)]
mod tests {
    use super::*;

    const C: Chrome = Chrome { menubar: 26, titlebar: 38, dock: 84 };
    const D: Rect = Rect::new(0, 0, 1440, 900);

    #[test]
    fn new_windows_are_centred_and_cascade() {
        let (x, y) = cascade(D, (800, 500), 0, C);
        assert_eq!(x, 320);
        assert!(y >= C.menubar + C.titlebar && y + 500 <= D.h);
        let (x2, y2) = cascade(D, (800, 500), 1, C);
        assert_eq!((x2 - x, y2 - y), (28, 28));
        assert_eq!(cascade(D, (800, 500), 6, C), (x, y), "cascade wraps after six");
        let second = Rect::new(1440, 0, 2560, 1440);
        let (sx, _) = cascade(second, (800, 500), 0, C);
        assert_eq!(sx, 1440 + (2560 - 800) / 2);
    }

    #[test]
    fn huge_windows_stay_below_the_menu_bar() {
        let (x, y) = cascade(D, (3000, 2000), 3, C);
        assert_eq!(x, 0);
        assert_eq!(y, C.menubar + C.titlebar);
        let (mw, mh) = max_initial_size(D, C);
        assert_eq!((mw, mh), (1360, 900 - 26 - 84 - 38 - 40));
    }

    #[test]
    fn zoom_and_fallback_rects() {
        let usable = Rect::new(0, 26, 1440, 790);
        let z = zoom_rect(usable, 38);
        assert_eq!(z, Rect::new(6, 26 + 38 + 6, 1428, 790 - 38 - 12));
        assert_eq!(zoom_rect(Rect::new(0, 0, 10, 10), 38).w, 64);
        let f = fallback_rect(usable, 38, (0, 0));
        assert_eq!((f.w, f.h), (960, 526));
        assert_eq!(f.x, 240);
        let big_min = fallback_rect(usable, 38, (2000, 2000));
        assert_eq!((big_min.w, big_min.h), (1440, 790 - 38));
    }

    #[test]
    fn restore_and_rescue() {
        let displays = [D, Rect::new(1440, 0, 1920, 1080)];
        assert!(restorable(Rect::new(1500, 100, 400, 300), &displays));
        assert!(!restorable(Rect::new(5000, 100, 400, 300), &displays));
        assert!(!restorable(Rect::new(10, 10, 20, 20), &displays));
        let gone = Rect::new(3500, 100, 600, 400);
        assert_eq!(rescue(gone, &displays, D, 26, 38), Some((420, 250)));
        assert_eq!(rescue(Rect::new(100, 100, 50, 50), &displays, D, 26, 38), None);
        assert_eq!(rescue(gone, &[], D, 26, 38), None);
        assert_eq!(rescue(Rect::new(4000, 0, 2000, 2000), &[D], D, 26, 38), Some((0, 64)));
    }

    #[test]
    fn display_membership_and_fullscreen() {
        let displays = [D, Rect::new(1440, 0, 1920, 1080)];
        assert_eq!(display_of(Rect::new(1300, 100, 400, 300), &displays), Some(1));
        assert_eq!(display_of(Rect::new(100, 100, 400, 300), &displays), Some(0));
        assert_eq!(display_of(Rect::new(9000, 0, 10, 10), &displays), None);
        assert!(covers_display(Rect::new(-1, -1, 1442, 902), D, false));
        assert!(!covers_display(Rect::new(0, 0, 1440, 900), D, true));
        assert!(!covers_display(Rect::new(0, 0, 1440, 899), D, false));
    }
}
