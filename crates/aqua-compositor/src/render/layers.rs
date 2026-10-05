//! Wallpaper, dimming, lock surfaces and shell / layer-shell layers.
use super::*;

impl Aqua {
    /// Upload the wallpaper variants the current appearance needs (day, night, or
    /// both while cross-fading) and drop the other one: a 4K buffer is ~33 MB.
    pub(super) fn ensure_wallpapers(&mut self) -> f32 {
        let wp_serial = self.wallpaper_serial;
        let dk = self.dark_progress();
        let rc = &mut self.render_cache;
        if dk < 1.0 {
            if rc.wallpaper.as_ref().map(|(s, _)| *s != wp_serial).unwrap_or(true) {
                rc.wallpaper = Some((wp_serial, buffer_from_pixmap(&self.wallpaper, true)));
            }
        } else if rc.wallpaper.take().is_some() {
            rc.freed += self.wallpaper.data().len() * 2;
        }
        if dk > 0.0 {
            if rc.wallpaper_night.as_ref().map(|(s, _)| *s != wp_serial).unwrap_or(true) {
                let night = aqua_wallpaper::night(&self.wallpaper);
                rc.wallpaper_night = Some((wp_serial, buffer_from_pixmap(&night, true)));
            }
        } else if rc.wallpaper_night.take().is_some() {
            rc.freed += self.wallpaper.data().len() * 2;
        }
        dk
    }

    pub(super) fn push_wallpaper(&mut self, renderer: &mut GlesRenderer, out: &mut Vec<AquaElement>) {
        let size = self.output_size();
        self.push_wallpaper_sized(renderer, out, size);
    }

    /// Black overlay for idle dimming (and "display off" when there is no DPMS).
    pub(super) fn push_dim(&mut self, out: &mut Vec<AquaElement>, scale: f64) {
        let mut a = self.idle.dim_amount();
        if self.udev.as_ref().map(|u| u.dpms_off).unwrap_or(false) {
            a = 1.0;
        }
        if a <= 0.001 {
            return;
        }
        let b = self.layout_bounds();
        let rc = &mut self.render_cache;
        if (rc.dim.2 - a).abs() > 0.0005 {
            rc.dim.1.increment();
            rc.dim.2 = a;
        }
        let geo = Rectangle::new(b.loc.to_physical_precise_round(scale), b.size.to_physical_precise_round(scale));
        out.push(AquaElement::Solid(smithay::backend::renderer::element::solid::SolidColorRenderElement::new(
            rc.dim.0.clone(),
            geo,
            rc.dim.1,
            [0.0, 0.0, 0.0, a],
            Kind::Unspecified,
        )));
    }

    pub(super) fn push_ext_lock(
        &mut self,
        renderer: &mut GlesRenderer,
        out: &mut Vec<AquaElement>,
        o: &smithay::output::Output,
        scale: f64,
    ) {
        let Some(og) = self.space.output_geometry(o) else { return };
        for (lo, s) in &self.lock.ext_surfaces {
            if lo == o {
                let loc = og.loc.to_physical_precise_round(scale);
                let els: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                    render_elements_from_surface_tree(renderer, s.wl_surface(), loc, scale, 1.0, Kind::Unspecified);
                out.extend(els.into_iter().map(AquaElement::Surface));
            }
        }
    }

    /// wlr-layer-shell surfaces of one layer on an output (front first), with their popups.
    pub(super) fn push_layer_surfaces(
        &mut self,
        renderer: &mut GlesRenderer,
        out: &mut Vec<AquaElement>,
        o: &smithay::output::Output,
        which: smithay::wayland::shell::wlr_layer::Layer,
        scale: f64,
    ) {
        let Some(og) = self.space.output_geometry(o) else { return };
        let map = smithay::desktop::layer_map_for_output(o);
        for l in map.layers_on(which).rev() {
            let Some(g) = map.layer_geometry(l) else { continue };
            let base = og.loc + g.loc;
            for (popup, ploc) in PopupManager::popups_for_surface(l.wl_surface()) {
                let pl = (base + ploc - popup.geometry().loc).to_physical_precise_round(scale);
                let els: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                    render_elements_from_surface_tree(renderer, popup.wl_surface(), pl, scale, 1.0, Kind::Unspecified);
                out.extend(els.into_iter().map(AquaElement::Surface));
            }
            let els: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = render_elements_from_surface_tree(
                renderer,
                l.wl_surface(),
                base.to_physical_precise_round(scale),
                scale,
                1.0,
                Kind::Unspecified,
            );
            out.extend(els.into_iter().map(AquaElement::Surface));
        }
    }

    pub(super) fn push_wallpaper_sized(
        &mut self,
        renderer: &mut GlesRenderer,
        out: &mut Vec<AquaElement>,
        size: (i32, i32),
    ) {
        let dk = self.ensure_wallpapers();
        let src = crate::render::pixmap_src(&self.wallpaper);
        if let Some((_, nb)) = self.render_cache.wallpaper_night.as_ref().filter(|_| dk > 0.0) {
            if let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                (0.0, 0.0),
                nb,
                Some(dk),
                src,
                Some(size.into()),
                Kind::Unspecified,
            ) {
                out.push(AquaElement::Memory(e));
            }
        }
        if let Some((_, buf)) = self.render_cache.wallpaper.as_ref().filter(|_| dk < 1.0) {
            if let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                (0.0, 0.0),
                buf,
                None,
                src,
                Some(size.into()),
                Kind::Unspecified,
            ) {
                out.push(AquaElement::Memory(e));
            }
        }
    }

    pub(super) fn push_layer(
        &mut self,
        renderer: &mut GlesRenderer,
        out: &mut Vec<AquaElement>,
        l: &aqua_shell::Layer,
        scale: f64,
    ) {
        if l.opacity <= 0.002 {
            return;
        }
        let rc = &mut self.render_cache;
        let now = Instant::now();
        let entry = rc.layers.get_mut(&l.id);
        let buf = match entry {
            Some((s, buf, used)) if *s == l.serial => {
                *used = now;
                buf.clone()
            }
            _ => {
                let buf = buffer_from_pixmap(&l.content, false);
                rc.layers.insert(l.id, (l.serial, buf.clone(), now));
                buf
            }
        };
        let zoom = l.zoom as f64;
        let cx = (l.rect.x + l.rect.w / 2.0) as f64;
        let cy = (l.rect.y + l.rect.h / 2.0) as f64;
        let origin: Point<i32, Physical> = ((cx * scale).round() as i32, (cy * scale).round() as i32).into();
        let loc: Point<f64, Physical> = ((l.rect.x as f64 * scale).round(), (l.rect.y as f64 * scale).round()).into();
        let size: Size<i32, Logical> = ((l.rect.w).round() as i32, (l.rect.h).round() as i32).into();
        if let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            loc,
            &buf,
            Some(l.opacity),
            crate::render::pixmap_src(&l.content),
            Some(size),
            Kind::Unspecified,
        ) {
            out.push(AquaElement::Scaled(RescaleRenderElement::from_element(e, origin, zoom)));
        }
        let zr = |r: &aqua_gfx::Rect| -> Rectangle<f64, Logical> {
            let x = cx + (r.x as f64 - cx) * zoom;
            let y = cy + (r.y as f64 - cy) * zoom;
            Rectangle::new((x, y).into(), (r.w as f64 * zoom, r.h as f64 * zoom).into())
        };
        for (i, (r, g)) in l.tiles.iter().enumerate().rev() {
            let mut p = glass_params(g, (scale * zoom) as f32);
            p.blur = g.blur * scale as f32;
            if let Some(e) = rc.glass_el(GlassKey::Layer(l.id, i + 1), to_phys(zr(r), scale), p, l.opacity) {
                out.push(AquaElement::Glass(e));
            }
        }
        if let Some(g) = &l.glass {
            let mut p = glass_params(g, (scale * zoom) as f32);
            p.blur = g.blur * scale as f32;
            if let Some(e) = rc.glass_el(GlassKey::Layer(l.id, 0), to_phys(zr(&l.rect), scale), p, l.opacity) {
                out.push(AquaElement::Glass(e));
            }
        }
    }
}
