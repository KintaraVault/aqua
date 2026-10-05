//! Scene assembly: wallpaper → desktop widgets → windows (shadow, glass frame,
//! rounded client surface, titlebar, popups) → shell layers (menu bar, dock, …).
use std::collections::HashMap;
use std::time::Instant;

use crate::state::{is_ssd, meta, title_of, Aqua};
use aqua_config::{material::Material, metrics, GlassStyle, Rgba};
use aqua_gfx::Pixmap;
use aqua_render::{aqua_style::GlassStyleLike, GlassElement, GlassParams, RoundedElement, Shaders};
use aqua_shell::LayerId;
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement},
                texture::TextureRenderElement,
                utils::{Relocate, RelocateRenderElement, RescaleRenderElement},
                Id, Kind,
            },
            gles::{element::PixelShaderElement, GlesRenderer, GlesTexture},
            utils::CommitCounter,
            ContextId, ExportMem, Renderer,
        },
    },
    desktop::{PopupManager, Window},
    input::{keyboard::Keycode, pointer::CursorImageStatus},
    utils::{Logical, Physical, Point, Rectangle, Scale, Size, Transform},
};

mod capture;
mod layers;
mod pointer;
mod snapshot;
mod window;

pub use capture::{render_output_pixels, render_output_with, screenshot, OffscreenTarget};

smithay::backend::renderer::element::render_elements! {
    pub AquaElement<=GlesRenderer>;
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
    Scaled=RescaleRenderElement<MemoryRenderBufferRenderElement<GlesRenderer>>,
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Rounded=RoundedElement,
    Glass=GlassElement,
    Shader=PixelShaderElement,
    Win=WinElement,
    WinMoved=RelocateRenderElement<RescaleRenderElement<WinElement>>,
    Ghost=RescaleRenderElement<RoundedElement<TextureRenderElement<GlesTexture>>>,
    RoundedMem=RoundedElement<MemoryRenderBufferRenderElement<GlesRenderer>>,
    Genie=smithay::backend::renderer::gles::element::TextureShaderElement,
    Solid=smithay::backend::renderer::element::solid::SolidColorRenderElement,
}

smithay::backend::renderer::element::render_elements! {
    /// What an output renders: the primary scene, or windows relocated onto a secondary display.
    pub OutElement<=GlesRenderer>;
    Main=AquaElement,
    Moved=RelocateRenderElement<AquaElement>,
}

/// Last frame of a closed window, faded out by the close animation.
pub struct Ghost {
    pub el_id: Id,
    pub tex: GlesTexture,
    pub tex_loc: Point<i32, Logical>,
    pub src: Rectangle<f64, Logical>,
    pub dst: Size<i32, Logical>,
    pub buffer_scale: i32,
    pub transform: Transform,
    pub frame: Rectangle<i32, Logical>,
    pub titlebar: Option<(MemoryRenderBuffer, (u32, u32))>,
    pub start: Instant,
}

pub const CLOSE_ANIM_MS: f32 = 190.0;
pub const MINIMIZE_ANIM_MS: f32 = 380.0;

smithay::backend::renderer::element::render_elements! {
    pub WinElement<=GlesRenderer>;
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Rounded=RoundedElement,
    Glass=GlassElement,
    Shader=PixelShaderElement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum GlassKey {
    Layer(LayerId, usize),
    Window(u64),
    Client(u64, u8),
    /// `aqua_glass_v1` shape of a window.
    App(u64, u8),
    Mission,
    TilePreview,
}

struct GlassSlot {
    id: Id,
    commit: CommitCounter,
    gate: std::sync::Arc<aqua_render::BlurGate>,
    last: Option<(Rectangle<i32, Physical>, GlassParams, f32)>,
}

/// The client-provided drag icon of a drag-and-drop in progress.
pub struct DndIcon {
    pub surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    /// Offset from the pointer (accumulated `wl_surface.offset`).
    pub offset: Point<i32, Logical>,
    /// Where the drag started (a refused drop flies back there).
    pub origin: Point<f64, Logical>,
    /// Refused drop: (drop location, start of the flight back).
    pub snap: Option<(Point<f64, Logical>, Instant)>,
}

/// Flight time of a refused drop back to its origin.
pub const DND_SNAP_SECS: f32 = 0.32;

pub struct RenderCache {
    pub hot_corner: Option<usize>,
    dim: (Id, CommitCounter, f32),
    pub cursor_status: CursorImageStatus,
    pub suppressed: Vec<Keycode>,
    pub last_title_click: Option<Instant>,
    pub shaders: Option<Shaders>,
    wallpaper: Option<(u64, MemoryRenderBuffer)>,
    wallpaper_night: Option<(u64, MemoryRenderBuffer)>,
    /// Small wallpaper copy for the Spaces bar: (serial, dark, w, h).
    wallpaper_thumb: Option<((u64, bool, u32, u32), MemoryRenderBuffer)>,
    /// Uploaded shell layers: (content serial, buffer, last drawn).
    layers: HashMap<LayerId, (u64, MemoryRenderBuffer, Instant)>,
    /// Bytes released by cache pruning since the last `malloc_trim`.
    pub freed: usize,
    titlebars: HashMap<u64, (u64, MemoryRenderBuffer, (u32, u32))>,
    shadows: HashMap<u64, (u64, PixelShaderElement)>,
    glass: HashMap<GlassKey, GlassSlot>,
    pub cursor: HashMap<(crate::input::cursors::Shape, u64), MemoryRenderBuffer>,
    /// Compositor-chosen pointer shape (window borders, shell UI); overrides the client's.
    pub cursor_override: Option<smithay::input::pointer::CursorIcon>,
    /// Shape to show while a compositor grab (move/resize) is active.
    pub grab_cursor: Option<smithay::input::pointer::CursorIcon>,
    /// Icon of a running Wayland drag-and-drop (drawn under the pointer).
    pub dnd_icon: Option<DndIcon>,
    /// Image of a compositor-side file drag (Finder → other apps), 64×64 logical.
    pub file_drag: Option<FileDragImage>,
    /// Pointer buttons physically held right now (to detect grabs that outlived them).
    pub held_buttons: Vec<u32>,
    /// Serial of the active xdg_popup pointer grab (menus legitimately keep it between clicks).
    pub popup_grab: Option<smithay::utils::Serial>,
    pub frames: u64,
    pub ghosts: Vec<Ghost>,
    pub ctx: Option<ContextId<GlesTexture>>,
    /// Whole-surface-tree renders of windows whose content lives in subsurfaces
    /// (Firefox, Zen, other Gecko/GTK browsers): used for thumbnails, genie, close fade.
    pub composites: HashMap<u64, (Instant, Snap)>,
    /// The whole window as drawn on screen (shadow, frame, title bar, content), frozen
    /// when a minimise/restore starts, for the genie effect: (animation start, card).
    pub genie_cards: HashMap<u64, (Instant, GenieCard)>,
    /// Offscreen capture targets by output name (dropped a few seconds after the last use).
    offscreen: HashMap<String, OffscreenTarget>,
    /// Where a dragged window will tile when released (drawn behind it).
    pub tile_preview: Option<crate::wm::tile::Preview>,
    /// Green title-bar button hovered since (window id, time): opens the tiling menu.
    pub zoom_hover: Option<(u64, Instant)>,
}

pub struct FileDragImage {
    pub buf: MemoryRenderBuffer,
    /// Pixel size of `buf`.
    pub px: (u32, u32),
    pub alive: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// A window's content as one texture, in `RendererSurfaceState::view()` terms
/// (offset from the surface origin, source rect, destination size).
#[derive(Clone)]
pub struct Snap {
    pub tex: GlesTexture,
    pub offset: Point<i32, Logical>,
    pub src: Rectangle<f64, Logical>,
    pub dst: Size<i32, Logical>,
    pub buffer_scale: i32,
    pub transform: Transform,
}

/// A window rendered offscreen with its decorations and shadow (see `genie_card`).
#[derive(Clone)]
pub struct GenieCard {
    pub tex: GlesTexture,
    /// Logical rect the texture covers (the frame plus the shadow margin).
    pub rect: Rectangle<f64, Logical>,
    /// The window frame (title bar + surface) inside `rect`.
    pub frame: Rectangle<f64, Logical>,
    /// Texture size in pixels.
    pub px: Size<i32, Physical>,
}

fn has_subsurfaces(s: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface) -> bool {
    std::env::var_os("AQUA_FORCE_COMPOSITE").is_some() || !smithay::wayland::compositor::get_children(s).is_empty()
}

impl Default for RenderCache {
    fn default() -> Self {
        Self {
            hot_corner: None,
            dim: (Id::new(), CommitCounter::default(), 0.0),
            cursor_status: CursorImageStatus::default_named(),
            suppressed: vec![],
            last_title_click: None,
            shaders: None,
            wallpaper: None,
            wallpaper_night: None,
            wallpaper_thumb: None,
            layers: HashMap::new(),
            freed: 0,
            titlebars: HashMap::new(),
            shadows: HashMap::new(),
            glass: HashMap::new(),
            cursor: HashMap::new(),
            cursor_override: None,
            grab_cursor: None,
            dnd_icon: None,
            file_drag: None,
            held_buttons: vec![],
            popup_grab: None,
            frames: 0,
            ghosts: vec![],
            ctx: None,
            composites: HashMap::new(),
            genie_cards: HashMap::new(),
            offscreen: HashMap::new(),
            tile_preview: None,
            zoom_hover: None,
        }
    }
}

impl Aqua {
    /// Stage Manager strip: the apps put aside, as small window stacks at the left edge
    /// (behind the windows on stage).
    fn push_stage_strip(&mut self, renderer: &mut GlesRenderer, out: &mut Vec<AquaElement>, scale: f64) {
        if !self.stage.on {
            return;
        }
        let thumbs = self.stage_thumbs();
        // `out` is front to back: the front window of each stage goes first
        for (w, r, alpha) in thumbs.into_iter().rev() {
            let k = (r.size.w / w.geometry().size.w.max(1) as f64) as f32;
            let radius = (self.cfg.window_radius * k).max(4.0);
            self.push_snapshot(renderer, out, &w, r, radius, alpha, Some((8.0, 3.0, 0.35)), scale);
            // server-side decorated (mostly X11) clients draw no background of their own:
            // give the thumbnail the same frame material the window has on stage
            if is_ssd(&w) {
                let id = meta(&w).borrow().id;
                let p = glass_params(&self.window_glass(radius, false), scale as f32);
                if let Some(e) = self.render_cache.glass_el(GlassKey::Window(id), to_phys(r, scale), p, alpha) {
                    out.push(AquaElement::Glass(e));
                }
            }
        }
    }

    /// The translucent glass area a dragged window will tile into.
    fn push_tile_preview(&mut self, out: &mut Vec<AquaElement>, scale: f64) {
        let Some(p) = self.render_cache.tile_preview.clone() else { return };
        let (r, alpha) = p.current();
        if p.animating() {
            self.needs_redraw = true;
        }
        let g = self.cfg.glass.material(Material::TilePreview, self.shell.style.dark);
        let params = glass_params(&g, scale as f32);
        if let Some(e) = self.render_cache.glass_el(GlassKey::TilePreview, to_phys(r, scale), params, alpha) {
            out.push(AquaElement::Glass(e));
        }
    }
}

/// Source rect covering a whole `w`×`h` pixel buffer created by [`buffer_from_pixmap`].
///
/// smithay defaults the source to the *destination* size, i.e. it assumes one buffer pixel
/// per logical pixel. Our pixmaps are drawn at physical resolution, so at scale 2 only the
/// top-left quarter was shown (blown up), and below 1 the edges were smeared.
pub fn full_src(w: impl Into<f64>, h: impl Into<f64>) -> Option<Rectangle<f64, Logical>> {
    Some(Rectangle::from_size((w.into(), h.into()).into()))
}

pub fn pixmap_src(pm: &Pixmap) -> Option<Rectangle<f64, Logical>> {
    full_src(pm.width(), pm.height())
}

pub fn buffer_from_pixmap(pm: &Pixmap, opaque: bool) -> MemoryRenderBuffer {
    let size = (pm.width() as i32, pm.height() as i32);
    let opaque = opaque.then(|| vec![Rectangle::from_size(size.into())]);
    MemoryRenderBuffer::from_slice(pm.data(), Fourcc::Abgr8888, size, 1, Transform::Normal, opaque)
}

pub fn glass_params(g: &GlassStyle, scale: f32) -> GlassParams {
    let Rgba(r, gg, b, a) = g.tint;
    GlassParams::from_style(
        &GlassStyleLike {
            radius: g.radius,
            roundness: g.roundness,
            blur: g.blur,
            tint: [r, gg, b, a],
            saturation: g.saturation,
            max_luma: g.max_luma,
            thickness: g.thickness,
            refraction: g.refraction,
            ior: g.ior,
            dispersion: g.dispersion,
            blur_edge: g.blur_edge,
            rim: g.rim,
            fresnel: g.fresnel,
            fresnel_range: g.fresnel_range,
            fresnel_hardness: g.fresnel_hardness,
            glare: g.glare,
            glare_range: g.glare_range,
            glare_hardness: g.glare_hardness,
            glare_convergence: g.glare_convergence,
            glare_opposite: g.glare_opposite,
            glare_angle: g.glare_angle,
        },
        scale,
    )
}

fn to_phys(r: Rectangle<f64, Logical>, s: f64) -> Rectangle<i32, Physical> {
    let x0 = (r.loc.x * s).round() as i32;
    let y0 = (r.loc.y * s).round() as i32;
    let x1 = ((r.loc.x + r.size.w) * s).round() as i32;
    let y1 = ((r.loc.y + r.size.h) * s).round() as i32;
    Rectangle::new((x0, y0).into(), (x1 - x0, y1 - y0).into())
}

impl Aqua {
    /// Material behind client-requested blur regions (third-party apps): no tint of its own
    /// (the app's pixels are the tint), strong blur + saturation, no lens.
    pub(crate) fn client_glass(&self, radius: f32) -> GlassStyle {
        self.cfg.glass.material(Material::Backdrop, self.shell.style.dark).with_radius(radius)
    }

    /// Material used for window chrome (titlebar + frame behind server-side decorated clients).
    pub(crate) fn window_glass(&self, radius: f32, focused: bool) -> GlassStyle {
        self.cfg.glass.material(Material::WindowFrame { focused }, self.shell.style.dark).with_radius(radius)
    }
}

impl RenderCache {
    fn glass_el(
        &mut self,
        key: GlassKey,
        geo: Rectangle<i32, Physical>,
        params: GlassParams,
        alpha: f32,
    ) -> Option<GlassElement> {
        let shaders = self.shaders.as_ref()?.clone();
        let slot = self.glass.entry(key).or_insert_with(|| GlassSlot {
            id: Id::new(),
            commit: CommitCounter::default(),
            gate: Default::default(),
            last: None,
        });
        if slot.last != Some((geo, params, alpha)) {
            slot.commit.increment();
            slot.last = Some((geo, params, alpha));
        } else if slot.gate.recapture_due(Instant::now()) {
            // The backdrop changed while re-blurring was rate limited: blur it again now.
            slot.commit.increment();
        }
        Some(GlassElement::new(slot.id.clone(), geo, params, alpha, &shaders, slot.commit).with_gate(slot.gate.clone()))
    }

    /// Some glass shows a blur older than its backdrop (another frame is needed later).
    pub fn blur_stale(&self) -> bool {
        self.glass.values().any(|s| s.gate.is_stale())
    }
}

impl Aqua {
    /// Build the render element list, front-most first.
    pub fn build_elements(&mut self, renderer: &mut GlesRenderer) -> Vec<AquaElement> {
        self.render_cache.ctx = Some(renderer.context_id());
        aqua_render::free_dropped_blurs(renderer);
        aqua_render::set_blur_unthrottled(self.windows_animating());
        if self.render_cache.shaders.is_none() {
            match Shaders::new(renderer) {
                Ok(s) => self.render_cache.shaders = Some(s),
                Err(e) => tracing::error!("shader compile failed: {e:?}"),
            }
        }
        let scale = self.scale;
        let sf = scale as f32;
        self.sync_shell_windows();
        self.sync_mission_shell();
        self.mission_targets = if self.mission_visible() {
            self.mission_layout().into_iter().map(|(w, c, t)| (meta(&w).borrow().id, (c, t))).collect()
        } else {
            Default::default()
        };
        use crate::system::lock::Mode as LockMode;
        use smithay::wayland::shell::wlr_layer::Layer as WlrLayer;
        self.shell.locked = self.lock.mode == LockMode::Internal;
        let lp = self.lock.lock_progress();
        let primary = self.output.clone();
        let mut out: Vec<AquaElement> = Vec::new();
        if self.draw_cursor {
            self.push_cursor(renderer, &mut out, scale);
        }
        self.push_dnd_icon(renderer, &mut out, scale);
        self.push_dim(&mut out, scale);
        if self.lock.mode == LockMode::External {
            if let Some(o) = &primary {
                self.push_ext_lock(renderer, &mut out, o, scale);
            }
            return out;
        }
        if lp > 0.0 {
            let ll = self.shell.lock_layers(lp);
            for l in ll.iter().rev() {
                self.push_layer(renderer, &mut out, l, scale);
            }
            if self.lock.mode == LockMode::Internal && lp >= 1.0 {
                self.push_wallpaper(renderer, &mut out);
                return out;
            }
        }
        let fs_front = !self.mission_visible() && self.front_is_fullscreen();
        self.shell.fullscreen = fs_front;
        let layers = if self.lock.mode == LockMode::Internal { vec![] } else { self.shell.layers() };
        {
            let rc = &mut self.render_cache;
            let mut freed = 0;
            rc.layers.retain(|_, (_, _, used)| {
                let keep = used.elapsed() < std::time::Duration::from_secs(5);
                if !keep {
                    freed += 1;
                }
                keep
            });
            rc.freed += freed * 1_000_000;
        }
        if let Some(o) = &primary {
            self.push_layer_surfaces(renderer, &mut out, o, WlrLayer::Overlay, scale);
        }
        let cursor_n = out.len();

        let (above, below): (Vec<_>, Vec<_>) = layers.into_iter().partition(|l| !matches!(l.id, LayerId::Widgets(_)));
        for l in above.iter().rev() {
            if l.id == LayerId::Dock {
                self.push_dock_thumbnails(renderer, &mut out, scale);
            }
            self.push_layer(renderer, &mut out, l, scale);
            if l.id == LayerId::MissionBar {
                self.push_desk_thumbnails(renderer, &mut out, scale);
            }
        }

        self.push_ghosts(renderer, &mut out, scale);

        if let Some(o) = primary.as_ref().filter(|_| !fs_front) {
            self.push_layer_surfaces(renderer, &mut out, o, WlrLayer::Top, scale);
        }

        let windows: Vec<Window> = self.space.elements().rev().cloned().collect();
        let focused = self.focused_window();
        let dragged = self.mission.drag.as_ref().filter(|d| d.active).map(|d| d.id);
        if let Some(w) = windows.iter().find(|w| Some(meta(w).borrow().id) == dragged) {
            let mut front = Vec::new();
            self.push_window(renderer, &mut front, w, false, scale);
            let at = cursor_n.min(out.len());
            out.splice(at..at, front);
        }
        for (i, w) in windows.iter().filter(|w| Some(meta(w).borrow().id) != dragged).enumerate() {
            self.push_window(renderer, &mut out, w, Some(w) == focused.as_ref(), scale);
            if i == 0 {
                // Edge-tiling highlight right behind the window being dragged (it is in front).
                self.push_tile_preview(&mut out, scale);
            }
        }
        if !self.mission_visible() && !fs_front {
            self.push_stage_strip(renderer, &mut out, scale);
        }

        let mc = self.mission.progress();
        if mc > 0.0 {
            let (ow, oh) = self.output_size();
            let g = self.cfg.glass.material(Material::Overlay, self.shell.style.dark);
            let p = glass_params(&g, sf);
            let full = to_phys(Rectangle::<f64, Logical>::new((0.0, 0.0).into(), (ow as f64, oh as f64).into()), scale);
            if let Some(e) = self.render_cache.glass_el(GlassKey::Mission, full, p, mc) {
                out.push(AquaElement::Glass(e));
            }
        }

        for l in below.iter().rev() {
            self.push_layer(renderer, &mut out, l, scale);
        }
        if let Some(o) = &primary {
            self.push_layer_surfaces(renderer, &mut out, o, WlrLayer::Bottom, scale);
            self.push_layer_surfaces(renderer, &mut out, o, WlrLayer::Background, scale);
        }

        self.push_wallpaper(renderer, &mut out);
        out
    }

    /// Scene for any output: the full desktop on the primary display; windows,
    /// layer surfaces and the wallpaper on secondary displays.
    pub fn elements_for_output(&mut self, renderer: &mut GlesRenderer, o: &smithay::output::Output) -> Vec<OutElement> {
        if Some(o) == self.output.as_ref() {
            return self.build_elements(renderer).into_iter().map(OutElement::Main).collect();
        }
        use smithay::wayland::shell::wlr_layer::Layer as WlrLayer;
        let scale = o.current_scale().fractional_scale();
        let Some(og) = self.space.output_geometry(o) else { return vec![] };
        let mut out: Vec<AquaElement> = Vec::new();
        let ptr = self.seat.get_pointer().map(|p| p.current_location()).unwrap_or_default();
        if self.draw_cursor && og.to_f64().contains(ptr) {
            self.push_cursor(renderer, &mut out, scale);
        }
        if og.to_f64().contains(ptr) {
            self.push_dnd_icon(renderer, &mut out, scale);
        }
        self.push_dim(&mut out, scale);
        match self.lock.mode {
            crate::system::lock::Mode::External => self.push_ext_lock(renderer, &mut out, o, scale),
            crate::system::lock::Mode::Internal => {}
            crate::system::lock::Mode::Unlocked => {
                self.push_layer_surfaces(renderer, &mut out, o, WlrLayer::Overlay, scale);
                self.push_layer_surfaces(renderer, &mut out, o, WlrLayer::Top, scale);
                let windows: Vec<Window> = self.space.elements().rev().cloned().collect();
                let focused = self.focused_window();
                for w in &windows {
                    let Some(r) = self.frame_rect(w) else { continue };
                    if !r.overlaps(og) {
                        continue;
                    }
                    self.push_window(renderer, &mut out, w, Some(w) == focused.as_ref(), scale);
                }
                self.push_layer_surfaces(renderer, &mut out, o, WlrLayer::Bottom, scale);
                self.push_layer_surfaces(renderer, &mut out, o, WlrLayer::Background, scale);
            }
        }
        let shift: Point<i32, Physical> =
            (-(og.loc.x as f64 * scale).round() as i32, -(og.loc.y as f64 * scale).round() as i32).into();
        let mut res: Vec<OutElement> = out
            .into_iter()
            .map(|e| OutElement::Moved(RelocateRenderElement::from_element(e, shift, Relocate::Relative)))
            .collect();
        if self.lock.mode != crate::system::lock::Mode::External {
            let mut wp = Vec::new();
            self.push_wallpaper_sized(renderer, &mut wp, (og.size.w, og.size.h));
            res.extend(wp.into_iter().map(OutElement::Main));
        }
        res
    }

    /// Give memory released by cache pruning back to the system. glibc keeps freed
    /// heap pages around (closed Launchpad / Spotlight / screenshots leave tens of MB
    /// resident otherwise); trimming is cheap when there is something to return.
    pub fn release_memory(&mut self) {
        let freed = std::mem::take(&mut self.render_cache.freed) + std::mem::take(&mut self.shell.freed);
        if freed >= 4 << 20 {
            tracing::debug!("returning ~{} MB of freed caches to the system", freed >> 20);
            #[cfg(target_env = "gnu")]
            unsafe {
                libc::malloc_trim(0);
            }
        } else {
            self.render_cache.freed += freed;
        }
        let before = self.render_cache.offscreen.len();
        self.render_cache.offscreen.retain(|_, t| t.used.elapsed() < std::time::Duration::from_secs(5));
        self.render_cache.freed += (before - self.render_cache.offscreen.len()) << 20;
        if !self.mission_visible() && self.render_cache.wallpaper_thumb.take().is_some() {
            self.render_cache.freed += 1 << 20;
        }
    }

    /// Drop caches belonging to windows that no longer exist.
    pub fn prune_render_cache(&mut self) {
        let ids: std::collections::HashSet<u64> =
            self.space.elements().chain(self.minimized.iter()).chain(self.stage.staged.iter()).map(|w| meta(w).borrow().id).collect();
        // genie cards live exactly as long as the minimise / restore they were made for
        let anims: std::collections::HashMap<u64, Instant> = self
            .space
            .elements()
            .chain(self.minimized.iter())
            .filter_map(|w| meta(w).borrow().minimizing.map(|(t, _)| (meta(w).borrow().id, t)))
            .collect();
        let rc = &mut self.render_cache;
        rc.genie_cards.retain(|k, (t, _)| anims.get(k) == Some(t));
        rc.titlebars.retain(|k, _| ids.contains(k));
        rc.shadows.retain(|k, _| ids.contains(k));
        rc.glass.retain(|k, _| match k {
            GlassKey::Window(id) | GlassKey::Client(id, _) => ids.contains(id),
            _ => true,
        });
    }
}

/// The root wl_surface of a Wayland or X11 window.
pub struct SurfRef(smithay::reexports::wayland_server::protocol::wl_surface::WlSurface);
impl SurfRef {
    pub fn of(w: &Window) -> Option<Self> {
        use smithay::wayland::seat::WaylandFocus;
        w.wl_surface().map(|s| SurfRef(s.into_owned()))
    }
    pub fn wl_surface(&self) -> &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface {
        &self.0
    }
}

fn aqua_shell_hash<T: std::hash::Hash>(t: &T) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

/// The classic arrow: black fill, white outline, soft shadow.
pub fn arrow_cursor(scale: f32) -> Pixmap {
    crate::input::cursors::draw(crate::input::cursors::Shape::Arrow, scale)
}
