//! XEmbed → StatusNotifierItem bridge for old X11 tray icons (Wine, GTK2/Java apps, older
//! Electron, …).
//!
//! On the XWayland display Aqua becomes the XEmbed system tray (`_NET_SYSTEM_TRAY_Sn`).
//! Every docked icon is reparented into an off-screen ARGB container, its pixels are read
//! back whenever it is damaged, and it is exported as a regular `org.kde.StatusNotifierItem`
//! on the session bus (so any SNI host shows it). Clicks and scrolling on the SNI item are
//! sent to the icon as synthetic X button events.
use crate::Icon;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::composite::{self, ConnectionExt as _};
use x11rb::protocol::damage::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{ConnectionExt as _, *};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{COPY_DEPTH_FROM_PARENT, CURRENT_TIME};

const SYSTEM_TRAY_REQUEST_DOCK: u32 = 0;
const XEMBED_EMBEDDED_NOTIFY: u32 = 0;
/// Size icons are asked to draw at.
pub const ICON_SIZE: u16 = 32;
/// WM_CLASS class of the icon containers (the compositor never shows them).
pub const CONTAINER_CLASS: &str = "AquaXEmbed";
/// Off-screen place of the containers (never on any display).
const PARK: i16 = -20000;

x11rb::atom_manager! {
    pub Atoms: AtomsCookie {
        MANAGER,
        _NET_SYSTEM_TRAY_OPCODE,
        _NET_SYSTEM_TRAY_ORIENTATION,
        _NET_SYSTEM_TRAY_VISUAL,
        _NET_WM_NAME,
        _XEMBED,
        UTF8_STRING,
    }
}

/// Receives the icons of the tray.
pub trait Sink: Send {
    fn added(&mut self, win: u32, id: &str, title: &str);
    fn icon(&mut self, win: u32, icon: &Icon);
    fn removed(&mut self, win: u32);
}

/// What a click on the bridged item does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Click {
    Primary,
    Middle,
    Secondary,
    ScrollUp,
    ScrollDown,
    ScrollLeft,
    ScrollRight,
}

impl Click {
    pub fn button(self) -> u8 {
        match self {
            Click::Primary => 1,
            Click::Middle => 2,
            Click::Secondary => 3,
            Click::ScrollUp => 4,
            Click::ScrollDown => 5,
            Click::ScrollLeft => 6,
            Click::ScrollRight => 7,
        }
    }
    /// SNI `Scroll(delta, orientation)` → wheel button.
    pub fn from_scroll(delta: i32, orientation: &str) -> Option<Self> {
        let horizontal = orientation.eq_ignore_ascii_case("horizontal");
        match (delta.signum(), horizontal) {
            (0, _) => None,
            (1, false) => Some(Click::ScrollDown),
            (_, false) => Some(Click::ScrollUp),
            (1, true) => Some(Click::ScrollRight),
            (_, true) => Some(Click::ScrollLeft),
        }
    }
}

/// Send a synthetic click (or wheel step) to an embedded icon window.
pub fn send_click(conn: &RustConnection, win: u32, size: (u16, u16), click: Click) -> Result<(), String> {
    let (x, y) = ((size.0 / 2) as i16, (size.1 / 2) as i16);
    let root = conn.setup().roots.first().map(|s| s.root).unwrap_or(0);
    let e = |r: x11rb::errors::ConnectionError| r.to_string();
    let crossing = EnterNotifyEvent {
        response_type: ENTER_NOTIFY_EVENT,
        detail: NotifyDetail::NONLINEAR,
        sequence: 0,
        time: CURRENT_TIME,
        root,
        event: win,
        child: x11rb::NONE,
        root_x: x,
        root_y: y,
        event_x: x,
        event_y: y,
        state: KeyButMask::default(),
        mode: NotifyMode::NORMAL,
        same_screen_focus: 2, // same-screen, not focus
    };
    conn.send_event(false, win, EventMask::ENTER_WINDOW, crossing).map_err(e)?;
    let button = |kind: u8, state: KeyButMask| ButtonPressEvent {
        response_type: kind,
        detail: click.button(),
        sequence: 0,
        time: CURRENT_TIME,
        root,
        event: win,
        child: x11rb::NONE,
        root_x: x,
        root_y: y,
        event_x: x,
        event_y: y,
        state,
        same_screen: true,
    };
    let mask = match click.button() {
        1 => KeyButMask::BUTTON1,
        2 => KeyButMask::BUTTON2,
        3 => KeyButMask::BUTTON3,
        4 => KeyButMask::BUTTON4,
        _ => KeyButMask::BUTTON5,
    };
    conn.send_event(false, win, EventMask::BUTTON_PRESS, button(BUTTON_PRESS_EVENT, KeyButMask::default()))
        .map_err(e)?;
    conn.send_event(false, win, EventMask::BUTTON_RELEASE, button(BUTTON_RELEASE_EVENT, mask)).map_err(e)?;
    conn.flush().map_err(e)
}

/// Convert a `GetImage` ZPixmap reply (32 bpp, little endian BGRA/BGRX) to premultiplied RGBA.
pub fn zpixmap_to_rgba(data: &[u8], w: u16, h: u16, depth: u8) -> Option<Vec<u8>> {
    let n = w as usize * h as usize;
    if data.len() < n * 4 {
        return None;
    }
    let mut out = Vec::with_capacity(n * 4);
    for px in data[..n * 4].as_chunks::<4>().0 {
        let a = if depth == 32 { px[3] } else { 255 };
        out.extend_from_slice(&[px[2], px[1], px[0], a]);
    }
    Some(out)
}

/// An icon that drew nothing yet (fully transparent) is not shown; opaque flat icons are real.
pub fn is_blank(rgba: &[u8]) -> bool {
    rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 0)
}

struct Docked {
    container: Window,
    damage: damage::Damage,
    size: (u16, u16),
    dirty: Option<Instant>,
    shown: bool,
    last: Option<Arc<Vec<u8>>>,
}

/// The XEmbed tray manager on one X display.
pub struct Host<S: Sink> {
    pub conn: Arc<RustConnection>,
    screen: usize,
    atoms: Atoms,
    owner: Window,
    argb: Option<(Visualid, Colormap)>,
    composite: bool,
    icons: HashMap<Window, Docked>,
    sizes: SizeMap,
    pending_names: HashMap<Window, (String, String)>,
    pub sink: S,
}

/// Icon sizes by window, shared with click handlers on other threads.
pub type SizeMap = Arc<Mutex<HashMap<u32, (u16, u16)>>>;

impl<S: Sink> Host<S> {
    /// Connect to `display` (":0") and take the tray selection. Fails when another tray
    /// manager owns it.
    pub fn new(display: &str, make_sink: impl FnOnce(&Arc<RustConnection>, &SizeMap) -> S) -> Result<Self, String> {
        let (conn, screen) = x11rb::connect(Some(display)).map_err(|e| e.to_string())?;
        let conn = Arc::new(conn);
        let atoms = Atoms::new(&*conn).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?;
        conn.extension_information(damage::X11_EXTENSION_NAME).map_err(|e| e.to_string())?.ok_or("no DAMAGE")?;
        conn.damage_query_version(1, 1).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?;
        let root = conn.setup().roots[screen].clone();
        let owner = conn.generate_id().map_err(|e| e.to_string())?;
        conn.create_window(
            COPY_DEPTH_FROM_PARENT,
            owner,
            root.root,
            PARK,
            PARK,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new().override_redirect(1),
        )
        .map_err(|e| e.to_string())?;
        let argb = find_argb(&conn, screen).and_then(|v| {
            let cm = conn.generate_id().ok()?;
            conn.create_colormap(ColormapAlloc::NONE, cm, root.root, v).ok()?;
            Some((v, cm))
        });
        let composite = conn.extension_information(composite::X11_EXTENSION_NAME).ok().flatten().is_some()
            && conn.composite_query_version(0, 4).ok().and_then(|c| c.reply().ok()).is_some();
        let sizes = SizeMap::default();
        let sink = make_sink(&conn, &sizes);
        let mut host = Self {
            conn,
            screen,
            atoms,
            owner,
            argb,
            composite,
            icons: HashMap::new(),
            sizes,
            pending_names: HashMap::new(),
            sink,
        };
        host.acquire()?;
        Ok(host)
    }

    fn acquire(&mut self) -> Result<(), String> {
        let c = &self.conn;
        let e = |x: x11rb::errors::ConnectionError| x.to_string();
        let name = format!("_NET_SYSTEM_TRAY_S{}", self.screen);
        let sel = c.intern_atom(false, name.as_bytes()).map_err(e)?.reply().map_err(|x| x.to_string())?.atom;
        let current = c.get_selection_owner(sel).map_err(e)?.reply().map_err(|x| x.to_string())?.owner;
        if current != x11rb::NONE {
            return Err("another XEmbed tray is running".into());
        }
        c.change_property32(
            PropMode::REPLACE,
            self.owner,
            self.atoms._NET_SYSTEM_TRAY_ORIENTATION,
            AtomEnum::CARDINAL,
            &[0],
        )
        .map_err(e)?;
        if let Some((v, _)) = self.argb {
            c.change_property32(
                PropMode::REPLACE,
                self.owner,
                self.atoms._NET_SYSTEM_TRAY_VISUAL,
                AtomEnum::VISUALID,
                &[v],
            )
            .map_err(e)?;
        }
        c.set_selection_owner(self.owner, sel, CURRENT_TIME).map_err(e)?;
        let root = c.setup().roots[self.screen].root;
        let ev = ClientMessageEvent::new(32, root, self.atoms.MANAGER, [CURRENT_TIME, sel, self.owner, 0, 0]);
        c.send_event(false, root, EventMask::STRUCTURE_NOTIFY, ev).map_err(e)?;
        c.flush().map_err(e)?;
        Ok(())
    }

    /// Handle X events until the connection breaks.
    pub fn run(&mut self) -> Result<(), String> {
        loop {
            while let Some(ev) = self.conn.poll_for_event().map_err(|e| e.to_string())? {
                self.handle(ev);
            }
            self.refresh();
            std::thread::sleep(Duration::from_millis(40));
        }
    }

    /// Process pending events and grab damaged icons once (used by tests).
    pub fn pump(&mut self) -> Result<(), String> {
        let _ = self.conn.sync();
        while let Some(ev) = self.conn.poll_for_event().map_err(|e| e.to_string())? {
            self.handle(ev);
        }
        self.refresh();
        Ok(())
    }

    fn handle(&mut self, ev: Event) {
        match ev {
            Event::ClientMessage(m) if m.type_ == self.atoms._NET_SYSTEM_TRAY_OPCODE => {
                let d = m.data.as_data32();
                if d[1] == SYSTEM_TRAY_REQUEST_DOCK {
                    if let Err(e) = self.dock(d[2]) {
                        tracing::debug!("xembed: dock {:#x} failed: {e}", d[2]);
                    }
                }
            }
            Event::DamageNotify(n) => {
                if let Some(d) = self.icons.get_mut(&n.drawable) {
                    d.dirty.get_or_insert_with(Instant::now);
                }
                let _ = self.conn.damage_subtract(n.damage, x11rb::NONE, x11rb::NONE);
            }
            Event::ConfigureNotify(c) => {
                if let Some(d) = self.icons.get_mut(&c.window) {
                    if (c.width, c.height) != (ICON_SIZE, ICON_SIZE) {
                        let _ = self.conn.configure_window(
                            c.window,
                            &ConfigureWindowAux::new().width(ICON_SIZE as u32).height(ICON_SIZE as u32),
                        );
                    }
                    d.dirty.get_or_insert_with(Instant::now);
                }
            }
            Event::DestroyNotify(d) => self.undock(d.window),
            Event::UnmapNotify(u) if self.icons.contains_key(&u.window) => self.undock(u.window),
            Event::ReparentNotify(r) if self.icons.get(&r.window).is_some_and(|d| d.container != r.parent) => {
                self.undock(r.window)
            }
            Event::SelectionClear(_) => {
                tracing::info!("xembed: another tray manager took over");
                for w in self.icons.keys().copied().collect::<Vec<_>>() {
                    self.undock(w);
                }
            }
            _ => {}
        }
    }

    fn dock(&mut self, icon: Window) -> Result<(), String> {
        if self.icons.contains_key(&icon) {
            return Ok(());
        }
        let c = self.conn.clone();
        let e = |x: x11rb::errors::ConnectionError| x.to_string();
        let root = c.setup().roots[self.screen].root;
        let container = c.generate_id().map_err(|x| x.to_string())?;
        let s = ICON_SIZE;
        let (depth, visual, aux) = match self.argb {
            Some((v, cm)) => {
                (32, v, CreateWindowAux::new().override_redirect(1).colormap(cm).border_pixel(0).background_pixel(0))
            }
            None => (COPY_DEPTH_FROM_PARENT, x11rb::COPY_FROM_PARENT, CreateWindowAux::new().override_redirect(1)),
        };
        c.create_window(depth, container, root, PARK, PARK, s, s, 0, WindowClass::INPUT_OUTPUT, visual, &aux)
            .map_err(e)?;
        // Redirected (off-screen rendered) like KDE's xembedsniproxy, so the icon paints
        // even though the container is never visible on any output.
        if self.composite {
            let _ = c.composite_redirect_window(container, composite::Redirect::MANUAL);
        }
        c.change_property8(
            PropMode::REPLACE,
            container,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"aqua-xembed\0AquaXEmbed\0",
        )
        .map_err(e)?;
        c.change_window_attributes(icon, &ChangeWindowAttributesAux::new().event_mask(EventMask::STRUCTURE_NOTIFY))
            .map_err(e)?;
        c.change_save_set(SetMode::INSERT, icon).map_err(e)?;
        c.reparent_window(icon, container, 0, 0).map_err(e)?;
        c.configure_window(icon, &ConfigureWindowAux::new().x(0).y(0).width(s as u32).height(s as u32)).map_err(e)?;
        c.map_window(icon).map_err(e)?;
        c.map_window(container).map_err(e)?;
        let notify = ClientMessageEvent::new(
            32,
            icon,
            self.atoms._XEMBED,
            [CURRENT_TIME, XEMBED_EMBEDDED_NOTIFY, 0, container, 0],
        );
        c.send_event(false, icon, EventMask::NO_EVENT, notify).map_err(e)?;
        let dmg = c.generate_id().map_err(|x| x.to_string())?;
        c.damage_create(dmg, icon, damage::ReportLevel::NON_EMPTY).map_err(e)?;
        c.flush().map_err(e)?;
        let (id, title) = names(&c, &self.atoms, icon);
        self.icons.insert(
            icon,
            Docked { container, damage: dmg, size: (s, s), dirty: Some(Instant::now()), shown: false, last: None },
        );
        self.sizes.lock().unwrap_or_else(|e| e.into_inner()).insert(icon, (s, s));
        tracing::info!("xembed: docked {icon:#x} ({id})");
        self.pending_names.insert(icon, (id, title));
        Ok(())
    }

    fn undock(&mut self, icon: Window) {
        let Some(d) = self.icons.remove(&icon) else { return };
        self.sizes.lock().unwrap_or_else(|e| e.into_inner()).remove(&icon);
        let _ = self.conn.damage_destroy(d.damage);
        let _ = self.conn.destroy_window(d.container);
        let _ = self.conn.flush();
        if d.shown {
            self.sink.removed(icon);
        }
    }

    /// Read back damaged icons (debounced a little: clients often paint in several steps).
    fn refresh(&mut self) {
        let now = Instant::now();
        let due: Vec<Window> = self
            .icons
            .iter()
            .filter(|(_, d)| d.dirty.is_some_and(|t| now.duration_since(t) >= Duration::from_millis(30)))
            .map(|(w, _)| *w)
            .collect();
        for w in due {
            let Some(d) = self.icons.get_mut(&w) else { continue };
            d.dirty = None;
            let (iw, ih) = d.size;
            let Ok(reply) = self.conn.get_image(ImageFormat::Z_PIXMAP, w, 0, 0, iw, ih, !0).map(|c| c.reply()) else {
                continue;
            };
            let Ok(img) = reply else { continue };
            let Some(rgba) = zpixmap_to_rgba(&img.data, iw, ih, img.depth) else { continue };
            if is_blank(&rgba) || d.last.as_deref() == Some(&rgba) {
                continue;
            }
            let rgba = Arc::new(rgba);
            d.last = Some(rgba.clone());
            if !d.shown {
                d.shown = true;
                let (id, title) = self.pending_names.remove(&w).unwrap_or_default();
                self.sink.added(w, &id, &title);
            }
            self.sink.icon(w, &Icon { width: iw as u32, height: ih as u32, rgba });
        }
    }
}

/// (Id from WM_CLASS, title from _NET_WM_NAME / WM_NAME) of an icon window.
fn names(c: &RustConnection, a: &Atoms, w: Window) -> (String, String) {
    let prop = |atom: u32, ty: u32| -> String {
        c.get_property(false, w, atom, ty, 0, 1024)
            .ok()
            .and_then(|r| r.reply().ok())
            .map(|r| String::from_utf8_lossy(&r.value).to_string())
            .unwrap_or_default()
    };
    let class = prop(AtomEnum::WM_CLASS.into(), AtomEnum::STRING.into());
    let id = class.split('\0').find(|s| !s.is_empty()).unwrap_or("xembed").to_string();
    let mut title = prop(a._NET_WM_NAME, a.UTF8_STRING);
    if title.is_empty() {
        title = prop(AtomEnum::WM_NAME.into(), AtomEnum::STRING.into());
    }
    if title.is_empty() {
        title = class.split('\0').nth(1).unwrap_or(&id).to_string();
    }
    (id, title)
}

/// A 32-bit TrueColor visual for transparent icons.
fn find_argb(c: &RustConnection, screen: usize) -> Option<Visualid> {
    c.setup().roots[screen]
        .allowed_depths
        .iter()
        .filter(|d| d.depth == 32)
        .flat_map(|d| d.visuals.iter())
        .find(|v| v.class == VisualClass::TRUE_COLOR)
        .map(|v| v.visual_id)
}

/// Premultiplied RGBA → SNI `IconPixmap` entry (ARGB32, network byte order, straight alpha).
pub fn to_sni_pixmap(icon: &Icon) -> (i32, i32, Vec<u8>) {
    let mut out = Vec::with_capacity(icon.rgba.len());
    for p in icon.rgba.as_chunks::<4>().0 {
        let a = p[3];
        let un = |c: u8| if a == 0 { 0 } else { ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8 };
        out.extend_from_slice(&[a, un(p[0]), un(p[1]), un(p[2])]);
    }
    (icon.width as i32, icon.height as i32, out)
}

mod sni {
    use super::*;
    use zbus::object_server::SignalEmitter;

    pub struct Item {
        pub win: u32,
        pub id: String,
        pub title: String,
        pub pixmap: Vec<(i32, i32, Vec<u8>)>,
        pub x: Arc<RustConnection>,
        pub sizes: SizeMap,
    }

    impl Item {
        fn click(&self, c: Click) {
            let size = self.sizes.lock().unwrap_or_else(|e| e.into_inner()).get(&self.win).copied();
            if let Some(size) = size {
                if let Err(e) = send_click(&self.x, self.win, size, c) {
                    tracing::debug!("xembed: click failed: {e}");
                }
            }
        }
    }

    #[zbus::interface(name = "org.kde.StatusNotifierItem")]
    impl Item {
        fn activate(&self, _x: i32, _y: i32) {
            self.click(Click::Primary);
        }
        fn secondary_activate(&self, _x: i32, _y: i32) {
            self.click(Click::Middle);
        }
        fn context_menu(&self, _x: i32, _y: i32) {
            self.click(Click::Secondary);
        }
        fn scroll(&self, delta: i32, orientation: &str) {
            if let Some(c) = Click::from_scroll(delta, orientation) {
                self.click(c);
            }
        }
        #[zbus(property)]
        fn category(&self) -> &str {
            "ApplicationStatus"
        }
        #[zbus(property)]
        fn id(&self) -> &str {
            &self.id
        }
        #[zbus(property)]
        fn title(&self) -> &str {
            &self.title
        }
        #[zbus(property)]
        fn status(&self) -> &str {
            "Active"
        }
        #[zbus(property)]
        fn icon_name(&self) -> &str {
            ""
        }
        #[zbus(property)]
        fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
            self.pixmap.clone()
        }
        #[zbus(property)]
        fn item_is_menu(&self) -> bool {
            false
        }
        #[zbus(property)]
        fn menu(&self) -> zbus::zvariant::OwnedObjectPath {
            zbus::zvariant::OwnedObjectPath::try_from("/NO_DBUSMENU").expect("valid path")
        }
        #[zbus(signal)]
        pub async fn new_icon(e: &SignalEmitter<'_>) -> zbus::Result<()>;
    }
}

/// Exports docked icons as StatusNotifierItems on the session bus.
pub struct DbusSink {
    conn: zbus::blocking::Connection,
    x: Arc<RustConnection>,
    sizes: SizeMap,
}

fn item_path(win: u32) -> String {
    format!("/org/aqua/XEmbed/{win}")
}

impl Sink for DbusSink {
    fn added(&mut self, win: u32, id: &str, title: &str) {
        let item = sni::Item {
            win,
            id: id.into(),
            title: title.into(),
            pixmap: vec![],
            x: self.x.clone(),
            sizes: self.sizes.clone(),
        };
        let path = item_path(win);
        if let Err(e) = self.conn.object_server().at(path.as_str(), item) {
            tracing::warn!("xembed: cannot export {path}: {e}");
            return;
        }
        let r = self.conn.call_method(
            Some("org.kde.StatusNotifierWatcher"),
            "/StatusNotifierWatcher",
            Some("org.kde.StatusNotifierWatcher"),
            "RegisterStatusNotifierItem",
            &(path.as_str(),),
        );
        if let Err(e) = r {
            tracing::warn!("xembed: cannot register {path}: {e}");
        }
    }

    fn icon(&mut self, win: u32, icon: &Icon) {
        let path = item_path(win);
        let Ok(iface) = self.conn.object_server().interface::<_, sni::Item>(path.as_str()) else { return };
        iface.get_mut().pixmap = vec![to_sni_pixmap(icon)];
        let _ = zbus::block_on(sni::Item::new_icon(iface.signal_emitter()));
    }

    fn removed(&mut self, win: u32) {
        let path = item_path(win);
        let _ = self.conn.object_server().remove::<sni::Item, _>(path.as_str());
        if let Some(bus) = self.conn.unique_name() {
            crate::service::send(crate::service::Cmd::Unregister(format!("{bus}{path}")));
        }
    }
}

/// Start the bridge on XWayland display `:n` (idempotent per display; restarts when
/// XWayland restarted).
pub fn start(disp: u32) {
    static RUNNING: OnceLock<Mutex<Option<u32>>> = OnceLock::new();
    let running = RUNNING.get_or_init(|| Mutex::new(None));
    {
        let mut r = running.lock().unwrap_or_else(|e| e.into_inner());
        if *r == Some(disp) {
            return;
        }
        *r = Some(disp);
    }
    std::thread::Builder::new()
        .name("aqua-xembed".into())
        .spawn(move || {
            let res = (|| -> Result<(), String> {
                let conn = zbus::blocking::Connection::session().map_err(|e| e.to_string())?;
                let mut host =
                    Host::new(&format!(":{disp}"), |x, sizes| DbusSink { conn, x: x.clone(), sizes: sizes.clone() })?;
                tracing::info!("xembed tray bridge running on :{}", disp);
                host.run()
            })();
            if let Err(e) = res {
                tracing::info!("xembed tray bridge stopped: {e}");
            }
            if let Some(m) = RUNNING.get() {
                let mut r = m.lock().unwrap_or_else(|e| e.into_inner());
                if *r == Some(disp) {
                    *r = None;
                }
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_conversion() {
        let bgra = [10, 20, 30, 128, 1, 2, 3, 0];
        assert_eq!(zpixmap_to_rgba(&bgra, 2, 1, 32).unwrap(), vec![30, 20, 10, 128, 3, 2, 1, 0]);
        assert_eq!(zpixmap_to_rgba(&bgra, 2, 1, 24).unwrap(), vec![30, 20, 10, 255, 3, 2, 1, 255]);
        assert!(zpixmap_to_rgba(&bgra, 3, 1, 32).is_none());
        let icon = Icon { width: 1, height: 1, rgba: Arc::new(vec![64, 32, 0, 128]) };
        assert_eq!(to_sni_pixmap(&icon), (1, 1, vec![128, 128, 64, 0]));
    }

    #[test]
    fn blank_detection() {
        assert!(is_blank(&[0, 0, 0, 0, 0, 0, 0, 0]));
        assert!(is_blank(&[]));
        assert!(!is_blank(&[255, 0, 0, 255, 255, 0, 0, 255]), "opaque flat icons are real icons");
        assert!(!is_blank(&[0, 0, 0, 0, 255, 0, 0, 255]));
    }

    #[test]
    fn scroll_mapping() {
        assert_eq!(Click::from_scroll(120, "vertical"), Some(Click::ScrollDown));
        assert_eq!(Click::from_scroll(-1, "Vertical"), Some(Click::ScrollUp));
        assert_eq!(Click::from_scroll(3, "horizontal"), Some(Click::ScrollRight));
        assert_eq!(Click::from_scroll(-3, "horizontal"), Some(Click::ScrollLeft));
        assert_eq!(Click::from_scroll(0, "vertical"), None);
        assert_eq!(Click::Secondary.button(), 3);
    }
}
