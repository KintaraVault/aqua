//! Focus-scoped text assistance. Never exports text through IPC or learns from typed input.
use crate::state::Aqua;
use aqua_gfx::{rgba, Canvas, Rect, Weight};
use aqua_shell::{Layer, LayerId};
use aqua_text::{Dictionary, Shape, Typing, MAX_TEXT};
use smithay::{
    reexports::{
        calloop::timer::{TimeoutAction, Timer},
        wayland_protocols::wp::text_input::zv3::server::zwp_text_input_v3::{
            ContentHint, ContentPurpose, Request, ZwpTextInputV3,
        },
        wayland_server::{backend::ObjectId, protocol::wl_surface::WlSurface, Resource},
    },
    wayland::{input_method::InputMethodSeat, seat::WaylandFocus, text_input::TextInputSeat},
};
use std::{
    collections::HashMap,
    sync::{mpsc, Arc},
    time::Duration,
};
use zeroize::Zeroizing;

#[derive(Default)]
struct Delta {
    enabled: Option<bool>,
    surrounding: Option<(Zeroizing<String>, i32, i32)>,
    content: Option<(ContentHint, ContentPurpose)>,
    caret: Option<Shape>,
}
struct Field {
    text: Zeroizing<String>,
    cursor: i32,
    anchor: i32,
    hint: ContentHint,
    purpose: ContentPurpose,
    caret: Option<Shape>,
}
impl Default for Field {
    fn default() -> Self {
        Self {
            text: Zeroizing::new(String::new()),
            cursor: 0,
            anchor: 0,
            hint: ContentHint::empty(),
            purpose: ContentPurpose::Normal,
            caret: None,
        }
    }
}
impl Field {
    fn sensitive(&self) -> bool {
        matches!(self.purpose, ContentPurpose::Password | ContentPurpose::Pin)
            || self.hint.intersects(ContentHint::HiddenText | ContentHint::SensitiveData)
    }
    fn apply(&mut self, delta: Delta) {
        if delta.enabled == Some(true) {
            *self = Self::default();
        }
        if let Some((hint, purpose)) = delta.content {
            self.hint = hint;
            self.purpose = purpose;
        }
        if let Some(caret) = delta.caret {
            self.caret = Some(caret);
        }
        if self.sensitive() {
            self.text = Zeroizing::new(String::new());
            self.cursor = 0;
            self.anchor = 0;
        } else if let Some((text, cursor, anchor)) = delta.surrounding {
            self.text = text;
            self.cursor = cursor;
            self.anchor = anchor;
        }
    }
    fn prefix(&self) -> Option<&str> {
        if self.sensitive()
            || !matches!(self.purpose, ContentPurpose::Normal | ContentPurpose::Alpha | ContentPurpose::Name)
        {
            return None;
        }
        aqua_text::word_before_cursor(&self.text, self.cursor, self.anchor)
    }
}
struct Active {
    id: ObjectId,
    surface: WlSurface,
    field: Field,
}
enum Row {
    Word(String),
    Passwords,
    Secret(aqua_text::passwords::Entry),
    Info(String),
}
impl Row {
    fn label(&self) -> &str {
        match self {
            Self::Word(s) | Self::Info(s) => s,
            Self::Passwords => "Passwords…",
            Self::Secret(e) => &e.label,
        }
    }
}
enum PasswordResult {
    List(u64, Result<Vec<aqua_text::passwords::Entry>, ()>),
    Secret(u64, Result<Option<Zeroizing<String>>, ()>),
}
pub struct Assist {
    pending: HashMap<ObjectId, Delta>,
    active: Option<Active>,
    dictionary: Dictionary,
    rows: Vec<Row>,
    selected: usize,
    prefix_bytes: usize,
    pub generation: u64,
    pub serial: u64,
    dismissed: bool,
    queue: Option<Typing>,
    timer_running: bool,
    password_rx: Option<mpsc::Receiver<PasswordResult>>,
    dictionary_rx: Option<mpsc::Receiver<Dictionary>>,
    popup: Option<Rect>,
    pub consumed_button: bool,
    enabled: bool,
    t9: bool,
    content_cache: Option<(u64, Arc<aqua_gfx::Pixmap>)>,
}
impl Default for Assist {
    fn default() -> Self {
        let dictionary_path = std::env::var_os("AQUA_DICTIONARY").map(std::path::PathBuf::from).or_else(|| {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(std::path::PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
                .map(|p| p.join("aqua/words.txt"))
                .filter(|p| p.exists())
        });
        let dictionary_rx = Some({
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                use std::io::Read;
                if let Some(file) = dictionary_path.and_then(|path| std::fs::File::open(path).ok()) {
                    let mut text = String::new();
                    if file.take(8 * 1024 * 1024).read_to_string(&mut text).is_ok() {
                        let _ = tx.send(Dictionary::from_text(&text));
                        return;
                    }
                }
                let _ = tx.send(Dictionary::bundled());
            });
            rx
        });
        Self {
            pending: HashMap::new(),
            active: None,
            dictionary: Dictionary::seed(),
            rows: vec![],
            selected: 0,
            prefix_bytes: 0,
            generation: 0,
            serial: 1,
            dismissed: false,
            queue: None,
            timer_running: false,
            password_rx: None,
            dictionary_rx,
            popup: None,
            consumed_button: false,
            content_cache: None,
            enabled: std::env::var("AQUA_TEXT_ASSIST").as_deref() != Ok("0"),
            t9: std::env::var("AQUA_T9").as_deref() == Ok("1"),
        }
    }
}
impl Assist {
    pub fn clear_focus(&mut self) {
        self.pending.clear();
        self.active = None;
        self.rows.clear();
        self.popup = None;
        self.queue = None;
        self.password_rx = None;
        self.dismissed = false;
        self.content_cache = None;
        self.generation = self.generation.wrapping_add(1);
        self.serial = self.serial.wrapping_add(1);
    }
    fn refresh(&mut self) {
        self.rows.clear();
        self.popup = None;
        self.selected = 0;
        self.prefix_bytes = 0;
        self.serial = self.serial.wrapping_add(1);
        if !self.enabled || self.dismissed || self.queue.is_some() || self.password_rx.is_some() {
            return;
        }
        let Some(active) = &self.active else { return };
        if active.field.purpose == ContentPurpose::Password {
            self.rows.push(Row::Passwords);
        } else if let Some(prefix) = active.field.prefix() {
            self.prefix_bytes = prefix.len();
            self.rows = self.dictionary.complete(prefix).into_iter().map(Row::Word).collect();
        } else if self.t9 && !active.field.sensitive() && active.field.purpose == ContentPurpose::Normal {
            let f = &active.field;
            if f.cursor == f.anchor && f.cursor >= 0 && f.text.is_char_boundary(f.cursor as usize) {
                let before = &f.text[..f.cursor as usize];
                let digits = before.rsplit(|c: char| !c.is_ascii_digit()).next().unwrap_or("");
                self.prefix_bytes = digits.len();
                self.rows = self.dictionary.predict_t9(digits).into_iter().map(Row::Word).collect();
            }
        }
    }
    fn dismiss(&mut self) {
        self.dismissed = true;
        self.rows.clear();
        self.popup = None;
        self.serial = self.serial.wrapping_add(1);
    }
}
impl Aqua {
    pub fn observe_text_input(&mut self, input: &ZwpTextInputV3, request: &Request) -> bool {
        let focus = self.seat.text_input().focus();
        if !focus.as_ref().is_some_and(|s| s.id().same_client_as(&input.id())) || self.lock.is_locked() {
            return false;
        }
        if !self.assist.pending.contains_key(&input.id()) && self.assist.pending.len() >= 16 {
            return false;
        }
        let pending = self.assist.pending.entry(input.id()).or_default();
        match request {
            Request::Enable => {
                *pending = Delta::default();
                pending.enabled = Some(true);
            }
            Request::Disable => pending.enabled = Some(false),
            Request::SetSurroundingText { text, cursor, anchor } => {
                let valid = text.len() <= MAX_TEXT
                    && *cursor >= 0
                    && *anchor >= 0
                    && text.is_char_boundary(*cursor as usize)
                    && text.is_char_boundary(*anchor as usize);
                pending.surrounding = Some(if valid {
                    (Zeroizing::new(text.clone()), *cursor, *anchor)
                } else {
                    (Zeroizing::new(String::new()), 0, 0)
                });
            }
            Request::SetContentType { hint, purpose } => {
                pending.content = Some((
                    ContentHint::from_bits_truncate(u32::from(*hint)),
                    purpose.into_result().unwrap_or(ContentPurpose::Password),
                ))
            }
            Request::SetCursorRectangle { x, y, width, height } => {
                pending.caret = Shape {
                    x: *x as f32,
                    y: *y as f32,
                    w: (*width).max(1) as f32,
                    h: (*height).max(1) as f32,
                    radius: 0.0,
                }
                .validated();
            }
            Request::Commit => return true,
            Request::Destroy => {
                self.destroy_text_input(input);
            }
            _ => {}
        }
        false
    }
    pub fn commit_text_input(&mut self, input: &ZwpTextInputV3) {
        let Some(delta) = self.assist.pending.remove(&input.id()) else { return };
        let mut active = None;
        self.seat.text_input().with_active_text_input(|ti, surface| {
            if ti == input {
                active = Some(surface.clone());
            }
        });
        let Some(surface) = active else {
            if self.assist.active.as_ref().is_some_and(|a| a.id == input.id()) {
                self.assist.clear_focus();
            }
            return;
        };
        let new_field = delta.enabled == Some(true);
        let changed_type = delta.content.is_some_and(|(hint, purpose)| {
            self.assist.active.as_ref().is_some_and(|a| a.field.hint != hint || a.field.purpose != purpose)
        });
        // A client changing text independently of our expected insertion invalidates the queue.
        let unexpected_text = delta.surrounding.as_ref().is_some_and(|(text, cursor, anchor)| {
            self.assist.active.as_ref().is_some_and(|a| {
                !a.field.sensitive()
                    && (*a.field.text != **text || a.field.cursor != *cursor || a.field.anchor != *anchor)
            })
        });
        if new_field || changed_type || unexpected_text && self.assist.queue.is_some() {
            self.assist.queue = None;
            self.assist.password_rx = None;
            self.assist.generation = self.assist.generation.wrapping_add(1);
        }
        if !self.assist.active.as_ref().is_some_and(|a| a.id == input.id()) {
            self.assist.clear_focus();
            self.assist.active = Some(Active { id: input.id(), surface, field: Field::default() });
        }
        let a = self.assist.active.as_mut().unwrap();
        a.field.apply(delta);
        self.assist.dismissed = false;
        self.assist.refresh();
        self.needs_redraw = true;
    }
    pub fn destroy_text_input(&mut self, input: &ZwpTextInputV3) {
        self.assist.pending.remove(&input.id());
        if self.assist.active.as_ref().is_some_and(|a| a.id == input.id()) {
            self.assist.clear_focus();
        }
        self.needs_redraw = true;
    }
    fn assist_available(&self) -> bool {
        !self.lock.is_locked()
            && !self.shell.has_modal()
            && !self.mission_visible()
            && !self.seat.input_method().has_instance()
    }
    pub fn assist_key(&mut self, raw: u32, ctrl: bool, alt: bool, logo: bool) -> bool {
        use smithay::input::keyboard::keysyms;
        self.assist.queue = None;
        self.assist.password_rx = None;
        self.assist.generation = self.assist.generation.wrapping_add(1);
        if !self.assist_available() {
            self.assist.dismiss();
            return false;
        }
        if ctrl || alt || logo || self.assist.rows.is_empty() {
            return false;
        }
        match raw {
            keysyms::KEY_Escape => {
                self.assist.dismiss();
                self.needs_redraw = true;
                true
            }
            keysyms::KEY_Tab => {
                self.choose_assist();
                true
            }
            // Return remains an application action until the user explicitly navigates the popup.
            keysyms::KEY_Down => {
                self.assist.selected = (self.assist.selected + 1) % self.assist.rows.len();
                self.assist.serial = self.assist.serial.wrapping_add(1);
                self.needs_redraw = true;
                true
            }
            keysyms::KEY_Up => {
                self.assist.selected = (self.assist.selected + self.assist.rows.len() - 1) % self.assist.rows.len();
                self.assist.serial = self.assist.serial.wrapping_add(1);
                self.needs_redraw = true;
                true
            }
            _ => false,
        }
    }
    pub fn assist_button(&mut self, x: f32, y: f32, pressed: bool) -> bool {
        if !pressed && self.assist.consumed_button {
            self.assist.consumed_button = false;
            return true;
        }
        if !pressed || !self.assist_available() {
            return false;
        }
        if let Some(rect) = self.assist.popup {
            if x >= rect.x && x < rect.x + rect.w && y >= rect.y && y < rect.y + rect.h {
                let row = ((y - rect.y - 30.0) / 30.0).floor() as isize;
                if row >= 0 && (row as usize) < self.assist.rows.len() {
                    self.assist.selected = row as usize;
                    self.choose_assist();
                }
                self.assist.consumed_button = true;
                return true;
            }
        }
        self.assist.queue = None;
        self.assist.password_rx = None;
        self.assist.generation = self.assist.generation.wrapping_add(1);
        self.assist.dismiss();
        false
    }
    fn choose_assist(&mut self) {
        if !self.assist_available() {
            return;
        }
        let Some(row) = self.assist.rows.get(self.assist.selected) else { return };
        match row {
            Row::Word(word) => {
                let word = word.clone();
                let Some(active) = self.assist.active.as_ref() else { return };
                if active.field.sensitive() {
                    return;
                }
                let prefix = active.field.prefix().unwrap_or("");
                let (delete, insert) = if !prefix.is_empty() && word.starts_with(prefix) {
                    (0, word[prefix.len()..].to_owned())
                } else {
                    (self.assist.prefix_bytes, word)
                };
                if delete > 0 {
                    let ti = self.seat.text_input().clone();
                    ti.with_active_text_input(|input, _| input.delete_surrounding_text(delete as u32, 0));
                    ti.done(false);
                    if let Some(a) = self.assist.active.as_mut() {
                        let cursor = a.field.cursor.max(0) as usize;
                        if cursor >= delete && a.field.text.is_char_boundary(cursor - delete) {
                            a.field.text.replace_range(cursor - delete..cursor, "");
                            a.field.cursor -= delete as i32;
                            a.field.anchor = a.field.cursor;
                        }
                    }
                }
                self.queue_assist_text(Zeroizing::new(insert));
            }
            Row::Passwords | Row::Secret(_) => {
                let Some(app) = self.focused_window().map(|w| crate::state::title_of(&w).0).filter(|s| !s.is_empty())
                else {
                    return;
                };
                let selected_path = match row {
                    Row::Secret(e) => Some(e.path.clone()),
                    _ => None,
                };
                let generation = self.assist.generation;
                let (tx, rx) = mpsc::channel();
                self.assist.password_rx = Some(rx);
                self.assist.rows = vec![Row::Info("Opening password store…".into())];
                self.assist.serial = self.assist.serial.wrapping_add(1);
                std::thread::spawn(move || {
                    let result = if let Some(path) = selected_path {
                        PasswordResult::Secret(generation, aqua_text::passwords::fetch(&app, &path).map_err(|_| ()))
                    } else {
                        PasswordResult::List(generation, aqua_text::passwords::list(&app).map_err(|_| ()))
                    };
                    let _ = tx.send(result);
                });
                self.needs_redraw = true;
            }
            Row::Info(_) => {}
        }
    }
    pub fn poll_assist(&mut self) -> bool {
        if self.lock.is_locked() || self.assist.active.as_ref().is_some_and(|a| !a.surface.is_alive()) {
            let dirty = self.assist.active.is_some();
            self.assist.clear_focus();
            return dirty;
        }
        let mut dirty = false;
        if let Some(dictionary) = self.assist.dictionary_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.assist.dictionary = dictionary;
            self.assist.dictionary_rx = None;
            self.assist.refresh();
            dirty = true;
        }
        let result = self.assist.password_rx.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some(result) = result {
            self.assist.password_rx = None;
            match result {
                PasswordResult::List(generation, result)
                    if generation == self.assist.generation && self.assist_available() =>
                {
                    self.assist.rows = match result {
                        Ok(entries) if !entries.is_empty() => entries.into_iter().take(5).map(Row::Secret).collect(),
                        Ok(_) => vec![Row::Info("No passwords saved for this app".into())],
                        Err(()) => vec![Row::Info("Password store unavailable".into())],
                    };
                    self.assist.selected = 0;
                }
                PasswordResult::Secret(generation, Ok(Some(secret)))
                    if generation == self.assist.generation && self.assist_available() =>
                {
                    self.queue_assist_text(secret);
                }
                PasswordResult::Secret(generation, _) if generation == self.assist.generation => {
                    self.assist.rows = vec![Row::Info("Password was not supplied".into())];
                }
                _ => {}
            }
            self.assist.serial = self.assist.serial.wrapping_add(1);
            dirty = true;
        }
        dirty
    }
    pub fn queue_assist_text(&mut self, text: Zeroizing<String>) -> bool {
        if !self.assist_available() {
            return false;
        }
        let mut active = false;
        self.seat.text_input().with_active_text_input(|_, _| active = true);
        if !active || self.assist.active.is_none() {
            return false;
        }
        self.assist.queue = Typing::new(text, self.assist.generation);
        self.assist.dismiss();
        if self.assist.queue.is_some() && !self.assist.timer_running {
            self.assist.timer_running = true;
            if self
                .loop_handle
                .insert_source(Timer::from_duration(Duration::from_millis(8)), |_, _, st| {
                    if st.tick_assist_typing() {
                        TimeoutAction::ToDuration(Duration::from_millis(8))
                    } else {
                        st.assist.timer_running = false;
                        TimeoutAction::Drop
                    }
                })
                .is_err()
            {
                self.assist.queue = None;
                self.assist.timer_running = false;
            }
        }
        self.needs_redraw = true;
        self.assist.queue.is_some()
    }
    fn tick_assist_typing(&mut self) -> bool {
        if !self.assist_available() {
            self.assist.queue = None;
            return false;
        }
        let Some(queue) = self.assist.queue.as_mut() else { return false };
        if queue.generation != self.assist.generation {
            self.assist.queue = None;
            return false;
        }
        let Some(part) = queue.next_grapheme() else {
            self.assist.queue = None;
            return false;
        };
        let Some(active) = &mut self.assist.active else {
            self.assist.queue = None;
            return false;
        };
        let id = active.id.clone();
        let surface = active.surface.clone();
        let ti = self.seat.text_input().clone();
        let mut sent = false;
        ti.with_active_text_input(|input, current_surface| {
            if input.id() == id && *current_surface == surface && input.is_alive() {
                input.commit_string(Some(part.to_string()));
                sent = true;
            }
        });
        if !sent {
            self.assist.queue = None;
            return false;
        }
        ti.done(false);
        if !active.field.sensitive() {
            let f = &mut active.field;
            if f.cursor >= 0 && f.text.is_char_boundary(f.cursor as usize) && f.text.len() + part.len() <= MAX_TEXT {
                f.text.insert_str(f.cursor as usize, &part);
                f.cursor += part.len() as i32;
                f.anchor = f.cursor;
            }
        }
        self.needs_redraw = true;
        if self.assist.queue.as_ref().is_some_and(Typing::finished) {
            self.assist.queue = None;
            return false;
        }
        true
    }
    /// Global logical caret plus optional exact field contour. Does not guess on unknown surfaces.
    pub fn assist_geometry(&self) -> Option<(Shape, Option<Shape>)> {
        let active = self.assist.active.as_ref()?;
        let caret = active.field.caret?;
        let surface = &active.surface;
        let mut origin = None;
        for w in self.space.elements() {
            let Some(root) = w.wl_surface() else { continue };
            let Some(loc) = self.space.element_location(w) else { continue };
            let base = loc - w.geometry().loc;
            if root.as_ref() == surface {
                origin = Some(base);
                break;
            }
            for (popup, offset) in smithay::desktop::PopupManager::popups_for_surface(root.as_ref()) {
                if popup.wl_surface() == surface {
                    origin = Some(base + w.geometry().loc + offset - popup.geometry().loc);
                    break;
                }
            }
        }
        let origin = origin?;
        let translate = |shape: Shape| Shape { x: shape.x + origin.x as f32, y: shape.y + origin.y as f32, ..shape };
        Some((translate(caret), crate::wayland::text_field::geometry(surface).map(translate)))
    }
    pub fn assist_layer(&mut self) -> Option<Layer> {
        if !self.assist_available() || self.assist.rows.is_empty() {
            self.assist.popup = None;
            return None;
        }
        let (caret, _) = self.assist_geometry()?;
        let bounds = self
            .outputs
            .list
            .iter()
            .filter_map(|o| self.space.output_geometry(o))
            .find(|r| r.to_f64().contains((caret.x as f64, caret.y as f64)))?;
        let w = 256.0f32.min((bounds.size.w - 12).max(1) as f32);
        let h = 38.0 + self.assist.rows.len() as f32 * 30.0;
        let rect = place_popup(
            caret,
            Shape {
                x: bounds.loc.x as f32,
                y: bounds.loc.y as f32,
                w: bounds.size.w as f32,
                h: bounds.size.h as f32,
                radius: 0.0,
            },
            w,
            h,
        );
        self.assist.popup = Some(rect);
        let dark = self.shell.style.dark;
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        (self.assist.serial, dark, self.scale.to_bits(), w.to_bits(), h.to_bits()).hash(&mut hash);
        let serial = hash.finish();
        let content = if let Some((_, content)) = self.assist.content_cache.as_ref().filter(|(key, _)| *key == serial) {
            content.clone()
        } else {
            let mut canvas = Canvas::new(w, h, self.scale as f32);
            let fg = if dark { rgba(245, 245, 250, 1.0) } else { rgba(25, 25, 30, 1.0) };
            let muted = if dark { rgba(215, 215, 225, 0.75) } else { rgba(45, 45, 55, 0.75) };
            let password = self.assist.active.as_ref().is_some_and(|a| a.field.sensitive());
            let app = self.focused_window().map(|w| crate::state::title_of(&w).0).unwrap_or_default();
            let title = if password { format!("Passwords · {app}") } else { "Suggestions · Tab to insert".into() };
            canvas.text_in(
                &self.shell.fonts,
                Rect::new(12.0, 6.0, w - 24.0, 19.0),
                0.0,
                11.0,
                Weight::Medium,
                muted,
                &title,
            );
            for (i, row) in self.assist.rows.iter().enumerate() {
                let y = 30.0 + i as f32 * 30.0;
                if i == self.assist.selected && !matches!(row, Row::Info(_)) {
                    canvas.fill_rrect(Rect::new(6.0, y, w - 12.0, 28.0), 7.0, rgba(60, 125, 245, 0.25));
                }
                canvas.text_in(
                    &self.shell.fonts,
                    Rect::new(14.0, y + 1.0, w - 28.0, 27.0),
                    0.0,
                    13.0,
                    Weight::Medium,
                    fg,
                    row.label(),
                );
            }
            let content = Arc::new(canvas.pm);
            self.assist.content_cache = Some((serial, content.clone()));
            content
        };
        Some(Layer {
            id: LayerId::TextAssist,
            rect,
            glass: Some(self.cfg.glass.material(aqua_config::material::Material::Menu, dark).with_radius(12.0)),
            tiles: vec![],
            content,
            serial,
            opacity: 1.0,
            zoom: 1.0,
        })
    }
}
fn place_popup(caret: Shape, output: Shape, w: f32, h: f32) -> Rect {
    let x = caret.x.clamp(output.x + 6.0, (output.x + output.w - w - 6.0).max(output.x + 6.0));
    let below = caret.y + caret.h + 6.0;
    let y = if below + h <= output.y + output.h - 6.0 { below } else { caret.y - h - 6.0 };
    let y = y.clamp(output.y + 6.0, (output.y + output.h - h - 6.0).max(output.y + 6.0));
    Rect::new(x, y, w, h)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn password_surrounding_text_is_not_retained() {
        let mut field = Field::default();
        field.apply(Delta {
            surrounding: Some((Zeroizing::new("do not retain".into()), 13, 13)),
            content: Some((ContentHint::empty(), ContentPurpose::Password)),
            ..Default::default()
        });
        assert!(field.text.is_empty());
        assert!(field.prefix().is_none());
    }
    #[test]
    fn sensitive_hint_clears_old_plain_text() {
        let mut field = Field::default();
        field.apply(Delta { surrounding: Some((Zeroizing::new("hello".into()), 5, 5)), ..Default::default() });
        field
            .apply(Delta { content: Some((ContentHint::SensitiveData, ContentPurpose::Normal)), ..Default::default() });
        assert!(field.text.is_empty());
        assert!(field.sensitive());
    }
    #[test]
    fn enable_resets_the_previous_field() {
        let mut field = Field::default();
        field.apply(Delta {
            content: Some((ContentHint::SensitiveData, ContentPurpose::Password)),
            ..Default::default()
        });
        field.apply(Delta { enabled: Some(true), ..Default::default() });
        assert!(!field.sensitive());
        assert!(field.text.is_empty());
        assert!(field.caret.is_none());
    }
    #[test]
    fn clear_cancels_and_invalidates_workers() {
        let mut assist = Assist::default();
        let generation = assist.generation;
        assist.queue = Typing::new(Zeroizing::new("private".into()), generation);
        assist.clear_focus();
        assert!(assist.queue.is_none());
        assert_ne!(generation, assist.generation);
    }
    #[test]
    fn placement_flips_and_clamps_on_offset_monitor() {
        let caret = Shape { x: 2900.0, y: 980.0, w: 1.0, h: 20.0, radius: 0.0 };
        let bounds = Shape { x: 1920.0, y: 0.0, w: 1080.0, h: 1000.0, radius: 0.0 };
        let r = place_popup(caret, bounds, 256.0, 188.0);
        assert!(r.x + r.w <= 2994.0);
        assert!(r.y + r.h < caret.y);
        assert!(r.x >= 1926.0);
    }
}
