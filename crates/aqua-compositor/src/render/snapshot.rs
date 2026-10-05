//! Window snapshots used for thumbnails (Dock, Mission Control desktops).
use super::*;

impl Aqua {
    /// The root surface's texture (most clients).
    pub(super) fn root_snap(&self, w: &Window) -> Option<Snap> {
        let ctx = self.render_cache.ctx.clone()?;
        let t = SurfRef::of(w)?;
        smithay::backend::renderer::utils::with_renderer_surface_state(t.wl_surface(), |s| {
            let tex = s.texture::<GlesTexture>(ctx.clone()).cloned()?;
            let view = s.view()?;
            Some(Snap {
                tex,
                offset: view.offset,
                src: view.src,
                dst: view.dst,
                buffer_scale: s.buffer_scale(),
                transform: s.buffer_transform(),
            })
        })
        .flatten()
    }

    /// Render a window's whole surface tree (subsurfaces included) into a texture.
    pub(super) fn render_composite(&mut self, renderer: &mut GlesRenderer, w: &Window) -> Option<Snap> {
        use smithay::backend::renderer::{damage::OutputDamageTracker, Bind, Offscreen};
        let t = SurfRef::of(w)?;
        let wl = t.wl_surface().clone();
        let geo = w.geometry();
        if geo.size.w <= 0 || geo.size.h <= 0 {
            return None;
        }
        let bs =
            self.output.as_ref().map(|o| o.current_scale().fractional_scale()).unwrap_or(1.0).ceil().max(1.0) as i32;
        let size: Size<i32, Physical> = (geo.size.w * bs, geo.size.h * bs).into();
        let mut tex: GlesTexture = renderer.create_buffer(Fourcc::Abgr8888, (size.w, size.h).into()).ok()?;
        let loc: Point<i32, Physical> = (-geo.loc.x * bs, -geo.loc.y * bs).into();
        let elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
            render_elements_from_surface_tree(renderer, &wl, loc, bs as f64, 1.0, Kind::Unspecified);
        {
            let mut fb = renderer.bind(&mut tex).ok()?;
            let mut dt = OutputDamageTracker::new(size, bs as f64, Transform::Normal);
            dt.render_output(renderer, &mut fb, 0, &elems, [0.0, 0.0, 0.0, 0.0]).ok()?;
        }
        Some(Snap {
            tex,
            offset: geo.loc,
            src: Rectangle::new((0.0, 0.0).into(), (geo.size.w as f64, geo.size.h as f64).into()),
            dst: geo.size,
            buffer_scale: bs,
            transform: Transform::Normal,
        })
    }

    /// Current content of a window as one texture.
    pub(super) fn window_snap(
        &mut self,
        renderer: Option<&mut GlesRenderer>,
        w: &Window,
        max_age_ms: u64,
    ) -> Option<Snap> {
        let t = SurfRef::of(w)?;
        let wl = t.wl_surface().clone();
        if !has_subsurfaces(&wl) {
            return self.root_snap(w);
        }
        let id = meta(w).borrow().id;
        let fresh = self
            .render_cache
            .composites
            .get(&id)
            .map(|(at, s)| at.elapsed().as_millis() as u64 <= max_age_ms && s.dst == w.geometry().size)
            .unwrap_or(false);
        if !fresh {
            if let Some(r) = renderer {
                if let Some(s) = self.render_composite(r, w) {
                    self.render_cache.composites.insert(id, (Instant::now(), s));
                }
            }
        }
        self.render_cache.composites.get(&id).map(|(_, s)| s.clone()).or_else(|| self.root_snap(w))
    }

    /// Keep a recent composite of subsurface-based windows so a close animation has
    /// something to fade even though the surfaces are gone by then.
    pub(super) fn refresh_composite(&mut self, renderer: &mut GlesRenderer, w: &Window) {
        let Some(t) = SurfRef::of(w) else { return };
        if has_subsurfaces(t.wl_surface()) {
            let _ = self.window_snap(Some(renderer), w, 400);
        }
    }

    /// Draw a window's current buffer (cropped to its geometry) into `lr`, rounded.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn push_snapshot(
        &mut self,
        renderer: &mut GlesRenderer,
        out: &mut Vec<AquaElement>,
        w: &Window,
        lr: Rectangle<f64, Logical>,
        radius: f32,
        alpha: f32,
        shadow: Option<(f32, f32, f32)>,
        scale: f64,
    ) -> bool {
        let (Some(ctx), Some(shaders)) = (self.render_cache.ctx.clone(), self.render_cache.shaders.clone()) else {
            return false;
        };
        let Some(t) = SurfRef::of(w) else { return false };
        let _ = smithay::backend::renderer::utils::import_surface_tree(renderer, t.wl_surface());
        let Some(Snap { tex, offset, src: vsrc, buffer_scale, transform, .. }) =
            self.window_snap(Some(renderer), w, 120)
        else {
            return false;
        };
        struct V {
            offset: Point<i32, Logical>,
            src: Rectangle<f64, Logical>,
        }
        let view = V { offset, src: vsrc };
        let geo = w.geometry();
        let src = Rectangle::<f64, Logical>::new(
            (view.src.loc.x + (geo.loc.x - view.offset.x) as f64, view.src.loc.y + (geo.loc.y - view.offset.y) as f64)
                .into(),
            (geo.size.w as f64, geo.size.h as f64).into(),
        );
        let pr = to_phys(lr, scale);
        let te = TextureRenderElement::from_static_texture(
            Id::new(),
            ctx.clone(),
            Point::<f64, Physical>::from((pr.loc.x as f64, pr.loc.y as f64)),
            tex,
            buffer_scale,
            transform,
            Some(alpha),
            Some(src),
            Some((lr.size.w.round().max(1.0) as i32, lr.size.h.round().max(1.0) as i32).into()),
            None,
            Kind::Unspecified,
        );
        let re = RoundedElement::new(te, pr, radius * scale as f32, &shaders, scale);
        out.push(AquaElement::Ghost(RescaleRenderElement::from_element(re, pr.loc, 1.0)));
        if let Some((blur, dy, op)) = shadow {
            let li = Rectangle::<i32, Logical>::new(
                (lr.loc.x.round() as i32, lr.loc.y.round() as i32).into(),
                (lr.size.w.round() as i32, lr.size.h.round() as i32).into(),
            );
            out.push(AquaElement::Shader(shaders.shadow(li, radius, blur, dy, op * alpha, scale)));
        }
        true
    }

    /// Live thumbnails of minimised windows in their Dock slots.
    pub(super) fn push_dock_thumbnails(&mut self, renderer: &mut GlesRenderer, out: &mut Vec<AquaElement>, scale: f64) {
        for (id, slot) in aqua_shell::dock::minimized_slots(&self.shell) {
            let Some(w) = self.minimized.iter().find(|w| meta(w).borrow().id == id).cloned() else { continue };
            let geo = w.geometry();
            let box_w = slot.w as f64 * 0.94;
            let box_h = slot.h as f64 * 0.94;
            let k = (box_w / geo.size.w as f64).min(box_h / geo.size.h as f64);
            let (tw, th) = ((geo.size.w as f64 * k).max(1.0), (geo.size.h as f64 * k).max(1.0));
            let tx = slot.cx() as f64 - tw / 2.0;
            let ty = slot.bottom() as f64 - th - (slot.h as f64 - box_h) / 2.0;
            let lr = Rectangle::<f64, Logical>::new((tx, ty).into(), (tw, th).into());
            let radius = (self.cfg.window_radius * k as f32).max(3.0);
            self.push_snapshot(renderer, out, &w, lr, radius, 1.0, Some((5.0, 2.0, 0.35)), scale);
        }
    }

    /// Spaces bar thumbnails: each desktop's wallpaper with its windows in miniature.
    pub(super) fn push_desk_thumbnails(&mut self, renderer: &mut GlesRenderer, out: &mut Vec<AquaElement>, scale: f64) {
        let e = self.shell.mission.expand;
        let alpha = self.shell.mission.progress * e;
        if alpha <= 0.01 {
            return;
        }
        let Some(shaders) = self.render_cache.shaders.clone() else { return };
        let (ow, _) = self.output_size();
        let desks = aqua_shell::mission::desk_rects(&self.shell);
        let dark = self.dark_progress() > 0.5;
        let wins: Vec<Window> = self
            .space
            .elements()
            .rev()
            .filter(|w| meta(w).borrow().placed && meta(w).borrow().minimizing.is_none())
            .cloned()
            .collect();
        let tb = |w: &Window| Aqua::titlebar_h(w) as f64;
        for (i, r) in desks.iter().enumerate() {
            let k = r.w as f64 / ow as f64;
            for w in wins.iter().filter(|w| meta(w).borrow().desk == i) {
                let Some(home) = self.home_loc(w) else { continue };
                let g = w.geometry();
                let lr = Rectangle::<f64, Logical>::new(
                    (r.x as f64 + home.x as f64 * k, r.y as f64 + (home.y as f64 - tb(w)) * k).into(),
                    ((g.size.w as f64 * k).max(1.0), ((g.size.h as f64 + tb(w)) * k).max(1.0)).into(),
                );
                self.push_snapshot(
                    renderer,
                    out,
                    w,
                    lr,
                    (self.cfg.window_radius * k as f32).max(1.5),
                    alpha,
                    Some((3.0, 1.0, 0.3)),
                    scale,
                );
            }
            let (pw, ph) = ((r.w as f64 * scale).round() as u32, (r.h as f64 * scale).round() as u32);
            let key = (self.wallpaper_serial, dark, pw, ph);
            if self.render_cache.wallpaper_thumb.as_ref().map(|(k, _)| *k != key).unwrap_or(true) {
                let src = if dark { aqua_wallpaper::night(&self.wallpaper) } else { (*self.wallpaper).clone() };
                let small = aqua_gfx::resize(&src, pw, ph);
                self.render_cache.wallpaper_thumb = Some((key, buffer_from_pixmap(&small, true)));
            }
            let buf = self.render_cache.wallpaper_thumb.as_ref().unwrap().1.clone();
            let lr = Rectangle::<f64, Logical>::new((r.x as f64, r.y as f64).into(), (r.w as f64, r.h as f64).into());
            let pr = to_phys(lr, scale);
            let size: Size<i32, Logical> = ((r.w).round() as i32, (r.h).round() as i32).into();
            if let Ok(m) = MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                (pr.loc.x as f64, pr.loc.y as f64),
                &buf,
                Some(alpha),
                crate::render::full_src(pw, ph),
                Some(size),
                Kind::Unspecified,
            ) {
                out.push(AquaElement::RoundedMem(RoundedElement::new(m, pr, 8.0 * scale as f32, &shaders, scale)));
            }
            let li = Rectangle::<i32, Logical>::new((r.x as i32, r.y as i32).into(), (r.w as i32, r.h as i32).into());
            out.push(AquaElement::Shader(shaders.shadow(li, 8.0, 10.0, 3.0, 0.35 * alpha, scale)));
        }
    }
}
