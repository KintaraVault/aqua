//! Now Playing: the most relevant MPRIS player on the session bus
//! (`org.mpris.MediaPlayer2.*`) — Spotify, browsers, mpv, VLC, Rhythmbox …
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
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
    /// Cover art location (`mpris:artUrl`).
    pub art: String,
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
            m.art = md.get("mpris:artUrl").and_then(string_of).unwrap_or_default();
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
    call_with(player, method, None)
}

fn call_with(player: &str, method: &str, offset: Option<i64>) {
    let (player, method) = (player.to_string(), method.to_string());
    crate::spawn_cmd(move || {
        let player = if player.is_empty() { probe().player } else { player };
        if player.is_empty() {
            return;
        }
        if let Some(c) = session_bus() {
            if let Ok(p) = zbus::blocking::Proxy::new(
                &c,
                player.as_str(),
                "/org/mpris/MediaPlayer2",
                "org.mpris.MediaPlayer2.Player",
            ) {
                let _ = match offset {
                    Some(us) => p.call::<_, _, ()>(method.as_str(), &(us,)),
                    None => p.call::<_, _, ()>(method.as_str(), &()),
                };
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
pub fn play(player: &str) {
    call(player, "Play")
}
pub fn pause(player: &str) {
    call(player, "Pause")
}
pub fn stop(player: &str) {
    call(player, "Stop")
}
/// Relative seek in seconds (negative = rewind).
pub fn seek(player: &str, secs: f64) {
    call_with(player, "Seek", Some((secs * 1e6) as i64))
}

const ART_MAX: u64 = 16 << 20;

type ArtCache = Mutex<HashMap<String, Option<Arc<Vec<u8>>>>>;

fn art_cache() -> &'static ArtCache {
    static C: OnceLock<ArtCache> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// Encoded cover image for `url` (file:// or http(s)://). Loaded in the background on the
/// first request; `None` until it arrives (the system serial changes then).
pub fn art_bytes(url: &str) -> Option<Arc<Vec<u8>>> {
    if url.is_empty() {
        return None;
    }
    let mut c = art_cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = c.get(url) {
        return v.clone();
    }
    if c.len() > 24 {
        c.clear();
    }
    c.insert(url.to_string(), None);
    let url = url.to_string();
    std::thread::Builder::new()
        .name("aqua-art".into())
        .spawn(move || {
            let Some(bytes) = fetch_art(&url) else { return };
            art_cache().lock().unwrap_or_else(|e| e.into_inner()).insert(url, Some(Arc::new(bytes)));
            crate::patch(|_| {});
        })
        .ok();
    None
}

fn fetch_art(url: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    if let Some(path) = url.strip_prefix("file://") {
        let path = std::path::PathBuf::from(percent_decode(path.strip_prefix("localhost").unwrap_or(path)));
        use std::os::unix::fs::OpenOptionsExt;
        let f = std::fs::File::options().read(true).custom_flags(0o4000).open(&path).ok()?;
        if !f.metadata().ok()?.is_file() {
            return None;
        }
        let mut v = Vec::new();
        f.take(ART_MAX + 1).read_to_end(&mut v).ok()?;
        return (v.len() as u64 <= ART_MAX).then_some(v);
    }
    if url.starts_with("https://") || url.starts_with("http://") {
        let agent = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(8)).build();
        let resp = agent.get(url).call().ok()?;
        let mut v = Vec::new();
        resp.into_reader().take(ART_MAX + 1).read_to_end(&mut v).ok()?;
        return (v.len() as u64 <= ART_MAX).then_some(v);
    }
    None
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let hex = |c: u8| (c as char).to_digit(16);
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
