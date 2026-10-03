//! aqua-sys: real system data and controls for the Aqua shell and settings app.
//!
//! Every probe runs on a background thread and the results are cached in a
//! process-wide [`Snapshot`], so UI code can call [`snapshot()`] every frame
//! without blocking. Commands (connect Wi-Fi, set volume, …) are fire-and-forget and
//! trigger an immediate refresh.
//!
//! Backends, in order of preference:
//! * Network: NetworkManager (D-Bus) → iwd (D-Bus) → `nmcli` → none
//! * Bluetooth: BlueZ (D-Bus)
//! * Power: UPower (D-Bus) → `/sys/class/power_supply`
//! * Sound: PipeWire via WirePlumber's `wpctl` → PulseAudio `pactl`
//! * Brightness: `/sys/class/backlight` + logind `SetBrightness` (no root needed)
//! * Session: logind Suspend / PowerOff / Reboot
pub mod audio;
pub mod backlight;
pub mod bluetooth;
pub mod media;
pub mod network;
pub mod power;
pub mod session;

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub use audio::Audio;
pub use bluetooth::{Bluetooth, BtDevice};
pub use media::Media;
pub use network::{NetBackend, NetState, Network};
pub use power::Power;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub net: NetState,
    pub bt: Bluetooth,
    pub power: Power,
    pub audio: Audio,
    /// 0..1, None = no controllable backlight.
    pub brightness: Option<f32>,
    /// Now Playing (MPRIS).
    pub media: Media,
    /// Bumped every time something changed.
    pub serial: u64,
}

struct Shared {
    snap: Snapshot,
    last: Option<Instant>,
    busy: bool,
    force: bool,
}

fn shared() -> &'static Mutex<Shared> {
    static S: OnceLock<Mutex<Shared>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Shared { snap: Snapshot::default(), last: None, busy: false, force: true }))
}

pub(crate) fn system_bus() -> Option<zbus::blocking::Connection> {
    static C: OnceLock<Option<zbus::blocking::Connection>> = OnceLock::new();
    C.get_or_init(|| zbus::blocking::Connection::system().map_err(|e| tracing::info!("no system bus: {e}")).ok())
        .clone()
}

/// Is `prog` on $PATH?
pub fn have(prog: &str) -> bool {
    std::env::var_os("PATH").map(|p| std::env::split_paths(&p).any(|d| d.join(prog).is_file())).unwrap_or(false)
}

pub(crate) fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(cmd)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn probe() -> Snapshot {
    Snapshot {
        net: network::probe(),
        bt: bluetooth::probe(),
        power: power::probe(),
        audio: audio::probe(),
        brightness: backlight::get(),
        media: media::probe(),
        serial: 0,
    }
}

/// Current cached state; schedules a background refresh when older than 4 s
/// (or after a command).
pub fn snapshot() -> Snapshot {
    let mut s = shared().lock().unwrap_or_else(|e| e.into_inner());
    refresh_if_stale(&mut s);
    s.snap.clone()
}

/// Change counter of the cached state (cheap; also schedules refreshes like [`snapshot`]).
pub fn serial() -> u64 {
    let mut s = shared().lock().unwrap_or_else(|e| e.into_inner());
    refresh_if_stale(&mut s);
    s.snap.serial
}

fn refresh_if_stale(s: &mut Shared) {
    let stale = s.force || s.last.map(|t| t.elapsed() > Duration::from_secs(4)).unwrap_or(true);
    if stale && !s.busy {
        s.busy = true;
        s.force = false;
        std::thread::Builder::new()
            .name("aqua-sys".into())
            .spawn(|| {
                let snap = std::panic::catch_unwind(probe).unwrap_or_default();
                let mut s = shared().lock().unwrap_or_else(|e| e.into_inner());
                if snap.net != s.snap.net
                    || snap.bt != s.snap.bt
                    || snap.power != s.snap.power
                    || snap.audio != s.snap.audio
                    || snap.brightness != s.snap.brightness
                    || snap.media != s.snap.media
                {
                    let serial = s.snap.serial + 1;
                    s.snap = Snapshot { serial, ..snap };
                }
                s.last = Some(Instant::now());
                s.busy = false;
            })
            .ok();
    }
}

/// Request a refresh at the next `snapshot()` call.
pub fn invalidate() {
    shared().lock().unwrap_or_else(|e| e.into_inner()).force = true;
}

/// Update the cache optimistically (so sliders feel instant) and refresh later.
pub fn patch(f: impl FnOnce(&mut Snapshot)) {
    let mut s = shared().lock().unwrap_or_else(|e| e.into_inner());
    f(&mut s.snap);
    s.snap.serial += 1;
}

/// Run `f` on a worker thread then refresh.
pub(crate) fn spawn_cmd(f: impl FnOnce() + Send + 'static) {
    std::thread::spawn(move || {
        f();
        invalidate();
    });
}
