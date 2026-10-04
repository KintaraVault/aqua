use aqua_store::history;
use aqua_store::jobs::{self, Event, Job, Kind};
use aqua_store::model::Origin;
use aqua_store::prefs::Prefs;
use aqua_store::runner::SystemRunner;
use aqua_store::store::Store;
use aqua_ui::{ntr, tr};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

const INTERVAL: i64 = 6 * 3600;

pub fn autostart_path() -> std::path::PathBuf {
    dirs::config_dir().unwrap_or_default().join("autostart/org.aqua.store-updates.desktop")
}

pub fn autostart_entry(on: bool) -> String {
    let mut s = String::from(
        "[Desktop Entry]\nType=Application\nName=App Store Updates\nExec=aqua-store --check-updates\nIcon=system-software-install\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n",
    );
    if !on {
        s.push_str("Hidden=true\n");
    }
    s
}

pub fn sync_autostart(on: bool) {
    let p = autostart_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&p, autostart_entry(on));
    if on && !running() {
        if let Ok(exe) = std::env::current_exe() {
            let _ = std::process::Command::new(exe).arg("--check-updates").spawn();
        }
    }
}

fn lock_path() -> std::path::PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    dir.join("aqua-store-updates.lock")
}

fn try_lock() -> Option<std::fs::File> {
    use std::os::fd::AsRawFd;
    let f = std::fs::File::options().create(true).truncate(false).write(true).open(lock_path()).ok()?;
    let r = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    (r == 0).then_some(f)
}

fn running() -> bool {
    try_lock().is_none()
}

pub fn due(last: i64, now: i64) -> bool {
    last <= 0 || now - last >= INTERVAL || now < last
}

pub fn notify(summary: &str, body: &str, open_updates: bool) {
    let Ok(conn) = zbus::blocking::Connection::session() else { return };
    let Ok(p) = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
    ) else {
        return;
    };
    let hints: HashMap<&str, zbus::zvariant::Value> = HashMap::new();
    let actions: Vec<&str> = if open_updates { vec!["default", tr("Show Updates")] } else { vec![] };
    let r: zbus::Result<u32> =
        p.call("Notify", &(tr("App Store"), 0u32, "system-software-install", summary, body, actions, hints, -1i32));
    let Ok(id) = r else { return };
    if !open_updates {
        return;
    }
    let _ = std::thread::Builder::new().name("store-notify".into()).spawn(move || {
        let Ok(it) = p.receive_signal("ActionInvoked") else { return };
        for m in it {
            if let Ok((nid, _action)) = m.body().deserialize::<(u32, String)>() {
                if nid == id {
                    if let Ok(exe) = std::env::current_exe() {
                        let _ = std::process::Command::new(exe).arg("--updates").spawn();
                    }
                    return;
                }
            }
        }
    });
}

fn check_once(store: &Store, prefs: &Prefs) {
    store.invalidate();
    let ups = store.updates().to_vec();
    let hidden = &prefs.hidden;
    let mut pending: Vec<_> = ups.iter().filter(|u| !hidden.contains(&u.key())).cloned().collect();
    let mut applied = 0usize;
    if prefs.auto_update {
        let auto: Vec<_> = pending
            .iter()
            .filter(|u| u.origin() == Origin::Flatpak && u.scope != Some(aqua_store::model::Scope::System))
            .cloned()
            .collect();
        if !auto.is_empty() {
            if let Ok(steps) = store.update_steps(&auto) {
                let job = Job { id: 1, key: "auto".into(), title: "auto".into(), kind: Kind::Update, steps };
                let ev = jobs::execute(&*store.run, &job, &AtomicBool::new(false), &|_| {});
                if let Event::Finished { ok: true, .. } = ev {
                    applied = auto.iter().filter(|u| u.is_app).count();
                    for u in &auto {
                        history::record(history::Entry {
                            time: aqua_store::units::now(),
                            action: "update".into(),
                            key: u.key(),
                            name: u.name.clone(),
                            version: u.to.clone(),
                            source: "Flathub".into(),
                        });
                    }
                    pending.retain(|u| !auto.iter().any(|a| a.key() == u.key()));
                }
            }
        }
    }
    if prefs.notify {
        if applied > 0 {
            notify(
                tr("Apps Updated"),
                &ntr("{n} app was updated automatically.", "{n} apps were updated automatically.", applied as i64),
                false,
            );
        }
        let apps = pending.iter().filter(|u| u.is_app).count();
        let total = pending.len();
        if total > 0 {
            let body = if apps > 0 {
                ntr("{n} app can be updated.", "{n} apps can be updated.", apps as i64)
            } else {
                ntr("{n} system package can be updated.", "{n} system packages can be updated.", total as i64)
            };
            notify(&ntr("{n} update available", "{n} updates available", total as i64), &body, true);
        }
    }
}

pub fn background() {
    let Some(_lock) = try_lock() else { return };
    aqua_i18n::init(&aqua_i18n::env_lang());
    std::thread::sleep(Duration::from_secs(60));
    loop {
        let mut prefs = Prefs::load();
        if !prefs.auto_check {
            return;
        }
        let now = aqua_store::units::now();
        if due(prefs.last_check, now) {
            let store = Store::new(
                std::sync::Arc::new(SystemRunner::default()),
                std::sync::Arc::new(Default::default()),
                prefs.clone(),
            );
            check_once(&store, &prefs);
            prefs = Prefs::load();
            prefs.last_check = aqua_store::units::now();
            prefs.save();
        }
        std::thread::sleep(Duration::from_secs(30 * 60));
    }
}
