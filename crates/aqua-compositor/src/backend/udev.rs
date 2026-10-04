//! Native backend: DRM/KMS output + libinput, running from a TTY via libseat.
//!
//! Single-GPU, primary-connector design (extra connectors are lit up mirroring the
//! primary layout origin). Adapted from Smithay's anvil.
use std::path::Path;

use crate::state::Aqua;
use smithay::{
    backend::{
        allocator::{
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
            Fourcc,
        },
        drm::{
            compositor::FrameFlags,
            exporter::gbm::GbmFramebufferExporter,
            output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements},
            DrmDevice, DrmDeviceFd, DrmEvent, DrmNode, NodeType,
        },
        egl::{EGLContext, EGLDisplay},
        input::InputEvent,
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{gles::GlesRenderer, ImportDma, ImportMemWl},
        session::{libseat::LibSeatSession, Event as SessionEvent, Session},
        udev::{all_gpus, primary_gpu, UdevBackend, UdevEvent},
    },
    output::{Mode as WlMode, Output, PhysicalProperties, Scale},
    reexports::{
        calloop::EventLoop,
        drm::control::{connector, crtc, ModeTypeFlags},
        input::Libinput,
        rustix::fs::OFlags,
        wayland_server::Display,
    },
    utils::{DeviceFd, Transform},
    wayland::dmabuf::{DmabufFeedbackBuilder, DmabufGlobal},
};
use smithay_drm_extras::drm_scanner::{DrmScanEvent, DrmScanner};

const FORMATS: &[Fourcc] = &[Fourcc::Abgr8888, Fourcc::Argb8888];

type Manager = DrmOutputManager<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;
type Out = DrmOutput<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;

pub struct Surface {
    pub output: Output,
    pub drm: Out,
    pub pending: bool,
}

pub struct UdevData {
    pub session: LibSeatSession,
    pub node: DrmNode,
    pub renderer: GlesRenderer,
    pub manager: Manager,
    pub scanner: DrmScanner,
    pub surfaces: Vec<(crtc::Handle, Surface)>,
    pub libinput: Libinput,
    pub dmabuf_global: Option<DmabufGlobal>,
    pub input_devices: Vec<smithay::reexports::input::Device>,
    pub dpms_off: bool,
    /// linux-drm-syncobj-v1 (explicit sync); required for artefact-free NVIDIA.
    pub syncobj_state: Option<smithay::wayland::drm_syncobj::DrmSyncobjState>,
    /// Connector handle + all modes for each CRTC (Displays settings, mode switching).
    pub connectors: Vec<(crtc::Handle, connector::Handle, Vec<smithay::reexports::drm::control::Mode>)>,
    pub frame_flags: FrameFlags,
}

fn pick_scale(conn: &connector::Info, mode_w: u16) -> f64 {
    if let Ok(s) = std::env::var("AQUA_SCALE") {
        if let Ok(v) = s.parse::<f64>() {
            return v.clamp(0.5, 4.0);
        }
    }
    let (mm_w, _) = conn.size().unwrap_or((0, 0));
    if mm_w == 0 {
        return 1.0;
    }
    let dpi = mode_w as f64 / (mm_w as f64 / 25.4);
    if dpi > 250.0 {
        2.5
    } else if dpi > 200.0 {
        2.0
    } else if dpi > 165.0 {
        1.75
    } else if dpi > 140.0 {
        1.5
    } else if dpi > 115.0 {
        1.25
    } else {
        1.0
    }
}

/// (make, model, serial) from the connector's EDID in sysfs (`/sys/class/drm/cardN-NAME/edid`).
pub fn edid_info(connector: &str) -> Option<(String, String, String)> {
    let dir = std::fs::read_dir("/sys/class/drm").ok()?;
    for e in dir.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if n.starts_with("card") && n.ends_with(&format!("-{connector}")) {
            let edid = std::fs::read(e.path().join("edid")).ok()?;
            return parse_edid(&edid);
        }
    }
    None
}

pub fn parse_edid(edid: &[u8]) -> Option<(String, String, String)> {
    if edid.len() < 128 || edid[0..8] != [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00] {
        return None;
    }
    let id = u16::from_be_bytes([edid[8], edid[9]]);
    let ch = |v: u16| (b'A' - 1 + (v & 0x1f) as u8) as char;
    let pnp: String = [ch(id >> 10), ch(id >> 5), ch(id)].iter().collect();
    let make = match pnp.as_str() {
        "AUO" => "AU Optronics",
        "BOE" => "BOE",
        "CMN" => "Chimei Innolux",
        "SDC" => "Samsung Display",
        "SAM" => "Samsung",
        "GSM" => "LG",
        "LGD" => "LG Display",
        "DEL" => "Dell",
        "ACR" => "Acer",
        "AUS" => "ASUS",
        "BNQ" => "BenQ",
        "HWP" => "HP",
        "LEN" => "Lenovo",
        "PHL" => "Philips",
        "VSC" => "ViewSonic",
        "AOC" => "AOC",
        "MSI" | "MSG" => "MSI",
        "GBT" => "Gigabyte",
        "SNY" => "Sony",
        "APP" => "Apple",
        "IVM" => "iiyama",
        "NEC" => "NEC",
        "SHP" => "Sharp",
        "HKC" => "HKC",
        "XMI" => "Xiaomi",
        "HPN" => "HP",
        "EIZ" => "EIZO",
        "ENC" => "EIZO",
        "FUS" => "Fujitsu",
        "TCL" => "TCL",
        "HSD" => "HannStar",
        _ => pnp.as_str(),
    }
    .to_string();
    let mut model = String::new();
    let mut serial = String::new();
    for d in 0..4 {
        let b = &edid[54 + d * 18..54 + (d + 1) * 18];
        if b[0] == 0 && b[1] == 0 {
            let text = String::from_utf8_lossy(&b[5..18]).split('\n').next().unwrap_or("").trim().to_string();
            match b[3] {
                0xfc => model = text,
                0xff => serial = text,
                _ => {}
            }
        }
    }
    if model.is_empty() {
        model = format!("{pnp} {:04X}", u16::from_le_bytes([edid[10], edid[11]]));
    }
    if serial.is_empty() {
        let sn = u32::from_le_bytes([edid[12], edid[13], edid[14], edid[15]]);
        if sn != 0 {
            serial = sn.to_string();
        }
    }
    Some((make, model, serial))
}

/// Kernel driver bound to a DRM device node (`/dev/dri/cardN` → "amdgpu", "i915", "nvidia" …).
pub fn gpu_driver(path: &Path) -> String {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    std::fs::read_link(format!("/sys/class/drm/{name}/device/driver"))
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "unknown".into())
}

fn nvidia_param(name: &str) -> Option<String> {
    std::fs::read_to_string(format!("/sys/module/nvidia_drm/parameters/{name}")).ok().map(|s| s.trim().to_string())
}

/// NVIDIA proprietary driver: needs KMS (`nvidia_drm.modeset=1`) and GBM; clients need a
/// few environment variables to pick the right GBM/GLX/VA-API backends.
fn prepare_nvidia() -> Result<(), Box<dyn std::error::Error>> {
    match nvidia_param("modeset").as_deref() {
        Some("Y") | Some("1") => {}
        other => {
            let msg = format!(
                "NVIDIA kernel modesetting is disabled (nvidia_drm.modeset={}). Enable it: add \
                 `nvidia_drm.modeset=1 nvidia_drm.fbdev=1` to the kernel command line (or \
                 `options nvidia_drm modeset=1 fbdev=1` to /etc/modprobe.d/nvidia.conf), \
                 then `sudo mkinitcpio -P` and reboot.",
                other.unwrap_or("?")
            );
            tracing::error!("{msg}");
            return Err(msg.into());
        }
    }
    if nvidia_param("fbdev").as_deref() == Some("N") {
        tracing::warn!("nvidia_drm.fbdev=0: VT switching may show a black screen; consider nvidia_drm.fbdev=1");
    }
    for (k, v) in [
        ("GBM_BACKEND", "nvidia-drm"),
        ("__GLX_VENDOR_LIBRARY_NAME", "nvidia"),
        ("LIBVA_DRIVER_NAME", "nvidia"),
        ("NVD_BACKEND", "direct"),
        ("ELECTRON_OZONE_PLATFORM_HINT", "auto"),
        // WebKitGTK's DMA-BUF renderer flickers / shows stale frames on NVIDIA (Telegram
        // mini apps, Tauri apps, GNOME Web …); the shared-memory path is stable.
        ("WEBKIT_DISABLE_DMABUF_RENDERER", "1"),
    ] {
        if std::env::var_os(k).is_none() {
            unsafe { std::env::set_var(k, v) };
        }
    }
    if let Ok(v) = std::fs::read_to_string("/sys/module/nvidia/version") {
        let major: u32 = v.trim().split('.').next().and_then(|m| m.parse().ok()).unwrap_or(0);
        tracing::info!("NVIDIA driver {}", v.trim());
        if major > 0 && major < 555 {
            tracing::warn!(
                "NVIDIA driver < 555 has no explicit sync; expect flicker in XWayland/Vulkan apps. Update to ≥ 555."
            );
        }
    }
    Ok(())
}

/// Planes the DRM compositor may scan client buffers out of.
fn frame_flags(nvidia: bool) -> FrameFlags {
    match std::env::var("AQUA_SCANOUT").as_deref() {
        Ok("0") => FrameFlags::empty(),
        Ok("all") => FrameFlags::DEFAULT,
        _ if nvidia => FrameFlags::ALLOW_PRIMARY_PLANE_SCANOUT | FrameFlags::ALLOW_CURSOR_PLANE_SCANOUT,
        _ => FrameFlags::DEFAULT,
    }
}

/// Open the seat, GPU, connectors and input devices and build the compositor state.
pub fn init(
    event_loop: &mut EventLoop<'static, Aqua>,
    display: Display<Aqua>,
    cfg: aqua_config::Config,
) -> Result<Aqua, Box<dyn std::error::Error>> {
    let (mut session, notifier) = LibSeatSession::new()
        .map_err(|e| format!("could not start a seat session (run from a TTY with seatd/logind): {e}"))?;
    let seat = session.seat();

    let gpu_path = match std::env::var("AQUA_DRM_DEVICE") {
        Ok(p) => p.into(),
        Err(_) => primary_gpu(&seat)?.or_else(|| all_gpus(&seat).ok()?.into_iter().next()).ok_or("no GPU found")?,
    };
    let node = DrmNode::from_path(&gpu_path)?;
    let driver = gpu_driver(&gpu_path);
    tracing::info!("GPU {} (driver {driver})", gpu_path.display());
    if driver == "nvidia" {
        prepare_nvidia()?;
    }
    let node = node.node_with_type(NodeType::Primary).and_then(|n| n.ok()).unwrap_or(node);
    let fd = session.open(&gpu_path, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK)?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let (drm, drm_notifier) = DrmDevice::new(fd.clone(), true)?;
    let gbm = GbmDevice::new(fd.clone())?;
    let egl = unsafe { EGLDisplay::new(gbm.clone())? };
    let ctx = EGLContext::new(&egl)?;
    let renderer = unsafe { GlesRenderer::new(ctx)? };
    let render_node = node.node_with_type(NodeType::Render).and_then(|n| n.ok());
    let render_formats = renderer.egl_context().dmabuf_render_formats().clone();
    let allocator = GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT);
    let exporter = GbmFramebufferExporter::new(gbm.clone(), render_node.into());
    let manager = DrmOutputManager::new(
        drm,
        allocator,
        exporter,
        Some(gbm),
        FORMATS.iter().copied(),
        render_formats.iter().copied(),
    );

    let mut scanner = DrmScanner::new();
    let scan = scanner.scan_connectors(manager.device())?;
    let mut connected: Vec<(connector::Info, crtc::Handle)> = vec![];
    for ev in scan {
        if let DrmScanEvent::Connected { connector, crtc: Some(crtc) } = ev {
            connected.push((connector, crtc));
        }
    }
    let (first, _) = connected.first().ok_or("no connected display found")?;
    let mode = preferred_mode(first);
    let scale = pick_scale(first, mode.size().0);
    let (pw, ph) = mode.size();
    let (lw, lh) = ((pw as f64 / scale) as f32, (ph as f64 / scale) as f32);

    let mut state = Aqua::new(event_loop, display, cfg, lw, lh, scale);
    state.draw_cursor = true;
    state.shm_state.update_formats(renderer.shm_formats());

    let dmabuf_formats = renderer.dmabuf_formats();
    let dmabuf_global = DmabufFeedbackBuilder::new(node.dev_id(), dmabuf_formats)
        .build()
        .ok()
        .map(|fb| state.dmabuf_state.create_global_with_default_feedback::<Aqua>(&state.display_handle, &fb));

    let mut libinput = Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput.udev_assign_seat(&seat).map_err(|_| "libinput: failed to assign seat")?;
    let input_backend = LibinputInputBackend::new(libinput.clone());
    event_loop.handle().insert_source(input_backend, |event, _, state| {
        if let InputEvent::DeviceAdded { .. } = &event {
            state.needs_redraw = true;
        }
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| state.process_input_event(event))).is_err() {
            tracing::error!("panic in input handling (recovered)");
        }
    })?;

    let syncobj_state = if std::env::var_os("AQUA_NO_EXPLICIT_SYNC").is_none()
        && smithay::wayland::drm_syncobj::supports_syncobj_eventfd(manager.device().device_fd())
    {
        tracing::info!("explicit sync (linux-drm-syncobj-v1) enabled");
        Some(smithay::wayland::drm_syncobj::DrmSyncobjState::new::<Aqua>(
            &state.display_handle,
            manager.device().device_fd().clone(),
        ))
    } else {
        None
    };
    state.udev = Some(UdevData {
        session,
        node,
        renderer,
        manager,
        scanner,
        surfaces: vec![],
        libinput,
        dmabuf_global,
        input_devices: vec![],
        dpms_off: false,
        syncobj_state,
        connectors: vec![],
        frame_flags: frame_flags(driver == "nvidia"),
    });
    for (conn, crtc) in connected {
        state.connector_connected(conn, crtc);
    }

    event_loop.handle().insert_source(drm_notifier, move |event, _, state| match event {
        DrmEvent::VBlank(crtc) => state.frame_finish(crtc),
        DrmEvent::Error(e) => tracing::error!("drm error: {e:?}"),
    })?;

    event_loop.handle().insert_source(notifier, |event, _, state| {
        let Some(ud) = state.udev.as_mut() else { return };
        match event {
            SessionEvent::PauseSession => {
                ud.libinput.suspend();
                ud.manager.pause();
            }
            SessionEvent::ActivateSession => {
                let _ = ud.libinput.resume();
                if let Err(e) = ud.manager.lock().activate(false) {
                    tracing::error!("failed to re-activate drm: {e:?}");
                }
                for (_, s) in ud.surfaces.iter_mut() {
                    s.pending = false;
                    s.drm.reset_buffers();
                }
                state.needs_redraw = true;
                state.render_udev();
                state.export_session_env();
            }
        }
    })?;

    if let Ok(udev) = UdevBackend::new(&seat) {
        event_loop.handle().insert_source(udev, |event, _, state| {
            if let UdevEvent::Changed { .. } = event {
                state.rescan_connectors();
            }
        })?;
    }

    super::schedule_frames(event_loop, |state| state.render_udev())?;
    Ok(state)
}

/// Exact refresh rate of a DRM mode in Hz (vrefresh() is rounded, e.g. 59.94 → 60).
pub fn mode_hz(m: &smithay::reexports::drm::control::Mode) -> f64 {
    let htotal = m.hsync().2 as f64;
    let vtotal = m.vsync().2 as f64;
    if htotal > 0.0 && vtotal > 0.0 {
        let mut hz = m.clock() as f64 * 1000.0 / (htotal * vtotal);
        use smithay::reexports::drm::control::ModeFlags;
        if m.flags().contains(ModeFlags::INTERLACE) {
            hz *= 2.0;
        }
        if m.flags().contains(ModeFlags::DBLSCAN) {
            hz /= 2.0;
        }
        (hz * 100.0).round() / 100.0
    } else {
        m.vrefresh() as f64
    }
}

pub fn fmt_hz(hz: f64) -> String {
    if (hz - hz.round()).abs() < 0.005 {
        format!("{}", hz.round() as i64)
    } else {
        format!("{hz:.2}")
    }
}

fn pick_mode(conn: &connector::Info, want: Option<String>) -> smithay::reexports::drm::control::Mode {
    pick_mode_from(conn.modes(), want)
}

/// Mode from config (`"2560x1440@143.91"` / `"2560x1440@144"` / `"1920x1080"`), else the preferred one.
fn pick_mode_from(
    modes: &[smithay::reexports::drm::control::Mode],
    want: Option<String>,
) -> smithay::reexports::drm::control::Mode {
    if let Some(w) = want {
        let (res, hz) =
            w.split_once('@').map(|(a, b)| (a.to_string(), b.parse::<f64>().ok())).unwrap_or((w.clone(), None));
        if let Some((x, y)) =
            res.split_once('x').and_then(|(x, y)| Some((x.parse::<u16>().ok()?, y.parse::<u16>().ok()?)))
        {
            let mut best: Option<smithay::reexports::drm::control::Mode> = None;
            for m in modes {
                if m.size() == (x, y) {
                    let better = match (best, hz) {
                        (None, _) => true,
                        (Some(b), Some(h)) => (mode_hz(m) - h).abs() < (mode_hz(&b) - h).abs(),
                        (Some(b), None) => {
                            let bp = b.mode_type().contains(ModeTypeFlags::PREFERRED);
                            let mp = m.mode_type().contains(ModeTypeFlags::PREFERRED);
                            (mp && !bp) || (!bp && mode_hz(m) > mode_hz(&b))
                        }
                    };
                    if better {
                        best = Some(*m);
                    }
                }
            }
            if let Some(b) = best {
                return b;
            }
        }
        tracing::warn!("mode {w} not available, using preferred");
    }
    let i = modes.iter().position(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED)).unwrap_or(0);
    modes[i]
}

fn preferred_mode(conn: &connector::Info) -> smithay::reexports::drm::control::Mode {
    pick_mode_from(conn.modes(), None)
}

impl Aqua {
    fn connector_connected(&mut self, conn: connector::Info, crtc: crtc::Handle) {
        let name = format!("{}-{}", conn.interface().as_str(), conn.interface_id());
        let mode = pick_mode(&conn, self.output_cfg(&name).map(|c| c.mode.clone()).filter(|m| !m.is_empty()));
        let scale = self.scale_for(&name, pick_scale(&conn, mode.size().0));
        let transform = self
            .output_cfg(&name)
            .map(|c| crate::wm::outputs::parse_transform(&c.transform))
            .unwrap_or(Transform::Normal);
        if self.output_cfg(&name).map(|c| !c.enabled).unwrap_or(false) {
            tracing::info!("output {name} disabled in config");
            return;
        }
        let Some(ud) = self.udev.as_mut() else { return };
        let wl_mode = WlMode::from(mode);
        let (mm_w, mm_h) = conn.size().unwrap_or((0, 0));
        let output = Output::new(name.clone(), {
            let (make, model, serial) =
                edid_info(&name).unwrap_or_else(|| ("Unknown".into(), name.clone(), String::new()));
            PhysicalProperties {
                size: (mm_w as i32, mm_h as i32).into(),
                subpixel: conn.subpixel().into(),
                make,
                model,
                serial_number: serial,
            }
        });
        let _global = output.create_global::<Aqua>(&self.display_handle);
        output.set_preferred(wl_mode);
        output.change_current_state(
            Some(wl_mode),
            Some(transform),
            Some(Scale::Fractional(scale)),
            Some((0, 0).into()),
        );
        let planes = ud.manager.device().planes(&crtc).ok();
        let res = ud.manager.lock().initialize_output::<GlesRenderer, crate::render::OutElement>(
            crtc,
            mode,
            &[conn.handle()],
            &output,
            planes,
            &mut ud.renderer,
            &DrmOutputRenderElements::default(),
        );
        match res {
            Ok(drm) => {
                tracing::info!("output {name} {}x{}@{} ready", mode.size().0, mode.size().1, mode.vrefresh());
                ud.connectors.retain(|(c, _, _)| *c != crtc);
                ud.connectors.push((crtc, conn.handle(), conn.modes().to_vec()));
                ud.surfaces.push((crtc, Surface { output: output.clone(), drm, pending: false }));
                self.add_output(output);
                self.needs_redraw = true;
            }
            Err(e) => tracing::warn!("failed to init output {name}: {e:?}"),
        }
    }

    fn rescan_connectors(&mut self) {
        let Some(ud) = self.udev.as_mut() else { return };
        let Ok(scan) = ud.scanner.scan_connectors(ud.manager.device()) else { return };
        let mut added = vec![];
        let mut removed = vec![];
        for ev in scan {
            match ev {
                DrmScanEvent::Connected { connector, crtc: Some(crtc) } => added.push((connector, crtc)),
                DrmScanEvent::Disconnected { crtc: Some(crtc), .. } => {
                    ud.connectors.retain(|(c, _, _)| *c != crtc);
                    if let Some(pos) = ud.surfaces.iter().position(|(c, _)| *c == crtc) {
                        let (_, s) = ud.surfaces.remove(pos);
                        removed.push(s.output.clone());
                    }
                }
                _ => {}
            }
        }
        for o in removed {
            tracing::info!("output {} disconnected", o.name());
            self.remove_output(&o);
        }
        for (c, crtc) in added {
            self.connector_connected(c, crtc);
        }
        self.on_output_resized();
    }

    fn frame_finish(&mut self, crtc: crtc::Handle) {
        let Some(ud) = self.udev.as_mut() else { return };
        let mut out = None;
        if let Some((_, s)) = ud.surfaces.iter_mut().find(|(c, _)| *c == crtc) {
            if let Err(e) = s.drm.frame_submitted() {
                tracing::warn!("frame_submitted: {e:?}");
            }
            s.pending = false;
            out = Some(s.output.clone());
        }
        if let Some(o) = out {
            self.after_frame(&o);
        }
        if self.needs_redraw {
            self.render_udev();
        }
    }

    /// Render all idle outputs.
    pub fn render_udev(&mut self) {
        let Some(mut ud) = self.udev.take() else { return };
        let mut skipped_pending = false;
        let mut idle_outputs: Vec<Output> = vec![];
        let shot = self.screenshot_request.take();
        let dpms_off = ud.dpms_off;
        for (_, s) in ud.surfaces.iter_mut() {
            if s.pending {
                skipped_pending = true;
                continue;
            }
            let output = s.output.clone();
            let t0 = std::time::Instant::now();
            let elements = if dpms_off { vec![] } else { self.elements_for_output(&mut ud.renderer, &output) };
            match s.drm.render_frame(&mut ud.renderer, &elements, crate::backend::winit::CLEAR, ud.frame_flags) {
                Ok(res) => {
                    {
                        use aqua_render::stats::{record_frame, Frame};
                        use smithay::backend::drm::compositor::PrimaryPlaneElement;
                        let size = output.current_mode().map(|m| (m.size.w, m.size.h)).unwrap_or((0, 0));
                        let took = t0.elapsed();
                        match &res.primary_element {
                            _ if res.is_empty => record_frame(&output.name(), size, took, Frame::Empty),
                            PrimaryPlaneElement::Swapchain(sw) => {
                                let rects: Vec<aqua_render::stats::R> = sw
                                    .damage
                                    .raw()
                                    .next()
                                    .map(|d| d.map(|r| (r.loc.x, r.loc.y, r.size.w, r.size.h)).collect())
                                    .unwrap_or_default();
                                record_frame(&output.name(), size, took, Frame::Rendered(Some(&rects)))
                            }
                            PrimaryPlaneElement::Element(_) => record_frame(&output.name(), size, took, Frame::Scanout),
                        }
                    }
                    if !res.is_empty {
                        match s.drm.queue_frame(()) {
                            Ok(()) => s.pending = true,
                            Err(e) => {
                                tracing::warn!("queue_frame: {e:?}");
                                idle_outputs.push(output.clone());
                            }
                        }
                    } else {
                        idle_outputs.push(output.clone());
                    }
                }
                Err(e) => {
                    tracing::warn!("render_frame: {e:?}");
                    idle_outputs.push(output.clone());
                }
            }
        }
        self.service_captures(&mut ud.renderer);
        self.service_casts(&mut ud.renderer);
        self.after_render_stats();
        if let Some(mode) = self.output.as_ref().and_then(|o| o.current_mode()) {
            self.service_screenshot(&mut ud.renderer, mode.size);
        }
        if let Some(path) = shot {
            if let Some(mode) = self.output.as_ref().and_then(|o| o.current_mode()) {
                if let Err(e) = crate::render::screenshot(self, &mut ud.renderer, mode.size, &path) {
                    tracing::error!("screenshot failed: {e}");
                } else {
                    tracing::info!("screenshot saved to {path}");
                }
            }
        }
        self.udev = Some(ud);
        self.needs_redraw = skipped_pending;
        let time = self.start_time.elapsed();
        for o in idle_outputs {
            self.send_frame_callbacks(&o, time);
        }
        let _ = self.display_handle.flush_clients();
    }

    /// Every mode of a DRM output as "WxH@HZ" (refresh with up to 2 decimals), best first.
    pub fn output_modes(&self, output: &Output) -> Vec<String> {
        let Some(ud) = self.udev.as_ref() else { return vec![] };
        let Some((crtc, _)) = ud.surfaces.iter().find(|(_, s)| &s.output == output) else { return vec![] };
        let Some((_, _, modes)) = ud.connectors.iter().find(|(c, _, _)| c == crtc) else { return vec![] };
        let mut v: Vec<(u16, u16, f64)> = modes.iter().map(|m| (m.size().0, m.size().1, mode_hz(m))).collect();
        v.sort_by(|a, b| {
            (b.0 as u32 * b.1 as u32)
                .cmp(&(a.0 as u32 * a.1 as u32))
                .then(b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal))
        });
        let mut out: Vec<String> = vec![];
        for (w, h, hz) in v {
            let s = format!("{w}x{h}@{}", fmt_hz(hz));
            if !out.contains(&s) {
                out.push(s);
            }
        }
        out
    }

    /// Apply `mode` / `vrr` from `[[outputs]]` to live DRM outputs (Displays settings).
    pub fn apply_drm_modes(&mut self) {
        let Some(mut ud) = self.udev.take() else { return };
        let mut changed = false;
        for (crtc, s) in ud.surfaces.iter_mut() {
            let name = s.output.name();
            let cfg = self.output_cfg(&name);
            let Some((_, conn, modes)) = ud.connectors.iter().find(|(c, _, _)| c == crtc) else { continue };
            let want = cfg.as_ref().map(|c| c.mode.clone()).filter(|m| !m.is_empty());
            let target = pick_mode_from(modes, want.clone());
            let cur = s.drm.with_compositor(|c| c.pending_mode());
            if target != cur {
                let res = s.drm.use_mode::<GlesRenderer, crate::render::OutElement>(
                    target,
                    &mut ud.renderer,
                    &DrmOutputRenderElements::default(),
                );
                match res {
                    Ok(()) => {
                        let wl = WlMode::from(target);
                        s.output.change_current_state(Some(wl), None, None, None);
                        s.output.set_preferred(wl);
                        s.pending = false;
                        changed = true;
                        tracing::info!(
                            "output {name}: mode {}x{}@{}",
                            target.size().0,
                            target.size().1,
                            fmt_hz(mode_hz(&target))
                        );
                    }
                    Err(e) => tracing::warn!("output {name}: could not set mode {want:?}: {e:?}"),
                }
            }
            let want_vrr = cfg.as_ref().map(|c| c.vrr).unwrap_or(false);
            let conn = *conn;
            s.drm.with_compositor(|c| {
                if c.vrr_enabled() != want_vrr {
                    let supported = c
                        .vrr_supported(conn)
                        .map(|v| !matches!(v, smithay::backend::drm::VrrSupport::NotSupported))
                        .unwrap_or(false);
                    if supported || !want_vrr {
                        if let Err(e) = c.use_vrr(want_vrr) {
                            tracing::warn!("output {name}: vrr {want_vrr}: {e:?}");
                        }
                    }
                }
            });
        }
        self.udev = Some(ud);
        if changed {
            self.arrange_outputs();
            self.on_output_resized();
            self.needs_redraw = true;
        }
    }

    pub fn change_vt(&mut self, vt: i32) {
        if let Some(ud) = self.udev.as_mut() {
            if let Err(e) = ud.session.change_vt(vt) {
                tracing::warn!("change_vt failed: {e:?}");
            }
        }
    }

    pub fn import_dmabuf(&mut self, dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf) -> bool {
        match self.udev.as_mut() {
            Some(ud) => ud.renderer.import_dmabuf(dmabuf, None).is_ok(),
            None => false,
        }
    }
}
