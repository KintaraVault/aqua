//! `org.freedesktop.impl.portal.*` backend interfaces for xdg-desktop-portal.
//!
//! * Settings — appearance (colour scheme, accent);
//! * FileChooser — OpenFile/SaveFile/SaveFiles through the Finder-style `aqua-filechooser`;
//! * Screenshot — Screenshot/PickColor rendered by the compositor (see [`take_requests`]);
//! * Inhibit — idle inhibition (queried by the compositor through [`inhibited`]).
//! * ScreenCast — monitor streams over PipeWire produced by the compositor (see
//!   [`RequestKind::ScreenCast`]); RemoteDesktop is delegated to xdg-desktop-portal-wlr via
//!   `aqua-portals.conf` (the compositor implements ext-image-copy-capture / wlr-screencopy).
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

type Opts = HashMap<String, OwnedValue>;
type Results = (u32, HashMap<String, OwnedValue>);

fn ov<'a>(v: impl Into<Value<'a>>) -> OwnedValue {
    v.into().try_to_owned().unwrap_or_else(|_| OwnedValue::from(0u32))
}
fn opt_bool(o: &Opts, k: &str) -> bool {
    o.get(k).and_then(|v| bool::try_from(v.clone()).ok()).unwrap_or(false)
}
fn opt_str(o: &Opts, k: &str) -> Option<String> {
    o.get(k).and_then(|v| String::try_from(v.clone()).ok())
}
fn opt_u32(o: &Opts, k: &str) -> Option<u32> {
    o.get(k).and_then(|v| u32::try_from(v.clone()).ok())
}
fn opt_bytes_path(o: &Opts, k: &str) -> Option<String> {
    let v = o.get(k)?;
    let b: Vec<u8> = Vec::<u8>::try_from(v.clone()).ok()?;
    let b: Vec<u8> = b.into_iter().take_while(|c| *c != 0).collect();
    String::from_utf8(b).ok().filter(|s| !s.is_empty())
}

pub struct ImplSettings {
    pub dark: Arc<AtomicBool>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Settings")]
impl ImplSettings {
    fn read_all(&self, namespaces: Vec<String>) -> HashMap<String, HashMap<String, OwnedValue>> {
        let s = crate::portal::Settings { dark: self.dark.clone() };
        let mut out = HashMap::new();
        let ns = "org.freedesktop.appearance";
        if namespaces.is_empty()
            || namespaces
                .iter()
                .any(|p| p == ns || p.is_empty() || (p.ends_with('*') && ns.starts_with(p.trim_end_matches('*'))))
        {
            out.insert(ns.to_string(), s.values());
        }
        let wm = crate::portal::WM_NS;
        if namespaces.is_empty()
            || namespaces
                .iter()
                .any(|p| p == wm || p.is_empty() || (p.ends_with('*') && wm.starts_with(p.trim_end_matches('*'))))
        {
            out.insert(wm.to_string(), s.wm_values());
        }
        let ifc = crate::portal::IF_NS;
        if namespaces.is_empty()
            || namespaces
                .iter()
                .any(|p| p == ifc || p.is_empty() || (p.ends_with('*') && ifc.starts_with(p.trim_end_matches('*'))))
        {
            out.insert(ifc.to_string(), s.interface_values());
        }
        out
    }
    fn read(&self, namespace: &str, key: &str) -> zbus::fdo::Result<OwnedValue> {
        crate::portal::Settings { dark: self.dark.clone() }.lookup(namespace, key)
    }
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}

pub struct FileChooser;

fn chooser_bin() -> String {
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) {
        let p = dir.join("aqua-filechooser");
        if p.exists() {
            return p.to_string_lossy().into_owned();
        }
    }
    "aqua-filechooser".into()
}

/// Glob patterns for a MIME type (`image/*` included), from shared-mime-info when available.
fn mime_glob(m: &str) -> String {
    static DB: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let db = DB.get_or_init(|| {
        let mut dirs = vec![std::path::PathBuf::from("/usr/share/mime/globs2")];
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
        if let Some(d) = data {
            dirs.insert(0, d.join("mime/globs2"));
        }
        dirs.iter().filter_map(|p| std::fs::read_to_string(p).ok()).collect::<Vec<_>>().join("\n")
    });
    mime_glob_from(db, m)
}

/// [`mime_glob`] against the contents of a `globs2` file (`weight:type:glob` lines).
fn mime_glob_from(db: &str, m: &str) -> String {
    let prefix = m.strip_suffix('*');
    let mut globs: Vec<&str> = vec![];
    for l in db.lines().filter(|l| !l.starts_with('#')) {
        let mut it = l.splitn(4, ':');
        let (Some(_), Some(ty), Some(glob)) = (it.next(), it.next(), it.next()) else { continue };
        let hit = match prefix {
            Some(p) => ty.starts_with(p),
            None => ty == m,
        };
        if hit && glob.starts_with("*.") && !globs.contains(&glob) {
            globs.push(glob);
        }
    }
    if !globs.is_empty() {
        return globs.join(";");
    }
    if prefix.is_some() {
        return "*".into();
    }
    let sub = m.rsplit('/').next().unwrap_or(m).trim_start_matches("x-");
    match sub {
        "jpeg" => "*.jpg;*.jpeg".into(),
        "plain" => "*.txt".into(),
        "svg+xml" => "*.svg".into(),
        "markdown" => "*.md".into(),
        s => format!("*.{s}"),
    }
}

fn chooser_args(title: &str, o: &Opts, save: bool) -> Vec<String> {
    let mut a = vec!["--title".to_string(), title.to_string()];
    if save {
        a.push("--save".into());
    }
    if opt_bool(o, "multiple") {
        a.push("--multiple".into());
    }
    if opt_bool(o, "directory") {
        a.push("--directory".into());
    }
    if let Some(l) = opt_str(o, "accept_label") {
        a.extend(["--accept-label".into(), l]);
    }
    if let Some(n) = opt_str(o, "current_name") {
        a.extend(["--name".into(), n]);
    }
    if let Some(f) = opt_bytes_path(o, "current_folder") {
        a.extend(["--folder".into(), f]);
    } else if let Some(f) = opt_bytes_path(o, "current_file") {
        let p = PathBuf::from(&f);
        if let Some(d) = p.parent() {
            a.extend(["--folder".into(), d.to_string_lossy().into_owned()]);
        }
        if let Some(n) = p.file_name() {
            a.extend(["--name".into(), n.to_string_lossy().into_owned()]);
        }
    }
    type Filter = (String, Vec<(u32, String)>);
    let filters: Vec<Filter> =
        o.get("filters").and_then(|v| Vec::<Filter>::try_from(v.clone()).ok()).unwrap_or_default();
    let current: Option<Filter> = o.get("current_filter").and_then(|v| Filter::try_from(v.clone()).ok());
    let mut ordered = filters.clone();
    if let Some(c) = current {
        ordered.retain(|f| f.0 != c.0);
        ordered.insert(0, c);
    }
    for (name, pats) in ordered {
        let globs: Vec<String> =
            pats.iter().map(|(kind, p)| if *kind == 1 { mime_glob(p) } else { p.clone() }).collect();
        a.extend(["--filter".into(), format!("{name}:{}", globs.join(";"))]);
    }
    a
}

async fn run_chooser(args: Vec<String>) -> Results {
    let out = blocking::unblock(move || {
        std::process::Command::new(chooser_bin()).args(&args).stderr(std::process::Stdio::null()).output()
    })
    .await;
    match out {
        Ok(o) if o.status.success() => {
            let uris: Vec<String> =
                String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.is_empty()).map(file_uri).collect();
            let mut r = HashMap::new();
            r.insert("uris".to_string(), ov(uris));
            (0, r)
        }
        Ok(o) if o.status.code() == Some(1) => (1, HashMap::new()),
        Ok(_) => (2, HashMap::new()),
        Err(e) => {
            tracing::warn!("aqua-filechooser failed: {e}");
            (2, HashMap::new())
        }
    }
}

fn file_uri(p: &str) -> String {
    let mut s = String::from("file://");
    for b in p.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

#[zbus::interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl FileChooser {
    async fn open_file(
        &self,
        _handle: OwnedObjectPath,
        _app_id: String,
        _parent_window: String,
        title: String,
        options: Opts,
    ) -> Results {
        run_chooser(chooser_args(&title, &options, false)).await
    }
    async fn save_file(
        &self,
        _handle: OwnedObjectPath,
        _app_id: String,
        _parent_window: String,
        title: String,
        options: Opts,
    ) -> Results {
        run_chooser(chooser_args(&title, &options, true)).await
    }
    /// Save several files into one folder: ask for the folder, return one URI per file name.
    async fn save_files(
        &self,
        _handle: OwnedObjectPath,
        _app_id: String,
        _parent_window: String,
        title: String,
        options: Opts,
    ) -> Results {
        let mut o = options.clone();
        o.insert("directory".into(), OwnedValue::from(true));
        let (code, r) = run_chooser(chooser_args(&title, &o, false)).await;
        if code != 0 {
            return (code, r);
        }
        let dir: String = r
            .get("uris")
            .and_then(|v| Vec::<String>::try_from(v.clone()).ok())
            .and_then(|v| v.into_iter().next())
            .unwrap_or_default();
        let files: Vec<Vec<u8>> =
            options.get("files").and_then(|v| Vec::<Vec<u8>>::try_from(v.clone()).ok()).unwrap_or_default();
        let uris: Vec<String> = files
            .iter()
            .map(|f| {
                format!(
                    "{}/{}",
                    dir.trim_end_matches('/'),
                    String::from_utf8_lossy(&f.iter().copied().take_while(|c| *c != 0).collect::<Vec<u8>>())
                )
            })
            .collect();
        let mut out = HashMap::new();
        out.insert("uris".to_string(), ov(uris));
        (0, out)
    }
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        4
    }
}

pub enum Reply {
    Saved(PathBuf),
    Color(f64, f64, f64),
    /// Screen cast streams that were started.
    Cast(Vec<CastStream>),
    Done,
}

/// One PipeWire stream of a screen cast session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CastStream {
    pub node: u32,
    /// Logical position and size of the shared monitor.
    pub position: (i32, i32),
    pub size: (i32, i32),
}

/// `SourceType` monitor.
pub const SOURCE_MONITOR: u32 = 1;
/// Cursor modes: hidden, embedded (drawn into the frames).
pub const CURSOR_HIDDEN: u32 = 1;
pub const CURSOR_EMBEDDED: u32 = 2;
pub enum RequestKind {
    /// Render the screen into this PNG.
    Screenshot(PathBuf),
    /// Colour under the pointer.
    PickColor,
    /// Start streaming a monitor for screen cast `session` (object path) of `app_id`.
    ScreenCast { session: String, app_id: String, cursor: bool },
    /// The screen cast session was closed.
    StopCast(String),
}
/// A request the compositor has to fulfil on its own thread.
pub struct Request {
    pub kind: RequestKind,
    pub reply: Sender<Result<Reply, String>>,
}

static REQUESTS: Mutex<Vec<Request>> = Mutex::new(Vec::new());

/// Pending screenshot / colour requests (polled by the compositor).
pub fn take_requests() -> Vec<Request> {
    std::mem::take(&mut *REQUESTS.lock().unwrap())
}

async fn ask(kind: RequestKind) -> Result<Reply, String> {
    let (tx, rx) = channel();
    REQUESTS.lock().unwrap().push(Request { kind, reply: tx });
    blocking::unblock(move || {
        rx.recv_timeout(std::time::Duration::from_secs(10)).map_err(|e| e.to_string()).and_then(|r| r)
    })
    .await
}

pub struct Screenshot;

fn shot_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let pics = std::process::Command::new("xdg-user-dir")
        .arg("PICTURES")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty() && s != &home)
        .unwrap_or_else(|| format!("{home}/Pictures"));
    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    PathBuf::from(pics).join("Screenshots").join(format!("Screenshot-{ts}.png"))
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Screenshot")]
impl Screenshot {
    async fn screenshot(
        &self,
        _handle: OwnedObjectPath,
        app_id: String,
        _parent_window: String,
        _options: Opts,
    ) -> Results {
        tracing::info!("portal screenshot for {app_id:?}");
        match ask(RequestKind::Screenshot(shot_path())).await {
            Ok(Reply::Saved(p)) => {
                let mut r = HashMap::new();
                r.insert("uri".to_string(), ov(file_uri(&p.to_string_lossy())));
                (0, r)
            }
            Ok(_) => (2, HashMap::new()),
            Err(e) => {
                tracing::warn!("portal screenshot failed: {e}");
                (2, HashMap::new())
            }
        }
    }
    async fn pick_color(
        &self,
        _handle: OwnedObjectPath,
        _app_id: String,
        _parent_window: String,
        _options: Opts,
    ) -> Results {
        match ask(RequestKind::PickColor).await {
            Ok(Reply::Color(r, g, b)) => {
                let mut m = HashMap::new();
                m.insert("color".to_string(), ov((r, g, b)));
                (0, m)
            }
            _ => (2, HashMap::new()),
        }
    }
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }
}

/// Screen cast streams as `Start` results.
pub fn cast_results(streams: &[CastStream]) -> HashMap<String, OwnedValue> {
    let list: Vec<(u32, HashMap<String, OwnedValue>)> = streams
        .iter()
        .map(|s| {
            let mut p = HashMap::new();
            p.insert("position".to_string(), ov(s.position));
            p.insert("size".to_string(), ov(s.size));
            p.insert("source_type".to_string(), ov(SOURCE_MONITOR));
            (s.node, p)
        })
        .collect();
    let mut r = HashMap::new();
    r.insert("streams".to_string(), ov(list));
    r.insert("persist_mode".to_string(), ov(0u32));
    r
}

/// Cursor mode a client asked for (embedded unless it explicitly wants it hidden).
pub fn cursor_wanted(mode: Option<u32>) -> bool {
    mode != Some(CURSOR_HIDDEN)
}

/// A screen cast session exported at its session handle.
struct CastSession {
    cursor_mode: Option<u32>,
    started: bool,
}

fn stop_cast(path: &str) {
    let (tx, _rx) = channel();
    REQUESTS.lock().unwrap().push(Request { kind: RequestKind::StopCast(path.to_string()), reply: tx });
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Session")]
impl CastSession {
    async fn close(
        &mut self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) {
        if let Some(p) = hdr.path() {
            if self.started {
                stop_cast(p.as_str());
            }
            let p: ObjectPath<'_> = p.clone();
            let _ = server.remove::<CastSession, _>(p).await;
        }
    }
    #[zbus(signal)]
    async fn closed(e: &zbus::object_server::SignalEmitter<'_>) -> zbus::Result<()>;
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}

pub struct ScreenCast;

#[zbus::interface(name = "org.freedesktop.impl.portal.ScreenCast")]
impl ScreenCast {
    async fn create_session(
        &self,
        _handle: OwnedObjectPath,
        session_handle: OwnedObjectPath,
        app_id: String,
        _options: Opts,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> Results {
        tracing::info!("portal screencast session for {app_id:?}");
        match server.at(session_handle.clone(), CastSession { cursor_mode: None, started: false }).await {
            Ok(_) => {
                let mut r = HashMap::new();
                r.insert("session_id".to_string(), ov(session_handle.as_str().to_string()));
                (0, r)
            }
            Err(e) => {
                tracing::warn!("screencast session: {e}");
                (2, HashMap::new())
            }
        }
    }
    async fn select_sources(
        &self,
        _handle: OwnedObjectPath,
        session_handle: OwnedObjectPath,
        _app_id: String,
        options: Opts,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> Results {
        let Ok(s) = server.interface::<_, CastSession>(session_handle).await else { return (2, HashMap::new()) };
        let types = opt_u32(&options, "types").unwrap_or(SOURCE_MONITOR);
        if types & SOURCE_MONITOR == 0 {
            tracing::info!("screencast: only monitors can be shared (asked for types {types})");
        }
        s.get_mut().await.cursor_mode = opt_u32(&options, "cursor_mode");
        (0, HashMap::new())
    }
    async fn start(
        &self,
        _handle: OwnedObjectPath,
        session_handle: OwnedObjectPath,
        app_id: String,
        _parent_window: String,
        _options: Opts,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> Results {
        let Ok(s) = server.interface::<_, CastSession>(session_handle.clone()).await else {
            return (2, HashMap::new());
        };
        let cursor = cursor_wanted(s.get().await.cursor_mode);
        let kind = RequestKind::ScreenCast { session: session_handle.as_str().to_string(), app_id, cursor };
        match ask(kind).await {
            Ok(Reply::Cast(streams)) if !streams.is_empty() => {
                s.get_mut().await.started = true;
                (0, cast_results(&streams))
            }
            Ok(_) => (1, HashMap::new()),
            Err(e) => {
                tracing::warn!("portal screencast failed: {e}");
                (2, HashMap::new())
            }
        }
    }
    #[zbus(property)]
    fn available_source_types(&self) -> u32 {
        SOURCE_MONITOR
    }
    #[zbus(property)]
    fn available_cursor_modes(&self) -> u32 {
        CURSOR_HIDDEN | CURSOR_EMBEDDED
    }
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        4
    }
}

/// Tell the portal that the compositor ended a screen cast session (stream died, output
/// unplugged, user stopped sharing).
pub async fn emit_cast_closed(conn: &zbus::Connection, session: &str) {
    if let Ok(path) = ObjectPath::try_from(session) {
        if let Ok(iface) = conn.object_server().interface::<_, CastSession>(path.clone()).await {
            let _ = CastSession::closed(iface.signal_emitter()).await;
        }
        let _ = conn.object_server().remove::<CastSession, _>(path).await;
    }
}

static INHIBIT_IDLE: AtomicU32 = AtomicU32::new(0);

/// Is idle currently inhibited through the portal?
pub fn inhibited() -> bool {
    INHIBIT_IDLE.load(Ordering::Relaxed) > 0
}

pub struct Inhibit;

/// Session object of [`Inhibit::create_monitor`].
struct InhibitMonitor;

#[zbus::interface(name = "org.freedesktop.impl.portal.Session")]
impl InhibitMonitor {
    async fn close(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) {
        if let Some(p) = hdr.path() {
            let p: ObjectPath<'_> = p.clone();
            let _ = server.remove::<InhibitMonitor, _>(p).await;
        }
    }
    #[zbus(signal)]
    async fn closed(e: &zbus::object_server::SignalEmitter<'_>) -> zbus::Result<()>;
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}

/// Exported at the request handle; Close() releases the inhibition.
struct InhibitRequest {
    idle: bool,
    closed: bool,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Request")]
impl InhibitRequest {
    async fn close(
        &mut self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) {
        if !self.closed && self.idle {
            INHIBIT_IDLE.fetch_sub(1, Ordering::Relaxed);
        }
        self.closed = true;
        if let Some(p) = hdr.path() {
            let p: ObjectPath<'_> = p.clone();
            let _ = server.remove::<InhibitRequest, _>(p).await;
        }
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Inhibit")]
impl Inhibit {
    /// flags: 1 logout, 2 user switch, 4 suspend, 8 idle.
    async fn inhibit(
        &self,
        handle: OwnedObjectPath,
        app_id: String,
        _window: String,
        flags: u32,
        options: Opts,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) {
        let idle = flags & (8 | 4) != 0;
        tracing::info!("portal inhibit by {app_id:?} flags={flags} reason={:?}", opt_str(&options, "reason"));
        if idle {
            INHIBIT_IDLE.fetch_add(1, Ordering::Relaxed);
        }
        let _ = server.at(handle, InhibitRequest { idle, closed: false }).await;
    }

    /// Session-state monitor (GTK 4 creates one for every application at startup). Aqua
    /// has no logout negotiation yet, so the session simply stays "running"; without this
    /// method every GTK app logs a failed portal call and keeps retrying.
    async fn create_monitor(
        &self,
        _handle: OwnedObjectPath,
        session_handle: OwnedObjectPath,
        app_id: String,
        _window: String,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> u32 {
        tracing::debug!("portal session monitor for {app_id:?}");
        match server.at(session_handle, InhibitMonitor).await {
            Ok(_) => 0,
            Err(_) => 2,
        }
    }

    /// Reply to a query-end StateChanged; nothing waits for it.
    fn query_end_response(&self, _session_handle: OwnedObjectPath) {}

    #[zbus(signal)]
    async fn state_changed(
        e: &zbus::object_server::SignalEmitter<'_>,
        session_handle: ObjectPath<'_>,
        state: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<()>;

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        3
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DB: &str = "# comment\n50:image/png:*.png\n50:image/jpeg:*.jpg\n50:image/jpeg:*.jpeg\n\
                      50:application/vnd.oasis.opendocument.text:*.odt\n50:text/plain:*.txt\n10:text/plain:README\n";

    #[test]
    fn mime_globs_from_database() {
        assert_eq!(mime_glob_from(DB, "image/png"), "*.png");
        assert_eq!(mime_glob_from(DB, "image/jpeg"), "*.jpg;*.jpeg");
        assert_eq!(mime_glob_from(DB, "image/*"), "*.png;*.jpg;*.jpeg");
        assert_eq!(mime_glob_from(DB, "application/vnd.oasis.opendocument.text"), "*.odt");
        assert_eq!(mime_glob_from(DB, "text/plain"), "*.txt");
    }

    #[test]
    fn mime_globs_fallback() {
        assert_eq!(mime_glob_from("", "image/jpeg"), "*.jpg;*.jpeg");
        assert_eq!(mime_glob_from("", "text/x-markdown"), "*.md");
        assert_eq!(mime_glob_from("", "application/pdf"), "*.pdf");
        assert_eq!(mime_glob_from("", "video/*"), "*");
    }

    #[test]
    fn file_uris_are_percent_encoded() {
        assert_eq!(file_uri("/home/u/a b.txt"), "file:///home/u/a%20b.txt");
        assert_eq!(file_uri("/tmp/ü"), "file:///tmp/%C3%BC");
        assert_eq!(file_uri("/x/y-z_1.~"), "file:///x/y-z_1.~");
    }

    fn bytes(s: &str) -> OwnedValue {
        let mut b = s.as_bytes().to_vec();
        b.push(0);
        ov(b)
    }

    #[test]
    fn chooser_arguments() {
        let mut o = Opts::new();
        o.insert("multiple".into(), ov(true));
        o.insert("accept_label".into(), ov("Pick"));
        o.insert("current_file".into(), bytes("/home/u/doc.txt"));
        let filters: Vec<(String, Vec<(u32, String)>)> =
            vec![("Images".into(), vec![(0, "*.png".into())]), ("Text".into(), vec![(1, "text/markdown".into())])];
        o.insert("filters".into(), ov(filters));
        o.insert("current_filter".into(), ov(("Text".to_string(), vec![(1u32, "text/markdown".to_string())])));
        let a = chooser_args("Open", &o, true);
        let pos = |x: &str| a.iter().position(|s| s == x);
        assert_eq!(&a[..3], &["--title", "Open", "--save"]);
        assert!(pos("--multiple").is_some());
        assert!(pos("--directory").is_none());
        assert_eq!(a[pos("--accept-label").unwrap() + 1], "Pick");
        assert_eq!(a[pos("--folder").unwrap() + 1], "/home/u");
        assert_eq!(a[pos("--name").unwrap() + 1], "doc.txt");
        let filters: Vec<&String> =
            a.iter().enumerate().filter(|(i, _)| *i > 0 && a[i - 1] == "--filter").map(|(_, s)| s).collect();
        assert_eq!(filters.len(), 2);
        assert!(filters[0].starts_with("Text:"), "current filter first: {filters:?}");
        assert_eq!(filters[1], "Images:*.png");
    }

    #[test]
    fn screencast_results() {
        let r = cast_results(&[CastStream { node: 42, position: (1920, 0), size: (1280, 720) }]);
        let streams: Vec<(u32, HashMap<String, OwnedValue>)> = r["streams"].clone().try_into().unwrap();
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].0, 42);
        let size: (i32, i32) = streams[0].1["size"].clone().try_into().unwrap();
        assert_eq!(size, (1280, 720));
        let st: u32 = streams[0].1["source_type"].clone().try_into().unwrap();
        assert_eq!(st, SOURCE_MONITOR);
        assert!(cursor_wanted(None));
        assert!(cursor_wanted(Some(CURSOR_EMBEDDED)));
        assert!(!cursor_wanted(Some(CURSOR_HIDDEN)));
    }

    #[test]
    fn option_helpers() {
        let mut o = Opts::new();
        o.insert("b".into(), ov(true));
        o.insert("s".into(), ov("x"));
        o.insert("p".into(), bytes(""));
        assert!(opt_bool(&o, "b"));
        assert!(!opt_bool(&o, "missing"));
        assert!(!opt_bool(&o, "s"));
        assert_eq!(opt_str(&o, "s").as_deref(), Some("x"));
        assert_eq!(opt_bytes_path(&o, "p"), None);
    }
}
