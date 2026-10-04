//! One logging setup for every Aqua program (compositor, Finder, Settings, App Store, …).
//!
//! `aqua_log::init("aqua-finder")` gives:
//! * `~/.cache/aqua/logs/<name>.log`, appended by every run (each starts with a "started"
//!   line); past 2 MiB at start-up (8 MiB while running) it moves to `<name>.prev.log`. Lines carry local time, level,
//!   thread and target. Also mirrored to stderr when that is a terminal.
//! * Level from `AQUA_LOG` (or `RUST_LOG`), e.g. `AQUA_LOG=debug` or
//!   `AQUA_LOG=info,aqua::wm=trace`. Default: `info` for Aqua, `warn` for libraries.
//! * Crash reports in `~/.cache/aqua/logs/crashes/<name>-<time>.txt` for panics (message,
//!   location, backtrace and the last log lines) and a note for fatal signals
//!   (SIGSEGV/SIGBUS/SIGFPE/SIGILL/SIGABRT); one line per crash in `crashes/index.log`.
//!
//! `aqua logs` / `aqua crash-report` (compositor CLI) print them.
use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Log lines kept in memory for crash reports.
const RING: usize = 400;
/// A log file is restarted past this size (a runaway warning must not fill the disk).
const MAX_BYTES: u64 = 8 << 20;
/// Crash reports kept per program.
const KEEP_CRASHES: usize = 15;
/// Library targets that are only interesting when something is wrong.
pub const DEFAULT_FILTER: &str = "info,smithay=warn,calloop=warn,wgpu=warn,wgpu_core=warn,wgpu_hal=warn,naga=warn,zbus=warn,winit=warn,sctk=warn,i_slint_core=warn,i_slint_backend_winit=warn,femtovg=warn";

struct State {
    name: String,
    dir: PathBuf,
    file: Option<std::fs::File>,
    written: u64,
    ring: VecDeque<String>,
    tty: bool,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

/// `~/.cache/aqua/logs` (`$XDG_CACHE_HOME/aqua/logs`).
pub fn log_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir)
        .join("aqua/logs")
}

pub fn crash_dir() -> PathBuf {
    log_dir().join("crashes")
}

/// Sanitised file stem for a program name.
pub fn file_stem(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '_' }).collect();
    if s.is_empty() {
        "aqua".into()
    } else {
        s
    }
}

/// Path of the current log of `name`.
pub fn log_path(name: &str) -> PathBuf {
    log_dir().join(format!("{}.log", file_stem(name)))
}

/// Local wall-clock time `YYYY-MM-DD HH:MM:SS.mmm`.
pub fn timestamp() -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = now.as_secs() as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&secs, &mut tm) };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
        now.subsec_millis()
    )
}

fn compact_time() -> String {
    timestamp().replace([' ', ':'], "-").chars().take(19).collect()
}

/// Open the log for appending (several instances of a program share it — every run
/// starts with a "started" line); a file past [`ROTATE_AT`] moves to `<name>.prev.log` first.
fn open_log(dir: &Path, name: &str, rotate_at: u64) -> Option<std::fs::File> {
    std::fs::create_dir_all(dir).ok()?;
    let path = dir.join(format!("{}.log", file_stem(name)));
    if std::fs::metadata(&path).map(|m| m.len() > rotate_at).unwrap_or(false) {
        let _ = std::fs::rename(&path, dir.join(format!("{}.prev.log", file_stem(name))));
    }
    std::fs::OpenOptions::new().create(true).append(true).open(path).ok()
}

/// Size at which an existing log is moved aside when a program starts.
pub const ROTATE_AT: u64 = 2 << 20;

fn push_line(st: &mut State, line: &str) {
    if st.ring.len() >= RING {
        st.ring.pop_front();
    }
    st.ring.push_back(line.trim_end().to_string());
    if st.written > MAX_BYTES {
        st.file = open_log(&st.dir, &st.name, 0);
        st.written = 0;
    }
    if let Some(f) = st.file.as_mut() {
        let _ = f.write_all(line.as_bytes());
        st.written += line.len() as u64;
    }
    if st.tty {
        let _ = std::io::stderr().write_all(line.as_bytes());
    }
}

/// Writer handed to tracing: one formatted event per `write` sequence, flushed per line.
struct Sink(Vec<u8>);

impl Write for Sink {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for Sink {
    fn drop(&mut self) {
        if self.0.is_empty() {
            return;
        }
        let text = String::from_utf8_lossy(&self.0).into_owned();
        if let Some(m) = STATE.get() {
            let mut st = m.lock().unwrap_or_else(|e| e.into_inner());
            push_line(&mut st, &text);
        }
    }
}

struct LocalTime;

impl tracing_subscriber::fmt::time::FormatTime for LocalTime {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> std::fmt::Result {
        write!(w, "{}", timestamp())
    }
}

/// Append a line to the log directly (for code paths without tracing, e.g. early start-up).
pub fn line(text: &str) {
    if let Some(m) = STATE.get() {
        let mut st = m.lock().unwrap_or_else(|e| e.into_inner());
        push_line(&mut st, &format!("{} {text}\n", timestamp()));
    }
}

/// The last log lines (oldest first).
pub fn recent() -> Vec<String> {
    STATE.get().map(|m| m.lock().unwrap_or_else(|e| e.into_inner()).ring.iter().cloned().collect()).unwrap_or_default()
}

/// Level filter: `AQUA_LOG`, else `RUST_LOG`, else [`DEFAULT_FILTER`].
pub fn filter_spec() -> String {
    for k in ["AQUA_LOG", "RUST_LOG"] {
        if let Ok(v) = std::env::var(k) {
            if !v.trim().is_empty() {
                // A bare level ("debug") still keeps the noisy libraries quiet.
                let v = v.trim();
                return if !v.contains(['=', ',']) && v != "trace" {
                    DEFAULT_FILTER.replacen("info", v, 1)
                } else {
                    v.to_string()
                };
            }
        }
    }
    DEFAULT_FILTER.into()
}

/// Set up logging and crash reports for this program. Safe to call more than once
/// (later calls do nothing).
pub fn init(name: &str) {
    init_in(name, log_dir());
}

/// [`init`] with an explicit directory (tests).
pub fn init_in(name: &str, dir: PathBuf) {
    if STATE.get().is_some() {
        return;
    }
    let file = open_log(&dir, name, ROTATE_AT);
    let tty = unsafe { libc::isatty(2) } == 1;
    let _ = STATE.set(Mutex::new(State { name: name.into(), dir, file, written: 0, ring: VecDeque::new(), tty }));
    let filter = tracing_subscriber::EnvFilter::try_new(filter_spec())
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(DEFAULT_FILTER));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_timer(LocalTime)
        .with_ansi(false)
        .with_thread_names(true)
        .with_writer(|| Sink(Vec::with_capacity(256)))
        .try_init();
    install_panic_hook();
    install_signal_handlers();
    tracing::info!(
        "{name} {} started (pid {}, log {})",
        env!("CARGO_PKG_VERSION"),
        std::process::id(),
        log_path(name).display()
    );
}

fn trim_crashes(dir: &Path, stem: &str) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&format!("{stem}-")) && n.ends_with(".txt")))
        .collect();
    v.sort();
    while v.len() > KEEP_CRASHES {
        let _ = std::fs::remove_file(v.remove(0));
    }
}

/// Write a crash report; returns its path.
pub fn write_crash_report(kind: &str, details: &str) -> Option<PathBuf> {
    let (name, dir) = STATE
        .get()
        .map(|m| {
            let st = m.lock().unwrap_or_else(|e| e.into_inner());
            (st.name.clone(), st.dir.join("crashes"))
        })
        .unwrap_or_else(|| ("aqua".into(), crash_dir()));
    std::fs::create_dir_all(&dir).ok()?;
    let stem = file_stem(&name);
    let path = dir.join(format!("{stem}-{}-{}.txt", compact_time(), std::process::id()));
    let mut f = std::fs::File::create(&path).ok()?;
    let thread = std::thread::current();
    let _ = writeln!(
        f,
        "Aqua crash report\nprogram: {name} {}\nkind: {kind}\ntime: {}\npid: {}\nthread: {}\nexe: {}\nsession: WAYLAND_DISPLAY={} DISPLAY={} XDG_CURRENT_DESKTOP={}\n",
        env!("CARGO_PKG_VERSION"),
        timestamp(),
        std::process::id(),
        thread.name().unwrap_or("?"),
        std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default(),
        std::env::var("WAYLAND_DISPLAY").unwrap_or_default(),
        std::env::var("DISPLAY").unwrap_or_default(),
        std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
    );
    let _ = writeln!(f, "{details}\n\n--- last log lines ---");
    for l in recent() {
        let _ = writeln!(f, "{l}");
    }
    if let Ok(mut idx) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("index.log")) {
        let first = details.lines().next().unwrap_or("");
        let _ = writeln!(idx, "{} {name} {kind}: {first} → {}", timestamp(), path.display());
    }
    trim_crashes(&dir, &stem);
    Some(path)
}

fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(non-string panic payload)".into());
        let loc = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_default();
        let bt = std::backtrace::Backtrace::force_capture();
        tracing::error!("PANIC at {loc}: {msg}");
        let report = write_crash_report("panic", &format!("panicked at {loc}:\n{msg}\n\nbacktrace:\n{bt}"));
        if let Some(p) = report {
            line(&format!("crash report: {}", p.display()));
        }
        prev(info);
    }));
}

/// Pre-opened file for the signal handler (it may not allocate).
static mut SIGNAL_FD: i32 = -1;
static mut SIGNAL_MSG: [u8; 512] = [0; 512];
static mut SIGNAL_MSG_LEN: usize = 0;

extern "C" fn on_fatal_signal(sig: libc::c_int) {
    // Only async-signal-safe calls here: write the prepared note, then die by the signal.
    unsafe {
        let fd = SIGNAL_FD;
        if fd >= 0 {
            let name: &[u8] = match sig {
                libc::SIGSEGV => b"SIGSEGV (invalid memory access)\n",
                libc::SIGBUS => b"SIGBUS\n",
                libc::SIGFPE => b"SIGFPE\n",
                libc::SIGILL => b"SIGILL\n",
                libc::SIGABRT => b"SIGABRT (abort)\n",
                _ => b"fatal signal\n",
            };
            let msg = &*std::ptr::addr_of!(SIGNAL_MSG);
            libc::write(fd, msg.as_ptr().cast(), SIGNAL_MSG_LEN);
            libc::write(fd, name.as_ptr().cast(), name.len());
            libc::fsync(fd);
        }
        libc::signal(sig, libc::SIG_DFL);
        libc::raise(sig);
    }
}

fn install_signal_handlers() {
    let dir = STATE.get().map(|m| m.lock().unwrap_or_else(|e| e.into_inner()).dir.join("crashes")).unwrap_or_else(crash_dir);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let name = STATE.get().map(|m| m.lock().unwrap_or_else(|e| e.into_inner()).name.clone()).unwrap_or_default();
    let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("index.log")) else { return };
    use std::os::fd::IntoRawFd;
    let prefix = format!("{name} (pid {}, started {}) killed by ", std::process::id(), timestamp());
    unsafe {
        let n = prefix.len().min(511);
        let buf = &mut *std::ptr::addr_of_mut!(SIGNAL_MSG);
        buf[..n].copy_from_slice(&prefix.as_bytes()[..n]);
        SIGNAL_MSG_LEN = n;
        SIGNAL_FD = f.into_raw_fd();
        for sig in [libc::SIGSEGV, libc::SIGBUS, libc::SIGFPE, libc::SIGILL, libc::SIGABRT] {
            libc::signal(sig, on_fatal_signal as extern "C" fn(libc::c_int) as libc::sighandler_t);
        }
    }
}

/// The newest crash report, if any.
pub fn latest_crash() -> Option<PathBuf> {
    let mut v: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(crash_dir())
        .ok()?
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "txt"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    v.sort();
    v.pop().map(|(_, p)| p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_stems_are_safe() {
        assert_eq!(file_stem("aqua-finder"), "aqua-finder");
        assert_eq!(file_stem("../etc/passwd"), ".._etc_passwd");
        assert_eq!(file_stem(""), "aqua");
    }

    #[test]
    fn timestamps_look_right() {
        let t = timestamp();
        assert_eq!(t.len(), 23, "{t}");
        assert_eq!(&t[4..5], "-");
        assert_eq!(&t[10..11], " ");
    }

    #[test]
    fn filter_defaults_keep_libraries_quiet() {
        assert!(DEFAULT_FILTER.starts_with("info,"));
        assert!(DEFAULT_FILTER.contains("smithay=warn"));
        assert!(tracing_subscriber::EnvFilter::try_new(DEFAULT_FILTER).is_ok());
    }

    #[test]
    fn logs_rotate_ring_and_crash_reports() {
        let dir = std::env::temp_dir().join(format!("aqua-log-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("t.log"), "old run\n").unwrap();
        // A big log is moved aside at start-up, a small one is appended to.
        std::fs::write(dir.join("big.log"), vec![b'x'; (ROTATE_AT + 1) as usize]).unwrap();
        drop(open_log(&dir, "big", ROTATE_AT));
        assert_eq!(std::fs::metadata(dir.join("big.log")).unwrap().len(), 0);
        assert!(dir.join("big.prev.log").exists());
        init_in("t", dir.clone());
        tracing::warn!("something odd {}", 42);
        line("plain line");
        let cur = std::fs::read_to_string(dir.join("t.log")).unwrap();
        assert!(cur.starts_with("old run\n"), "{cur}");
        assert!(cur.contains("t 0.1.0 started"), "{cur}");
        assert!(cur.contains("WARN") && cur.contains("something odd 42"), "{cur}");
        assert!(cur.contains("plain line"));
        assert!(recent().iter().any(|l| l.contains("something odd 42")));
        // A panic writes a report with the message, the location and the last log lines.
        let r = std::panic::catch_unwind(|| panic!("boom in test"));
        assert!(r.is_err());
        let crashes: Vec<PathBuf> = std::fs::read_dir(dir.join("crashes"))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "txt"))
            .collect();
        assert_eq!(crashes.len(), 1, "{crashes:?}");
        let rep = std::fs::read_to_string(&crashes[0]).unwrap();
        assert!(rep.contains("boom in test"), "{rep}");
        assert!(rep.contains("src/lib.rs"), "{rep}");
        assert!(rep.contains("something odd 42"), "{rep}");
        let idx = std::fs::read_to_string(dir.join("crashes/index.log")).unwrap();
        assert!(idx.contains("panic: panicked at"), "{idx}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
