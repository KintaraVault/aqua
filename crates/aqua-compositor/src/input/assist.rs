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
    collections::{HashMap, VecDeque},
    sync::{mpsc, Arc},
    time::Duration,
};
use zeroize::Zeroizing;

#[derive(Default)]
struct Delta {
    enabled: Option<bool>,
    surrounding: Option<(Zeroizing<String>, i32, i32)>,
    content: Option<(ContentHint, ContentPurpose)>,
    caret: Option<Option<Shape>>,
}
struct Field {
    text: Zeroizing<String>,
    surrounding_reported: bool,
    caret_reported: bool,
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
            surrounding_reported: false,
            caret_reported: false,
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
            self.caret_reported = caret.is_some();
            self.caret = caret;
        }
        if self.sensitive() {
            self.text = Zeroizing::new(String::new());
            self.cursor = 0;
            self.anchor = 0;
        } else if let Some((text, cursor, anchor)) = delta.surrounding {
            self.text = text;
            self.surrounding_reported = true;
            self.cursor = cursor;
            self.anchor = anchor;
        }
    }
    fn protected_reenable(&self, delta: &Delta) -> bool {
        self.sensitive() && delta.enabled == Some(true) && delta.content == Some((self.hint, self.purpose))
    }
    fn expect_insertion(&mut self, part: &str) {
        if self.sensitive() || self.cursor < 0 || self.anchor < 0 {
            return;
        }
        let lo = self.cursor.min(self.anchor) as usize;
        let hi = self.cursor.max(self.anchor) as usize;
        if self.text.is_char_boundary(lo)
            && self.text.is_char_boundary(hi)
            && self.text.len() - (hi - lo) + part.len() <= MAX_TEXT
        {
            self.text.replace_range(lo..hi, part);
            self.cursor = (lo + part.len()) as i32;
            self.anchor = self.cursor;
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
/// Native clients may acknowledge one edit more than once, or acknowledge deletion separately.
/// Keep only a small, zeroized history; an old acknowledgement must never release the next edit.
#[derive(Default)]
struct Acknowledgements(VecDeque<(Zeroizing<String>, i32, i32)>);
impl Acknowledgements {
    fn remember(&mut self, field: &Field) {
        if field.sensitive() {
            return;
        }
        if self.0.len() == 16 {
            self.0.pop_front();
        }
        self.0.push_back((field.text.clone(), field.cursor, field.anchor));
    }
    fn contains(&self, state: &(Zeroizing<String>, i32, i32)) -> bool {
        self.0.iter().any(|old| old.0.as_str() == state.0.as_str() && old.1 == state.1 && old.2 == state.2)
    }
    fn clear(&mut self) {
        self.0.clear();
    }
}
struct Active {
    id: ObjectId,
    surface: WlSurface,
    field: Field,
    native: bool,
    accessible: Option<aqua_text::accessibility::Target>,
    bounds: Option<Shape>,
    origin: Option<String>,
    browser: bool,
}
impl Active {
    fn use_native_delivery(&self) -> bool {
        self.native
    }
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
            Self::Passwords => aqua_i18n::tr("Passwords…"),
            Self::Secret(e) => &e.label,
        }
    }
}
enum PasswordResult {
    List(u64, Result<Vec<aqua_text::passwords::Entry>, ()>),
    Secret(u64, Result<Option<Zeroizing<String>>, ()>),
}
/// Compositor-only, borrowed snapshot. Sensitive fields never expose surrounding text.
#[allow(dead_code)]
pub struct TextContext<'a> {
    pub surface: &'a WlSurface,
    pub generation: u64,
    pub purpose: ContentPurpose,
    pub hints: ContentHint,
    pub sensitive: bool,
    pub surrounding: Option<&'a str>,
    pub cursor: i32,
    pub anchor: i32,
    pub caret: Option<Shape>,
    pub field: Option<Shape>,
    pub field_radius_known: bool,
}

pub struct Assist {
    bridge: aqua_text::accessibility::Bridge,
    bridge_window: Option<(u64, String, u32, bool)>,
    access_typing: bool,
    permit_pending: bool,
    native_waiting: bool,
    native_suspended: bool,
    native_deadline: Option<std::time::Instant>,
    pending_delete: Option<u32>,
    acknowledgements: Acknowledgements,
    replace_prefix: Option<Zeroizing<String>>,
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
            bridge: aqua_text::accessibility::Bridge::new(),
            bridge_window: None,
            access_typing: false,
            permit_pending: false,
            native_waiting: false,
            native_suspended: false,
            native_deadline: None,
            pending_delete: None,
            acknowledgements: Acknowledgements::default(),
            replace_prefix: None,
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
        self.access_typing = false;
        self.permit_pending = false;
        self.native_waiting = false;
        self.native_suspended = false;
        self.native_deadline = None;
        self.pending_delete = None;
        self.acknowledgements.clear();
        self.replace_prefix = None;
        self.bridge_window = None;
        self.pending.clear();
        self.active = None;
        self.rows.clear();
        self.popup = None;
        self.queue = None;
        self.password_rx = None;
        self.dismissed = false;
        self.content_cache = None;
        self.generation = self.generation.wrapping_add(1);
        self.bridge.cancel(self.generation);
        self.serial = self.serial.wrapping_add(1);
    }
    fn refresh(&mut self) {
        self.rows.clear();
        self.popup = None;
        self.selected = 0;
        self.prefix_bytes = 0;
        self.serial = self.serial.wrapping_add(1);
        if !self.enabled || self.dismissed || self.queue.is_some() || self.access_typing || self.password_rx.is_some() {
            return;
        }
        let Some(active) = &self.active else { return };
        if active.field.purpose == ContentPurpose::Password {
            if self.bridge_window.is_none() {
                self.rows.push(Row::Info(aqua_i18n::tr("Input target could not be verified").into()));
            } else if active.browser && active.origin.is_none() {
                self.rows.push(Row::Info(aqua_i18n::tr("Website could not be verified").into()));
            } else {
                self.rows.push(Row::Passwords);
            }
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
        if !self.assist.enabled || self.seat.input_method().has_instance() {
            return false;
        }
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
                pending.caret = Some(
                    Shape { x: *x as f32, y: *y as f32, w: *width as f32, h: *height as f32, radius: 0.0 }
                        .validated_caret(),
                );
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
        let Some(mut delta) = self.assist.pending.remove(&input.id()) else { return };
        let mut active = None;
        self.seat.text_input().with_active_text_input(|ti, surface| {
            if ti == input {
                active = Some(surface.clone());
            }
        });
        let Some(surface) = active else {
            // Chromium brackets protected edits with Disable/Enable. Suspend delivery, never
            // send into a disabled text-input, and require fresh object/origin authorization
            // after re-enable. A real blur, destruction or timeout still cancels immediately.
            if delta.enabled == Some(false)
                && self.assist.queue.is_some()
                && self.assist.active.as_ref().is_some_and(|a| {
                    a.native
                        && a.id == input.id()
                        && a.browser
                        && a.field.sensitive()
                        && a.accessible.is_some()
                        && a.origin.is_some()
                })
            {
                self.assist.generation = self.assist.generation.wrapping_add(1);
                if let Some(queue) = self.assist.queue.as_mut() {
                    queue.generation = self.assist.generation;
                }
                self.assist.permit_pending = false;
                self.assist.native_waiting = false;
                self.assist.native_suspended = true;
                self.assist.native_deadline = Some(std::time::Instant::now() + Duration::from_millis(250));
                self.assist.bridge.cancel(self.assist.generation);
                return;
            }
            if self.assist.active.as_ref().is_some_and(|a| a.native && a.id == input.id()) {
                self.assist.clear_focus();
            }
            return;
        };
        let current = self.assist.active.as_ref().filter(|a| a.native && a.id == input.id());
        let exact = current.is_some_and(|a| {
            delta.surrounding.as_ref().is_some_and(|(text, cursor, anchor)| {
                text.as_str() == a.field.text.as_str() && *cursor == a.field.cursor && *anchor == a.field.anchor
            })
        });
        let old_ack = self.assist.native_waiting
            && !exact
            && delta.surrounding.as_ref().is_some_and(|state| self.assist.acknowledgements.contains(state));
        if old_ack {
            delta.surrounding = None;
        }
        let acknowledged = !old_ack
            && current.is_some_and(|a| {
                a.field.sensitive() && !a.browser
                    || exact
                    || !a.field.surrounding_reported && delta.caret.flatten().is_some()
            });
        if acknowledged {
            self.assist.native_waiting = false;
            self.assist.native_suspended = false;
            self.assist.native_deadline = None;
        }
        // Chromium may re-enable the same protected text-input after each commit string.
        // Native Enable alone cannot prove field identity: keep the old AT-SPI object and
        // require a new generation's focus/protection/origin permit before another grapheme.
        let resume_protected = self.assist.queue.is_some()
            && self.assist.active.as_ref().is_some_and(|a| {
                a.native
                    && a.id == input.id()
                    && a.browser
                    && a.accessible.is_some()
                    && a.origin.is_some()
                    && a.field.protected_reenable(&delta)
            });
        if resume_protected {
            delta.enabled = None;
            self.assist.generation = self.assist.generation.wrapping_add(1);
            if let Some(queue) = self.assist.queue.as_mut() {
                queue.generation = self.assist.generation;
            }
            self.assist.permit_pending = false;
            self.assist.native_waiting = false;
            self.assist.native_suspended = false;
            self.assist.native_deadline = None;
            self.assist.bridge.cancel(self.assist.generation);
        }
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
            self.assist.acknowledgements.clear();
            self.assist.queue = None;
            self.assist.password_rx = None;
            self.assist.generation = self.assist.generation.wrapping_add(1);
            self.assist.access_typing = false;
            self.assist.permit_pending = false;
            self.assist.native_waiting = false;
            self.assist.native_suspended = false;
            self.assist.native_deadline = None;
            self.assist.bridge.cancel(self.assist.generation);
        }
        if !self.assist.active.as_ref().is_some_and(|a| a.native && a.id == input.id()) {
            self.assist.clear_focus();
            self.assist.active = Some(Active {
                id: input.id(),
                surface,
                field: Field::default(),
                native: true,
                accessible: None,
                bounds: None,
                origin: None,
                browser: false,
            });
        }
        let a = self.assist.active.as_mut().unwrap();
        a.field.apply(delta);
        if new_field {
            self.seat.text_input().done(false);
        }
        self.poll_accessibility();
        self.assist.dismissed = false;
        self.assist.refresh();
        self.needs_redraw = true;
    }
    pub fn destroy_text_input(&mut self, input: &ZwpTextInputV3) {
        self.assist.pending.remove(&input.id());
        if self.assist.active.as_ref().is_some_and(|a| a.native && a.id == input.id()) {
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
    pub fn assist_key(&mut self, raw: u32, ctrl: bool, alt: bool, logo: bool, shift: bool) -> bool {
        use smithay::input::keyboard::keysyms;
        self.assist.queue = None;
        self.assist.password_rx = None;
        self.assist.generation = self.assist.generation.wrapping_add(1);
        self.assist.access_typing = false;
        self.assist.permit_pending = false;
        self.assist.native_waiting = false;
        self.assist.native_suspended = false;
        self.assist.native_deadline = None;
        self.assist.pending_delete = None;
        self.assist.acknowledgements.clear();
        self.assist.bridge.cancel(self.assist.generation);
        if !self.assist_available() {
            self.assist.dismiss();
            return false;
        }
        if ctrl || alt || logo || shift || self.assist.rows.is_empty() {
            self.assist.dismiss();
            self.needs_redraw = true;
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
            _ => {
                self.assist.dismiss();
                self.needs_redraw = true;
                false
            }
        }
    }
    pub fn assist_wants_pointer(&self, x: f32, y: f32) -> bool {
        self.assist_available()
            && self.assist.popup.is_some_and(|r| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
    }
    pub fn assist_pointer_motion(&mut self, x: f32, y: f32) {
        if !self.assist_wants_pointer(x, y) {
            return;
        }
        let Some(r) = self.assist.popup else {
            return;
        };
        let row = ((y - r.y - 30.0) / 30.0).floor() as isize;
        if row >= 0 && (row as usize) < self.assist.rows.len() && self.assist.selected != row as usize {
            self.assist.selected = row as usize;
            self.assist.serial = self.assist.serial.wrapping_add(1);
            self.needs_redraw = true;
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
        self.assist.access_typing = false;
        self.assist.permit_pending = false;
        self.assist.native_waiting = false;
        self.assist.native_suspended = false;
        self.assist.native_deadline = None;
        self.assist.pending_delete = None;
        self.assist.acknowledgements.clear();
        self.assist.bridge.cancel(self.assist.generation);
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
                let native = active.use_native_delivery();
                if delete > 0 && native {
                    self.assist.pending_delete = Some(delete as u32);
                    if let Some(a) = self.assist.active.as_mut() {
                        self.assist.acknowledgements.remember(&a.field);
                        let cursor = a.field.cursor.max(0) as usize;
                        if cursor >= delete && a.field.text.is_char_boundary(cursor - delete) {
                            a.field.text.replace_range(cursor - delete..cursor, "");
                            a.field.cursor -= delete as i32;
                            a.field.anchor = a.field.cursor;
                            self.assist.acknowledgements.remember(&a.field);
                        }
                    }
                }
                if delete > 0 && !native {
                    let f = &self.assist.active.as_ref().unwrap().field;
                    let cursor = f.cursor.max(0) as usize;
                    if cursor >= delete && f.text.is_char_boundary(cursor - delete) {
                        self.assist.replace_prefix = Some(Zeroizing::new(f.text[cursor - delete..cursor].to_owned()));
                    } else {
                        return;
                    }
                }
                self.queue_assist_text(Zeroizing::new(insert));
            }
            Row::Passwords | Row::Secret(_) => {
                // _NET_WM_PID is a client claim, not an authenticated credential target.
                if self.focused_window().is_some_and(|w| w.x11_surface().is_some()) {
                    self.assist.dismiss();
                    return;
                }
                let Some(app) = self.focused_window().map(|w| crate::state::title_of(&w).0).filter(|s| !s.is_empty())
                else {
                    return;
                };
                let selected_path = match row {
                    Row::Secret(e) => Some(e.path.clone()),
                    _ => None,
                };
                let Some(active) = self.assist.active.as_ref() else {
                    return;
                };
                if self.assist.bridge_window.is_none() {
                    return;
                }
                let scope = if active.browser {
                    let Some(origin) = active.origin.clone() else {
                        return;
                    };
                    aqua_text::passwords::Scope::Origin(origin)
                } else {
                    aqua_text::passwords::Scope::App(app.clone())
                };
                let generation = self.assist.generation;
                let (tx, rx) = mpsc::channel();
                self.assist.password_rx = Some(rx);
                self.assist.rows = vec![Row::Info(aqua_i18n::tr("Opening password store…").into())];
                self.assist.serial = self.assist.serial.wrapping_add(1);
                std::thread::spawn(move || {
                    let result = if let Some(path) = selected_path {
                        PasswordResult::Secret(
                            generation,
                            aqua_text::passwords::fetch_scoped(&scope, &path).map_err(|_| ()),
                        )
                    } else {
                        PasswordResult::List(generation, aqua_text::passwords::list_scoped(&scope).map_err(|_| ()))
                    };
                    let _ = tx.send(result);
                });
                self.needs_redraw = true;
            }
            Row::Info(_) => {}
        }
    }
    fn revoke_accessible_context(&mut self) -> bool {
        let Some(active) = self.assist.active.as_mut() else {
            return false;
        };
        let changed = active.accessible.is_some() || active.bounds.is_some() || active.origin.is_some();
        active.accessible = None;
        active.bounds = None;
        active.origin = None;
        if changed {
            self.assist.password_rx = None;
            self.assist.queue = None;
            self.assist.access_typing = false;
            self.assist.permit_pending = false;
            self.assist.native_waiting = false;
            self.assist.native_suspended = false;
            self.assist.native_deadline = None;
            self.assist.generation = self.assist.generation.wrapping_add(1);
            self.assist.bridge.cancel(self.assist.generation);
            self.assist.refresh();
        }
        changed
    }
    fn poll_accessibility(&mut self) -> bool {
        use aqua_text::accessibility::{is_browser_executable, Focus};
        let window = self.focused_window();
        let permitted = self.assist.enabled && self.assist_available();
        let identity = window.as_ref().filter(|_| permitted).and_then(|w| {
            let title = crate::state::title_of(w).1;
            let id = crate::state::meta(w).borrow().id;
            if self.space.elements().filter(|other| crate::state::title_of(other).1 == title).count() != 1 {
                return None;
            }
            if let Some(cached) = &self.assist.bridge_window {
                if cached.0 == id && cached.1 == title {
                    return Some(cached.clone());
                }
            }
            let pid = w.x11_surface().and_then(|s| s.pid()).or_else(|| {
                w.wl_surface()?.client()?.get_credentials(&self.display_handle).ok().map(|c| c.pid as u32)
            })?;
            if pid == 0 {
                return None;
            }
            let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
            let browser = is_browser_executable(exe.file_name()?.to_str()?);
            let pid = if browser { aqua_text::accessibility::browser_process_root(pid)? } else { pid };
            Some((id, title, pid, browser))
        });
        self.assist.bridge_window = identity.clone();
        if let (Some(active), Some(identity)) = (self.assist.active.as_mut(), identity.as_ref()) {
            active.browser = identity.3;
        }
        if identity.is_none() {
            self.assist.bridge.focus(None, self.assist.generation);
            if self.assist.active.as_ref().is_some_and(|active| !active.native) {
                self.assist.clear_focus();
                return true;
            }
            let pending = self.assist.password_rx.is_some() || self.assist.queue.is_some() || self.assist.access_typing;
            if pending {
                self.assist.password_rx = None;
                self.assist.queue = None;
                self.assist.access_typing = false;
                self.assist.permit_pending = false;
                self.assist.native_waiting = false;
                self.assist.native_suspended = false;
                self.assist.native_deadline = None;
                self.assist.generation = self.assist.generation.wrapping_add(1);
                self.assist.bridge.cancel(self.assist.generation);
            }
            return self.revoke_accessible_context() || pending;
        }
        let focus = identity.map(|(window, title, pid, browser)| Focus {
            generation: self.assist.generation,
            window,
            pid,
            title,
            browser,
            protected: self.assist.active.as_ref().is_some_and(|a| a.native && a.field.sensitive()),
        });
        self.assist.bridge.focus(focus, self.assist.generation);
        let mut dirty = false;
        if let Some((generation, _success)) = self.assist.bridge.take_done() {
            if generation == self.assist.generation {
                self.assist.access_typing = false;
                self.assist.permit_pending = false;
                self.assist.native_waiting = false;
                self.assist.native_suspended = false;
                self.assist.native_deadline = None;
                self.assist.replace_prefix = None;
                dirty = true;
            }
        }
        let Some(update) = self.assist.bridge.take_snapshot() else {
            return dirty;
        };
        if update.generation != self.assist.generation {
            return dirty;
        }
        let Some(snapshot) = update.snapshot else {
            if self.assist.active.as_ref().is_some_and(|a| !a.native) {
                self.assist.clear_focus();
                self.needs_redraw = true;
                return true;
            }
            dirty |= self.revoke_accessible_context();
            return dirty;
        };
        let Some(w) = window else {
            return dirty;
        };
        let Some(surface) = w.wl_surface().map(|s| s.into_owned()) else {
            return dirty;
        };
        let native = self.assist.active.as_ref().is_some_and(|a| a.native);
        // Do not attach document metadata from another widget to a native password field.
        if native {
            let a = self.assist.active.as_ref().unwrap();
            if a.field.sensitive() != snapshot.password {
                return self.revoke_accessible_context() || dirty;
            }
            if let Some(caret) = a.field.caret {
                let b = snapshot.field;
                if caret.x < b.x - 2.0 || caret.y < b.y - 2.0 || caret.x > b.x + b.w + 2.0 || caret.y > b.y + b.h + 2.0
                {
                    return self.revoke_accessible_context() || dirty;
                }
            }
        }
        let target_changed = self.assist.active.as_ref().is_some_and(|a| {
            a.accessible.as_ref().is_some_and(|t| t.object != snapshot.target.object)
                || a.origin != snapshot.origin
                || (!a.native && a.field.sensitive() != snapshot.password)
        });
        if target_changed {
            self.assist.queue = None;
            self.assist.password_rx = None;
            self.assist.access_typing = false;
            self.assist.permit_pending = false;
            self.assist.native_waiting = false;
            self.assist.native_suspended = false;
            self.assist.native_deadline = None;
            self.assist.generation = self.assist.generation.wrapping_add(1);
            self.assist.bridge.cancel(self.assist.generation);
        }
        if !native && self.assist.active.is_none() {
            self.assist.active = Some(Active {
                id: surface.id(),
                surface,
                field: Field::default(),
                native: false,
                accessible: None,
                bounds: None,
                origin: None,
                browser: self.assist.bridge_window.as_ref().is_some_and(|i| i.3),
            });
        }
        let Some(active) = self.assist.active.as_mut() else {
            return dirty;
        };
        // Some clients acknowledge text-input with no surrounding-text update. A fresh,
        // independently observed AT-SPI state may acknowledge exactly this edit, never a prediction.
        if active.native
            && !snapshot.password
            && self.assist.native_waiting
            && active.field.text.as_str() == snapshot.text.as_str()
            && active.field.cursor == snapshot.cursor
            && active.field.anchor == snapshot.anchor
        {
            self.assist.native_waiting = false;
            self.assist.native_suspended = false;
            self.assist.native_deadline = None;
        }
        let merge_text = self.assist.queue.is_none()
            && !self.assist.access_typing
            && (!active.native || !active.field.surrounding_reported || !active.field.caret_reported);
        let text_changed = merge_text
            && !snapshot.password
            && (active.field.text.as_str() != snapshot.text.as_str()
                || active.field.cursor != snapshot.cursor
                || active.field.anchor != snapshot.anchor);
        let changed = text_changed
            || active.bounds != Some(snapshot.field)
            || (!active.native && active.field.caret != snapshot.caret)
            || target_changed;
        active.bounds = Some(snapshot.field);
        active.origin = snapshot.origin;
        active.accessible = Some(snapshot.target);
        if active.field.caret.is_none() {
            active.field.caret = snapshot.caret;
        }
        if merge_text {
            if !active.native {
                active.field.purpose =
                    if snapshot.password { ContentPurpose::Password } else { ContentPurpose::Normal };
                active.field.caret = snapshot.caret;
            }
            if !snapshot.password {
                active.field.text = snapshot.text;
                active.field.cursor = snapshot.cursor;
                active.field.anchor = snapshot.anchor;
            }
        }
        if changed && self.assist.queue.is_none() && !self.assist.access_typing {
            self.assist.dismissed = false;
            self.assist.refresh();
            dirty = true;
        }
        dirty
    }
    pub fn poll_assist(&mut self) -> bool {
        if self.lock.is_locked() || self.assist.active.as_ref().is_some_and(|a| !a.surface.is_alive()) {
            let dirty = self.assist.active.is_some();
            self.assist.clear_focus();
            return dirty;
        }
        let mut dirty = self.poll_accessibility();
        if let Some(dictionary) = self.assist.dictionary_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.assist.dictionary = dictionary;
            self.assist.dictionary_rx = None;
            if self.assist.active.as_ref().is_some_and(|a| !a.field.sensitive()) {
                self.assist.refresh();
                dirty = true;
            }
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
                        Ok(_) => vec![Row::Info(aqua_i18n::tr("No passwords saved for this app").into())],
                        Err(()) => vec![Row::Info(aqua_i18n::tr("Password store unavailable").into())],
                    };
                    self.assist.selected = 0;
                }
                PasswordResult::Secret(generation, Ok(Some(secret)))
                    if generation == self.assist.generation && self.assist_available() =>
                {
                    self.queue_assist_text(secret);
                }
                PasswordResult::Secret(generation, _) if generation == self.assist.generation => {
                    self.assist.rows = vec![Row::Info(aqua_i18n::tr("Password was not supplied").into())];
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
        let Some(field) = self.assist.active.as_ref() else {
            return false;
        };
        // Browser credentials are bound to the verified document before every grapheme.
        if !field.use_native_delivery() {
            let Some(mut target) = field.accessible.clone() else {
                return false;
            };
            if !field.field.sensitive() {
                let f = &field.field;
                if f.cursor < 0
                    || f.anchor < 0
                    || !f.text.is_char_boundary(f.cursor as usize)
                    || !f.text.is_char_boundary(f.anchor as usize)
                {
                    return false;
                }
                target.caret = target.text_start + f.text[..f.cursor as usize].chars().count() as i32;
                target.anchor = target.text_start + f.text[..f.anchor as usize].chars().count() as i32;
            }
            let job = aqua_text::accessibility::Job {
                generation: self.assist.generation,
                target,
                text,
                replace_prefix: self.assist.replace_prefix.take(),
                origin: field.origin.clone(),
                expected_before: (!field.field.sensitive())
                    .then(|| Zeroizing::new(field.field.text[..field.field.cursor as usize].to_owned())),
                protected: field.field.sensitive(),
            };
            self.assist.access_typing = self.assist.bridge.type_text(job);
            self.assist.dismiss();
            self.needs_redraw = true;
            return self.assist.access_typing;
        }
        if !active {
            return false;
        }
        if field.browser && field.field.sensitive() && (field.origin.is_none() || field.accessible.is_none()) {
            return false;
        }
        self.assist.permit_pending = false;
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
        if self.assist.queue.is_none() {
            return false;
        }
        if self.assist.native_suspended {
            if self.assist.native_deadline.is_none_or(|deadline| std::time::Instant::now() >= deadline) {
                self.assist.clear_focus();
                return false;
            }
            return true;
        }
        if self.assist.native_waiting {
            if self.assist.native_deadline.is_none_or(|deadline| std::time::Instant::now() >= deadline) {
                self.assist.queue = None;
                self.assist.native_waiting = false;
                self.assist.native_suspended = false;
                self.assist.native_deadline = None;
                return false;
            }
            return true;
        }
        let requires_permit = self.assist.active.as_ref().is_some_and(|field| field.browser && field.field.sensitive());
        if requires_permit {
            if self.assist.permit_pending {
                let Some((generation, ok)) = self.assist.bridge.take_permit() else {
                    if self.assist.native_deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
                        self.assist.queue = None;
                        self.assist.permit_pending = false;
                        self.assist.native_deadline = None;
                        return false;
                    }
                    return true;
                };
                self.assist.permit_pending = false;
                self.assist.native_waiting = false;
                self.assist.native_suspended = false;
                self.assist.native_deadline = None;
                if generation != self.assist.generation {
                    return true;
                }
                if !ok {
                    self.assist.queue = None;
                    return false;
                }
            } else {
                let field = self.assist.active.as_ref().unwrap();
                let (Some(target), Some(origin)) = (&field.accessible, &field.origin) else {
                    self.assist.queue = None;
                    return false;
                };
                self.assist.permit_pending =
                    self.assist.bridge.request_permit(self.assist.generation, target.object.clone(), origin.clone());
                if !self.assist.permit_pending {
                    self.assist.queue = None;
                    return false;
                }
                self.assist.native_deadline = Some(std::time::Instant::now() + Duration::from_secs(2));
                return true;
            }
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
        let delete = self.assist.pending_delete.take();
        ti.with_active_text_input(|input, current_surface| {
            if input.id() == id && *current_surface == surface && input.is_alive() {
                if let Some(bytes) = delete {
                    input.delete_surrounding_text(bytes, 0);
                }
                input.commit_string(Some(part.to_string()));
                sent = true;
            }
        });
        if !sent {
            self.assist.queue = None;
            return false;
        }
        ti.done(false);
        // Protected browser fields may omit every text-input update for privacy. Their
        // next grapheme is authorized by a fresh focus/protection/origin permit instead.
        self.assist.native_waiting = !(active.browser && active.field.sensitive());
        self.assist.native_deadline =
            self.assist.native_waiting.then(|| std::time::Instant::now() + Duration::from_millis(500));
        self.assist.acknowledgements.remember(&active.field);
        active.field.expect_insertion(&part);
        self.assist.bridge.refresh();
        self.needs_redraw = true;
        if self.assist.queue.as_ref().is_some_and(Typing::finished) {
            self.assist.queue = None;
            return false;
        }
        true
    }
    /// Global placement anchor and field bounds. An unknown caret uses field bounds only for popup placement.
    pub fn assist_geometry(&self) -> Option<(Shape, Option<Shape>)> {
        let active = self.assist.active.as_ref()?;
        let caret = active.field.caret.or(active.bounds)?;
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
        Some((translate(caret), crate::wayland::text_field::geometry(surface).or(active.bounds).map(translate)))
    }
    #[allow(dead_code)] // Shared internal contract for future native overlay consumers.
    pub fn text_context(&self) -> Option<TextContext<'_>> {
        if self.assist.native_suspended {
            return None;
        }
        if self.lock.is_locked() {
            return None;
        }
        let a = self.assist.active.as_ref()?;
        let (caret, field) = self.assist_geometry()?;
        let sensitive = a.field.sensitive();
        Some(TextContext {
            surface: &a.surface,
            generation: self.assist.generation,
            purpose: a.field.purpose,
            hints: a.field.hint,
            sensitive,
            surrounding: (!sensitive && self.assist.queue.is_none() && !self.assist.access_typing)
                .then_some(a.field.text.as_str()),
            cursor: a.field.cursor,
            anchor: a.field.anchor,
            caret: a.field.caret.map(|local| Shape {
                x: local.x + caret.x - a.field.caret.or(a.bounds).unwrap().x,
                y: local.y + caret.y - a.field.caret.or(a.bounds).unwrap().y,
                ..local
            }),
            field,
            field_radius_known: crate::wayland::text_field::geometry(&a.surface).is_some(),
        })
    }
    pub fn assist_status(&self) -> String {
        let Some(active) = self.assist.active.as_ref() else {
            return "source=none".into();
        };
        let geometry = self.text_context();
        format!("source={} sensitive={} browser={} origin-verified={} caret-known={} field={:?} radius-known={} surrounding-bytes={} cursor={} rows={}",
            if active.native {"wayland"} else {"atspi"}, active.field.sensitive(), active.browser,
            active.origin.is_some(), active.field.caret.is_some(), geometry.as_ref().and_then(|g| g.field),
            geometry.is_some_and(|g| g.field_radius_known), active.field.text.len(), active.field.cursor, self.assist.rows.len())
    }
    pub fn assist_layer(&mut self) -> Option<Layer> {
        if !self.assist_available() || self.assist.rows.is_empty() {
            self.assist.popup = None;
            return None;
        }
        let (caret, field) = self.assist_geometry()?;
        let bounds = self
            .outputs
            .list
            .iter()
            .filter_map(|o| self.space.output_geometry(o))
            .find(|r| r.to_f64().contains((caret.x as f64, caret.y as f64)))?;
        let w = 256.0f32.min((bounds.size.w - 12).max(1) as f32);
        let h = 38.0 + self.assist.rows.len() as f32 * 30.0;
        let anchor = field.map(|f| Shape { x: caret.x, w: caret.w, ..f }).unwrap_or(caret);
        let rect = place_popup(
            anchor,
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
            let title = if password {
                format!(
                    "{} · {}",
                    aqua_i18n::tr("Passwords"),
                    self.assist.active.as_ref().and_then(|a| a.origin.as_deref()).unwrap_or(&app)
                )
            } else {
                aqua_i18n::tr("Suggestions · Tab to insert").into()
            };
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
        assist.native_suspended = true;
        assist.queue = Typing::new(Zeroizing::new("private".into()), generation);
        assist.clear_focus();
        assert!(assist.queue.is_none());
        assert!(!assist.native_suspended);
        assert_ne!(generation, assist.generation);
    }
    #[test]
    fn paced_insertion_replaces_utf8_selection() {
        let mut field = Field::default();
        field.apply(Delta { surrounding: Some((Zeroizing::new("привет".into()), 12, 6)), ..Default::default() });
        field.expect_insertion("ё");
        assert_eq!(field.text.as_str(), "приё");
        assert_eq!((field.cursor, field.anchor), (8, 8));
        field.expect_insertion("т");
        assert_eq!(field.text.as_str(), "приёт");
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
    #[test]
    fn acknowledgement_history_is_bounded_and_ignores_secrets() {
        let mut history = Acknowledgements::default();
        let mut field = Field::default();
        for n in 0..32 {
            field.text = Zeroizing::new(n.to_string());
            history.remember(&field);
        }
        assert_eq!(history.0.len(), 16);
        assert!(!history.contains(&(Zeroizing::new("0".into()), 0, 0)));
        assert!(history.contains(&(Zeroizing::new("31".into()), 0, 0)));
        field.purpose = ContentPurpose::Password;
        field.text = Zeroizing::new("never retain".into());
        history.remember(&field);
        assert!(!history.contains(&(field.text.clone(), 0, 0)));
        history.clear();
        assert!(history.0.is_empty());
    }

    #[test]
    fn invalid_caret_clears_previous_geometry_and_zero_width_is_valid() {
        let valid = Shape { x: 10.0, y: 20.0, w: 0.0, h: 18.0, radius: 0.0 }.validated_caret();
        assert!(valid.is_some());
        let mut field = Field::default();
        field.apply(Delta { caret: Some(valid), ..Default::default() });
        assert!(field.caret_reported);
        let invalid = Shape { w: -1.0, ..valid.unwrap() }.validated_caret();
        assert!(invalid.is_none());
        field.apply(Delta { caret: Some(invalid), ..Default::default() });
        assert!(!field.caret_reported);
        assert!(field.caret.is_none());
    }
    #[test]
    fn protected_reenable_requires_explicit_unchanged_sensitivity() {
        let mut field = Field::default();
        let same = Delta {
            enabled: Some(true),
            content: Some((ContentHint::empty(), ContentPurpose::Password)),
            ..Default::default()
        };
        assert!(!field.protected_reenable(&same));
        field.purpose = ContentPurpose::Password;
        assert!(field.protected_reenable(&same));
        assert!(!field.protected_reenable(&Delta { enabled: Some(true), ..Default::default() }));
        assert!(!field.protected_reenable(&Delta {
            enabled: Some(true),
            content: Some((ContentHint::empty(), ContentPurpose::Normal)),
            ..Default::default()
        }));
    }
}
