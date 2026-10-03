//! Idle handling: dim → screen off → lock → suspend, honouring inhibitors from
//! `zwp_idle_inhibit` (video players, games) and `org.freedesktop.ScreenSaver.Inhibit`
//! (browsers, Electron, mpv …), which Aqua serves on the session bus.
use crate::state::Aqua;
use aqua_config::{Config, IdleCfg};
use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Active,
    Dimmed,
    Off,
}

pub enum DbusReq {
    Lock,
    Activity,
}

pub struct Idle {
    pub last: Instant,
    pub cfg: IdleCfg,
    pub inhibited: bool,
    pub stage: Stage,
    pub dim_anim: Option<(Instant, bool)>,
    locked_by_idle: bool,
    suspended: bool,
    dbus: Arc<Mutex<HashMap<u32, (String, String)>>>,
    rx: Option<mpsc::Receiver<DbusReq>>,
}

struct ScreenSaver {
    inhibitors: Arc<Mutex<HashMap<u32, (String, String)>>>,
    next: u32,
    tx: mpsc::Sender<DbusReq>,
    since: Arc<Mutex<Instant>>,
}

#[zbus::interface(name = "org.freedesktop.ScreenSaver")]
impl ScreenSaver {
    fn inhibit(&mut self, application_name: String, reason_for_inhibit: String) -> u32 {
        self.next += 1;
        tracing::info!("ScreenSaver.Inhibit by {application_name:?}: {reason_for_inhibit:?}");
        self.inhibitors.lock().unwrap().insert(self.next, (application_name, reason_for_inhibit));
        self.next
    }
    fn un_inhibit(&mut self, cookie: u32) {
        self.inhibitors.lock().unwrap().remove(&cookie);
    }
    fn lock(&self) {
        let _ = self.tx.send(DbusReq::Lock);
    }
    fn simulate_user_activity(&self) {
        let _ = self.tx.send(DbusReq::Activity);
    }
    fn get_active(&self) -> bool {
        false
    }
    fn get_active_time(&self) -> u32 {
        0
    }
    fn get_session_idle_time(&self) -> u32 {
        self.since.lock().unwrap().elapsed().as_secs() as u32
    }
    fn set_active(&self, active: bool) -> bool {
        if active {
            let _ = self.tx.send(DbusReq::Lock);
        }
        true
    }
}

fn serve(inhibitors: Arc<Mutex<HashMap<u32, (String, String)>>>, tx: mpsc::Sender<DbusReq>) {
    std::thread::Builder::new()
        .name("aqua-screensaver".into())
        .spawn(move || {
            let since = Arc::new(Mutex::new(Instant::now()));
            let mk = || ScreenSaver { inhibitors: inhibitors.clone(), next: 0, tx: tx.clone(), since: since.clone() };
            let r = zbus::blocking::connection::Builder::session()
                .and_then(|b| b.name("org.freedesktop.ScreenSaver"))
                .and_then(|b| b.serve_at("/org/freedesktop/ScreenSaver", mk()))
                .and_then(|b| b.serve_at("/ScreenSaver", mk()))
                .and_then(|b| b.build());
            match r {
                Ok(conn) => {
                    tracing::info!("org.freedesktop.ScreenSaver ready");
                    loop {
                        std::thread::sleep(Duration::from_secs(3600));
                        let _ = &conn;
                    }
                }
                Err(e) => tracing::info!("ScreenSaver service unavailable: {e}"),
            }
        })
        .ok();
}

impl Idle {
    pub fn new(cfg: &Config) -> Self {
        let dbus = Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = mpsc::channel();
        if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some() {
            serve(dbus.clone(), tx);
        }
        Self {
            last: Instant::now(),
            cfg: cfg.idle.clone(),
            inhibited: false,
            stage: Stage::Active,
            dim_anim: None,
            locked_by_idle: false,
            suspended: false,
            dbus,
            rx: Some(rx),
        }
    }
    pub fn dbus_inhibited(&self) -> bool {
        !self.dbus.lock().unwrap().is_empty()
    }
    /// User activity. Returns true when the screen was dimmed/off (needs a redraw).
    pub fn reset(&mut self) -> bool {
        self.last = Instant::now();
        self.locked_by_idle = false;
        self.suspended = false;
        let was = self.stage;
        if was != Stage::Active {
            self.stage = Stage::Active;
            self.dim_anim = Some((Instant::now(), false));
        }
        was != Stage::Active
    }
    /// 0..1 dim overlay strength (0.55 when dimmed, 1 when off).
    pub fn dim_amount(&self) -> f32 {
        let target = match self.stage {
            Stage::Active => 0.0,
            Stage::Dimmed => 0.5,
            Stage::Off => 1.0,
        };
        match self.dim_anim {
            Some((t, _)) => {
                let x = (t.elapsed().as_secs_f32() / 0.6).min(1.0);
                let from = if target == 0.0 || target == 1.0 { 0.5 } else { 0.0 };
                from + (target - from) * x
            }
            None => target,
        }
    }
    pub fn animating(&self) -> bool {
        self.dim_anim.map(|(t, _)| t.elapsed().as_secs_f32() < 0.65).unwrap_or(false)
    }
}

impl Aqua {
    /// Called ~2×/s.
    pub fn tick_idle(&mut self) {
        let reqs: Vec<DbusReq> = self.idle.rx.as_ref().map(|r| r.try_iter().collect()).unwrap_or_default();
        for r in reqs {
            match r {
                DbusReq::Lock => self.lock_session(),
                DbusReq::Activity => {
                    self.notify_activity();
                }
            }
        }
        self.refresh_idle_inhibit();
        if self.idle.inhibited && !self.lock.is_locked() {
            self.idle.last = Instant::now();
            return;
        }
        let secs = self.idle.last.elapsed().as_secs() as u32;
        let c = self.idle.cfg.clone();
        let hit = |limit: u32| limit > 0 && secs >= limit;
        let off_after =
            if self.lock.is_locked() && c.screen_off_secs > 0 { c.screen_off_secs.min(60) } else { c.screen_off_secs };
        if hit(off_after) && self.idle.stage != Stage::Off {
            tracing::info!("idle: display off");
            self.idle.stage = Stage::Off;
            self.idle.dim_anim = Some((Instant::now(), true));
            self.set_dpms(false);
            self.needs_redraw = true;
        } else if hit(c.dim_secs) && self.idle.stage == Stage::Active {
            self.idle.stage = Stage::Dimmed;
            self.idle.dim_anim = Some((Instant::now(), true));
            self.needs_redraw = true;
        }
        if hit(c.lock_secs) && !self.lock.is_locked() && !self.idle.locked_by_idle {
            self.idle.locked_by_idle = true;
            self.lock_session();
        }
        if hit(c.suspend_secs) && !self.idle.suspended {
            self.idle.suspended = true;
            self.handle_actions(vec![aqua_shell::Action::Sleep]);
        }
    }

    /// Any user input.
    pub fn notify_activity(&mut self) {
        self.p.idle_notifier.notify_activity(&self.seat);
        if self.idle.reset() {
            self.set_dpms(true);
            self.needs_redraw = true;
        }
    }

    /// Turn displays on/off (DRM backend); nested sessions just draw black.
    pub fn set_dpms(&mut self, on: bool) {
        if let Some(ud) = self.udev.as_mut() {
            ud.dpms_off = !on;
        }
        self.needs_redraw = true;
    }
}
