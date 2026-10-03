//! Clipboard history (⇧⌘V): every new clipboard selection — from Wayland or X11
//! clients — is read once in the background and kept in memory (never on disk).
//! Choosing an entry makes Aqua the selection owner and serves the stored bytes to
//! Wayland and X11 clients alike. Password managers that tag their selection with
//! `x-kde-passwordManagerHint: secret` are skipped.
use crate::state::Aqua;
use aqua_shell::clipboard::{ClipItem, ClipKind};
use smithay::wayland::selection::data_device::{request_data_device_client_selection, set_data_device_selection};
use std::io::{Read, Write};
use std::os::unix::io::{FromRawFd, OwnedFd};
use std::sync::{mpsc, Arc};

const MAX_ENTRIES: usize = 60;
const MAX_BYTES: usize = 24 * 1024 * 1024;

#[derive(Debug)]
pub struct Entry {
    pub id: u64,
    /// (mime type, data) — the first is the preferred representation.
    pub data: Vec<(String, Arc<Vec<u8>>)>,
}

/// Selection user data: who owns the current selection.
#[derive(Debug, Clone)]
pub enum SelData {
    /// X11 client (served through the XWayland WM).
    Xwm,
    /// A history entry re-published by Aqua.
    History(Arc<Entry>),
}

pub struct History {
    pub entries: Vec<Arc<Entry>>,
    /// Mime types of a Wayland selection that was just set.
    pub pending: Option<Vec<String>>,
    next: u64,
    tx: mpsc::Sender<(Vec<String>, Vec<(String, Vec<u8>)>)>,
    rx: mpsc::Receiver<(Vec<String>, Vec<(String, Vec<u8>)>)>,
}

impl History {
    pub fn next_id(&mut self) -> u64 {
        let id = self.next;
        self.next += 1;
        id
    }
}

impl Default for History {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { entries: vec![], pending: None, next: 1, tx, rx }
    }
}

fn pipe() -> Option<(OwnedFd, OwnedFd)> {
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return None;
    }
    Some(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// Which representations we keep, in order of preference.
fn wanted(mimes: &[String]) -> Vec<String> {
    let order = [
        "image/png",
        "image/jpeg",
        "text/uri-list",
        "x-special/gnome-copied-files",
        "text/html",
        "text/plain;charset=utf-8",
        "UTF8_STRING",
        "text/plain",
        "STRING",
        "TEXT",
    ];
    let mut v: Vec<String> = order.iter().filter(|m| mimes.iter().any(|x| x == *m)).map(|s| s.to_string()).collect();
    if v.is_empty() {
        if let Some(img) = mimes.iter().find(|m| m.starts_with("image/")) {
            v.push(img.clone());
        }
    }
    v.truncate(4);
    v
}

fn read_all(fd: OwnedFd) -> Vec<u8> {
    let mut f = std::fs::File::from(fd);
    let mut buf = Vec::new();
    let mut chunk = [0u8; 65536];
    let start = std::time::Instant::now();
    loop {
        let mut p = libc::pollfd { fd: std::os::unix::io::AsRawFd::as_raw_fd(&f), events: libc::POLLIN, revents: 0 };
        let left = 3000i64 - start.elapsed().as_millis() as i64;
        if left <= 0 || unsafe { libc::poll(&mut p, 1, left as i32) } <= 0 {
            break;
        }
        match f.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > MAX_BYTES {
                    return vec![];
                }
            }
        }
    }
    buf
}

/// Serve a stored entry to a pasting client.
pub fn write_entry(e: Arc<Entry>, mime: &str, fd: OwnedFd) {
    let mime = mime.to_string();
    std::thread::spawn(move || {
        let mime = mime.as_str();
        let text_like = |m: &str| m.starts_with("text/plain") || m == "UTF8_STRING" || m == "STRING" || m == "TEXT";
        let data = e
            .data
            .iter()
            .find(|(m, _)| m == mime)
            .or_else(|| if text_like(mime) { e.data.iter().find(|(m, _)| text_like(m)) } else { None })
            .map(|(_, d)| d.clone());
        if let Some(d) = data {
            let mut f = std::fs::File::from(fd);
            let _ = f.write_all(&d);
        }
    });
}

impl Aqua {
    /// A Wayland client set the clipboard.
    pub fn clipboard_record_wayland(&mut self, mimes: Vec<String>) {
        if mimes.iter().any(|m| m == "x-kde-passwordManagerHint") {
            return;
        }
        let want = wanted(&mimes);
        let mut reads = vec![];
        for m in &want {
            let Some((r, w)) = pipe() else { return };
            if request_data_device_client_selection(&self.seat, m.clone(), w).is_ok() {
                reads.push((m.clone(), r));
            }
        }
        self.spawn_clip_read(mimes, reads);
    }

    /// An X11 client owns the clipboard: read through the WM.
    pub fn clipboard_record_x11(&mut self, mimes: Vec<String>) {
        let want = wanted(&mimes);
        let mut reads = vec![];
        if let Some(xwm) = self.xwm.as_mut() {
            for m in &want {
                let Some((r, w)) = pipe() else { return };
                if xwm.send_selection(smithay::wayland::selection::SelectionTarget::Clipboard, m.clone(), w).is_ok() {
                    reads.push((m.clone(), r));
                }
            }
        }
        self.spawn_clip_read(mimes, reads);
    }

    fn spawn_clip_read(&mut self, mimes: Vec<String>, reads: Vec<(String, OwnedFd)>) {
        if reads.is_empty() {
            return;
        }
        let _ = self.display_handle.flush_clients();
        let tx = self.clip.tx.clone();
        std::thread::spawn(move || {
            let got: Vec<(String, Vec<u8>)> =
                reads.into_iter().map(|(m, fd)| (m, read_all(fd))).filter(|(_, d)| !d.is_empty()).collect();
            if !got.is_empty() {
                let _ = tx.send((mimes, got));
            }
        });
    }

    /// Move finished reads into the history (called from the frame timer).
    pub fn poll_clipboard(&mut self) -> bool {
        if let Some(m) = self.clip.pending.take() {
            self.clipboard_record_wayland(m);
        }
        let got: Vec<_> = self.clip.rx.try_iter().collect();
        let any = !got.is_empty();
        for (_mimes, data) in got {
            if let Some(last) = self.clip.entries.first() {
                if last.data.first().map(|(m, d)| (m.as_str(), d.as_slice()))
                    == data.first().map(|(m, d)| (m.as_str(), d.as_slice()))
                {
                    continue;
                }
            }
            let id = self.clip.next;
            self.clip.next += 1;
            let e = Arc::new(Entry { id, data: data.into_iter().map(|(m, d)| (m, Arc::new(d))).collect() });
            if let Some(item) = describe(&e) {
                self.shell.clipboard.push(item);
                self.clip.entries.insert(0, e);
            }
            self.clip.entries.truncate(MAX_ENTRIES);
            let keep: Vec<u64> = self.clip.entries.iter().map(|e| e.id).collect();
            self.shell.clipboard.retain_ids(&keep);
        }
        any
    }

    /// Re-publish a history entry as the current clipboard.
    pub fn clipboard_activate(&mut self, id: u64) {
        let Some(e) = self.clip.entries.iter().find(|e| e.id == id).cloned() else { return };
        let mut mimes: Vec<String> = e.data.iter().map(|(m, _)| m.clone()).collect();
        if mimes.iter().any(|m| m.starts_with("text/plain") || m == "UTF8_STRING") {
            for extra in ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING", "STRING", "TEXT"] {
                if !mimes.iter().any(|m| m == extra) {
                    mimes.push(extra.into());
                }
            }
        }
        set_data_device_selection(&self.display_handle, &self.seat, mimes.clone(), SelData::History(e.clone()));
        if let Some(xwm) = self.xwm.as_mut() {
            let _ = xwm.new_selection(smithay::wayland::selection::SelectionTarget::Clipboard, Some(mimes));
        }
        self.clip.entries.retain(|x| x.id != id);
        self.clip.entries.insert(0, e);
        self.shell.clipboard.bump(id);
    }

    /// Put plain text on the clipboard (used by "type text" fallbacks and the Character Viewer).
    pub fn clipboard_put_text(&mut self, text: &str) {
        let id = self.clip.next_id();
        let e =
            Arc::new(Entry { id, data: vec![("text/plain;charset=utf-8".into(), Arc::new(text.as_bytes().to_vec()))] });
        self.clipboard_push_entry(e);
    }

    /// Record an entry Aqua produced itself and make it the current clipboard.
    pub fn clipboard_push_entry(&mut self, e: Arc<Entry>) {
        let id = e.id;
        if let Some(item) = describe(&e) {
            self.shell.clipboard.push(item);
        }
        self.clip.entries.insert(0, e);
        self.clip.entries.truncate(MAX_ENTRIES);
        self.clipboard_activate(id);
    }

    /// Put a PNG image on the clipboard (screenshots).
    pub fn clipboard_put_png(&mut self, png: Vec<u8>) {
        let id = self.clip.next;
        self.clip.next += 1;
        let e = Arc::new(Entry { id, data: vec![("image/png".into(), Arc::new(png))] });
        if let Some(item) = describe(&e) {
            self.shell.clipboard.push(item);
        }
        self.clip.entries.insert(0, e);
        self.clip.entries.truncate(MAX_ENTRIES);
        self.clipboard_activate(id);
    }

    pub fn clipboard_clear(&mut self) {
        self.clip.entries.clear();
        self.shell.clipboard.items.clear();
    }
}

/// Visible text of an HTML fragment (browsers put `<meta …><b>…` on the clipboard).
fn html_text(h: &str) -> String {
    let mut out = String::new();
    let mut rest = h;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let lower: String = tail.chars().take(8).collect::<String>().to_ascii_lowercase();
        let skip = ["<style", "<script", "<head", "<title"].iter().find(|t| lower.starts_with(**t)).map(|t| &t[1..]);
        if let Some(tag) = skip {
            let close = format!("</{tag}");
            let tl = tail.to_ascii_lowercase();
            match tl.find(&close).and_then(|c| tl[c..].find('>').map(|e| c + e + 1)) {
                Some(end) => rest = &tail[end..],
                None => rest = "",
            }
            continue;
        }
        match tail.find('>') {
            Some(end) => {
                let t = tail[1..end].trim_start_matches('/').to_ascii_lowercase();
                if ["br", "p", "div", "li", "tr", "h1", "h2", "h3"]
                    .iter()
                    .any(|b| t == *b || t.starts_with(&format!("{b} ")))
                {
                    out.push(' ');
                }
                rest = &tail[end + 1..];
            }
            None => rest = "",
        }
    }
    out.push_str(rest);
    out.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

fn describe(e: &Entry) -> Option<ClipItem> {
    let rank = |m: &str| -> u8 {
        match m {
            m if m.starts_with("image/") => 0,
            "text/uri-list" | "x-special/gnome-copied-files" => 1,
            m if m.starts_with("text/plain") || m == "UTF8_STRING" || m == "STRING" || m == "TEXT" => 2,
            _ => 3,
        }
    };
    let (mime, data) = e.data.iter().min_by_key(|(m, _)| rank(m))?;
    let html;
    let data: &[u8] = if mime == "text/html" {
        html = html_text(&String::from_utf8_lossy(data));
        html.as_bytes()
    } else {
        data
    };
    let text = |d: &[u8]| String::from_utf8_lossy(d).to_string();
    let (kind, title, detail, thumb) = if mime.starts_with("image/") {
        let img = image::load_from_memory(data).ok();
        let (w, h) = img.as_ref().map(|i| (i.width(), i.height())).unwrap_or((0, 0));
        let thumb = img.and_then(|i| {
            let t = i.thumbnail(96, 96).to_rgba8();
            aqua_gfx::from_rgba(t.width(), t.height(), t.as_raw()).map(Arc::new)
        });
        let ext = mime.trim_start_matches("image/").to_uppercase();
        (ClipKind::Image, format!("Image {w}×{h}"), format!("{ext} image"), thumb)
    } else if mime == "text/uri-list" || mime == "x-special/gnome-copied-files" {
        let t = text(data);
        let uris: Vec<&str> = t.lines().filter(|l| l.contains("://")).collect();
        let first = uris.first().copied().unwrap_or("");
        if first.starts_with("file://") {
            let name = first.rsplit('/').next().unwrap_or(first).replace("%20", " ");
            let more = if uris.len() > 1 { format!(" + {} more", uris.len() - 1) } else { String::new() };
            (ClipKind::Files, format!("{name}{more}"), "File".into(), None)
        } else {
            let host = first.split("://").nth(1).unwrap_or("").split('/').next().unwrap_or("").to_string();
            (ClipKind::Url, first.to_string(), format!("URL · {host}"), None)
        }
    } else {
        let t = text(data);
        let one = t.split_whitespace().collect::<Vec<_>>().join(" ");
        if one.is_empty() {
            return None;
        }
        let is_url = !one.contains(' ') && (one.starts_with("http://") || one.starts_with("https://"));
        if is_url {
            let host = one.split("://").nth(1).unwrap_or("").split('/').next().unwrap_or("").to_string();
            (ClipKind::Url, one, format!("URL · {host}"), None)
        } else if one.len() == 7 && one.starts_with('#') && one[1..].chars().all(|c| c.is_ascii_hexdigit()) {
            (ClipKind::Color, one, "Colour".into(), None)
        } else {
            (ClipKind::Text, one.chars().take(200).collect(), "Text".into(), None)
        }
    };
    Some(ClipItem { id: e.id, kind, title, detail, time: aqua_shell::clock::now_hm(), thumb })
}
