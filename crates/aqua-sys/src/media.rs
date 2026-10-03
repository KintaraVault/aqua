//! Now Playing: the most relevant MPRIS player on the session bus
//! (`org.mpris.MediaPlayer2.*`) — Spotify, browsers, mpv, VLC, Rhythmbox …
use std::collections::HashMap;
use std::sync::OnceLock;
use zbus::zvariant::OwnedValue;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Media {
    /// Bus name of the player, empty = nothing to control.
    pub player: String,
    /// Human player name ("Spotify", "Firefox" …).
    pub app: String,
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub can_next: bool,
    pub can_prev: bool,
}

fn session_bus() -> Option<zbus::blocking::Connection> {
    static C: OnceLock<Option<zbus::blocking::Connection>> = OnceLock::new();
    C.get_or_init(|| zbus::blocking::Connection::session().ok()).clone()
}

fn props(c: &zbus::blocking::Connection, name: &str) -> Option<HashMap<String, OwnedValue>> {
    let p = zbus::blocking::Proxy::new(c, name, "/org/mpris/MediaPlayer2", "org.freedesktop.DBus.Properties").ok()?;
    p.call("GetAll", &("org.mpris.MediaPlayer2.Player",)).ok()
}

fn string_of(v: &OwnedValue) -> Option<String> {
    String::try_from(v.clone()).ok()
}

pub fn probe() -> Media {
    let Some(c) = session_bus() else { return Media::default() };
    let Ok(dbus) = zbus::blocking::fdo::DBusProxy::new(&c) else { return Media::default() };
    let Ok(names) = dbus.list_names() else { return Media::default() };
    let mut best: Option<(u8, Media)> = None;
    for n in names.iter().map(|n| n.to_string()).filter(|n| n.starts_with("org.mpris.MediaPlayer2.")) {
        let Some(p) = props(&c, &n) else { continue };
        let status = p.get("PlaybackStatus").and_then(string_of).unwrap_or_default();
        let mut m = Media { player: n.clone(), ..Default::default() };
        m.playing = status == "Playing";
        m.can_next = p.get("CanGoNext").and_then(|v| bool::try_from(v.clone()).ok()).unwrap_or(false);
        m.can_prev = p.get("CanGoPrevious").and_then(|v| bool::try_from(v.clone()).ok()).unwrap_or(false);
        if let Some(md) = p.get("Metadata").and_then(|v| HashMap::<String, OwnedValue>::try_from(v.clone()).ok()) {
            m.title = md.get("xesam:title").and_then(string_of).unwrap_or_default();
            m.artist = md
                .get("xesam:artist")
                .and_then(|v| Vec::<String>::try_from(v.clone()).ok())
                .map(|a| a.join(", "))
                .or_else(|| md.get("xesam:artist").and_then(string_of))
                .unwrap_or_default();
        }
        let base = n.trim_start_matches("org.mpris.MediaPlayer2.").split('.').next().unwrap_or("").to_string();
        let mut app = base.replace(['-', '_'], " ");
        if let Some(f) = app.get(0..1) {
            app = f.to_uppercase() + &app[1..];
        }
        m.app = app;
        let rank = match status.as_str() {
            "Playing" => 3,
            "Paused" => 2,
            _ => 1,
        } + (!m.title.is_empty()) as u8 * 3;
        if best.as_ref().map(|(r, _)| rank > *r).unwrap_or(true) {
            best = Some((rank, m));
        }
    }
    best.map(|b| b.1).unwrap_or_default()
}

fn call(player: &str, method: &str) {
    let (player, method) = (player.to_string(), method.to_string());
    crate::spawn_cmd(move || {
        if let Some(c) = session_bus() {
            if let Ok(p) = zbus::blocking::Proxy::new(
                &c,
                player.as_str(),
                "/org/mpris/MediaPlayer2",
                "org.mpris.MediaPlayer2.Player",
            ) {
                let _ = p.call::<_, _, ()>(method.as_str(), &());
            }
        }
    });
}

pub fn play_pause(player: &str) {
    call(player, "PlayPause")
}
pub fn next(player: &str) {
    call(player, "Next")
}
pub fn previous(player: &str) {
    call(player, "Previous")
}
