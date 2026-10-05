//! Multi-monitor layout: outputs are arranged left-to-right (or at the positions
//! from `[[outputs]]` in aqua.toml) with the primary display — the one with the menu
//! bar and Dock — at the origin. Each output has its own (fractional) scale;
//! clients get `wp_fractional_scale` / `preferred_buffer_scale` hints for the
//! display they are on. Hotplug re-arranges and rescues windows from vanished displays.
use crate::state::{meta, Aqua};
use smithay::{
    output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
    utils::{Logical, Point, Rectangle, Transform},
    wayland::{compositor::with_surface_tree_downward, fractional_scale::with_fractional_scale, seat::WaylandFocus},
};

/// How long a withdrawn wl_output global stays bindable (inert) after an unplug.
pub const OUTPUT_GLOBAL_GRACE: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Default)]
pub struct Outputs {
    /// All active outputs, primary first.
    pub list: Vec<Output>,
    /// Off-screen outputs created for testing/headless use (`output add`).
    pub virtuals: Vec<Output>,
}

pub fn parse_transform(s: &str) -> Transform {
    match s {
        "90" => Transform::_90,
        "180" => Transform::_180,
        "270" => Transform::_270,
        "flipped" => Transform::Flipped,
        "flipped-90" => Transform::Flipped90,
        "flipped-180" => Transform::Flipped180,
        "flipped-270" => Transform::Flipped270,
        _ => Transform::Normal,
    }
}

impl Aqua {
    pub fn output_cfg(&self, name: &str) -> Option<aqua_config::OutputCfg> {
        self.cfg
            .outputs
            .iter()
            .find(|o| o.name == name)
            .or_else(|| self.cfg.outputs.iter().find(|o| o.name == "*"))
            .cloned()
    }

    /// Scale for an output: config override, else `auto`.
    pub fn scale_for(&self, name: &str, auto: f64) -> f64 {
        match self.output_cfg(name) {
            Some(c) if c.scale > 0.0 => (c.scale * 4.0).round() / 4.0,
            _ => auto,
        }
        .clamp(0.5, 4.0)
    }

    pub fn add_output(&mut self, output: Output) {
        if !self.outputs.list.contains(&output) {
            self.outputs.list.push(output);
        }
        self.arrange_outputs();
    }

    /// Advertise `output` to clients; the global is withdrawn again in `remove_output`
    /// (otherwise every unplug left a dead wl_output behind and replugging duplicated it).
    pub fn publish_output(&mut self, output: &Output) {
        publish(&self.display_handle, output);
    }

    pub fn remove_output(&mut self, output: &Output) {
        if let Some(OutputGlobal(id)) = output.user_data().get::<OutputGlobal>() {
            let id = id.clone();
            // disable first so no new client binds it, destroy once in-flight binds are done.
            // A client that was busy when the output appeared still binds it after reading
            // the stale announcement; once the global is gone that bind is a fatal protocol
            // error ("Invalid binding of wl_output"), so keep the inert global around long
            // enough to cover a frozen app, not just a round-trip.
            self.display_handle.disable_global::<Aqua>(id.clone());
            let _ = self.loop_handle.insert_source(
                smithay::reexports::calloop::timer::Timer::from_duration(OUTPUT_GLOBAL_GRACE),
                move |_, _, st| {
                    st.display_handle.remove_global::<Aqua>(id.clone());
                    smithay::reexports::calloop::timer::TimeoutAction::Drop
                },
            );
        }
        self.outputs.list.retain(|o| o != output);
        self.outputs.virtuals.retain(|o| o != output);
        self.space.unmap_output(output);
        self.lock.ext_surfaces.retain(|(o, _)| o != output);
        self.arrange_outputs();
    }

    fn primary_index(&self) -> usize {
        self.outputs
            .list
            .iter()
            .position(|o| self.output_cfg(&o.name()).map(|c| c.primary).unwrap_or(false))
            .unwrap_or(0)
    }

    /// Recompute positions; keeps the primary output at (0, 0).
    pub fn arrange_outputs(&mut self) {
        let pi = self.primary_index();
        if pi != 0 && pi < self.outputs.list.len() {
            let p = self.outputs.list.remove(pi);
            self.outputs.list.insert(0, p);
        }
        let mut x = 0;
        let list = self.outputs.list.clone();
        for o in &list {
            let cfg = self.output_cfg(&o.name());
            let size = o.current_mode().map(|m| m.size).unwrap_or((1920, 1080).into());
            let scale = o.current_scale().fractional_scale();
            let transform = cfg.as_ref().map(|c| parse_transform(&c.transform)).unwrap_or(o.current_transform());
            let lsize = transform.transform_size(size).to_f64().to_logical(scale).to_i32_round::<i32>();
            let loc: Point<i32, Logical> = match cfg.as_ref().and_then(|c| c.position) {
                Some((px, py)) if Some(o) != list.first() => (px, py).into(),
                _ if Some(o) == list.first() => (0, 0).into(),
                _ => (x, 0).into(),
            };
            o.change_current_state(None, None, None, Some(loc));
            self.space.map_output(o, loc);
            x = x.max(loc.x + lsize.w);
        }
        let new_primary = list.first().cloned();
        let changed = new_primary != self.output;
        let old_size = self.output_size();
        self.output = new_primary;
        if let Some(p) = &self.output {
            self.scale = p.current_scale().fractional_scale();
        }
        // `old_size` already reflects the new mode/scale (the caller changed the output
        // before calling us), so also compare with what the shell was last laid out for:
        // otherwise a scale change left the menu bar, dock and wallpaper at the old size.
        let (w, h) = self.output_size();
        let stale = (self.shell.w - w as f32).abs() > 0.5
            || (self.shell.h - h as f32).abs() > 0.5
            || (self.shell.scale - self.scale as f32).abs() > 1e-4;
        if changed || old_size != (w, h) || stale {
            self.on_output_resized();
        }
        // `Space::map_element` also raises; moving windows around must not reshuffle the
        // stack (a rescued window ended up above a fullscreen one after a scale change).
        let stack: Vec<_> = self.space.elements().cloned().collect();
        self.rescue_windows();
        self.refit_to_outputs();
        if !self.space.elements().eq(stack.iter()) {
            for w in &stack {
                self.space.raise_element(w, false);
            }
        }
        self.update_scale_hints();
        self.needs_redraw = true;
    }

    /// Union of all outputs (logical).
    pub fn layout_bounds(&self) -> Rectangle<i32, Logical> {
        let mut r: Option<Rectangle<i32, Logical>> = None;
        for o in self.outputs.list.iter().chain(self.outputs.virtuals.iter()) {
            if let Some(g) = self.space.output_geometry(o) {
                r = Some(r.map(|r| r.merge(g)).unwrap_or(g));
            }
        }
        r.unwrap_or_else(|| Rectangle::from_size(self.output_size().into()))
    }

    pub fn layout_width(&self) -> i32 {
        let b = self.layout_bounds();
        b.loc.x + b.size.w
    }

    pub fn output_at(&self, pos: Point<f64, Logical>) -> Option<Output> {
        self.outputs
            .list
            .iter()
            .chain(self.outputs.virtuals.iter())
            .find(|o| self.space.output_geometry(o).map(|g| g.to_f64().contains(pos)).unwrap_or(false))
            .cloned()
    }

    /// Keep the pointer on some output (multi-monitor aware clamp).
    pub fn clamp_to_outputs(&self, pos: Point<f64, Logical>) -> Point<f64, Logical> {
        if self.output_at(pos).is_some() {
            return pos;
        }
        let mut best = pos;
        let mut best_d = f64::MAX;
        for o in self.outputs.list.iter().chain(self.outputs.virtuals.iter()) {
            if let Some(g) = self.space.output_geometry(o) {
                let g = g.to_f64();
                let p: Point<f64, Logical> =
                    (pos.x.clamp(g.loc.x, g.loc.x + g.size.w - 1.0), pos.y.clamp(g.loc.y, g.loc.y + g.size.h - 1.0))
                        .into();
                let d = (p.x - pos.x).powi(2) + (p.y - pos.y).powi(2);
                if d < best_d {
                    best_d = d;
                    best = p;
                }
            }
        }
        best
    }

    /// Windows that ended up entirely outside every output (display unplugged) go to the
    /// primary display, keeping their desk where it exists there.
    fn rescue_windows(&mut self) {
        self.sync_display_spaces();
        let displays: Vec<aqua_wm::Rect> = self
            .outputs
            .list
            .iter()
            .chain(self.outputs.virtuals.iter())
            .filter_map(|o| self.space.output_geometry(o))
            .map(crate::wm::to_rect)
            .collect();
        let Some(primary) = self.output.as_ref().and_then(|o| self.space.output_geometry(o)).map(crate::wm::to_rect)
        else {
            return;
        };
        let wins: Vec<_> = self.space.elements().cloned().collect();
        for w in wins {
            if meta(&w).borrow().override_redirect || !meta(&w).borrow().placed {
                continue;
            }
            let (Some(loc), Some(home)) = (self.space.element_location(&w), self.home_loc(&w)) else { continue };
            let size = w.geometry().size;
            let home_rect = aqua_wm::Rect::new(home.x, home.y, size.w, size.h);
            let mb = self.cfg.menubar_height as i32;
            let Some((nx, ny)) = aqua_wm::place::rescue(home_rect, &displays, primary, mb, Aqua::titlebar_h(&w)) else {
                continue;
            };
            let shift = loc.x - home.x;
            self.space.map_element(w.clone(), (nx + shift, ny), false);
        }
    }

    /// Send preferred (fractional) scale to every surface for the output it is on.
    pub fn update_scale_hints(&mut self) {
        let wins: Vec<_> = self.space.elements().cloned().collect();
        for w in wins {
            let outs = self.space.outputs_for_element(&w);
            let Some(o) = outs.first().or(self.output.as_ref()).cloned() else { continue };
            let scale = o.current_scale();
            let transform = o.current_transform();
            if let Some(s) = w.wl_surface() {
                with_surface_tree_downward(
                    &s,
                    (),
                    |_, _, _| smithay::wayland::compositor::TraversalAction::DoChildren(()),
                    |surface, states, _| {
                        with_fractional_scale(states, |fs| fs.set_preferred_scale(scale.fractional_scale()));
                        smithay::wayland::compositor::send_surface_state(
                            surface,
                            states,
                            scale.integer_scale(),
                            transform,
                        );
                    },
                    |_, _, _| true,
                );
            }
        }
        for o in self.outputs.list.clone() {
            let map = smithay::desktop::layer_map_for_output(&o);
            let scale = o.current_scale();
            for l in map.layers() {
                with_surface_tree_downward(
                    l.wl_surface(),
                    (),
                    |_, _, _| smithay::wayland::compositor::TraversalAction::DoChildren(()),
                    |surface, states, _| {
                        with_fractional_scale(states, |fs| fs.set_preferred_scale(scale.fractional_scale()));
                        smithay::wayland::compositor::send_surface_state(
                            surface,
                            states,
                            scale.integer_scale(),
                            o.current_transform(),
                        );
                    },
                    |_, _, _| true,
                );
            }
        }
    }

    /// Off-screen output (testing multi-monitor without hardware, headless VNC …).
    pub fn add_virtual_output(&mut self, name: &str, w: i32, h: i32, scale: f64) -> Option<Output> {
        let (w, h) = (w.clamp(64, 16384), h.clamp(64, 16384));
        let scale = if scale.is_finite() { scale } else { 1.0 };
        if let Some(o) = self.outputs.list.iter().find(|o| o.name() == name).cloned() {
            // the name identifies config, Spaces and screencasts: never create a twin
            if !Self::is_virtual(&o) {
                tracing::warn!("virtual output {name}: a real display has that name");
                return None;
            }
            let mode = Mode { size: (w, h).into(), refresh: 60_000 };
            let scale = self.scale_for(name, scale);
            o.change_current_state(Some(mode), None, Some(Scale::Fractional(scale)), None);
            o.set_preferred(mode);
            self.arrange_outputs();
            return Some(o);
        }
        let output = Output::new(
            name.to_string(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "Aqua".into(),
                model: "Virtual".into(),
                serial_number: name.into(),
            },
        );
        self.publish_output(&output);
        let mode = Mode { size: (w, h).into(), refresh: 60_000 };
        let scale = self.scale_for(name, scale);
        output.change_current_state(Some(mode), Some(Transform::Normal), Some(Scale::Fractional(scale)), None);
        output.set_preferred(mode);
        output.user_data().insert_if_missing(|| VirtualMarker);
        self.outputs.virtuals.push(output.clone());
        self.outputs.list.push(output.clone());
        self.arrange_outputs();
        tracing::info!("virtual output {name} {w}x{h}@{scale} added");
        Some(output)
    }

    pub fn is_virtual(o: &Output) -> bool {
        o.user_data().get::<VirtualMarker>().is_some()
    }

    /// Re-apply output scales from the config (hot reload / Settings app).
    pub fn apply_output_config(&mut self) {
        self.apply_drm_modes();
        for o in self.outputs.list.clone() {
            let cur = o.current_scale().fractional_scale();
            let want = self.scale_for(&o.name(), cur);
            let transform = if o.name() == "winit" {
                None
            } else {
                self.output_cfg(&o.name()).map(|c| parse_transform(&c.transform))
            };
            if (want - cur).abs() > 0.001 || transform.map(|t| t != o.current_transform()).unwrap_or(false) {
                tracing::info!("output {}: scale {cur} -> {want}", o.name());
                o.change_current_state(None, transform, Some(Scale::Fractional(want)), None);
            }
        }
        self.arrange_outputs();
    }
}

struct VirtualMarker;
pub fn publish(dh: &smithay::reexports::wayland_server::DisplayHandle, output: &Output) {
    if output.user_data().get::<OutputGlobal>().is_none() {
        let id = output.create_global::<Aqua>(dh);
        output.user_data().insert_if_missing(|| OutputGlobal(id));
    }
}

struct OutputGlobal(smithay::reexports::wayland_server::backend::GlobalId);
