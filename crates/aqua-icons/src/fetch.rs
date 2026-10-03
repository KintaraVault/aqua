//! Background fetcher for vendor icon artwork.
//!
//! Source: the public iTunes Search API. The Mac App Store (`entity=macSoftware`)
//! has the real icon (transparent PNG rendition on the icon-grid). Apps that
//! aren't sold there (Firefox, Telegram, VLC, Spotify…) usually have an iOS build: its
//! full-bleed artwork is masked into the squircle by `normalize::fit_macos`.
//!
//! Matching is strict (exact name after normalising "Desktop", "for Mac", taglines…,
//! or the bundle id matching the desktop id) so unrelated apps never get a wrong icon.
//! Results are cached as PNG; misses are remembered for a month.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};

struct Job {
    id: String,
    terms: Vec<String>,
    /// Reverse-DNS desktop id (matches App Store bundle ids for many apps).
    bundle: Option<String>,
    /// Also try iOS artwork (not for branded counterparts: those must be the Mac icon).
    ios: bool,
    /// Publisher hint ("mozilla", "videolan"): must match the store's developer.
    vendor: Option<String>,
}

/// Publishers of popular apps whose desktop ids carry no reverse-DNS vendor.
const VENDORS: &[(&str, &str)] = &[
    ("firefox", "mozilla"),
    ("thunderbird", "mozilla"),
    ("code", "microsoft"),
    ("teams", "microsoft"),
    ("skype", "microsoft"),
    ("vlc", "videolan"),
    ("telegram", "telegram"),
    ("telegramdesktop", "telegram"),
    ("discord", "discord"),
    ("spotify", "spotify"),
    ("signal", "signal"),
    ("signaldesktop", "signal"),
    ("slack", "slack"),
    ("zoom", "zoom"),
    ("obsidian", "obsidian"),
    ("whatsapp", "whatsapp"),
    ("chrome", "google"),
    ("googlechrome", "google"),
    ("brave", "brave"),
    ("bravebrowser", "brave"),
    ("opera", "opera"),
    ("vivaldi", "vivaldi"),
    ("blender", "blender"),
    ("steam", "valve"),
    ("notion", "notion"),
    ("figma", "figma"),
    ("1password", "agilebits"),
    ("bitwarden", "bitwarden"),
    ("element", "element"),
    ("todoist", "doist"),
    ("dropbox", "dropbox"),
];

/// Does a store result come from the expected publisher?
fn vendor_matches(r: &serde_json::Value, vendor: &str) -> bool {
    let v = norm(vendor);
    if v.len() < 3 {
        return false;
    }
    let field = |k: &str| r.get(k).and_then(|x| x.as_str()).map(norm).unwrap_or_default();
    let artist = field("artistName");
    let seller = field("sellerName");
    let host = r
        .get("sellerUrl")
        .and_then(|x| x.as_str())
        .and_then(|u| u.split("//").nth(1))
        .and_then(|h| h.split('/').next())
        .unwrap_or("")
        .to_lowercase();
    let labels: Vec<&str> = host.split('.').collect();
    let domain = if labels.len() >= 2 { norm(labels[labels.len() - 2]) } else { String::new() };
    let bundle = r.get("bundleId").and_then(|x| x.as_str()).unwrap_or("").to_lowercase();
    let first = |s: &str| s.split_whitespace().next().map(norm).unwrap_or_default();
    let artist_first = r.get("artistName").and_then(|x| x.as_str()).map(first).unwrap_or_default();
    artist.contains(&v)
        || seller.contains(&v)
        || domain == v
        || bundle.split('.').take(2).any(|p| norm(p) == v)
        || (artist_first.len() >= 4 && v.starts_with(&artist_first))
}

pub struct Fetcher {
    tx: Sender<Job>,
    seen: Mutex<std::collections::HashSet<String>>,
}

const MISS_TTL: std::time::Duration = std::time::Duration::from_secs(30 * 24 * 3600);

impl Fetcher {
    pub fn spawn(cache: PathBuf, dirty: Arc<Mutex<bool>>) -> Self {
        let (tx, rx) = channel::<Job>();
        std::thread::Builder::new()
            .name("aqua-icon-fetch".into())
            .spawn(move || {
                for job in rx {
                    let file = cache.join(format!("{}.png", crate::sanitize(&job.id)));
                    let miss = cache.join(format!("{}.miss", crate::sanitize(&job.id)));
                    let fresh_miss = miss
                        .metadata()
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age < MISS_TTL);
                    if file.exists() || fresh_miss {
                        continue;
                    }
                    match fetch(&job) {
                        Ok(Some(bytes)) => {
                            let _ = std::fs::write(&file, bytes);
                            let _ = std::fs::remove_file(&miss);
                            *dirty.lock().unwrap() = true;
                        }
                        Ok(None) => {
                            let _ = std::fs::write(&miss, b"");
                        }
                        Err(()) => {}
                    }
                }
            })
            .ok();
        Self { tx, seen: Mutex::new(Default::default()) }
    }

    /// Mac icon of a branded app by exact store name ("Safari", "Pages").
    pub fn request(&self, id: &str, name: &str) {
        if self.seen.lock().unwrap().insert(id.to_string()) {
            let _ = self.tx.send(Job {
                id: id.into(),
                terms: vec![name.into()],
                bundle: None,
                ios: false,
                vendor: Some("apple".into()),
            });
        }
    }

    /// Icon of an installed app: tries its name, a cleaned-up name and its id.
    pub fn request_app(&self, id: &str, name: &str, icon: &str) {
        if !self.seen.lock().unwrap().insert(id.to_string()) {
            return;
        }
        let mut terms: Vec<String> = vec![];
        let mut add = |t: String| {
            let t = t.trim().to_string();
            if t.chars().filter(|c| c.is_alphanumeric()).count() >= 3 && !terms.iter().any(|o| norm(o) == norm(&t)) {
                terms.push(t);
            }
        };
        add(name.to_string());
        add(clean_name(name));
        let parts: Vec<&str> = id.split('.').collect();
        if parts.len() >= 3 {
            let generic = ["desktop", "client", "app", "application", "gui", "linux", "flatpak"];
            if let Some(p) = parts.iter().rev().find(|p| !generic.contains(&p.to_lowercase().as_str())) {
                add(p.to_string());
            }
        } else if !icon.contains('/') && !icon.is_empty() {
            add(icon.replace(['-', '_'], " "));
        }
        terms.truncate(3);
        let bundle = (parts.len() >= 3).then(|| id.to_lowercase());
        let vendor = if parts.len() >= 3 {
            Some(parts[1].to_lowercase())
        } else {
            let key = norm(id);
            let key2 = norm(&clean_name(name));
            VENDORS.iter().find(|(k, _)| *k == key || *k == key2).map(|(_, v)| v.to_string())
        };
        let _ = self.tx.send(Job { id: id.into(), terms, bundle, ios: true, vendor });
    }
}

fn norm(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Strip Linux packaging noise: "Telegram Desktop" → "Telegram", "Firefox Web Browser"
/// → "Firefox", "GIMP (Flatpak)" → "GIMP".
fn clean_name(name: &str) -> String {
    let mut s = name.to_string();
    if let Some(i) = s.find(['(', '[']) {
        s.truncate(i);
    }
    let noise = ["web browser", "browser", "desktop", "for linux", "linux", "client", "editor", "app"];
    let lower = s.to_lowercase();
    for n in noise {
        if lower.ends_with(&format!(" {n}")) {
            s.truncate(s.len() - n.len() - 1);
            break;
        }
    }
    s.trim().to_string()
}

/// Score a store result against what we look for (0 = reject).
fn score(r: &serde_json::Value, want: &str, bundle: Option<&str>, vendor_ok: bool) -> i32 {
    let Some(track) = r.get("trackName").and_then(|t| t.as_str()) else { return 0 };
    let apple = r.get("artistName").and_then(|a| a.as_str()).is_some_and(|a| a == "Apple");
    if let (Some(b), Some(rb)) = (bundle, r.get("bundleId").and_then(|b| b.as_str())) {
        if rb.to_lowercase() == b {
            return 10;
        }
    }
    let t = norm(track);
    let raw = track.to_lowercase();
    let head = norm(raw.split([':', '-', '–', '—', '|']).next().unwrap_or(""));
    let stripped = norm(&clean_name(&raw.replace(" for mac", "").replace(" for macos", "")));
    if GENERIC.contains(&want) && !apple {
        return 0;
    }
    if t == want {
        4 + apple as i32
    } else if head == want || stripped == want {
        3 + apple as i32
    } else if vendor_ok && want.len() >= 4 && t.starts_with(want) {
        2
    } else {
        0
    }
}

const GENERIC: &[&str] = &[
    "files",
    "terminal",
    "calculator",
    "clocks",
    "clock",
    "weather",
    "maps",
    "disks",
    "texteditor",
    "settings",
    "systemmonitor",
    "calendar",
    "contacts",
    "photos",
    "music",
    "videos",
    "video",
    "camera",
    "notes",
    "characters",
    "fonts",
    "logs",
    "help",
    "software",
    "boxes",
    "console",
    "documentviewer",
    "imageviewer",
    "archivemanager",
    "screenshot",
    "web",
    "mail",
    "tasks",
    "browser",
    "editor",
    "player",
    "viewer",
    "monitor",
    "scanner",
    "recorder",
    "soundrecorder",
    "passwords",
    "connections",
    "extensions",
    "tour",
    "about",
    "files",
    "manager",
    "reader",
    "chat",
    "messages",
    "phone",
    "news",
];

/// One store search. Err = network trouble or rate limiting (don't remember a miss).
fn search(agent: &ureq::Agent, term: &str, entity: &str) -> Result<Vec<serde_json::Value>, ()> {
    static LAST: Mutex<Option<std::time::Instant>> = Mutex::new(None);
    {
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = *last {
            let gap = std::time::Duration::from_millis(3100);
            if t.elapsed() < gap {
                std::thread::sleep(gap - t.elapsed());
            }
        }
        *last = Some(std::time::Instant::now());
    }
    let resp: serde_json::Value = agent
        .get("https://itunes.apple.com/search")
        .query("term", term)
        .query("entity", entity)
        .query("country", "us")
        .query("limit", "10")
        .call()
        .map_err(|_| ())?
        .into_json()
        .map_err(|_| ())?;
    Ok(resp.get("results").and_then(|v| v.as_array()).cloned().unwrap_or_default())
}

/// Ok(None) = definitely not in the stores; Err = try again another time.
fn fetch(job: &Job) -> Result<Option<Vec<u8>>, ()> {
    let agent = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(8)).build();
    let entities: &[&str] =
        if job.ios && job.vendor.is_some() { &["macSoftware", "software"] } else { &["macSoftware"] };
    for entity in entities {
        for term in &job.terms {
            let want = norm(term);
            let results = search(&agent, term, entity)?;
            let mut best: Option<(i32, &serde_json::Value)> = None;
            for r in &results {
                let vendor_ok = job.vendor.as_deref().is_some_and(|v| vendor_matches(r, v));
                if job.vendor.is_some() && !vendor_ok || job.vendor.is_none() && *entity != "macSoftware" {
                    continue;
                }
                let sc = score(r, &want, job.bundle.as_deref(), vendor_ok);
                if sc > 0 && best.is_none_or(|(b, _)| sc > b) {
                    best = Some((sc, r));
                }
            }
            if let Some((_, r)) = best {
                if let Some(bytes) = download(&agent, r) {
                    return Ok(Some(bytes));
                }
            }
        }
    }
    Ok(None)
}

/// The 1024 px PNG rendition of a result's artwork (keeps the Mac icon's alpha).
fn download(agent: &ureq::Agent, r: &serde_json::Value) -> Option<Vec<u8>> {
    let url = r.get("artworkUrl512").or_else(|| r.get("artworkUrl100"))?.as_str()?;
    let url = match url.rfind('/') {
        Some(i) => format!("{}/1024x1024bb.png", &url[..i]),
        None => url.to_string(),
    };
    let mut bytes = Vec::new();
    agent.get(&url).call().ok()?.into_reader().read_to_end(&mut bytes).ok()?;
    (bytes.len() > 1000).then_some(bytes)
}

use std::io::Read;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cleans_names() {
        assert_eq!(clean_name("Telegram Desktop"), "Telegram");
        assert_eq!(clean_name("Firefox Web Browser"), "Firefox");
        assert_eq!(clean_name("GIMP (Flatpak)"), "GIMP");
        assert_eq!(clean_name("Visual Studio Code"), "Visual Studio Code");
    }
}
