//! Watcher / host service and the worker thread that owns all item state.
use crate::value::{self, Props};
use crate::{shared, Event, Snapshot, Status, TrayItem};
use std::collections::HashSet;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::fdo::RequestNameFlags;
use zbus::message::Header;
use zbus::zvariant::{OwnedValue, Value};

const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM_IFACE: &str = "org.kde.StatusNotifierItem";
const MENU_IFACE: &str = "com.canonical.dbusmenu";
const DEFAULT_ITEM_PATH: &str = "/StatusNotifierItem";

pub enum Cmd {
    Register { bus: String, path: String },
    Unregister(String),
    NameLost(String),
    Signal { sender: String, path: String, iface: String, member: String },
    Activate(String, i32, i32),
    SecondaryActivate(String, i32, i32),
    ContextMenu(String, i32, i32),
    Scroll(String, i32, bool),
    MenuOpened(String),
    MenuClicked(String, i32),
}

fn sender() -> &'static Mutex<Option<Sender<Cmd>>> {
    static TX: OnceLock<Mutex<Option<Sender<Cmd>>>> = OnceLock::new();
    TX.get_or_init(|| Mutex::new(None))
}

pub fn send(c: Cmd) {
    if let Some(tx) = sender().lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = tx.send(c);
    }
}

pub fn start() {
    let mut guard = sender().lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_some() {
        return;
    }
    let (tx, rx) = channel();
    *guard = Some(tx.clone());
    drop(guard);
    std::thread::Builder::new()
        .name("aqua-tray".into())
        .spawn(move || {
            if let Err(e) = run(tx, rx) {
                tracing::warn!("system tray unavailable: {e}");
            }
        })
        .ok();
}

/// Registered item keys, shared with the watcher interface for its properties.
type Keys = Arc<Mutex<Vec<String>>>;

struct Watcher {
    tx: Mutex<Sender<Cmd>>,
    keys: Keys,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    fn register_status_notifier_item(&self, service: &str, #[zbus(header)] hdr: Header<'_>) {
        let caller = hdr.sender().map(|s| s.to_string()).unwrap_or_default();
        let (bus, path) = if service.starts_with('/') {
            (caller, service.to_string())
        } else {
            (service.to_string(), DEFAULT_ITEM_PATH.to_string())
        };
        if !bus.is_empty() {
            let _ = self.tx.lock().unwrap_or_else(|e| e.into_inner()).send(Cmd::Register { bus, path });
        }
    }

    fn register_status_notifier_host(&self, _service: &str) {}

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.keys.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }
}

/// Split a watcher key ("bus/path" or a bare bus name) into bus and object path.
fn split_key(s: &str) -> (String, String) {
    match s.find('/') {
        Some(i) => (s[..i].to_string(), s[i..].to_string()),
        None => (s.to_string(), DEFAULT_ITEM_PATH.to_string()),
    }
}

/// Forward every message matching `rule` to the worker.
fn listen(conn: &Connection, rule: &str, tx: Sender<Cmd>, map: fn(&zbus::Message) -> Option<Cmd>) {
    let it = match MessageIterator::for_match_rule(rule, conn, Some(256)) {
        Ok(it) => it,
        Err(e) => {
            tracing::warn!("tray: cannot subscribe to {rule}: {e}");
            return;
        }
    };
    std::thread::Builder::new()
        .name("aqua-tray-sig".into())
        .spawn(move || {
            for m in it.flatten() {
                if let Some(c) = map(&m) {
                    if tx.send(c).is_err() {
                        break;
                    }
                }
            }
        })
        .ok();
}

fn signal_cmd(m: &zbus::Message) -> Option<Cmd> {
    let h = m.header();
    Some(Cmd::Signal {
        sender: h.sender()?.to_string(),
        path: h.path()?.to_string(),
        iface: h.interface()?.to_string(),
        member: h.member()?.to_string(),
    })
}

fn owner_changed(m: &zbus::Message) -> Option<Cmd> {
    let (name, _old, new): (String, String, String) = m.body().deserialize().ok()?;
    new.is_empty().then_some(Cmd::NameLost(name))
}

fn watcher_signal(m: &zbus::Message) -> Option<Cmd> {
    let member = m.header().member()?.to_string();
    let (key,): (String,) = m.body().deserialize().ok()?;
    let (bus, path) = split_key(&key);
    match member.as_str() {
        "StatusNotifierItemRegistered" => Some(Cmd::Register { bus, path }),
        "StatusNotifierItemUnregistered" => Some(Cmd::Unregister(format!("{bus}{path}"))),
        _ => None,
    }
}

struct Entry {
    item: TrayItem,
    bus: String,
    path: String,
    /// Unique connection name of the process behind `bus`.
    owner: String,
    menu_path: String,
}

struct Worker {
    conn: Connection,
    /// We own the watcher name (otherwise we mirror another watcher).
    watcher: bool,
    keys: Keys,
    entries: Vec<Entry>,
    serial: u64,
}

fn run(tx: Sender<Cmd>, rx: Receiver<Cmd>) -> zbus::Result<()> {
    let keys: Keys = Arc::new(Mutex::new(vec![]));
    let conn = zbus::blocking::connection::Builder::session()?
        .method_timeout(Duration::from_secs(3))
        .serve_at(WATCHER_PATH, Watcher { tx: Mutex::new(tx.clone()), keys: keys.clone() })?
        .build()?;
    let host = format!("org.kde.StatusNotifierHost-{}", std::process::id());
    let _ = conn.request_name(host.as_str());
    let watcher = matches!(
        conn.request_name_with_flags(WATCHER_NAME, RequestNameFlags::DoNotQueue.into()),
        Ok(zbus::fdo::RequestNameReply::PrimaryOwner) | Ok(zbus::fdo::RequestNameReply::AlreadyOwner)
    );

    listen(&conn, &format!("type='signal',interface='{ITEM_IFACE}'"), tx.clone(), signal_cmd);
    listen(&conn, &format!("type='signal',interface='{MENU_IFACE}'"), tx.clone(), signal_cmd);
    listen(
        &conn,
        "type='signal',sender='org.freedesktop.DBus',interface='org.freedesktop.DBus',member='NameOwnerChanged'",
        tx.clone(),
        owner_changed,
    );

    if watcher {
        tracing::info!("system tray: StatusNotifierWatcher running");
        let _ = conn.emit_signal(None::<&str>, WATCHER_PATH, WATCHER_NAME, "StatusNotifierHostRegistered", &());
    } else {
        tracing::info!("system tray: another watcher is running, registering as host");
        listen(
            &conn,
            &format!("type='signal',sender='{WATCHER_NAME}',interface='{WATCHER_NAME}'"),
            tx.clone(),
            watcher_signal,
        );
        if let Ok(p) = Proxy::new(&conn, WATCHER_NAME, WATCHER_PATH, WATCHER_NAME) {
            let _ = p.call_method("RegisterStatusNotifierHost", &(host.as_str(),));
            let existing: Vec<String> = p.get_property("RegisteredStatusNotifierItems").unwrap_or_default();
            for k in existing {
                let (bus, path) = split_key(&k);
                let _ = tx.send(Cmd::Register { bus, path });
            }
        }
    }
    drop(tx);

    let mut w = Worker { conn, watcher, keys, entries: vec![], serial: 0 };
    while let Ok(first) = rx.recv() {
        std::thread::sleep(Duration::from_millis(30));
        let mut batch = vec![first];
        batch.extend(rx.try_iter());
        let mut refresh: HashSet<usize> = HashSet::new();
        let mut refresh_menu: HashSet<usize> = HashSet::new();
        let mut changed = false;
        for c in batch {
            match c {
                Cmd::Register { bus, path } => changed |= w.register(bus, path),
                Cmd::Unregister(key) => changed |= w.remove(|e| e.item.key == key),
                Cmd::NameLost(name) => changed |= w.remove(|e| e.owner == name || e.bus == name),
                Cmd::Signal { sender, path, iface, member } => {
                    for (i, e) in w.entries.iter().enumerate() {
                        if e.owner != sender {
                            continue;
                        }
                        if iface == ITEM_IFACE && e.path == path {
                            refresh.insert(i);
                            if member == "NewMenu" {
                                refresh_menu.insert(i);
                            }
                        } else if iface == MENU_IFACE
                            && e.menu_path == path
                            && (member == "LayoutUpdated" || member == "ItemsPropertiesUpdated")
                        {
                            refresh_menu.insert(i);
                        }
                    }
                }
                Cmd::Activate(key, x, y) => w.activate(&key, x, y),
                Cmd::SecondaryActivate(key, x, y) => w.call_item(&key, "SecondaryActivate", &(x, y)),
                Cmd::ContextMenu(key, x, y) => w.call_item(&key, "ContextMenu", &(x, y)),
                Cmd::Scroll(key, delta, vertical) => {
                    w.call_item(&key, "Scroll", &(delta, if vertical { "vertical" } else { "horizontal" }))
                }
                Cmd::MenuOpened(key) => {
                    if let Some(i) = w.index(&key) {
                        if w.about_to_show(i) {
                            refresh_menu.insert(i);
                        }
                    }
                }
                Cmd::MenuClicked(key, id) => w.menu_event(&key, id),
            }
        }
        for i in refresh {
            if i < w.entries.len() {
                changed |= w.load_props(i);
            }
        }
        for i in refresh_menu {
            if i < w.entries.len() {
                changed |= w.load_menu(i);
            }
        }
        if changed {
            w.publish();
        }
    }
    Ok(())
}

impl Worker {
    fn index(&self, key: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.item.key == key)
    }

    fn register(&mut self, bus: String, path: String) -> bool {
        let key = format!("{bus}{path}");
        if self.index(&key).is_some() {
            return false;
        }
        let owner = if bus.starts_with(':') {
            bus.clone()
        } else {
            let owner = zbus::blocking::fdo::DBusProxy::new(&self.conn)
                .ok()
                .and_then(|p| p.get_name_owner(zbus::names::BusName::try_from(bus.as_str()).ok()?).ok());
            match owner {
                Some(o) => o.to_string(),
                None => return false,
            }
        };
        tracing::info!("tray item registered: {key}");
        self.entries.push(Entry {
            item: TrayItem { key: key.clone(), ..Default::default() },
            bus,
            path,
            owner,
            menu_path: String::new(),
        });
        let i = self.entries.len() - 1;
        if !self.load_props(i) {
            self.entries.pop();
            return false;
        }
        self.load_menu(i);
        if self.watcher {
            let _ = self.conn.emit_signal(
                None::<&str>,
                WATCHER_PATH,
                WATCHER_NAME,
                "StatusNotifierItemRegistered",
                &(key.as_str(),),
            );
        }
        true
    }

    fn remove(&mut self, pred: impl Fn(&Entry) -> bool) -> bool {
        let gone: Vec<String> = self.entries.iter().filter(|e| pred(e)).map(|e| e.item.key.clone()).collect();
        if gone.is_empty() {
            return false;
        }
        self.entries.retain(|e| !pred(e));
        for key in gone {
            tracing::info!("tray item removed: {key}");
            if self.watcher {
                let _ = self.conn.emit_signal(
                    None::<&str>,
                    WATCHER_PATH,
                    WATCHER_NAME,
                    "StatusNotifierItemUnregistered",
                    &(key.as_str(),),
                );
            }
        }
        true
    }

    /// Re-read the item properties. `false` when the item is unreachable.
    fn load_props(&mut self, i: usize) -> bool {
        let e = &self.entries[i];
        let props: Props =
            match Proxy::new(&self.conn, e.bus.as_str(), e.path.as_str(), "org.freedesktop.DBus.Properties")
                .and_then(|p| p.call("GetAll", &(ITEM_IFACE,)))
            {
                Ok(p) => p,
                Err(err) => {
                    tracing::debug!("tray item {} unreachable: {err}", e.item.key);
                    return false;
                }
            };
        let get = |k: &str| props.get(k).map(|v: &OwnedValue| -> &Value { v });
        let item = TrayItem {
            key: e.item.key.clone(),
            id: value::prop_str(&props, "Id"),
            title: value::prop_str(&props, "Title"),
            tooltip: get("ToolTip").map(value::tooltip).unwrap_or_default(),
            status: match value::prop_str(&props, "Status").as_str() {
                "Passive" => Status::Passive,
                "NeedsAttention" => Status::NeedsAttention,
                _ => Status::Active,
            },
            icon_name: value::prop_str(&props, "IconName"),
            icon_theme_path: value::prop_str(&props, "IconThemePath"),
            pixmaps: get("IconPixmap").map(value::pixmaps).unwrap_or_default(),
            attention_icon_name: value::prop_str(&props, "AttentionIconName"),
            attention_pixmaps: get("AttentionIconPixmap").map(value::pixmaps).unwrap_or_default(),
            item_is_menu: value::prop_bool(&props, "ItemIsMenu"),
            menu: None,
        };
        let menu_path = value::prop_str(&props, "Menu");
        let e = &mut self.entries[i];
        let menu_changed = menu_path != e.menu_path;
        e.menu_path = menu_path;
        if menu_changed {
            self.load_menu(i);
        }
        let e = &mut self.entries[i];
        let menu = e.item.menu.take();
        e.item = TrayItem { menu, ..item };
        true
    }

    fn menu_proxy(&self, i: usize) -> Option<Proxy<'_>> {
        let e = &self.entries[i];
        if e.menu_path.is_empty() || e.menu_path == "/" {
            return None;
        }
        Proxy::new(&self.conn, e.bus.clone(), e.menu_path.clone(), MENU_IFACE).ok()
    }

    fn load_menu(&mut self, i: usize) -> bool {
        let menu = self.menu_proxy(i).and_then(|p| {
            let reply = p.call_method("GetLayout", &(0i32, -1i32, Vec::<String>::new())).ok()?;
            let body = reply.body();
            let s: zbus::zvariant::Structure = body.deserialize().ok()?;
            crate::menu::parse(s.fields().get(1)?)
        });
        let e = &mut self.entries[i];
        if e.item.menu == menu {
            return false;
        }
        e.item.menu = menu;
        true
    }

    /// `true` when the app updated the menu and it must be fetched again.
    fn about_to_show(&self, i: usize) -> bool {
        self.menu_proxy(i).and_then(|p| p.call::<_, _, bool>("AboutToShow", &(0i32,)).ok()).unwrap_or(false)
    }

    fn menu_event(&self, key: &str, id: i32) {
        let Some(i) = self.index(key) else { return };
        let Some(p) = self.menu_proxy(i) else { return };
        let ts =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(0);
        if let Err(e) = p.call_method("Event", &(id, "clicked", Value::from(0i32), ts)) {
            tracing::debug!("tray menu event failed: {e}");
        }
    }

    fn item_proxy(&self, key: &str) -> Option<Proxy<'_>> {
        let e = &self.entries[self.index(key)?];
        Proxy::new(&self.conn, e.bus.clone(), e.path.clone(), ITEM_IFACE).ok()
    }

    fn call_item<B: serde::Serialize + zbus::zvariant::DynamicType>(&self, key: &str, method: &str, body: &B) {
        if let Some(p) = self.item_proxy(key) {
            if let Err(e) = p.call_method(method, body) {
                tracing::debug!("tray {method} failed: {e}");
            }
        }
    }

    fn activate(&self, key: &str, x: i32, y: i32) {
        let Some(p) = self.item_proxy(key) else { return };
        if p.call_method("Activate", &(x, y)).is_err() {
            let has_menu = self.index(key).map(|i| self.entries[i].item.has_menu()).unwrap_or(false);
            if has_menu {
                shared().lock().unwrap_or_else(|e| e.into_inner()).events.push(Event::OpenMenu(key.to_string()));
            } else {
                let _ = p.call_method("ContextMenu", &(x, y));
            }
        }
    }

    fn publish(&mut self) {
        self.serial += 1;
        let items: Vec<TrayItem> = self.entries.iter().map(|e| e.item.clone()).collect();
        *self.keys.lock().unwrap_or_else(|e| e.into_inner()) = items.iter().map(|i| i.key.clone()).collect();
        shared().lock().unwrap_or_else(|e| e.into_inner()).snap = Arc::new(Snapshot { items, serial: self.serial });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watcher_keys() {
        assert_eq!(
            split_key(":1.42/org/ayatana/NotificationItem/app"),
            (":1.42".into(), "/org/ayatana/NotificationItem/app".into())
        );
        assert_eq!(
            split_key("org.kde.StatusNotifierItem-1-1"),
            ("org.kde.StatusNotifierItem-1-1".into(), DEFAULT_ITEM_PATH.into())
        );
    }
}
