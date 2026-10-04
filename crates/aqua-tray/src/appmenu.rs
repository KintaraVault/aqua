//! Global application menus.
//!
//! Applications publish their menu bar as a `com.canonical.dbusmenu` object and tell the
//! desktop where it lives:
//! * X11 apps (Qt, Electron/Chromium, GTK with appmenu-gtk-module, LibreOffice …) call
//!   `com.canonical.AppMenu.Registrar.RegisterWindow(xid, path)` — we own that name;
//! * Wayland apps (Qt with the KDE platform theme) use the `org_kde_kwin_appmenu`
//!   protocol, handled by the compositor, which passes the address in directly.
//!
//! The compositor reports the focused window's address with [`set_active`]; a worker
//! thread fetches that menu and keeps it current. The shell reads [`current`] every frame
//! (lock + `Arc` clone, never blocks on D-Bus) and sends clicks back with [`clicked`].
use crate::menu::MenuNode;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::message::Header;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, Value};

const REGISTRAR_NAME: &str = "com.canonical.AppMenu.Registrar";
const REGISTRAR_PATH: &str = "/com/canonical/AppMenu/Registrar";
const MENU_IFACE: &str = "com.canonical.dbusmenu";

/// Where a menu lives: (bus name, object path).
pub type Address = (String, String);

/// The menu of the focused application.
#[derive(Clone, Debug, PartialEq)]
pub struct AppMenu {
    pub address: Address,
    /// Root node; its children are the menu-bar titles (File, Edit, …).
    pub root: MenuNode,
}

impl AppMenu {
    /// Top-level menus worth showing in the menu bar.
    pub fn titles(&self) -> Vec<&MenuNode> {
        top_level(&self.root)
    }
}

/// Visible, labelled top-level entries (separators and empty titles are skipped).
pub fn top_level(root: &MenuNode) -> Vec<&MenuNode> {
    root.children.iter().filter(|n| !n.separator && !n.label.trim().is_empty()).collect()
}

/// X11 window → menu address table kept by the registrar.
#[derive(Default, Debug)]
pub struct Registry {
    windows: HashMap<u32, Address>,
}

impl Registry {
    pub fn register(&mut self, xid: u32, bus: &str, path: &str) -> bool {
        if xid == 0 || bus.is_empty() || path.is_empty() || path == "/" {
            return false;
        }
        let a = (bus.to_string(), path.to_string());
        self.windows.insert(xid, a.clone()) != Some(a)
    }

    pub fn unregister(&mut self, xid: u32) -> bool {
        self.windows.remove(&xid).is_some()
    }

    /// Drop every window of a bus name that left the bus.
    pub fn drop_owner(&mut self, bus: &str) -> bool {
        let n = self.windows.len();
        self.windows.retain(|_, (b, _)| b != bus);
        n != self.windows.len()
    }

    pub fn get(&self, xid: u32) -> Option<&Address> {
        self.windows.get(&xid)
    }

    pub fn all(&self) -> Vec<(u32, Address)> {
        let mut v: Vec<_> = self.windows.iter().map(|(k, a)| (*k, a.clone())).collect();
        v.sort();
        v
    }
}

#[derive(Default)]
struct Shared {
    registry: Registry,
    /// Address the compositor asked for.
    wanted: Option<Address>,
    menu: Option<Arc<AppMenu>>,
    serial: u64,
}

fn shared() -> &'static Mutex<Shared> {
    static S: OnceLock<Mutex<Shared>> = OnceLock::new();
    S.get_or_init(Default::default)
}

fn lock() -> std::sync::MutexGuard<'static, Shared> {
    shared().lock().unwrap_or_else(|e| e.into_inner())
}

enum Cmd {
    Active(Option<Address>),
    Reload,
    Opened(i32),
    Clicked(i32),
    NameLost(String),
}

fn sender() -> &'static Mutex<Option<Sender<Cmd>>> {
    static TX: OnceLock<Mutex<Option<Sender<Cmd>>>> = OnceLock::new();
    TX.get_or_init(|| Mutex::new(None))
}

fn send(c: Cmd) {
    if let Some(tx) = sender().lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = tx.send(c);
    }
}

/// Start the registrar and the menu worker (idempotent).
pub fn start() {
    let mut guard = sender().lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_some() {
        return;
    }
    let (tx, rx) = channel();
    *guard = Some(tx.clone());
    drop(guard);
    std::thread::Builder::new()
        .name("aqua-appmenu".into())
        .spawn(move || {
            if let Err(e) = run(tx, rx) {
                tracing::warn!("global menu unavailable: {e}");
            }
        })
        .ok();
}

/// Menu address an X11 window registered.
pub fn for_x11_window(xid: u32) -> Option<Address> {
    lock().registry.get(xid).cloned()
}

/// The focused window's menu address changed (`None`: no global menu).
pub fn set_active(addr: Option<Address>) {
    let mut s = lock();
    if s.wanted == addr {
        return;
    }
    s.wanted = addr.clone();
    s.serial += 1;
    drop(s);
    send(Cmd::Active(addr));
}

/// The focused application's menu, once loaded.
pub fn current() -> Option<Arc<AppMenu>> {
    let s = lock();
    let m = s.menu.as_ref()?;
    (Some(&m.address) == s.wanted.as_ref()).then(|| m.clone())
}

/// Bumped whenever [`current`] may have changed.
pub fn serial() -> u64 {
    lock().serial
}

/// The submenu `id` is about to be shown (apps fill some menus lazily).
pub fn opened(id: i32) {
    send(Cmd::Opened(id));
}

/// A menu row was chosen.
pub fn clicked(id: i32) {
    send(Cmd::Clicked(id));
}

struct Registrar {
    tx: Mutex<Sender<Cmd>>,
}

#[zbus::interface(name = "com.canonical.AppMenu.Registrar")]
impl Registrar {
    fn register_window(&self, window_id: u32, menu_object_path: ObjectPath<'_>, #[zbus(header)] hdr: Header<'_>) {
        let bus = hdr.sender().map(|s| s.to_string()).unwrap_or_default();
        let changed = lock().registry.register(window_id, &bus, menu_object_path.as_str());
        tracing::debug!("appmenu: window {window_id:#x} → {bus}{menu_object_path}");
        if changed {
            self.notify();
        }
    }

    fn unregister_window(&self, window_id: u32) {
        if lock().registry.unregister(window_id) {
            self.notify();
        }
    }

    fn get_menu_for_window(&self, window_id: u32) -> zbus::fdo::Result<(String, OwnedObjectPath)> {
        let a = lock().registry.get(window_id).cloned();
        let (bus, path) = a.ok_or_else(|| zbus::fdo::Error::Failed("no menu for this window".into()))?;
        let path = OwnedObjectPath::try_from(path).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok((bus, path))
    }

    fn get_menus(&self) -> Vec<(u32, String, OwnedObjectPath)> {
        lock()
            .registry
            .all()
            .into_iter()
            .filter_map(|(x, (b, p))| Some((x, b, OwnedObjectPath::try_from(p).ok()?)))
            .collect()
    }
}

impl Registrar {
    fn notify(&self) {
        lock().serial += 1;
        let _ = self.tx.lock().unwrap_or_else(|e| e.into_inner()).send(Cmd::Reload);
    }
}

fn listen(conn: &Connection, rule: &str, tx: Sender<Cmd>, map: fn(&zbus::Message, &str) -> Option<Cmd>) {
    let it = match MessageIterator::for_match_rule(rule, conn, Some(256)) {
        Ok(it) => it,
        Err(e) => {
            tracing::warn!("appmenu: cannot subscribe to {rule}: {e}");
            return;
        }
    };
    std::thread::Builder::new()
        .name("aqua-appmenu-sig".into())
        .spawn(move || {
            for m in it.flatten() {
                let member = m.header().member().map(|m| m.to_string()).unwrap_or_default();
                if let Some(c) = map(&m, &member) {
                    if tx.send(c).is_err() {
                        break;
                    }
                }
            }
        })
        .ok();
}

fn run(tx: Sender<Cmd>, rx: Receiver<Cmd>) -> zbus::Result<()> {
    let conn = zbus::blocking::connection::Builder::session()?
        .method_timeout(Duration::from_secs(2))
        .serve_at(REGISTRAR_PATH, Registrar { tx: Mutex::new(tx.clone()) })?
        .build()?;
    match conn.request_name_with_flags(REGISTRAR_NAME, zbus::fdo::RequestNameFlags::DoNotQueue.into()) {
        Ok(zbus::fdo::RequestNameReply::PrimaryOwner | zbus::fdo::RequestNameReply::AlreadyOwner) => {
            tracing::info!("global menu: AppMenu registrar running")
        }
        _ => tracing::warn!("global menu: another AppMenu registrar owns the name; X11 menus go there"),
    }
    listen(&conn, &format!("type='signal',interface='{MENU_IFACE}'"), tx.clone(), |m, member| {
        matches!(member, "LayoutUpdated" | "ItemsPropertiesUpdated").then(|| {
            let _ = m;
            Cmd::Reload
        })
    });
    listen(
        &conn,
        "type='signal',sender='org.freedesktop.DBus',interface='org.freedesktop.DBus',member='NameOwnerChanged'",
        tx.clone(),
        |m, _| {
            let (name, _old, new): (String, String, String) = m.body().deserialize().ok()?;
            new.is_empty().then_some(Cmd::NameLost(name))
        },
    );
    drop(tx);

    let mut active: Option<Address> = None;
    while let Ok(first) = rx.recv() {
        // coalesce bursts (LayoutUpdated storms while an app builds its menus)
        std::thread::sleep(Duration::from_millis(25));
        let mut batch = vec![first];
        batch.extend(rx.try_iter());
        let mut reload = false;
        for c in batch {
            match c {
                Cmd::Active(a) => {
                    reload |= a != active;
                    active = a;
                }
                Cmd::Reload => reload = true,
                Cmd::NameLost(name) => {
                    lock().registry.drop_owner(&name);
                    if active.as_ref().is_some_and(|a| a.0 == name) {
                        reload = true;
                    }
                }
                Cmd::Opened(id) => {
                    if let Some(p) = active.as_ref().and_then(|a| proxy(&conn, a)) {
                        // `true`: the app changed the menu and it must be fetched again
                        reload |= p.call::<_, _, bool>("AboutToShow", &(id,)).unwrap_or(false);
                    }
                }
                Cmd::Clicked(id) => {
                    if let Some(p) = active.as_ref().and_then(|a| proxy(&conn, a)) {
                        let ts = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs() as u32)
                            .unwrap_or(0);
                        if let Err(e) = p.call_method("Event", &(id, "clicked", Value::from(0i32), ts)) {
                            tracing::debug!("appmenu click failed: {e}");
                        }
                    }
                }
            }
        }
        if !reload {
            continue;
        }
        let menu = active.as_ref().and_then(|a| {
            let root = load(&conn, a)?;
            Some(Arc::new(AppMenu { address: a.clone(), root }))
        });
        let mut s = lock();
        if s.menu.as_deref() != menu.as_deref() {
            s.menu = menu;
            s.serial += 1;
        }
    }
    Ok(())
}

fn proxy<'a>(conn: &'a Connection, a: &Address) -> Option<Proxy<'a>> {
    Proxy::new(conn, a.0.clone(), a.1.clone(), MENU_IFACE).ok()
}

/// Whole layout of the menu at `a`. Top-level menus get an `AboutToShow` first: Qt and
/// GTK fill submenus only when they are about to appear.
fn load(conn: &Connection, a: &Address) -> Option<MenuNode> {
    let p = proxy(conn, a)?;
    let get = || -> Option<MenuNode> {
        let reply = p.call_method("GetLayout", &(0i32, -1i32, Vec::<String>::new())).ok()?;
        let body = reply.body();
        let s: zbus::zvariant::Structure = body.deserialize().ok()?;
        crate::menu::parse(s.fields().get(1)?)
    };
    let root = get()?;
    let mut again = false;
    for n in top_level(&root) {
        if n.children.is_empty() {
            again |= p.call::<_, _, bool>("AboutToShow", &(n.id,)).unwrap_or(false);
        }
    }
    if again {
        get().or(Some(root))
    } else {
        Some(root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(id: i32, label: &str) -> MenuNode {
        MenuNode { id, label: label.into(), enabled: true, ..Default::default() }
    }

    #[test]
    fn registry_tracks_windows_per_owner() {
        let mut r = Registry::default();
        assert!(r.register(0x400003, ":1.7", "/MenuBar/1"));
        assert!(!r.register(0x400003, ":1.7", "/MenuBar/1"), "same registration is no change");
        assert!(r.register(0x600001, ":1.9", "/com/canonical/menu/600001"));
        assert!(!r.register(0, ":1.9", "/x"), "window 0 is invalid");
        assert!(!r.register(5, ":1.9", "/"), "root path means no menu");
        assert_eq!(r.get(0x400003), Some(&(":1.7".to_string(), "/MenuBar/1".to_string())));
        assert!(r.drop_owner(":1.7"));
        assert_eq!(r.get(0x400003), None);
        assert!(!r.drop_owner(":1.7"));
        assert_eq!(r.all().len(), 1);
        assert!(r.unregister(0x600001));
        assert!(!r.unregister(0x600001));
        assert!(r.all().is_empty());
    }

    #[test]
    fn titles_skip_separators_and_blank_labels() {
        let mut root = n(0, "");
        root.children = vec![n(1, "File"), MenuNode { separator: true, ..n(2, "") }, n(3, "  "), n(4, "Edit")];
        let m = AppMenu { address: (":1.1".into(), "/m".into()), root };
        let t: Vec<&str> = m.titles().iter().map(|n| n.label.as_str()).collect();
        assert_eq!(t, vec!["File", "Edit"]);
    }

    #[test]
    fn current_only_for_the_wanted_address() {
        // no worker running: set_active just records the request
        let a: Address = (":1.5".into(), "/MenuBar/1".into());
        let s0 = serial();
        set_active(Some(a.clone()));
        assert!(serial() > s0);
        lock().menu = Some(Arc::new(AppMenu { address: a.clone(), root: n(0, "") }));
        assert!(current().is_some());
        set_active(Some((":1.6".into(), "/MenuBar/1".into())));
        assert!(current().is_none(), "a stale menu of the previous app is never shown");
        set_active(None);
        assert!(current().is_none());
    }
}
