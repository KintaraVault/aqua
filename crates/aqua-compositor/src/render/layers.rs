//! Wallpaper, dimming, lock surfaces and shell / layer-shell layers.
use super::*;

impl Aqua {
    /// Upload the wallpaper variants the current appearance needs (day, night, or
    /// both while cross-fading) and drop the other one: a 4K buffer is ~33 MB.
    pub(super) fn ensure_wallpapers(&mut self) -> f32 {
        let wp_serial = self.wallpaper_serial;
        // macOS shows a user-chosen still image unchanged in Dark Mode; only the built-in
        // generated wallpaper has a night variant.
        let dk = if self.cfg.wallpaper.is_some() { 0.0 } else { self.dark_progress() };
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
        let out_size = self.output_size();
        let wallpaper = self.wallpaper.clone();
        let veil = self.veil_params(l, scale);
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
            adapt_to_backdrop(&mut p, &wallpaper, r, out_size);
            if let Some(e) = rc.glass_el(GlassKey::Layer(l.id, i + 1), to_phys(zr(r), scale), p, l.opacity) {
                out.push(AquaElement::Glass(e));
            }
        }
        if let Some(g) = &l.glass {
            let mut p = glass_params(g, (scale * zoom) as f32);
            p.blur = g.blur * scale as f32;
            adapt_to_backdrop(&mut p, &wallpaper, &l.rect, out_size);
            if let Some(e) = rc.glass_el(GlassKey::Layer(l.id, 0), to_phys(zr(&l.rect), scale), p, l.opacity) {
                out.push(AquaElement::Glass(e));
            }
        }
        // behind everything of the layer (`out` is front to back)
        if let Some((r, p)) = veil {
            if let Some(e) = rc.glass_el(GlassKey::Layer(l.id, VEIL_KEY), to_phys(r, scale), p, l.opacity) {
                out.push(AquaElement::Glass(e));
            }
        }
    }

    /// macOS blurs the desktop around popovers that drop from the menu bar (Control Center,
    /// Notification Center, menu-bar menus): a progressive blur that is full near the panel
    /// and fades to the sharp backdrop ~40 pt outside it (measured on the Control Center
    /// capture). Context menus elsewhere do not get it.
    fn veil_params(&self, l: &aqua_shell::Layer, scale: f64) -> Option<(Rectangle<f64, Logical>, GlassParams)> {
        use aqua_shell::LayerId as L;
        let mb = self.cfg.menubar_height;
        let from_bar = match l.id {
            L::ControlCenter | L::NotificationCenter => true,
            L::Menu | L::MenuFade => l.rect.y <= mb + 12.0,
            _ => false,
        };
        if !from_bar {
            return None;
        }
        // the visible extent of the layer: its glass, else the union of its tiles
        let mut b = l.glass.as_ref().map(|_| l.rect);
        for (r, _) in &l.tiles {
            b = Some(match b {
                None => *r,
                Some(u) => {
                    let (x0, y0) = (u.x.min(r.x), u.y.min(r.y));
                    aqua_gfx::Rect::new(x0, y0, u.right().max(r.right()) - x0, u.bottom().max(r.bottom()) - y0)
                }
            });
        }
        let b = b?;
        let m = VEIL_MARGIN;
        // stops at the menu bar (drawn below the popover, it must stay sharp)
        let (x0, y0) = (b.x - m, (b.y - m).max(mb));
        let (x1, y1) = (b.right() + m, b.bottom() + m);
        let r =
            Rectangle::<f64, Logical>::new((x0 as f64, y0 as f64).into(), ((x1 - x0) as f64, (y1 - y0) as f64).into());
        let g = self.cfg.glass.material(Material::Overlay, false).with_radius(m + 20.0);
        let mut p = glass_params(&g, scale as f32);
        p.variant = 210;
        p.dark = false;
        p.blur = VEIL_BLUR * scale as f32;
        Some((r, p))
    }
}

/// Tile index of the popover veil in [`GlassKey::Layer`].
const VEIL_KEY: usize = 10_000;
/// How far the veil reaches outside the panel, and its blur radius (points).
const VEIL_MARGIN: f32 = 56.0;
const VEIL_BLUR: f32 = 10.0;

/// macOS 26 Dock / Control Center glass adapts to what is behind it: over a dark backdrop the
/// light appearance switches to the dark filter set (verified against the captures: the Dock
/// over a dark wallpaper in Light Mode matches the dark face matrix within 2 %).
fn adapt_to_backdrop(p: &mut GlassParams, wp: &Pixmap, r: &aqua_gfx::Rect, out: (i32, i32)) {
    if p.dark || !matches!(p.variant, 2 | 3 | 7 | 8) {
        return;
    }
    let (ww, wh) = (wp.width() as f32, wp.height() as f32);
    if ww < 1.0 || out.0 <= 0 || out.1 <= 0 {
        return;
    }
    let (sx, sy) = (ww / out.0 as f32, wh / out.1 as f32);
    let (mut sum, mut n) = (0.0f32, 0.0f32);
    for j in 0..6 {
        for i in 0..24 {
            let x = ((r.x + r.w * (i as f32 + 0.5) / 24.0) * sx).clamp(0.0, ww - 1.0) as u32;
            let y = ((r.y + r.h * (j as f32 + 0.5) / 6.0) * sy).clamp(0.0, wh - 1.0) as u32;
            if let Some(c) = wp.pixel(x, y) {
                let f = |v: u8| (v as f32 / 255.0).powf(2.2);
                sum += 0.2126 * f(c.red()) + 0.7152 * f(c.green()) + 0.0722 * f(c.blue());
                n += 1.0;
            }
        }
    }
    // The captured Light Mode Dock sat on a backdrop of mean relative luminance 0.27 and still
    // used the dark face; 0.35 (≈ sRGB 63 % grey) keeps light glass for genuinely light backdrops.
    if n > 0.0 && sum / n < 0.35 {
        p.dark = true;
    }
}
