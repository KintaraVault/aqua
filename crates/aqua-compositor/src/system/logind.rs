//! logind integration (system bus): lock before suspend (delay inhibitor +
//! `PrepareForSleep`), `loginctl lock-session` / `unlock-session`, idle hint.
use std::sync::mpsc;
use zbus::blocking::{Connection, Proxy};

#[derive(Debug, Clone)]
pub enum SysEvent {
    /// The system is about to sleep: lock now.
    PrepareSleep,
    /// Resumed from sleep.
    Resumed,
    Lock,
    Unlock,
}

fn session_path(c: &Connection) -> Option<zbus::zvariant::OwnedObjectPath> {
    let m =
        Proxy::new(c, "org.freedesktop.login1", "/org/freedesktop/login1", "org.freedesktop.login1.Manager").ok()?;
    if let Ok(id) = std::env::var("XDG_SESSION_ID") {
        if let Ok(p) = m.call("GetSession", &(id,)) {
            return Some(p);
        }
    }
    m.call("GetSessionByPID", &(std::process::id(),)).ok()
}

fn take_inhibitor(c: &Connection) -> Option<zbus::zvariant::OwnedFd> {
    let m =
        Proxy::new(c, "org.freedesktop.login1", "/org/freedesktop/login1", "org.freedesktop.login1.Manager").ok()?;
    m.call("Inhibit", &("sleep", "Aqua", "Lock the screen before sleeping", "delay")).ok()
}

/// Start listening; returns None when logind is not reachable (containers, nested).
pub fn spawn() -> Option<mpsc::Receiver<SysEvent>> {
    let c = Connection::system().ok()?;
    let m =
        Proxy::new(&c, "org.freedesktop.login1", "/org/freedesktop/login1", "org.freedesktop.login1.Manager").ok()?;
    m.get_property::<String>("IdleAction").ok()?;
    let (tx, rx) = mpsc::channel();
    {
        let c = c.clone();
        let tx = tx.clone();
        std::thread::Builder::new()
            .name("aqua-logind-sleep".into())
            .spawn(move || {
                let mut inhibitor = take_inhibitor(&c);
                let Ok(m) = Proxy::new(
                    &c,
                    "org.freedesktop.login1",
                    "/org/freedesktop/login1",
                    "org.freedesktop.login1.Manager",
                ) else {
                    return;
                };
                let Ok(it) = m.receive_signal("PrepareForSleep") else { return };
                for msg in it {
                    let Ok((start,)) = msg.body().deserialize::<(bool,)>() else { continue };
                    if start {
                        let _ = tx.send(SysEvent::PrepareSleep);
                        std::thread::sleep(std::time::Duration::from_millis(400));
                        inhibitor = None;
                    } else {
                        let _ = tx.send(SysEvent::Resumed);
                        if inhibitor.is_none() {
                            inhibitor = take_inhibitor(&c);
                        }
                    }
                    let _ = &inhibitor;
                }
            })
            .ok();
    }
    if let Some(path) = session_path(&c) {
        for (sig, ev) in [("Lock", SysEvent::Lock), ("Unlock", SysEvent::Unlock)] {
            let c = c.clone();
            let tx = tx.clone();
            let path = path.clone();
            std::thread::spawn(move || {
                let Ok(p) = Proxy::new(&c, "org.freedesktop.login1", path.as_str(), "org.freedesktop.login1.Session")
                else {
                    return;
                };
                let Ok(it) = p.receive_signal(sig) else { return };
                for _ in it {
                    let _ = tx.send(ev.clone());
                }
            });
        }
    }
    tracing::info!("logind integration active");
    Some(rx)
}

/// Tell logind whether the session is idle / locked (for `loginctl` and power managers).
pub fn set_hints(idle: bool, locked: bool) {
    std::thread::spawn(move || {
        let Ok(c) = Connection::system() else { return };
        let Some(path) = session_path(&c) else { return };
        let p = Proxy::new(&c, "org.freedesktop.login1", path.clone(), "org.freedesktop.login1.Session");
        if let Ok(p) = p {
            let _ = p.call::<_, _, ()>("SetIdleHint", &(idle,));
            let _ = p.call::<_, _, ()>("SetLockedHint", &(locked,));
        }
    });
}
