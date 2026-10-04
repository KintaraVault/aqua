//! System tray: a `org.kde.StatusNotifierWatcher` + host on the session bus.
//!
//! Applications register StatusNotifierItems (Telegram, Discord, Steam, nm-applet,
//! KeePassXC, …). A background thread tracks them, keeps their icons, tooltips and
//! dbusmenu layouts current and publishes an immutable [`Snapshot`] the shell reads
//! every frame without blocking. User interaction is sent back with the fire-and-forget
//! functions below.
//!
//! When another watcher already owns the name, Aqua registers as a host with it and
//! mirrors its items instead.
pub mod appmenu;
mod menu;
mod service;
mod value;
pub mod xembed;

use std::sync::{Arc, Mutex, OnceLock};

pub use menu::{shortcut_label, MenuNode, MenuToggle};

/// Premultiplied RGBA image.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Icon {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Status {
    Passive,
    #[default]
    Active,
    NeedsAttention,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrayItem {
    /// Stable identifier: bus name + object path.
    pub key: String,
    pub id: String,
    pub title: String,
    pub tooltip: String,
    pub status: Status,
    pub icon_name: String,
    pub icon_theme_path: String,
    pub pixmaps: Vec<Icon>,
    pub attention_icon_name: String,
    pub attention_pixmaps: Vec<Icon>,
    /// The item only provides a menu: a primary click opens it.
    pub item_is_menu: bool,
    /// Root of the dbusmenu tree (its children are the menu rows).
    pub menu: Option<MenuNode>,
}

impl TrayItem {
    /// Icon name and pixmaps for the current status.
    pub fn current_icon(&self) -> (&str, &[Icon]) {
        if self.status == Status::NeedsAttention
            && (!self.attention_icon_name.is_empty() || !self.attention_pixmaps.is_empty())
        {
            (&self.attention_icon_name, &self.attention_pixmaps)
        } else {
            (&self.icon_name, &self.pixmaps)
        }
    }

    pub fn has_menu(&self) -> bool {
        self.menu.as_ref().map(|m| !m.children.is_empty()).unwrap_or(false)
    }

    pub fn label(&self) -> &str {
        if !self.tooltip.is_empty() {
            &self.tooltip
        } else if !self.title.is_empty() {
            &self.title
        } else {
            &self.id
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub items: Vec<TrayItem>,
    /// Bumped on every change.
    pub serial: u64,
}

/// Notifications from the tray thread to the shell.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The item could not handle a primary activation: show its menu instead.
    OpenMenu(String),
}

pub(crate) struct Shared {
    pub snap: Arc<Snapshot>,
    pub events: Vec<Event>,
}

pub(crate) fn shared() -> &'static Mutex<Shared> {
    static S: OnceLock<Mutex<Shared>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Shared { snap: Arc::new(Snapshot::default()), events: vec![] }))
}

/// Start the tray service (idempotent).
pub fn start() {
    service::start();
}

/// Current items in registration order.
pub fn snapshot() -> Arc<Snapshot> {
    shared().lock().unwrap_or_else(|e| e.into_inner()).snap.clone()
}

/// Events produced since the last call.
pub fn take_events() -> Vec<Event> {
    std::mem::take(&mut shared().lock().unwrap_or_else(|e| e.into_inner()).events)
}

/// Primary click at screen position `(x, y)` (physical pixels).
pub fn activate(key: &str, x: i32, y: i32) {
    service::send(service::Cmd::Activate(key.into(), x, y));
}

/// Middle click.
pub fn secondary_activate(key: &str, x: i32, y: i32) {
    service::send(service::Cmd::SecondaryActivate(key.into(), x, y));
}

/// Secondary click on an item without a dbusmenu: the app shows its own menu.
pub fn context_menu(key: &str, x: i32, y: i32) {
    service::send(service::Cmd::ContextMenu(key.into(), x, y));
}

/// Scroll wheel over the item (`vertical` = vertical axis).
pub fn scroll(key: &str, delta: i32, vertical: bool) {
    service::send(service::Cmd::Scroll(key.into(), delta, vertical));
}

/// The menu of `key` is about to be shown (apps may update it lazily).
pub fn menu_opened(key: &str) {
    service::send(service::Cmd::MenuOpened(key.into()));
}

/// A menu row was chosen.
pub fn menu_clicked(key: &str, id: i32) {
    service::send(service::Cmd::MenuClicked(key.into(), id));
}
