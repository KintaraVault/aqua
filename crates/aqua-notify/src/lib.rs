//! Notification daemon implementing the freedesktop.org Desktop Notifications spec
//! (`org.freedesktop.Notifications` on the session bus). Incoming notifications are
//! forwarded over a channel to the shell, which shows banners and the Notification Center.
pub mod portal;
pub mod portal_impl;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, OnceLock};
use zbus::zvariant::OwnedValue;

#[derive(Debug, Clone)]
pub struct Notification {
    pub id: u32,
    pub app_name: String,
    pub app_icon: String,
    pub summary: String,
    pub body: String,
    /// Milliseconds; -1 = server default, 0 = never.
    pub expire_timeout: i32,
    pub desktop_entry: Option<String>,
    /// Action invoked when the notification is clicked ("default" if offered).
    pub default_action: Option<String>,
    /// Extra actions shown as buttons: (key, label).
    pub actions: Vec<(String, String)>,
    /// Inline reply requested (`inline-reply` action): placeholder text for the field.
    pub reply: Option<String>,
    /// `resident` hint: stays (with its actions) after the banner hides.
    pub resident: bool,
    /// Urgency 2 (critical): the banner stays until dismissed.
    pub critical: bool,
}

#[derive(Debug, Clone)]
pub enum Event {
    Show(Box<Notification>),
    Close(u32),
}

/// Why a notification went away (`NotificationClosed` reason codes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    Expired = 1,
    Dismissed = 2,
    Requested = 3,
}

const PATH: &str = "/org/freedesktop/Notifications";
const IFACE: &str = "org.freedesktop.Notifications";
static CONN: OnceLock<zbus::blocking::Connection> = OnceLock::new();

/// Emit `NotificationClosed` for `id`.
pub fn closed(id: u32, reason: CloseReason) {
    if let Some(c) = CONN.get() {
        let _ = c.emit_signal(None::<&str>, PATH, IFACE, "NotificationClosed", &(id, reason as u32));
    }
}

/// Emit `ActionInvoked` for `id` followed by `NotificationClosed`.
pub fn invoke(id: u32, action: &str) {
    if let Some(c) = CONN.get() {
        let _ = c.emit_signal(None::<&str>, PATH, IFACE, "ActionInvoked", &(id, action));
    }
    closed(id, CloseReason::Dismissed);
}

/// Emit `NotificationReplied` (KDE inline-reply extension) followed by `NotificationClosed`.
pub fn replied(id: u32, text: &str) {
    if let Some(c) = CONN.get() {
        let _ = c.emit_signal(None::<&str>, PATH, IFACE, "NotificationReplied", &(id, text));
    }
    closed(id, CloseReason::Dismissed);
}

struct Server {
    tx: Sender<Event>,
    next: Arc<AtomicU32>,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Server {
    fn get_capabilities(&self) -> Vec<String> {
        ["actions", "body", "icon-static", "persistence", "inline-reply", "x-kde-display-appname"]
            .map(String::from)
            .to_vec()
    }

    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let id = if replaces_id != 0 { replaces_id } else { self.next.fetch_add(1, Ordering::Relaxed) };
        let desktop_entry = hints.get("desktop-entry").and_then(|v| String::try_from(v.clone()).ok());
        let body = strip_markup(&body);
        let default_action = default_action(&actions);
        let str_hint = |k: &str| hints.get(k).and_then(|v| String::try_from(v.clone()).ok());
        let bool_hint = |k: &str| hints.get(k).and_then(|v| bool::try_from(v).ok()).unwrap_or(false);
        let urgency = hints.get("urgency").and_then(|v| u8::try_from(v).ok()).unwrap_or(1);
        let has_reply = actions.iter().step_by(2).any(|k| k == "inline-reply");
        let reply = has_reply.then(|| str_hint("x-kde-reply-placeholder-text").unwrap_or_default());
        let _ = self.tx.send(Event::Show(Box::new(Notification {
            id,
            app_name,
            app_icon,
            summary,
            body,
            expire_timeout,
            desktop_entry,
            actions: button_actions(&actions, default_action.as_deref()),
            default_action,
            reply,
            resident: bool_hint("resident"),
            critical: urgency >= 2,
        })));
        id
    }

    fn close_notification(&self, id: u32) {
        let _ = self.tx.send(Event::Close(id));
        closed(id, CloseReason::Requested);
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        ("Aqua".into(), "Aqua".into(), env!("CARGO_PKG_VERSION").into(), "1.2".into())
    }
}

/// Action to invoke when a notification is clicked: "default" if offered (other actions are
/// shown as buttons; without "default" a click activates the app).
fn default_action(actions: &[String]) -> Option<String> {
    actions.iter().step_by(2).find(|k| k.as_str() == "default").cloned()
}

/// Actions shown as buttons: every (key, label) pair except the click action and inline reply.
fn button_actions(actions: &[String], default: Option<&str>) -> Vec<(String, String)> {
    actions
        .as_chunks::<2>()
        .0
        .iter()
        .filter(|p| Some(p[0].as_str()) != default && p[0] != "inline-reply" && !p[1].is_empty())
        .map(|p| (p[0].clone(), p[1].clone()))
        .take(3)
        .collect()
}

/// Very small markup stripper (spec allows <b>, <i>, <u>, <a>, <img>).
fn strip_markup(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'")
}

/// Start the daemon on a background thread.
/// Keep owning the well-known bus `name` on `conn`: report when someone else holds it and
/// claim it again whenever it has no owner (a queued request can get lost, e.g. after another
/// session on the same bus came and went — notifications and file dialogs then silently stop
/// working).
pub fn keep_name(conn: &zbus::blocking::Connection, name: &'static str) {
    let conn = conn.clone();
    let _ = std::thread::Builder::new().name("dbus-name".into()).spawn(move || {
        let Ok(p) = zbus::blocking::fdo::DBusProxy::new(&conn) else { return };
        let Ok(wk) = zbus::names::WellKnownName::try_from(name) else { return };
        let me = conn.unique_name().map(|n| n.to_string());
        let mut reported = false;
        loop {
            match p.get_name_owner(wk.clone().into()) {
                Ok(owner) => {
                    let owner = owner.to_string();
                    if Some(&owner) != me.as_ref() && !reported {
                        tracing::warn!("{name} is owned by {owner}, not this session; waiting for it");
                        reported = true;
                    } else if Some(&owner) == me.as_ref() {
                        reported = false;
                    }
                }
                Err(_) => match p.request_name(wk.clone(), Default::default()) {
                    Ok(r) => tracing::info!("claimed {name} on the session bus: {r:?}"),
                    Err(e) => tracing::warn!("cannot claim {name}: {e}"),
                },
            }
            std::thread::sleep(std::time::Duration::from_secs(10));
        }
    });
}

pub fn spawn() -> Receiver<Event> {
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("aqua-notify".into())
        .spawn(move || {
            let server = Server { tx, next: Arc::new(AtomicU32::new(1)) };
            let conn = zbus::blocking::connection::Builder::session()
                .and_then(|b| b.name("org.freedesktop.Notifications"))
                .and_then(|b| b.serve_at(PATH, server))
                .and_then(|b| b.build());
            match conn {
                Ok(c) => {
                    keep_name(&c, "org.freedesktop.Notifications");
                    let _ = CONN.set(c);
                    tracing::info!("notification server running on the session bus");
                    loop {
                        std::thread::park();
                    }
                }
                Err(e) => tracing::warn!("notification server unavailable: {e}"),
            }
        })
        .ok();
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_is_stripped_and_entities_decoded() {
        assert_eq!(strip_markup("<b>Bold</b> &amp; <i>it</i>"), "Bold & it");
        assert_eq!(strip_markup("a &lt;tag&gt; &quot;q&quot; &apos;s&apos;"), "a <tag> \"q\" 's'");
        assert_eq!(strip_markup("<a href=\"x\">link</a>"), "link");
        assert_eq!(strip_markup("plain"), "plain");
    }

    #[test]
    fn button_actions_skip_default_and_reply() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let a = v(&["default", "Open", "inline-reply", "Reply", "mark", "Mark read", "x", ""]);
        assert_eq!(button_actions(&a, Some("default")), vec![("mark".to_string(), "Mark read".to_string())]);
        let a = v(&["a", "A", "b", "B", "c", "C", "d", "D", "e", "E"]);
        assert_eq!(button_actions(&a, Some("a")).len(), 3);
        assert!(button_actions(&v(&["odd"]), None).is_empty());
    }

    #[test]
    fn default_action_selection() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(default_action(&v(&["reply", "Reply", "default", "Open"])).as_deref(), Some("default"));
        assert_eq!(default_action(&v(&["reply", "Reply", "mark", "Mark read"])), None);
        assert_eq!(default_action(&[]), None);
    }

    #[test]
    fn close_reason_codes_match_spec() {
        assert_eq!(CloseReason::Expired as u32, 1);
        assert_eq!(CloseReason::Dismissed as u32, 2);
        assert_eq!(CloseReason::Requested as u32, 3);
    }
}
