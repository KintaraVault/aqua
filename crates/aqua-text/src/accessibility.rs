//! Focus-scoped AT-SPI bridge. All D-Bus work and paced editing run off the compositor thread.
use crate::{Shape, Typing, MAX_TEXT};
use atspi_proxies::{
    accessible::AccessibleProxyBlocking,
    common::{CoordType, Role, State},
    component::ComponentProxyBlocking,
    editable_text::EditableTextProxyBlocking,
    registry::RegistryProxyBlocking,
    text::TextProxyBlocking,
};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};
use zbus::{
    blocking::{Connection, MessageIterator},
    message::Type,
    names::BusName,
};
use zeroize::Zeroizing;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Object {
    pub bus: String,
    pub path: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Focus {
    pub generation: u64,
    pub window: u64,
    pub pid: u32,
    pub title: String,
    pub browser: bool,
    pub protected: bool,
}
#[derive(Clone)]
pub struct Target {
    pub object: Object,
    pub caret: i32,
    pub anchor: i32,
    pub text_start: i32,
}
pub struct Snapshot {
    pub generation: u64,
    pub target: Target,
    pub text: Zeroizing<String>,
    pub cursor: i32,
    pub anchor: i32,
    pub password: bool,
    pub field: Shape,
    pub caret: Option<Shape>,
    pub origin: Option<String>,
}
pub struct Update {
    pub generation: u64,
    pub snapshot: Option<Snapshot>,
}
pub struct Job {
    pub generation: u64,
    pub target: Target,
    pub text: Zeroizing<String>,
    pub replace_prefix: Option<Zeroizing<String>>,
    pub origin: Option<String>,
    pub expected_before: Option<Zeroizing<String>>,
    pub protected: bool,
}
enum Command {
    Wake(Option<Object>),
    Type(Job),
    Permit { generation: u64, object: Object, origin: String },
}
enum Completion {
    Typing(u64, bool),
    Permit(u64, bool),
}
pub struct Bridge {
    tx: mpsc::SyncSender<Command>,
    focus: Arc<Mutex<Option<Focus>>>,
    guard: Arc<AtomicU64>,
    snapshot: Arc<Mutex<Option<Update>>>,
    done: Arc<Mutex<Option<Completion>>>,
    stop: Arc<AtomicBool>,
}
impl Default for Bridge {
    fn default() -> Self {
        Self::new()
    }
}
impl Bridge {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::sync_channel(32);
        let focus = Arc::new(Mutex::new(None));
        let guard = Arc::new(AtomicU64::new(0));
        let snapshot = Arc::new(Mutex::new(None));
        let done = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let halted = stop.clone();
        let (f, g, s, d, wake) = (focus.clone(), guard.clone(), snapshot.clone(), done.clone(), tx.clone());
        std::thread::Builder::new()
            .name("aqua-accessibility".into())
            .spawn(move || {
                // An absent or disabled accessibility bus is not an error in the compositor.
                let conn = loop {
                    if halted.load(Ordering::Acquire) {
                        return;
                    }
                    if let Ok(conn) = connect() {
                        break conn;
                    }
                    std::thread::sleep(Duration::from_secs(2));
                };
                watch(&conn, wake);
                run(conn.clone(), rx, f, g, s, d, halted);
                let _ = conn.close();
            })
            .ok();
        Self { tx, focus, guard, snapshot, done, stop }
    }
    pub fn cancel(&self, generation: u64) {
        self.guard.store(generation, Ordering::Release);
        if let Some(focus) = self.focus.lock().unwrap().as_mut() {
            focus.generation = generation;
        }
    }
    pub fn focus(&self, focus: Option<Focus>, generation: u64) {
        self.cancel(generation);
        let mut old = self.focus.lock().unwrap();
        if *old != focus {
            *old = focus;
            let _ = self.tx.try_send(Command::Wake(None));
        }
    }
    pub fn refresh(&self) {
        let _ = self.tx.try_send(Command::Wake(None));
    }
    pub fn take_snapshot(&self) -> Option<Update> {
        self.snapshot.lock().unwrap().take()
    }
    pub fn take_done(&self) -> Option<(u64, bool)> {
        let mut done = self.done.lock().unwrap();
        if !matches!(*done, Some(Completion::Typing(..))) {
            return None;
        }
        match done.take()? {
            Completion::Typing(generation, ok) => Some((generation, ok)),
            _ => None,
        }
    }
    pub fn take_permit(&self) -> Option<(u64, bool)> {
        let mut done = self.done.lock().unwrap();
        if !matches!(*done, Some(Completion::Permit(..))) {
            return None;
        }
        match done.take()? {
            Completion::Permit(generation, ok) => Some((generation, ok)),
            _ => None,
        }
    }
    pub fn request_permit(&self, generation: u64, object: Object, origin: String) -> bool {
        self.tx.try_send(Command::Permit { generation, object, origin }).is_ok()
    }
    pub fn type_text(&self, job: Job) -> bool {
        self.tx.try_send(Command::Type(job)).is_ok()
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.guard.fetch_add(1, Ordering::AcqRel);
        let _ = self.tx.try_send(Command::Wake(None));
    }
}
fn connect() -> zbus::Result<Connection> {
    let session = Connection::session()?;
    let bus = zbus::blocking::Proxy::new(&session, "org.a11y.Bus", "/org/a11y/bus", "org.a11y.Bus")?;
    let address: String = bus.call("GetAddress", &())?;
    zbus::blocking::connection::Builder::address(address.as_str())?
        .method_timeout(Duration::from_millis(300))
        .max_queued(64)
        .build()
}
fn watch(conn: &Connection, tx: mpsc::SyncSender<Command>) {
    if let Ok(registry) = RegistryProxyBlocking::new(conn) {
        for event in
            ["object:state-changed:focused", "object:text-caret-moved", "object:text-changed", "object:bounds-changed"]
        {
            let _ = registry.register_event(event);
        }
    }
    let conn = conn.clone();
    std::thread::Builder::new()
        .name("aqua-accessibility-events".into())
        .spawn(move || {
            let rule = zbus::MatchRule::builder().msg_type(Type::Signal).build();
            let Ok(messages) = MessageIterator::for_match_rule(rule, &conn, Some(32)) else { return };
            for message in messages.flatten() {
                let header = message.header();
                if !header.interface().is_some_and(|i| i.as_str() == "org.a11y.atspi.Event.Object") {
                    continue;
                }
                let Some(member) = header.member() else { continue };
                if !["StateChanged", "TextCaretMoved", "TextChanged", "BoundsChanged"].contains(&member.as_str()) {
                    continue;
                }
                let (Some(bus), Some(path)) = (header.sender(), header.path()) else { continue };
                // Never deserialize event payloads: they may contain protected text.
                let _ = tx.try_send(Command::Wake(Some(Object { bus: bus.to_string(), path: path.to_string() })));
            }
        })
        .ok();
}
fn accessible<'a>(conn: &'a Connection, obj: &'a Object) -> zbus::Result<AccessibleProxyBlocking<'a>> {
    AccessibleProxyBlocking::builder(conn)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .destination(obj.bus.as_str())?
        .path(obj.path.as_str())?
        .build()
}
fn text<'a>(conn: &'a Connection, obj: &'a Object) -> zbus::Result<TextProxyBlocking<'a>> {
    TextProxyBlocking::builder(conn)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .destination(obj.bus.as_str())?
        .path(obj.path.as_str())?
        .build()
}
fn object(reference: atspi_proxies::common::object_ref::ObjectRefOwned) -> Option<Object> {
    Some(Object { bus: reference.name()?.to_string(), path: reference.path().to_string() })
}
fn owner_pid(conn: &Connection, bus: &str) -> zbus::Result<u32> {
    let proxy = zbus::blocking::fdo::DBusProxy::new(conn)?;
    Ok(proxy.get_connection_unix_process_id(BusName::try_from(bus)?)?)
}
fn belongs_to_process(conn: &Connection, bus: &str, focus: &Focus) -> bool {
    let Ok(mut pid) = owner_pid(conn, bus) else { return false };
    for _ in 0..16 {
        if pid == focus.pid {
            return true;
        }
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else { return false };
        let Some(rest) = stat.rsplit_once(") ").map(|(_, rest)| rest) else { return false };
        let Some(parent) = rest.split_whitespace().nth(1).and_then(|s| s.parse::<u32>().ok()) else { return false };
        if parent == 0 || parent == pid {
            return false;
        }
        pid = parent;
    }
    false
}
fn focused_editor(conn: &Connection, obj: &Object, focus: &Focus) -> zbus::Result<bool> {
    if !belongs_to_process(conn, &obj.bus, focus) {
        return Ok(false);
    }
    let proxy = accessible(conn, obj)?;
    let state = proxy.get_state()?;
    let role = proxy.get_role()?;
    if !state.contains(State::Focused)
        || !state.contains(State::Editable)
        || !state.contains(State::Enabled)
        || state.contains(State::Defunct)
        || !matches!(role, Role::Entry | Role::Text | Role::PasswordText)
    {
        return Ok(false);
    }
    let mut ancestor = obj.clone();
    for _ in 0..24 {
        let node = accessible(conn, &ancestor)?;
        if matches!(node.get_role()?, Role::Frame | Role::Dialog | Role::Window) {
            // Window-local coordinates are meaningful only for the focused compositor window.
            return Ok(!focus.title.is_empty()
                && node.name()? == focus.title
                && node.get_state()?.contains(State::Active));
        }
        let Some(parent) = object(node.parent()?) else { break };
        if parent == ancestor || !belongs_to_process(conn, &parent.bus, focus) {
            break;
        }
        drop(node);
        ancestor = parent;
    }
    Ok(false)
}
fn discover(conn: &Connection, focus: &Focus, guard: &AtomicU64) -> Option<Object> {
    let root = AccessibleProxyBlocking::builder(conn)
        .destination("org.a11y.atspi.Registry")
        .ok()?
        .path("/org/a11y/atspi/accessible/root")
        .ok()?
        .build()
        .ok()?;
    let apps = root.get_children().ok()?;
    let mut queue = VecDeque::new();
    for app in apps.into_iter().take(128).filter_map(object) {
        if belongs_to_process(conn, &app.bus, focus) {
            queue.push_back((app, 0));
        }
    }
    let deadline = Instant::now() + Duration::from_millis(500);
    let mut visited = std::collections::HashSet::new();
    while let Some((node, depth)) = queue.pop_back() {
        if visited.len() >= 2048 || Instant::now() > deadline || guard.load(Ordering::Acquire) != focus.generation {
            break;
        }
        if !visited.insert((node.bus.clone(), node.path.clone())) {
            continue;
        }
        if focused_editor(conn, &node, focus).ok() == Some(true) {
            return Some(node);
        }
        if depth < 16 {
            if let Ok(children) = accessible(conn, &node).and_then(|a| a.get_children()) {
                queue.extend(children.into_iter().take(128).filter_map(object).map(|o| (o, depth + 1)));
            }
        }
    }
    None
}
fn shape(rect: (i32, i32, i32, i32)) -> Option<Shape> {
    Shape { x: rect.0 as f32, y: rect.1 as f32, w: rect.2 as f32, h: rect.3 as f32, radius: 0.0 }.validated()
}
fn document_origin(conn: &Connection, obj: &Object) -> Option<String> {
    let mut ancestor = obj.clone();
    let mut origins = Vec::new();
    for _ in 0..24 {
        let node = accessible(conn, &ancestor).ok()?;
        let role = node.get_role().ok()?;
        if matches!(role, Role::Frame | Role::Dialog | Role::Window) {
            break;
        }
        if matches!(role, Role::DocumentWeb | Role::DocumentFrame) {
            let document = atspi_proxies::document::DocumentProxyBlocking::builder(conn)
                .destination(ancestor.bus.as_str())
                .ok()?
                .path(ancestor.path.as_str())
                .ok()?
                .build()
                .ok()?;
            let attributes = document.get_attributes().ok()?;
            let value = consistent_origins(
                ["DocURL", "doc-url", "document-uri", "uri", "URI", "URL"]
                    .iter()
                    .filter_map(|key| attributes.get(*key))
                    .map(String::as_str),
            )?;
            origins.push(value);
        }
        let Some(parent) = object(node.parent().ok()?) else { break };
        if parent == ancestor {
            break;
        }
        drop(node);
        ancestor = parent;
    }
    // Cross-origin embedded frames require user/browser integration; never guess a top-level origin.
    consistent_origins(origins.iter().map(String::as_str))
}
fn consistent_origins<'a>(values: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let mut values = values.into_iter();
    let first = canonical_origin(values.next()?)?;
    values.all(|value| canonical_origin(value).as_ref() == Some(&first)).then_some(first)
}
/// Credential origins require the browser's own accessibility connection and a system-owned binary.
/// Descendant processes may describe ordinary fields, but cannot assert website credential scope.
fn verified_document_origin(conn: &Connection, obj: &Object, focus: &Focus) -> Option<String> {
    if !focus.browser || owner_pid(conn, &obj.bus).ok()? != focus.pid || !trusted_browser_binary(focus.pid) {
        return None;
    }
    document_origin(conn, obj)
}
fn trusted_browser_binary(pid: u32) -> bool {
    use std::os::unix::fs::MetadataExt;
    let process = format!("/proc/{pid}/exe");
    let Ok(path) = std::fs::read_link(&process) else {
        return false;
    };
    if !matches!(
        path.to_str(),
        Some(
            "/usr/lib/chromium/chromium"
                | "/usr/lib/firefox/firefox"
                | "/usr/lib/firefox/firefox-bin"
                | "/opt/google/chrome/chrome"
                | "/opt/brave.com/brave/brave"
                | "/opt/vivaldi/vivaldi-bin"
                | "/opt/microsoft/msedge/msedge"
                | "/usr/bin/epiphany"
        )
    ) {
        return false;
    }
    let (Ok(running), Ok(installed)) = (std::fs::metadata(&process), std::fs::metadata(&path)) else {
        return false;
    };
    if running.dev() != installed.dev() || running.ino() != installed.ino() {
        return false;
    }
    path.ancestors().all(|part| {
        std::fs::symlink_metadata(part)
            .is_ok_and(|meta| !meta.file_type().is_symlink() && meta.uid() == 0 && meta.mode() & 0o022 == 0)
    })
}
pub fn canonical_origin(value: &str) -> Option<String> {
    let url = url::Url::parse(value).ok()?;
    if url.scheme() != "https" || url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    Some(url.origin().ascii_serialization())
}
pub fn is_browser_executable(exe: &str) -> bool {
    matches!(
        exe,
        "firefox"
            | "firefox-bin"
            | "chromium"
            | "chrome"
            | "google-chrome"
            | "brave"
            | "vivaldi-bin"
            | "epiphany"
            | "msedge"
    )
}
/// Chromium may own its Wayland socket in a GPU child, while AT-SPI belongs to the main process.
pub fn browser_process_root(mut pid: u32) -> Option<u32> {
    let executable = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    if !is_browser_executable(executable.file_name()?.to_str()?) {
        return None;
    }
    for _ in 0..16 {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let parent = stat.rsplit_once(") ")?.1.split_whitespace().nth(1)?.parse::<u32>().ok()?;
        if parent == 0
            || parent == pid
            || std::fs::read_link(format!("/proc/{parent}/exe")).ok().as_ref() != Some(&executable)
        {
            return Some(pid);
        }
        pid = parent;
    }
    None
}
fn protected(conn: &Connection, obj: &Object) -> Option<bool> {
    let node = accessible(conn, obj).ok()?;
    let attrs = node.get_attributes().ok()?;
    Some(
        node.get_role().ok()? == Role::PasswordText
            || attrs.get("type").is_some_and(|v| v == "password")
            || attrs.get("password").is_some_and(|v| v == "true")
            || attrs.get("protected").is_some_and(|v| v == "true")
            || attrs.get("sensitive").is_some_and(|v| v == "true"),
    )
}
fn read_snapshot(conn: &Connection, obj: &Object, focus: &Focus) -> Option<Snapshot> {
    if !focused_editor(conn, obj, focus).ok()? {
        return None;
    }
    let accessible = accessible(conn, obj).ok()?;
    let role = accessible.get_role().ok()?;
    let attrs = accessible.get_attributes().ok()?;
    let password = focus.protected
        || role == Role::PasswordText
        || attrs.get("type").is_some_and(|v| v == "password")
        || attrs.get("password").is_some_and(|v| v == "true")
        || attrs.get("protected").is_some_and(|v| v == "true")
        || attrs.get("sensitive").is_some_and(|v| v == "true");
    let component = ComponentProxyBlocking::builder(conn)
        .destination(obj.bus.as_str())
        .ok()?
        .path(obj.path.as_str())
        .ok()?
        .build()
        .ok()?;
    let field = shape(component.get_extents(CoordType::Window).ok()?)?;
    let t = text(conn, obj).ok()?;
    let caret = t.caret_offset().ok()?;
    if caret < 0 {
        return None;
    }
    let selection = match t.get_n_selections().ok()? {
        0 => (caret, caret),
        1 => t.get_selection(0).ok()?,
        _ => return None,
    };
    let anchor = if selection.0 == caret {
        selection.1
    } else if selection.1 == caret {
        selection.0
    } else {
        return None;
    };
    if anchor < 0 {
        return None;
    }
    let mut snapshot = Snapshot {
        generation: focus.generation,
        target: Target { object: obj.clone(), caret, anchor, text_start: 0 },
        text: Zeroizing::new(String::new()),
        cursor: 0,
        anchor: 0,
        password,
        field,
        caret: t.get_character_extents(caret, CoordType::Window).ok().and_then(|(x, y, w, h)| {
            Shape { x: x as f32, y: y as f32, w: w as f32, h: h as f32, radius: 0.0 }.validated_caret()
        }),
        origin: if focus.browser { verified_document_origin(conn, obj, focus) } else { None },
    };
    if !password {
        let count = t.character_count().ok()?;
        if count < caret || count < anchor {
            return None;
        }
        let lo = (caret.min(anchor) - 512).max(0);
        let hi = (caret.max(anchor) + 512).min(count);
        if hi - lo > 1024 {
            return None;
        }
        let surrounding = Zeroizing::new(t.get_text(lo, hi).ok()?);
        if surrounding.len() > MAX_TEXT {
            return None;
        }
        snapshot.cursor = char_to_byte(&surrounding, caret - lo)? as i32;
        snapshot.anchor = char_to_byte(&surrounding, anchor - lo)? as i32;
        snapshot.target.text_start = lo;
        snapshot.text = surrounding;
    }
    Some(snapshot)
}
pub fn char_to_byte(text: &str, offset: i32) -> Option<usize> {
    if offset < 0 {
        return None;
    }
    if offset as usize == text.chars().count() {
        return Some(text.len());
    }
    text.char_indices().nth(offset as usize).map(|(byte, _)| byte)
}
fn run(
    conn: Connection,
    rx: mpsc::Receiver<Command>,
    focus: Arc<Mutex<Option<Focus>>>,
    guard: Arc<AtomicU64>,
    snapshot: Arc<Mutex<Option<Update>>>,
    done: Arc<Mutex<Option<Completion>>>,
    stop: Arc<AtomicBool>,
) {
    let mut active: Option<Object> = None;
    let mut last_window = None;
    let mut discover_after = Instant::now();
    while let Ok(command) = match rx.recv_timeout(Duration::from_millis(200)) {
        Ok(command) => Ok(command),
        Err(mpsc::RecvTimeoutError::Timeout) => Ok(Command::Wake(None)),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(()),
    } {
        if stop.load(Ordering::Acquire) {
            break;
        }
        let expected = focus.lock().unwrap().clone();
        let Some(expected) = expected else {
            active = None;
            *snapshot.lock().unwrap() = None;
            continue;
        };
        if guard.load(Ordering::Acquire) != expected.generation {
            continue;
        }
        let key = (expected.window, expected.pid, expected.title.clone());
        if last_window.as_ref() != Some(&key) {
            active = None;
            last_window = Some(key);
            discover_after = Instant::now();
        }
        match command {
            Command::Type(job) => {
                let ok = edit_job(&conn, &job, &expected, &guard).unwrap_or(false);
                *done.lock().unwrap() = Some(Completion::Typing(job.generation, ok));
            }
            Command::Wake(Some(candidate)) => {
                if focused_editor(&conn, &candidate, &expected).unwrap_or(false) {
                    active = Some(candidate);
                }
            }
            Command::Wake(None) => {}
            Command::Permit { generation, object, origin } => {
                let ok = generation == expected.generation
                    && guard.load(Ordering::Acquire) == generation
                    && focused_editor(&conn, &object, &expected).unwrap_or(false)
                    && protected(&conn, &object) == Some(true)
                    && verified_document_origin(&conn, &object, &expected).as_ref() == Some(&origin)
                    && guard.load(Ordering::Acquire) == generation;
                *done.lock().unwrap() = Some(Completion::Permit(generation, ok));
            }
        }
        if let Some(obj) = active.as_ref() {
            if !focused_editor(&conn, obj, &expected).unwrap_or(false) {
                active = None;
            }
        }
        if active.is_none() && Instant::now() >= discover_after {
            active = discover(&conn, &expected, &guard);
            discover_after = Instant::now() + Duration::from_secs(1);
        }
        let value = active.as_ref().and_then(|obj| read_snapshot(&conn, obj, &expected));
        if guard.load(Ordering::Acquire) == expected.generation {
            *snapshot.lock().unwrap() = Some(Update { generation: expected.generation, snapshot: value });
        }
    }
}
fn edit_job(conn: &Connection, job: &Job, focus: &Focus, guard: &AtomicU64) -> zbus::Result<bool> {
    let valid = || {
        guard.load(Ordering::Acquire) == job.generation
            && focus.generation == job.generation
            && focused_editor(conn, &job.target.object, focus).unwrap_or(false)
            && protected(conn, &job.target.object).is_some_and(|password| password == job.protected)
            && (!focus.browser
                || (!job.protected && job.origin.is_none())
                || job.origin.as_ref().is_some_and(|origin| {
                    verified_document_origin(conn, &job.target.object, focus).as_ref() == Some(origin)
                }))
            && guard.load(Ordering::Acquire) == job.generation
    };
    if !valid() {
        return Ok(false);
    }
    let t = text(conn, &job.target.object)?;
    let e = EditableTextProxyBlocking::builder(conn)
        .destination(job.target.object.bus.as_str())?
        .path(job.target.object.path.as_str())?
        .build()?;
    let mut caret = job.target.caret;
    if t.caret_offset()? != caret {
        return Ok(false);
    }
    let selection = match t.get_n_selections()? {
        0 => (caret, caret),
        1 => t.get_selection(0)?,
        _ => return Ok(false),
    };
    if selection.0.min(selection.1) != caret.min(job.target.anchor)
        || selection.0.max(selection.1) != caret.max(job.target.anchor)
    {
        return Ok(false);
    }
    if let Some(expected) = &job.expected_before {
        let node = accessible(conn, &job.target.object)?;
        let attrs = node.get_attributes()?;
        if node.get_role()? == Role::PasswordText
            || attrs.get("type").is_some_and(|v| v == "password")
            || attrs.get("password").is_some_and(|v| v == "true")
        {
            return Ok(false);
        }
        let begin = caret - expected.chars().count() as i32;
        if begin < 0 {
            return Ok(false);
        }
        let actual = Zeroizing::new(t.get_text(begin, caret)?);
        if actual.as_str() != expected.as_str() || !valid() {
            return Ok(false);
        }
    }
    if let Some(prefix) = &job.replace_prefix {
        let node = accessible(conn, &job.target.object)?;
        let attrs = node.get_attributes()?;
        if node.get_role()? == Role::PasswordText
            || attrs.get("type").is_some_and(|v| v == "password")
            || attrs.get("password").is_some_and(|v| v == "true")
        {
            return Ok(false);
        }
        if caret != job.target.anchor {
            return Ok(false);
        }
        let begin = caret - prefix.chars().count() as i32;
        if begin < 0 {
            return Ok(false);
        }
        let actual = Zeroizing::new(t.get_text(begin, caret)?);
        if actual.as_str() != prefix.as_str() || !valid() || !e.delete_text(begin, caret)? {
            return Ok(false);
        }
        caret = begin;
        if !valid() || !t.set_caret_offset(caret)? {
            return Ok(false);
        }
    } else if caret != job.target.anchor {
        let lo = caret.min(job.target.anchor);
        let hi = caret.max(job.target.anchor);
        if !valid() || !e.delete_text(lo, hi)? {
            return Ok(false);
        }
        caret = lo;
        if !valid() || !t.set_caret_offset(caret)? {
            return Ok(false);
        }
    }
    let Some(mut typing) = Typing::new(job.text.clone(), job.generation) else { return Ok(false) };
    while let Some(part) = typing.next_grapheme() {
        if !valid() || t.caret_offset()? != caret {
            return Ok(false);
        }
        let selected = t.get_n_selections()?;
        if selected > 1 || selected == 1 && t.get_selection(0)? != (caret, caret) {
            return Ok(false);
        }
        let inserted = e.insert_text(caret, &part, part.len() as i32)?;
        if !inserted {
            return Ok(false);
        }
        caret += part.chars().count() as i32;
        if !valid() || !t.set_caret_offset(caret)? {
            return Ok(false);
        }
        if !typing.finished() {
            std::thread::sleep(Duration::from_millis(8));
        }
    }
    Ok(true)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_updates_worker_focus_before_a_new_job() {
        let (tx, _rx) = mpsc::sync_channel(1);
        let bridge = Bridge {
            tx,
            focus: Arc::new(Mutex::new(Some(Focus {
                generation: 1,
                window: 2,
                pid: 3,
                title: "test".into(),
                browser: false,
                protected: false,
            }))),
            guard: Arc::new(AtomicU64::new(1)),
            snapshot: Arc::new(Mutex::new(None)),
            done: Arc::new(Mutex::new(None)),
            stop: Arc::new(AtomicBool::new(false)),
        };
        bridge.cancel(2);
        assert_eq!(bridge.guard.load(Ordering::Acquire), 2);
        assert_eq!(bridge.focus.lock().unwrap().as_ref().unwrap().generation, 2);
    }
    #[test]
    fn codepoint_offsets_are_converted_to_bytes() {
        assert_eq!(char_to_byte("привет👋", 3), Some(6));
        assert_eq!(char_to_byte("привет👋", 7), Some(16));
        assert_eq!(char_to_byte("привет", -1), None);
        assert_eq!(char_to_byte("привет", 7), None);
    }
    #[test]
    fn origins_are_exact_and_require_https() {
        assert_eq!(canonical_origin("https://EXAMPLE.org:443/login?q=1"), Some("https://example.org".into()));
        assert_eq!(canonical_origin("https://example.org:8443/login"), Some("https://example.org:8443".into()));
        for invalid in
            ["http://example.org", "https://user@example.org", "file:///tmp/a", "https://example.org.evil.invalid"]
        {
            if invalid.ends_with("evil.invalid") {
                assert_ne!(canonical_origin(invalid), canonical_origin("https://example.org"));
            } else {
                assert!(canonical_origin(invalid).is_none());
            }
        }
    }
    #[test]
    fn unknown_executables_are_not_browser_identity() {
        assert!(is_browser_executable("firefox"));
        assert!(!is_browser_executable("firefox-lookalike"));
    }
    #[test]
    fn document_scopes_require_consensus_and_never_invent_an_origin() {
        assert!(consistent_origins([]).is_none());
        assert_eq!(
            consistent_origins(["https://EXAMPLE.org/login", "https://example.org:443/frame"]),
            Some("https://example.org".into())
        );
        assert!(consistent_origins(["https://example.org", "https://other.org"]).is_none());
        assert!(consistent_origins(["https://example.org", "https://example.org:8443"]).is_none());
        assert!(consistent_origins(["https://example.org", "http://example.org"]).is_none());
    }
}
