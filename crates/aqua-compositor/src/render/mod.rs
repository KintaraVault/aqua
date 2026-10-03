//! Scene assembly: wallpaper → desktop widgets → windows (shadow, glass frame,
//! rounded client surface, titlebar, popups) → shell layers (menu bar, dock, …).
use std::collections::HashMap;
use std::time::Instant;

use crate::state::{is_ssd, meta, title_of, Aqua};
use aqua_config::{metrics, GlassStyle, Rgba};
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
    pub titlebar: Option<MemoryRenderBuffer>,
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
    Mission,
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
    titlebars: HashMap<u64, (u64, MemoryRenderBuffer)>,
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
    /// Offscreen capture targets by output name (dropped a few seconds after the last use).
    offscreen: HashMap<String, OffscreenTarget>,
}

pub struct FileDragImage {
    pub buf: MemoryRenderBuffer,
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
            offscreen: HashMap::new(),
        }
    }
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
            blur: g.blur,
            tint: [r, gg, b, a],
            saturation: g.saturation,
            refraction: g.refraction,
            bevel: g.bevel,
            rim: g.rim,
            max_luma: g.max_luma,
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

/// Material behind client-requested blur regions: no tint of its own (the app's pixels
/// are the tint), strong blur + saturation.
fn client_glass(radius: f32, dark: bool) -> GlassStyle {
    GlassStyle {
        blur: 40.0,
        tint: if dark { Rgba(0.10, 0.10, 0.12, 0.10) } else { Rgba(1.0, 1.0, 1.0, 0.10) },
        saturation: 1.9,
        refraction: 0.0,
        bevel: 0.0,
        rim: 0.0,
        radius,
        shadow: 0.0,
        max_luma: 1.0,
    }
}

/// Material used for window chrome (titlebar + frame behind translucent clients).
fn window_glass(radius: f32, focused: bool, dark: bool) -> GlassStyle {
    GlassStyle {
        blur: 34.0,
        tint: if dark {
            if focused {
                Rgba(0.16, 0.16, 0.18, 0.88)
            } else {
                Rgba(0.14, 0.14, 0.15, 0.92)
            }
        } else if focused {
            Rgba(0.965, 0.965, 0.975, 0.86)
        } else {
            Rgba(0.94, 0.94, 0.95, 0.90)
        },
        saturation: 1.8,
        refraction: 0.0,
        bevel: 6.0,
        rim: 0.35,
        radius,
        shadow: 0.0,
        max_luma: 1.0,
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
        for w in windows.iter().filter(|w| Some(meta(w).borrow().id) != dragged) {
            self.push_window(renderer, &mut out, w, Some(w) == focused.as_ref(), scale);
        }

        let mc = self.mission.progress();
        if mc > 0.0 {
            let (ow, oh) = self.output_size();
            let g = GlassStyle {
                blur: 30.0,
                tint: Rgba(0.04, 0.05, 0.10, 0.38),
                saturation: 1.3,
                refraction: 0.0,
                bevel: 0.0,
                rim: 0.0,
                radius: 0.0,
                shadow: 0.0,
                max_luma: 1.0,
            };
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
            self.space.elements().chain(self.minimized.iter()).map(|w| meta(w).borrow().id).collect();
        let rc = &mut self.render_cache;
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
