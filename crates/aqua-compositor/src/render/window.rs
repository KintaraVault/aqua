//! Window drawing: frame, glass, client surfaces, close ghosts and the genie effect.
use super::*;

impl Aqua {
    pub(super) fn push_window(
        &mut self,
        renderer: &mut GlesRenderer,
        final_out: &mut Vec<AquaElement>,
        w: &Window,
        focused: bool,
        scale: f64,
    ) {
        let t = crate::state::open_progress(w);
        let ease = 1.0 - (1.0 - t).powi(3);
        let mut alpha = ease.clamp(0.0, 1.0);
        let mut zoom = 0.86 + 0.14 * ease as f64;
        let mut launch_off: Point<f64, Logical> = (0.0, 0.0).into();
        if t < 1.0 {
            if let (Some(icon), Some(fr)) = (meta(w).borrow().launch_from, self.frame_rect(w)) {
                let fr = fr.to_f64();
                if fr.size.w > 1.0 && fr.size.h > 1.0 {
                    let e = (1.0 - (1.0 - t).powi(4)) as f64;
                    let k0 = ((icon.w as f64 / fr.size.w).min(icon.h as f64 / fr.size.h)).clamp(0.02, 1.0);
                    zoom = k0 + (1.0 - k0) * e;
                    alpha = (t * 3.0).min(1.0);
                    let (cx, cy) = (fr.loc.x + fr.size.w / 2.0, fr.loc.y + fr.size.h / 2.0);
                    launch_off = ((icon.cx() as f64 - cx) * (1.0 - e), (icon.cy() as f64 - cy) * (1.0 - e)).into();
                }
            }
        }
        let (mp, target) = crate::state::minimize_progress(w);
        alpha *= 1.0 - 0.75 * mp * mp;
        zoom *= 1.0 - 0.9 * mp as f64;
        let mc = self.mission.progress() as f64;
        let mut mc_off: Point<f64, Logical> = (0.0, 0.0).into();
        if mc > 0.0 {
            if let Some((cur, tgt)) = self.mission_targets.get(&meta(w).borrow().id) {
                let s = tgt.size.w / cur.size.w;
                zoom *= 1.0 + (s - 1.0) * mc;
                mc_off = (
                    (tgt.loc.x + tgt.size.w / 2.0 - cur.loc.x - cur.size.w / 2.0) * mc,
                    (tgt.loc.y + tgt.size.h / 2.0 - cur.loc.y - cur.size.h / 2.0) * mc,
                )
                    .into();
            } else {
                alpha *= (1.0 - mc as f32).max(0.0);
                if alpha <= 0.01 {
                    return;
                }
            }
        }
        if mp > 0.0 && mc <= 0.0 && self.cfg.minimize_effect != "scale" {
            if let Some((tx, ty)) = target {
                if self.push_genie(renderer, final_out, w, focused, mp, (tx, ty), scale) {
                    return;
                }
            }
        }
        let mut geo_off: Point<f64, Logical> = (0.0, 0.0).into();
        let mut geo_k = (1.0f64, 1.0f64);
        let geo = crate::state::geo_anim(w);
        if let (Some((r, _)), Some(cur)) = (geo, self.frame_rect(w)) {
            let cur = cur.to_f64();
            if cur.size.w > 1.0 && cur.size.h > 1.0 {
                geo_k = ((r.size.w / cur.size.w).clamp(0.05, 20.0), (r.size.h / cur.size.h).clamp(0.05, 20.0));
                geo_off = (
                    r.loc.x + r.size.w / 2.0 - cur.loc.x - cur.size.w / 2.0,
                    r.loc.y + r.size.h / 2.0 - cur.loc.y - cur.size.h / 2.0,
                )
                    .into();
            }
        }
        let mut out: Vec<WinElement> = Vec::new();
        let anim_origin = self.push_window_inner(renderer, &mut out, w, focused, scale, alpha);
        if t >= 1.0 && mp <= 0.0 && mc <= 0.0 && geo.is_none() {
            final_out.extend(out.into_iter().map(AquaElement::Win));
        } else if let Some(origin) = anim_origin {
            let offset: Point<i32, Physical> = match target {
                Some((tx, ty)) if mp > 0.0 => (
                    ((tx as f64 * scale - origin.x as f64) * mp as f64).round() as i32,
                    ((ty as f64 * scale - origin.y as f64) * mp as f64).round() as i32,
                )
                    .into(),
                _ => (0, 0).into(),
            };
            let offset = offset
                + Point::<i32, Physical>::from((
                    ((mc_off.x + geo_off.x + launch_off.x) * scale).round() as i32,
                    ((mc_off.y + geo_off.y + launch_off.y) * scale).round() as i32,
                ));
            final_out.extend(out.into_iter().map(|e| {
                AquaElement::WinMoved(RelocateRenderElement::from_element(
                    RescaleRenderElement::from_element(e, origin, Scale::from((zoom * geo_k.0, zoom * geo_k.1))),
                    offset,
                    Relocate::Relative,
                ))
            }));
        }
    }

    pub(super) fn push_window_inner(
        &mut self,
        renderer: &mut GlesRenderer,
        out: &mut Vec<WinElement>,
        w: &Window,
        focused: bool,
        scale: f64,
        alpha: f32,
    ) -> Option<Point<i32, Physical>> {
        let loc = self.space.element_location(w)?;
        if !meta(w).borrow().placed {
            return None;
        }
        let toplevel = SurfRef::of(w)?;
        let override_redirect = meta(w).borrow().override_redirect;
        if !override_redirect && self.cfg.animate_windows {
            self.refresh_composite(renderer, w);
        }
        let geo = w.geometry();
        let ssd = is_ssd(w);
        let tb = if ssd { metrics::TITLEBAR_HEIGHT as i32 } else { 0 };
        let fullscreen = !override_redirect
            && self.mission.progress() <= 0.0
            && crate::state::geo_anim(w).is_none()
            && self.covers_output(w);
        let menu_popup = meta(w).borrow().menu_popup;
        let radius = if menu_popup {
            crate::wm::popups::MENU_RADIUS
        } else if override_redirect || fullscreen {
            0.0
        } else {
            self.cfg.window_radius
        };
        let id = meta(w).borrow().id;
        let frame = Rectangle::<i32, Logical>::new((loc.x, loc.y - tb).into(), (geo.size.w, geo.size.h + tb).into());
        let frame_phys = to_phys(frame.to_f64(), scale);
        let surface_origin = loc - geo.loc;
        let surface_phys: Point<i32, Physical> = surface_origin.to_f64().to_physical(scale).to_i32_round();

        let wl = toplevel.wl_surface().clone();
        for (popup, ploc) in PopupManager::popups_for_surface(&wl) {
            let pgeo = popup.geometry();
            let p = (surface_origin + geo.loc + ploc - pgeo.loc).to_f64().to_physical(scale).to_i32_round();
            let elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                render_elements_from_surface_tree(renderer, popup.wl_surface(), p, scale, alpha, Kind::Unspecified);
            out.extend(elems.into_iter().map(WinElement::Surface));
        }

        let shaders = self.render_cache.shaders.clone()?;

        if ssd {
            let hover = meta(w).borrow().hover_lights;
            let (_, title) = title_of(w);
            let key = aqua_shell_hash(&(
                title.as_str(),
                focused,
                hover,
                self.shell.style.dark,
                self.cfg.glass_traffic_lights,
                geo.size.w,
                (scale * 100.0) as i32,
            ));
            let rc = &mut self.render_cache;
            if rc.titlebars.get(&id).map(|(k, _, _)| *k != key).unwrap_or(true) {
                let pm = aqua_shell::decor::titlebar(
                    &self.shell.fonts,
                    geo.size.w as f32,
                    scale as f32,
                    &title,
                    focused,
                    hover,
                    self.shell.style.dark,
                    self.cfg.glass_traffic_lights,
                );
                rc.titlebars.insert(id, (key, buffer_from_pixmap(&pm, false), (pm.width(), pm.height())));
            }
            let (_, buf, (tw, th)) = rc.titlebars.get(&id).unwrap();
            let tloc: Point<f64, Physical> = (frame_phys.loc.x as f64, frame_phys.loc.y as f64).into();
            if let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                tloc,
                buf,
                Some(alpha),
                crate::render::full_src(*tw, *th),
                Some((geo.size.w, tb).into()),
                Kind::Unspecified,
            ) {
                out.push(WinElement::Memory(e));
            }
        }

        let clip = if ssd { frame_phys } else { to_phys(Rectangle::new(loc, geo.size).to_f64(), scale) };
        let elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
            render_elements_from_surface_tree(renderer, &wl, surface_phys, scale, alpha, Kind::Unspecified);
        let rpx = radius * scale as f32;
        // Liquid Glass shapes of Aqua's own apps (aqua_glass_v1): lenses over the app's controls
        // go in front of the surface, sidebar / menu glass behind it.
        let app_glass = if ssd { vec![] } else { crate::wayland::glass::shapes(&wl) };
        let (over, behind): (Vec<_>, Vec<_>) = app_glass
            .iter()
            .enumerate()
            .partition(|(_, s)| aqua_config::material::Material::from_wire(s.material).over_content());
        let glassy = !ssd && (crate::wayland::blur::region(&wl).is_some() || !behind.is_empty());
        let win_rect = Rectangle::new(loc, geo.size).to_f64();
        for (i, s) in over.iter().copied() {
            self.push_app_glass(out, id, i, s, surface_origin, win_rect, radius, alpha, scale);
        }
        out.extend(
            elems
                .into_iter()
                .map(|e| WinElement::Rounded(RoundedElement::new(e, clip, rpx, &shaders, scale).see_through(glassy))),
        );
        for (i, s) in behind.iter().copied() {
            self.push_app_glass(out, id, i, s, surface_origin, win_rect, radius, alpha, scale);
        }

        if !ssd {
            if let Some(rs) = crate::wayland::blur::region(&wl) {
                let win = Rectangle::new(loc, geo.size);
                let whole = vec![Rectangle::new(geo.loc, geo.size)];
                let rs = if rs.is_empty() { whole } else { rs };
                for (i, r) in rs.iter().enumerate().take(8) {
                    let r = Rectangle::new(surface_origin + r.loc, r.size);
                    let Some(r) = r.intersection(win) else { continue };
                    let g = self.client_glass(if r == win { radius } else { 0.0 });
                    let p = glass_params(&g, scale as f32);
                    if let Some(e) =
                        self.render_cache.glass_el(GlassKey::Client(id, i as u8), to_phys(r.to_f64(), scale), p, alpha)
                    {
                        out.push(WinElement::Glass(e));
                    }
                }
            }
        }

        if ssd {
            let g = self.window_glass(radius, focused);
            let p = glass_params(&g, scale as f32);
            if let Some(e) = self.render_cache.glass_el(GlassKey::Window(id), frame_phys, p, alpha) {
                out.push(WinElement::Glass(e));
            }
        }

        if (override_redirect && !menu_popup) || fullscreen {
            return Some(Point::from((
                frame_phys.loc.x + frame_phys.size.w / 2,
                frame_phys.loc.y + frame_phys.size.h / 2,
            )));
        }
        let shadow_rect = if ssd { frame } else { Rectangle::new(loc, geo.size) };
        let key =
            aqua_shell_hash(&(shadow_rect.loc.x, shadow_rect.loc.y, shadow_rect.size.w, shadow_rect.size.h, focused));
        let rc = &mut self.render_cache;
        if rc.shadows.get(&id).map(|(k, _)| *k != key).unwrap_or(true) {
            let (sigma, dy, strength) = if menu_popup {
                (12.0, 6.0, 0.30)
            } else if focused {
                (26.0, 14.0, 0.42)
            } else {
                (16.0, 8.0, 0.24)
            };
            let el = shaders.shadow(shadow_rect, radius, sigma, dy, strength, scale);
            rc.shadows.insert(id, (key, el));
        }
        if alpha >= 1.0 {
            out.push(WinElement::Shader(rc.shadows.get(&id).unwrap().1.clone()));
        } else {
            let (sigma, dy, strength) = if focused { (26.0, 14.0, 0.42) } else { (16.0, 8.0, 0.24) };
            out.push(WinElement::Shader(shaders.shadow(
                shadow_rect,
                radius,
                sigma,
                dy,
                strength * alpha * alpha,
                scale,
            )));
        }
        Some(Point::from((frame_phys.loc.x + frame_phys.size.w / 2, frame_phys.loc.y + frame_phys.size.h / 2)))
    }

    pub(super) fn push_ghosts(&mut self, renderer: &mut GlesRenderer, out: &mut Vec<AquaElement>, scale: f64) {
        let Some(shaders) = self.render_cache.shaders.clone() else { return };
        let Some(ctx) = self.render_cache.ctx.clone() else { return };
        self.render_cache
            .ghosts
            .retain(|g| g.start.elapsed().as_secs_f32() * 1000.0 < CLOSE_ANIM_MS * crate::state::anim_slow());
        let radius = self.cfg.window_radius;
        for g in &self.render_cache.ghosts {
            let t = (g.start.elapsed().as_secs_f32() * 1000.0 / (CLOSE_ANIM_MS * crate::state::anim_slow())).min(1.0);
            let ease = 1.0 - (1.0 - t).powi(2);
            let alpha = 1.0 - ease;
            let zoom = 1.0 - 0.07 * ease as f64;
            let fp = to_phys(g.frame.to_f64(), scale);
            let origin: Point<i32, Physical> = (fp.loc.x + fp.size.w / 2, fp.loc.y + fp.size.h / 2).into();
            if let Some((tb, (tw, tph))) = &g.titlebar {
                let tloc: Point<f64, Physical> = (fp.loc.x as f64, fp.loc.y as f64).into();
                let th = metrics::TITLEBAR_HEIGHT as i32;
                if let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    tloc,
                    tb,
                    Some(alpha),
                    crate::render::full_src(*tw, *tph),
                    Some((g.frame.size.w, th).into()),
                    Kind::Unspecified,
                ) {
                    out.push(AquaElement::Scaled(RescaleRenderElement::from_element(e, origin, zoom)));
                }
            }
            let loc = g.tex_loc.to_f64().to_physical(scale);
            let te = TextureRenderElement::from_static_texture(
                g.el_id.clone(),
                ctx.clone(),
                loc,
                g.tex.clone(),
                g.buffer_scale,
                g.transform,
                Some(alpha),
                Some(g.src),
                Some(g.dst),
                None,
                Kind::Unspecified,
            );
            let re = RoundedElement::new(te, fp, radius * scale as f32, &shaders, scale);
            out.push(AquaElement::Ghost(RescaleRenderElement::from_element(re, origin, zoom)));
            out.push(AquaElement::Shader(shaders.shadow(g.frame, radius, 26.0, 14.0, 0.42 * alpha * alpha, scale)));
        }
    }

    /// Keep the last frame of a window so it can fade out after the client destroyed it.
    pub fn capture_ghost(&mut self, w: &Window) {
        let Some(ctx) = self.render_cache.ctx.clone() else { return };
        let Some(loc) = self.space.element_location(w) else { return };
        if !meta(w).borrow().placed || meta(w).borrow().menu_popup || crate::state::minimize_progress(w).0 > 0.0 {
            return;
        }
        let Some(t) = SurfRef::of(w) else { return };
        let geo = w.geometry();
        let id = meta(w).borrow().id;
        let ssd = is_ssd(w);
        let tb = if ssd { metrics::TITLEBAR_HEIGHT as i32 } else { 0 };
        let _ = (&ctx, &t);
        let Some(view) = self.window_snap(None, w, u64::MAX) else { return };
        self.render_cache.composites.remove(&id);
        let (tex, buffer_scale, transform) = (view.tex.clone(), view.buffer_scale, view.transform);
        let surface_origin = loc - geo.loc;
        self.render_cache.ghosts.push(Ghost {
            el_id: Id::new(),
            tex,
            tex_loc: surface_origin + view.offset,
            src: view.src,
            dst: view.dst,
            buffer_scale,
            transform,
            frame: Rectangle::new((loc.x, loc.y - tb).into(), (geo.size.w, geo.size.h + tb).into()),
            titlebar: if ssd { self.render_cache.titlebars.get(&id).map(|(_, b, px)| (b.clone(), *px)) } else { None },
            start: Instant::now(),
        });
        self.needs_redraw = true;
    }

    /// One `aqua_glass_v1` shape of window `id` (surface-local `s`, clipped to the window).
    #[allow(clippy::too_many_arguments)]
    fn push_app_glass(
        &mut self,
        out: &mut Vec<WinElement>,
        id: u64,
        i: usize,
        s: &crate::wayland::glass::Shape,
        surface_origin: Point<i32, Logical>,
        win: Rectangle<f64, Logical>,
        win_radius: f32,
        alpha: f32,
        scale: f64,
    ) {
        let r = Rectangle::new(surface_origin.to_f64() + s.rect.loc, s.rect.size);
        let Some(mut r) = r.intersection(win) else { return };
        let m = aqua_config::material::Material::from_wire(s.material);
        let near = |a: f64, b: f64| (a - b).abs() < 1.0;
        let full_h = near(r.loc.y, win.loc.y) && near(r.size.h, win.size.h);
        let (left, right) = (near(r.loc.x, win.loc.x), near(r.loc.x + r.size.w, win.loc.x + win.size.w));
        let mut radius = s.radius;
        if full_h && left && right {
            // A shape filling the window (a menu window) follows the window's own corners.
            radius = win_radius.max(s.radius);
        } else if full_h && (left || right) && s.radius < win_radius && !m.over_content() {
            // A full-height sidebar: round its window-side corners like the window. Over an
            // opaque body the inner corners hide under it (the glass extends past the edge);
            // over a glass body (`window_glass`) that shows in the tiny gaps anyway.
            if !self.cfg.window_glass {
                let ext = win_radius as f64;
                if right {
                    r.loc.x -= ext;
                }
                r.size.w += ext;
            }
            radius = win_radius;
        }
        let g = self.cfg.glass.material(m, self.shell.style.dark).with_radius(radius);
        let p = glass_params(&g, scale as f32);
        if let Some(e) = self.render_cache.glass_el(GlassKey::App(id, i as u8), to_phys(r, scale), p, alpha) {
            out.push(WinElement::Glass(e));
        }
    }

    /// The window exactly as it is drawn on screen — shadow, glass frame, title bar,
    /// rounded content and popups — rendered into one texture, so the genie effect
    /// deforms all of it (it used to bend the bare client buffer: server-side title bars
    /// vanished and the shadow popped off as soon as a minimise started).
    fn genie_card(&mut self, renderer: &mut GlesRenderer, w: &Window, focused: bool, scale: f64) -> Option<GenieCard> {
        use smithay::backend::renderer::{damage::OutputDamageTracker, Bind, Offscreen};
        let frame = self.frame_rect(w)?.to_f64();
        if frame.size.w < 1.0 || frame.size.h < 1.0 {
            return None;
        }
        // the focused shadow reaches 2.5σ + dy = 79 px beyond the frame
        let pad = 80.0;
        let rect = Rectangle::<f64, Logical>::new(
            (frame.loc.x - pad, frame.loc.y - pad).into(),
            (frame.size.w + 2.0 * pad, frame.size.h + 2.0 * pad).into(),
        );
        let card_phys = to_phys(rect, scale);
        let px: Size<i32, Physical> = (card_phys.size.w.max(1), card_phys.size.h.max(1)).into();
        let mut els: Vec<WinElement> = Vec::new();
        self.push_window_inner(renderer, &mut els, w, focused, scale, 1.0)?;
        let shift: Point<i32, Physical> = (-card_phys.loc.x, -card_phys.loc.y).into();
        let els: Vec<RelocateRenderElement<WinElement>> =
            els.into_iter().map(|e| RelocateRenderElement::from_element(e, shift, Relocate::Relative)).collect();
        let mut tex: GlesTexture = renderer.create_buffer(Fourcc::Abgr8888, (px.w, px.h).into()).ok()?;
        {
            let mut fb = renderer.bind(&mut tex).ok()?;
            let mut dt = OutputDamageTracker::new(px, scale, Transform::Normal);
            dt.render_output(renderer, &mut fb, 0, &els, [0.0, 0.0, 0.0, 0.0]).ok()?;
        }
        Some(GenieCard { tex, rect, frame, px })
    }

    /// Genie minimise: deform the window into the dock icon (see genie.frag).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn push_genie(
        &mut self,
        renderer: &mut GlesRenderer,
        out: &mut Vec<AquaElement>,
        w: &Window,
        focused: bool,
        mp: f32,
        target: (f32, f32),
        scale: f64,
    ) -> bool {
        let (Some(ctx), Some(shaders)) = (self.render_cache.ctx.clone(), self.render_cache.shaders.clone()) else {
            return false;
        };
        let Some(t) = SurfRef::of(w) else { return false };
        let id = meta(w).borrow().id;
        let Some((started, _)) = meta(w).borrow().minimizing else { return false };
        let fresh = self.render_cache.genie_cards.get(&id).is_some_and(|(at, _)| *at == started);
        if !fresh {
            let _ = smithay::backend::renderer::utils::import_surface_tree(renderer, t.wl_surface());
            match self.genie_card(renderer, w, focused, scale) {
                Some(card) => {
                    self.render_cache.genie_cards.insert(id, (started, card));
                }
                None => return false,
            }
        }
        let Some((_, card)) = self.render_cache.genie_cards.get(&id).cloned() else { return false };
        let win = card.rect;
        // land the *frame* (not the shadow margin) on the icon
        let isz = self.cfg.dock_icon_size as f64;
        let k = isz / card.frame.size.w.max(card.frame.size.h).max(1.0);
        let (mx, my) = (card.frame.loc.x - win.loc.x, card.frame.loc.y - win.loc.y);
        let (tw, th) = (win.size.w * k, win.size.h * k);
        let fcx = (mx + card.frame.size.w / 2.0) * k;
        let fcy = (my + card.frame.size.h / 2.0) * k;
        let tgt = Rectangle::<f64, Logical>::new(
            (target.0 as f64 - fcx, target.1 as f64 - fcy).into(),
            (tw, th).into(),
        );
        let x0 = win.loc.x.min(tgt.loc.x).floor();
        let y0 = win.loc.y.min(tgt.loc.y).floor();
        let x1 = (win.loc.x + win.size.w).max(tgt.loc.x + tgt.size.w).ceil();
        let y1 = (win.loc.y + win.size.h).max(tgt.loc.y + tgt.size.h).ceil();
        let bw = (x1 - x0) as i32;
        let bh = (y1 - y0) as i32;
        let s = scale as f32;
        let rel = |r: &Rectangle<f64, Logical>| {
            [((r.loc.x - x0) as f32) * s, ((r.loc.y - y0) as f32) * s, r.size.w as f32 * s, r.size.h as f32 * s]
        };
        // the card is already rounded and shadowed: no extra clipping in the shader
        let clip = [0.0, 0.0, win.size.w as f32 * s, win.size.h as f32 * s];
        let te = TextureRenderElement::from_static_texture(
            Id::new(),
            ctx,
            Point::<f64, Physical>::from((x0 * scale, y0 * scale)),
            card.tex.clone(),
            1,
            Transform::Normal,
            Some(1.0),
            Some(Rectangle::from_size((card.px.w as f64, card.px.h as f64).into())),
            Some((bw, bh).into()),
            None,
            Kind::Unspecified,
        );
        let el = shaders.genie(te, [bw as f32 * s, bh as f32 * s], rel(&win), rel(&tgt), clip, 0.0, mp);
        out.push(AquaElement::Genie(el));
        true
    }
}
