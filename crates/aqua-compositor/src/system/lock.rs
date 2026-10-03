//! Screen locking.
//!
//! * **Internal lock** (default): Aqua draws its own lock screen
//!   (`aqua_shell::lockscreen`) and checks the password with PAM on a worker
//!   thread. While locked no client content is rendered, captured or receives input.
//! * **External lockers** (`ext-session-lock-v1`, e.g. swaylock) are supported too.
//!
//! Locks come from ⌃⌘Q / system menu, idle timeout, lid close / suspend (logind
//! `PrepareForSleep`), `loginctl lock-session`, `org.freedesktop.ScreenSaver.Lock`.
//! The lock state is persisted in `$XDG_RUNTIME_DIR/aqua-locked` so that a restarted
//! compositor (after a crash) comes back locked instead of exposing the session.
use crate::state::Aqua;
use smithay::{
    output::Output,
    wayland::session_lock::{LockSurface, SessionLocker},
};
use std::sync::mpsc;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Unlocked,
    Internal,
    External,
}

pub struct Lock {
    pub mode: Mode,
    pub since: Option<Instant>,
    /// Unlock animation start.
    pub unlocking: Option<Instant>,
    pub ext_surfaces: Vec<(Output, LockSurface)>,
    pub auth_rx: Option<mpsc::Receiver<Result<(), String>>>,
    pub failures: u32,
    pub user: (String, String),
}

pub const UNLOCK_ANIM_MS: f32 = 420.0;
pub const LOCK_ANIM_MS: f32 = 320.0;

pub fn marker_path() -> std::path::PathBuf {
    marker()
}

fn marker() -> std::path::PathBuf {
    aqua_config::paths::runtime_dir().join("aqua-locked")
}

impl Lock {
    pub fn new(lock_on_start: bool) -> Self {
        let crashed_locked = marker().exists();
        let mut l = Self {
            mode: Mode::Unlocked,
            since: None,
            unlocking: None,
            ext_surfaces: vec![],
            auth_rx: None,
            failures: 0,
            user: crate::system::pam::current_user(),
        };
        if lock_on_start || crashed_locked {
            if crashed_locked {
                tracing::warn!("previous session ended while locked: starting locked");
            }
            l.mode = Mode::Internal;
            l.since = Some(Instant::now() - std::time::Duration::from_secs(5));
        }
        l
    }
    pub fn is_locked(&self) -> bool {
        self.mode != Mode::Unlocked
    }
    pub fn external_lock(&mut self, locker: SessionLocker) {
        self.mode = Mode::External;
        self.since.get_or_insert_with(Instant::now);
        locker.lock();
        let _ = std::fs::write(marker(), "external");
    }
    pub fn external_unlock(&mut self) {
        self.ext_surfaces.clear();
        self.mode = Mode::Unlocked;
        self.unlocking = None;
        self.since = None;
        let _ = std::fs::remove_file(marker());
    }
    /// 0..1 fade of the lock screen.
    pub fn lock_progress(&self) -> f32 {
        let slow = crate::state::anim_slow();
        if let Some(t) = self.unlocking {
            let x = (t.elapsed().as_secs_f32() * 1000.0 / (UNLOCK_ANIM_MS * slow)).min(1.0);
            return 1.0 - (1.0 - (1.0 - x).powi(3)).min(1.0);
        }
        match self.since {
            Some(t) if self.is_locked() => {
                let x = (t.elapsed().as_secs_f32() * 1000.0 / (LOCK_ANIM_MS * slow)).min(1.0);
                1.0 - (1.0 - x).powi(3)
            }
            _ => 0.0,
        }
    }
    pub fn animating(&self) -> bool {
        let p = self.lock_progress();
        (self.unlocking.is_some() && p > 0.0) || (self.is_locked() && p < 1.0)
    }
}

impl Aqua {
    /// Lock the session with the built-in lock screen.
    pub fn lock_session(&mut self) {
        if self.lock.is_locked() {
            return;
        }
        tracing::info!("locking session");
        self.lock.mode = Mode::Internal;
        self.lock.since = Some(Instant::now());
        self.lock.unlocking = None;
        let _ = std::fs::write(marker(), "internal");
        self.on_locked();
    }

    /// Common work once locked: drop client focus, close shell popovers.
    pub fn on_locked(&mut self) {
        self.shell.close_transients();
        if self.mission.open {
            self.toggle_mission();
        }
        self.shell.lockscreen.reset();
        self.shell.locked = self.lock.mode == Mode::Internal;
        crate::system::logind::set_hints(false, true);
        self.shell.lockscreen.user = self.lock.user.clone();
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        if let Some(kb) = self.seat.get_keyboard() {
            if self.lock.mode == Mode::Internal {
                kb.set_focus(self, None, serial);
            }
        }
        if let Some(ptr) = self.seat.get_pointer() {
            let loc = ptr.current_location();
            ptr.motion(
                self,
                None,
                &smithay::input::pointer::MotionEvent {
                    location: loc,
                    serial,
                    time: smithay::backend::input::InputTime::now(),
                },
            );
            ptr.frame(self);
        }
        for (f, _, _) in self.pending_captures.drain(..) {
            f.fail(smithay::wayland::image_copy_capture::CaptureFailureReason::Stopped);
        }
        self.needs_redraw = true;
    }

    pub fn on_unlocked(&mut self) {
        let _ = std::fs::remove_file(marker());
        self.lock.failures = 0;
        self.shell.locked = false;
        crate::system::logind::set_hints(false, false);
        let w = self
            .space
            .elements()
            .rev()
            .find(|w| self.on_current_space(w) && !crate::state::meta(w).borrow().override_redirect)
            .cloned();
        if let Some(w) = w {
            self.focus_window(&w);
        }
        self.idle.reset();
        self.needs_redraw = true;
    }

    /// Start a PAM check for the typed password.
    pub fn lock_try_password(&mut self, password: String) {
        if self.lock.auth_rx.is_some() || self.lock.mode != Mode::Internal {
            return;
        }
        let user = self.lock.user.0.clone();
        let (tx, rx) = mpsc::channel();
        self.lock.auth_rx = Some(rx);
        self.shell.lockscreen.busy = true;
        std::thread::spawn(move || {
            let r = match std::env::var("AQUA_TEST_PASSWORD") {
                Ok(p) if !p.is_empty() => {
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    if p == password {
                        Ok(())
                    } else {
                        Err("Authentication failure".into())
                    }
                }
                _ => crate::system::pam::authenticate(&user, &password),
            };
            let _ = tx.send(r);
        });
    }

    /// Poll the PAM worker; returns true when something changed.
    pub fn poll_lock(&mut self) -> bool {
        let Some(rx) = &self.lock.auth_rx else { return self.lock.animating() };
        let Ok(r) = rx.try_recv() else { return true };
        self.lock.auth_rx = None;
        self.shell.lockscreen.busy = false;
        match r {
            Ok(()) => {
                tracing::info!("unlocked");
                self.lock.mode = Mode::Unlocked;
                self.lock.unlocking = Some(Instant::now());
                self.lock.since = None;
                self.on_unlocked();
            }
            Err(e) => {
                tracing::info!("unlock failed: {e}");
                self.lock.failures += 1;
                if self.lock.failures >= 5 {
                    self.shell.lockscreen.lockout_until =
                        Some(Instant::now() + std::time::Duration::from_secs(30 * (self.lock.failures as u64 - 4)));
                }
                self.shell.lockscreen.fail();
            }
        }
        true
    }

    pub fn lock_surface_under(
        &self,
        pos: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) -> Option<(crate::input::focus::PointerFocusTarget, smithay::utils::Point<f64, smithay::utils::Logical>)> {
        if self.lock.mode != Mode::External {
            return None;
        }
        for (o, s) in &self.lock.ext_surfaces {
            let g = self.space.output_geometry(o)?;
            if g.to_f64().contains(pos) {
                return Some((s.wl_surface().clone().into(), g.loc.to_f64()));
            }
        }
        None
    }
}
