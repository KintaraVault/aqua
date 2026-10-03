//! Pointer and drag-and-drop icon.
use super::*;

impl Aqua {
    pub(super) fn push_cursor(&mut self, renderer: &mut GlesRenderer, out: &mut Vec<AquaElement>, scale: f64) {
        let Some(ptr) = self.seat.get_pointer() else { return };
        let pos = ptr.current_location();
        let named = match (&self.render_cache.cursor_override, &self.render_cache.cursor_status) {
            (Some(i), _) => Some(*i),
            (None, CursorImageStatus::Named(i)) => Some(*i),
            _ => None,
        };
        if let Some(icon) = named {
            let size = (self.cfg.cursor_size as f64).clamp(0.5, 4.0);
            let shape = crate::input::cursors::shape(icon);
            let key = (shape, (scale * size * 100.0).round() as u64);
            let buf = self.render_cache.cursor.entry(key).or_insert_with(|| {
                buffer_from_pixmap(&crate::input::cursors::draw(shape, (scale * size) as f32), false)
            });
            let (hx, hy) = crate::input::cursors::hotspot(shape);
            let loc: Point<f64, Physical> =
                (((pos.x - hx as f64 * size) * scale).round(), ((pos.y - hy as f64 * size) * scale).round()).into();
            let side = (crate::input::cursors::BOX as f64 * size).round() as i32;
            if let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                loc,
                buf,
                None,
                None,
                Some((side, side).into()),
                Kind::Cursor,
            ) {
                out.push(AquaElement::Memory(e));
            }
            return;
        }
        if let CursorImageStatus::Surface(surface) = &self.render_cache.cursor_status {
            let hotspot = smithay::wayland::compositor::with_states(surface, |s| {
                s.data_map
                    .get::<std::sync::Mutex<smithay::input::pointer::CursorImageAttributes>>()
                    .map(|a| a.lock().unwrap().hotspot)
                    .unwrap_or_default()
            });
            let loc = (pos - hotspot.to_f64()).to_physical(scale).to_i32_round();
            let elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                render_elements_from_surface_tree(renderer, surface, loc, scale, 1.0, Kind::Cursor);
            out.extend(elems.into_iter().map(AquaElement::Surface));
        }
    }

    /// The drag-and-drop icon under the pointer (or flying back after a refused drop).
    pub(super) fn push_dnd_icon(&mut self, renderer: &mut GlesRenderer, out: &mut Vec<AquaElement>, scale: f64) {
        if let Some(fd) = &self.render_cache.file_drag {
            let grabbed = self.seat.get_pointer().map(|p| p.is_grabbed()).unwrap_or(false);
            if !grabbed || !fd.alive.load(std::sync::atomic::Ordering::Relaxed) {
                self.render_cache.file_drag = None;
                self.needs_redraw = true;
            } else {
                let ptr = self.seat.get_pointer().map(|p| p.current_location()).unwrap_or_default();
                let loc = (ptr - Point::<f64, Logical>::from((32.0, 26.0))).to_physical(scale);
                let size: Size<i32, Logical> = (64, 64).into();
                if let Ok(m) = MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    loc,
                    &fd.buf,
                    Some(0.9),
                    None,
                    Some(size),
                    Kind::Unspecified,
                ) {
                    out.push(AquaElement::Memory(m));
                }
            }
        }
        let Some(icon) = &self.render_cache.dnd_icon else { return };
        if !smithay::utils::IsAlive::alive(&icon.surface) {
            self.render_cache.dnd_icon = None;
            return;
        }
        let ptr = self.seat.get_pointer().map(|p| p.current_location()).unwrap_or_default();
        let (pos, alpha) = match icon.snap {
            None => (ptr, 1.0f32),
            Some((from, at)) => {
                let t = (at.elapsed().as_secs_f32() / DND_SNAP_SECS).min(1.0);
                if t >= 1.0 {
                    self.render_cache.dnd_icon = None;
                    return;
                }
                let e = 1.0 - (1.0 - t).powi(3);
                self.needs_redraw = true;
                let p = from + (icon.origin - from).upscale(e as f64);
                (p, 1.0 - 0.6 * t * t)
            }
        };
        let loc = (pos + icon.offset.to_f64()).to_physical(scale).to_i32_round();
        let elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
            render_elements_from_surface_tree(renderer, &icon.surface, loc, scale, alpha, Kind::Unspecified);
        out.extend(elems.into_iter().map(AquaElement::Surface));
    }
}
