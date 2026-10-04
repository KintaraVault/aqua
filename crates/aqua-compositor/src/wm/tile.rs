//! Window tiling (macOS 15/26): halves, quarters, fill and centre — by dragging a window to
//! a screen edge, from the green button's menu, with ⌃⌥ shortcuts or `aqua msg action
//! tile-left`. Geometry lives in `aqua_wm::tile`; this applies it to Smithay windows.
use super::{from_rect, to_rect};
use crate::state::{meta, Aqua};
use aqua_wm::tile::{self, Arrange, Tile};
use smithay::desktop::Window;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State;
use smithay::utils::{Logical, Point, Rectangle};
use std::time::Instant;

/// The highlighted area a dragged window will fill when released.
#[derive(Clone, Debug)]
pub struct Preview {
    pub tile: Tile,
    /// Target frame (logical, title bar included).
    pub to: Rectangle<i32, Logical>,
    /// Frame the highlight grows out of (the window, or the previous target).
    pub from: Rectangle<f64, Logical>,
    pub since: Instant,
}

/// Morph time of the preview highlight.
pub const PREVIEW_SECS: f32 = 0.18;

impl Preview {
    /// Current (animated) rect and opacity.
    pub fn current(&self) -> (Rectangle<f64, Logical>, f32) {
        let t = if crate::state::reduce_motion() {
            1.0
        } else {
            (self.since.elapsed().as_secs_f32() / (PREVIEW_SECS * crate::state::anim_slow())).min(1.0)
        };
        let e = aqua_wm::spaces::ease(t) as f64;
        let to = self.to.to_f64();
        let lerp = |a: f64, b: f64| a + (b - a) * e;
        let r = Rectangle::new(
            (lerp(self.from.loc.x, to.loc.x), lerp(self.from.loc.y, to.loc.y)).into(),
            (lerp(self.from.size.w, to.size.w), lerp(self.from.size.h, to.size.h)).into(),
        );
        (r, t.min(1.0).max(0.25))
    }

    pub fn animating(&self) -> bool {
        self.since.elapsed().as_secs_f32() < PREVIEW_SECS * crate::state::anim_slow()
    }
}

impl Aqua {
    fn tile_gap(&self) -> i32 {
        if self.cfg.tile_margins {
            tile::MARGIN
        } else {
            0
        }
    }

    /// Frame a window would get when tiled at `t` on its display.
    pub fn tile_frame(&self, w: &Window, t: Tile) -> Option<Rectangle<i32, Logical>> {
        let out = self.output_of(w)?;
        let cur = self.frame_rect(w).map(|r| (r.size.w, r.size.h)).unwrap_or((800, 600));
        Some(from_rect(tile::frame(to_rect(self.usable_area(&out)), t, self.tile_gap(), cur)))
    }

    /// Tile a window. Remembers its size first so "Return to Previous Size" (or dragging it
    /// out of the tile) can bring it back.
    pub fn tile_window(&mut self, w: &Window, t: Tile) {
        if self.minimized.contains(w) || meta(w).borrow().menu_popup {
            return;
        }
        if meta(w).borrow().fullscreen.is_some() {
            self.set_fullscreen(w, false);
        }
        let Some(frame) = self.tile_frame(w, t) else { return };
        let tb = Aqua::titlebar_h(w);
        let client = from_rect(tile::client(to_rect(frame), tb));
        let was_max = Self::is_maximized(w);
        {
            let mut m = meta(w).borrow_mut();
            if m.tiled.is_none() {
                let before = if was_max { m.saved.take() } else { None };
                m.pre_tile = before.or_else(|| self.space.element_location(w).map(|loc| Rectangle::new(loc, w.geometry().size)));
            }
            m.tiled = Some(t);
        }
        self.start_geo_anim(w, frame);
        let (l, tp, r, b) = t.edges();
        if let Some(top) = w.toplevel() {
            top.with_pending_state(|s| {
                s.states.unset(State::Maximized);
                for (on, st) in [(l, State::TiledLeft), (tp, State::TiledTop), (r, State::TiledRight), (b, State::TiledBottom)] {
                    if on {
                        s.states.set(st);
                    } else {
                        s.states.unset(st);
                    }
                }
                s.size = Some(client.size);
            });
            if top.is_initial_configure_sent() {
                top.send_pending_configure();
            }
        } else if let Some(x) = w.x11_surface() {
            if was_max {
                let _ = x.set_maximized(false);
            }
            super::x11_configure(x, client);
        }
        self.space.map_element(w.clone(), client.loc, true);
        self.window_moved(w);
        self.needs_redraw = true;
    }

    fn clear_tiled_states(w: &Window, size: Option<smithay::utils::Size<i32, Logical>>) {
        if let Some(top) = w.toplevel() {
            top.with_pending_state(|s| {
                for st in [State::TiledLeft, State::TiledTop, State::TiledRight, State::TiledBottom] {
                    s.states.unset(st);
                }
                s.size = size;
            });
            if top.is_initial_configure_sent() {
                top.send_pending_configure();
            }
        }
    }

    /// "Return to Previous Size": back to the size (and place) it had before tiling.
    pub fn untile(&mut self, w: &Window) {
        let (tiled, pre) = {
            let mut m = meta(w).borrow_mut();
            (m.tiled.take(), m.pre_tile.take())
        };
        if tiled.is_none() {
            return;
        }
        let Some(out) = self.output_of(w) else { return };
        let r = match pre {
            Some(r) if self.restorable(r) => r,
            _ => self.fallback_rect(w, &out),
        };
        let tb = Aqua::titlebar_h(w);
        self.start_geo_anim(w, Rectangle::new((r.loc.x, r.loc.y - tb).into(), (r.size.w, r.size.h + tb).into()));
        Self::clear_tiled_states(w, Some(r.size));
        if let Some(x) = w.x11_surface() {
            super::x11_configure(x, r);
        }
        self.space.map_element(w.clone(), r.loc, true);
        self.needs_redraw = true;
    }

    /// A tiled window is being dragged away: give it its previous size back, keeping the
    /// grabbed point under the pointer. Returns the new client location.
    pub fn untile_for_drag(&mut self, w: &Window, pointer: Point<f64, Logical>) -> Option<Point<i32, Logical>> {
        let (tiled, pre) = {
            let m = meta(w).borrow();
            (m.tiled, m.pre_tile)
        };
        let t = tiled?;
        if t == Tile::Center {
            meta(w).borrow_mut().tiled = None;
            return None;
        }
        let cur = self.frame_rect(w)?;
        let size = pre.map(|r| r.size).filter(|s| s.w >= 48 && s.h >= 32)?;
        {
            let mut m = meta(w).borrow_mut();
            m.tiled = None;
            m.pre_tile = None;
        }
        let x = tile::untile_x(cur.loc.x, cur.size.w, pointer.x, size.w);
        let loc: Point<i32, Logical> = (x, self.space.element_location(w)?.y).into();
        Self::clear_tiled_states(w, Some(size));
        if let Some(xs) = w.x11_surface() {
            super::x11_configure(xs, Rectangle::new(loc, size));
        }
        self.space.map_element(w.clone(), loc, true);
        self.needs_redraw = true;
        Some(loc)
    }

    /// A window was moved or resized by the user: it no longer fills its tile.
    pub fn forget_tile(&mut self, w: &Window) {
        let had = meta(w).borrow_mut().tiled.take().is_some();
        if had {
            meta(w).borrow_mut().pre_tile = None;
            Self::clear_tiled_states(w, Some(w.geometry().size));
        }
    }

    /// "Fill & Arrange": the focused window and the ones behind it (same display, not
    /// minimised) share the desktop.
    pub fn arrange(&mut self, first: &Window, a: Arrange) {
        let display = self.output_of(first).map(|o| o.name());
        let mut wins = vec![first.clone()];
        for w in self.space.elements().rev() {
            if wins.len() >= a.tiles().len() {
                break;
            }
            if w == first || meta(w).borrow().menu_popup || meta(w).borrow().override_redirect {
                continue;
            }
            if self.output_of(w).map(|o| o.name()) != display || !self.on_current_space(w) {
                continue;
            }
            let transient = w.toplevel().map(|t| t.parent().is_some()).unwrap_or(false)
                || w.x11_surface().map(|x| x.is_transient_for().is_some()).unwrap_or(false);
            if !transient {
                wins.push(w.clone());
            }
        }
        for (w, t) in wins.iter().zip(a.tiles()) {
            self.tile_window(w, *t);
        }
        self.focus_window(first);
    }

    /// Pointer moved while dragging `w`: update the edge-tiling highlight.
    pub fn update_tile_preview(&mut self, w: &Window, pos: Point<f64, Logical>) {
        let target = if self.cfg.tile_by_drag {
            self.output_at(pos)
                .and_then(|o| self.space.output_geometry(&o))
                .and_then(|g| tile::edge_target((pos.x, pos.y), to_rect(g)))
        } else {
            None
        };
        let cur = self.render_cache.tile_preview.as_ref().map(|p| p.tile);
        if cur == target {
            return;
        }
        self.render_cache.tile_preview = match target.and_then(|t| self.tile_frame(w, t).map(|f| (t, f))) {
            Some((t, to)) => {
                let from = match &self.render_cache.tile_preview {
                    Some(p) => p.current().0,
                    None => self.frame_rect(w).map(|r| r.to_f64()).unwrap_or(to.to_f64()),
                };
                Some(Preview { tile: t, to, from, since: Instant::now() })
            }
            None => None,
        };
        self.needs_redraw = true;
    }

    /// Named tiling actions: `tile-left`, `tile-fill`, `tile-restore`, `arrange-quarters` …
    /// for the focused window.
    pub fn run_tile_action(&mut self, name: &str) -> bool {
        let Some(w) = self.focused_window() else { return name.starts_with("tile-") || name.starts_with("arrange-") };
        if name == "tile-restore" {
            self.untile(&w);
            return true;
        }
        if let Some(t) = name.strip_prefix("tile-").and_then(Tile::parse) {
            self.tile_window(&w, t);
            return true;
        }
        if let Some(a) = name.strip_prefix("arrange-").and_then(Arrange::parse) {
            self.arrange(&w, a);
            return true;
        }
        false
    }
}

/// Hover time on the green button before its tiling menu opens (macOS ≈ 0.5 s).
pub const ZOOM_HOVER_SECS: f32 = 0.55;

impl Aqua {
    /// Open the green button's menu once it was hovered long enough. Returns true while
    /// waiting (keeps frames coming).
    pub fn check_zoom_hover(&mut self) -> bool {
        let Some((id, since)) = self.render_cache.zoom_hover else { return false };
        if since.elapsed().as_secs_f32() < ZOOM_HOVER_SECS {
            return true;
        }
        self.render_cache.zoom_hover = Some((id, since + std::time::Duration::from_secs(3600)));
        let Some(w) = self.window_by_id(id) else { return false };
        let Some(fr) = self.frame_rect(&w) else { return false };
        let b = aqua_shell::decor::button_rect(aqua_shell::decor::Button::Zoom);
        let (x, y) = (fr.loc.x as f32 + b.x - 6.0, fr.loc.y as f32 + b.y + b.h + 6.0);
        self.open_tile_menu(&w, x, y);
        false
    }

    /// Show the tiling menu for `w` at (`x`, `y`) (logical, output coordinates).
    pub fn open_tile_menu(&mut self, w: &Window, x: f32, y: f32) {
        let (id, tiled) = {
            let m = meta(w).borrow();
            (m.id, m.tiled.is_some())
        };
        let o = self.output.as_ref().and_then(|o| self.space.output_geometry(o)).map(|g| g.loc).unwrap_or_default();
        self.shell.open_window_menu(id, tiled, x - o.x as f32, y - o.y as f32);
        self.needs_redraw = true;
    }

    /// `tilemenu` on the control socket: an app with its own title bar (Finder, Settings…)
    /// asks for the menu of its focused window under the pointer.
    pub fn tile_menu_at_pointer(&mut self) {
        let Some(w) = self.focused_window() else { return };
        let p = self.seat.get_pointer().map(|p| p.current_location()).unwrap_or_default();
        self.open_tile_menu(&w, p.x as f32 - 10.0, p.y as f32 + 14.0);
    }
}
