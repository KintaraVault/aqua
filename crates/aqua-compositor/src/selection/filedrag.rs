//! Files from Aqua's own (Slint) apps to every other app.
//!
//! Slint can only put plain text on the clipboard and cannot start a Wayland
//! drag-and-drop, so Finder hands the selected files to the compositor
//! (`clipfiles` / `dragfiles` on the control socket) and Aqua serves them with the
//! usual file mime types (`text/uri-list`, GNOME's `x-special/gnome-copied-files`,
//! KDE's cut marker, plain paths): pasting into Files / Telegram / a browser upload
//! field and dropping onto other windows then works like with any file manager.
use crate::selection::clipboard::Entry;
use crate::state::Aqua;
use smithay::input::dnd::{DnDGrab, DndAction, Source, SourceMetadata};
use smithay::input::pointer::Focus;
use smithay::utils::{IsAlive, SERIAL_COUNTER};
use std::io::Write;
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// `file://` URIs (already percent-encoded by the sender) → local paths.
fn uri_path(u: &str) -> String {
    let p = u.strip_prefix("file://").unwrap_or(u);
    let b = p.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&p[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Representations of a list of files, preferred first.
fn file_data(uris: &[String], cut: bool) -> Vec<(String, Arc<Vec<u8>>)> {
    let list: String = uris.iter().map(|u| format!("{u}\r\n")).collect();
    let gnome = format!("{}\n{}", if cut { "cut" } else { "copy" }, uris.join("\n"));
    let paths = uris.iter().map(|u| uri_path(u)).collect::<Vec<_>>().join("\n");
    let mut v = vec![
        ("text/uri-list".to_string(), Arc::new(list.into_bytes())),
        ("x-special/gnome-copied-files".to_string(), Arc::new(gnome.into_bytes())),
    ];
    if cut {
        v.push(("application/x-kde-cutselection".to_string(), Arc::new(b"1".to_vec())));
    }
    v.push(("text/plain;charset=utf-8".to_string(), Arc::new(paths.into_bytes())));
    v
}

/// Parse `copy|cut uri uri …` (the URIs contain no spaces: they are percent-encoded).
fn parse(rest: &str) -> (bool, Vec<String>) {
    let mut it = rest.split_whitespace();
    let first = it.next().unwrap_or("");
    let (cut, mut uris) = match first {
        "cut" => (true, vec![]),
        "copy" => (false, vec![]),
        u => (false, vec![u.to_string()]),
    };
    uris.extend(it.map(str::to_string));
    uris.retain(|u| u.starts_with("file://"));
    (cut, uris)
}

/// Server-side drag source carrying files.
struct FileSource {
    data: Vec<(String, Arc<Vec<u8>>)>,
    alive: Arc<AtomicBool>,
}

impl IsAlive for FileSource {
    fn alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }
}

/// Drag image under the cursor: a document / folder glyph, with a count badge for
/// several items (Finder hands the drag over, it has no client-side icon surface).
pub fn drag_image(fonts: &aqua_gfx::Fonts, uris: &[String], scale: f32) -> aqua_gfx::Pixmap {
    use aqua_gfx::{tiny_skia::Color, Canvas, Rect, Weight};
    let rgba = |r: u8, g: u8, b: u8, a: f32| Color::from_rgba8(r, g, b, (a * 255.0) as u8);
    let mut c = Canvas::new(64.0, 64.0, scale);
    let dir = uris.first().map(|u| std::path::Path::new(&uri_path(u)).is_dir()).unwrap_or(false);
    if dir {
        c.fill_rrect(Rect::new(9.0, 14.0, 22.0, 10.0), 3.0, rgba(74, 150, 222, 0.92));
        c.fill_rrect(Rect::new(8.0, 18.0, 48.0, 34.0), 5.0, rgba(86, 166, 236, 0.92));
        c.fill_rrect_vgrad(Rect::new(8.0, 23.0, 48.0, 29.0), 5.0, rgba(132, 200, 250, 0.95), rgba(98, 178, 242, 0.95));
    } else {
        c.fill_rrect(Rect::new(15.0, 7.0, 34.0, 46.0), 4.0, rgba(0, 0, 0, 0.18));
        c.fill_rrect(Rect::new(16.0, 8.0, 32.0, 44.0), 3.5, rgba(255, 255, 255, 0.96));
        for i in 0..5 {
            c.fill_rect(
                Rect::new(21.0, 24.0 + i as f32 * 5.0, if i == 4 { 14.0 } else { 22.0 }, 1.6),
                rgba(150, 156, 166, 0.7),
            );
        }
        c.fill_rrect(Rect::new(37.0, 8.0, 11.0, 11.0), 2.0, rgba(222, 226, 232, 1.0));
    }
    let n = uris.len();
    if n > 1 {
        let t = n.to_string();
        let w = (fonts.measure(&t, 12.0, Weight::Semibold) + 10.0).max(20.0);
        let r = Rect::new(60.0 - w, 2.0, w, 20.0);
        c.fill_rrect(r, 10.0, rgba(255, 59, 48, 1.0));
        c.text_in(fonts, r, 0.5, 12.0, Weight::Semibold, Color::WHITE, &t);
    }
    c.pm
}

impl Source for FileSource {
    fn metadata(&self) -> Option<SourceMetadata> {
        Some(SourceMetadata {
            mime_types: self
                .data
                .iter()
                .map(|(m, _)| m.clone())
                .chain(["text/plain".to_string(), "UTF8_STRING".to_string()])
                .collect(),
            dnd_actions: vec![DndAction::Copy, DndAction::Move].into(),
        })
    }
    fn choose_action(&self, _action: DndAction) {}
    fn send(&self, mime_type: &str, fd: OwnedFd) {
        let text = |m: &str| m.starts_with("text/plain") || m == "UTF8_STRING";
        let d = self
            .data
            .iter()
            .find(|(m, _)| m == mime_type)
            .or_else(|| if text(mime_type) { self.data.iter().find(|(m, _)| text(m)) } else { None })
            .map(|(_, d)| d.clone());
        if let Some(d) = d {
            std::thread::spawn(move || {
                let mut f = std::fs::File::from(fd);
                let _ = f.write_all(&d);
            });
        }
    }
    fn drop_performed(&self) {}
    fn cancel(&self) {
        self.alive.store(false, Ordering::Relaxed);
    }
    fn finished(&self) {
        self.alive.store(false, Ordering::Relaxed);
    }
}

impl Aqua {
    /// `clipfiles copy|cut URI…`: put files on the clipboard (Finder ⌘C / ⌘X).
    pub fn clipboard_put_files(&mut self, rest: &str) {
        let (cut, uris) = parse(rest);
        if uris.is_empty() {
            return;
        }
        let id = self.clip.next_id();
        let e = Arc::new(Entry { id, data: file_data(&uris, cut) });
        self.clipboard_push_entry(e);
    }

    /// `dragfiles URI…`: the pointer (still pressed) left a Finder window while dragging
    /// files: continue as a real drag-and-drop the other apps can accept.
    pub fn start_file_drag(&mut self, rest: &str) {
        let (_, uris) = parse(rest);
        if uris.is_empty() {
            return;
        }
        let Some(ptr) = self.seat.get_pointer() else { return };
        let Some(start) = ptr.grab_start_data() else {
            tracing::debug!("dragfiles: no button pressed");
            return;
        };
        let alive = Arc::new(AtomicBool::new(true));
        let source = FileSource { data: file_data(&uris, false), alive: alive.clone() };
        let pm = drag_image(&self.shell.fonts, &uris, self.scale as f32);
        self.render_cache.file_drag =
            Some(crate::render::FileDragImage { buf: crate::render::buffer_from_pixmap(&pm, false), px: (pm.width(), pm.height()), alive });
        self.render_cache.grab_cursor = Some(smithay::input::pointer::CursorIcon::Copy);
        self.render_cache.cursor_override = self.render_cache.grab_cursor;
        let grab = DnDGrab::new_pointer(&self.display_handle, start, source, self.seat.clone());
        ptr.set_grab(self, grab, SERIAL_COUNTER.next_serial(), Focus::Clear);
        let loc = ptr.current_location();
        self.on_motion(loc, smithay::backend::input::InputTime::now());
        self.needs_redraw = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_and_formats() {
        let (cut, u) = parse("cut file:///home/a/My%20File.txt file:///tmp/b");
        assert!(cut);
        assert_eq!(u.len(), 2);
        assert_eq!(uri_path(&u[0]), "/home/a/My File.txt");
        let d = file_data(&u, true);
        assert_eq!(d[0].0, "text/uri-list");
        assert_eq!(String::from_utf8_lossy(&d[1].1), "cut\nfile:///home/a/My%20File.txt\nfile:///tmp/b");
    }
}
