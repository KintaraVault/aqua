//! Small system helpers: alert sound, exporting the session environment to
//! D-Bus activation and systemd --user (so portals and services see
//! WAYLAND_DISPLAY / DISPLAY / XDG_CURRENT_DESKTOP).
use std::collections::HashMap;

pub fn export_env(key: &str, value: &str) {
    export_vars(vec![(key.to_string(), value.to_string())]);
}

/// Set variables locally and publish them in one go to the D-Bus activation environment
/// and to systemd --user, so D-Bus-activated apps (Nautilus, GNOME Terminal, portals …)
/// connect to *this* compositor and not to another session of the same user.
pub fn export_vars(vars: Vec<(String, String)>) {
    for (k, v) in &vars {
        unsafe { std::env::set_var(k, v) };
    }
    std::thread::spawn(move || publish(&vars));
}

fn publish(vars: &[(String, String)]) {
    {
        let mut ok = false;
        if let Ok(c) = zbus::blocking::Connection::session() {
            if let Ok(p) =
                zbus::blocking::Proxy::new(&c, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus")
            {
                let m: HashMap<&str, &str> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                ok = p.call::<_, _, ()>("UpdateActivationEnvironment", &(m,)).is_ok();
            }
            if let Ok(p) = zbus::blocking::Proxy::new(
                &c,
                "org.freedesktop.systemd1",
                "/org/freedesktop/systemd1",
                "org.freedesktop.systemd1.Manager",
            ) {
                let list: Vec<String> = vars.iter().map(|(k, v)| format!("{k}={v}")).collect();
                let _ = p.call::<_, _, ()>("SetEnvironment", &(list,));
            }
        }
        if !ok {
            let keys: Vec<&str> = vars.iter().map(|(k, _)| k.as_str()).collect();
            let _ =
                std::process::Command::new("dbus-update-activation-environment").arg("--systemd").args(&keys).status();
        }
    }
}

/// An xdg-desktop-portal that was started (D-Bus/systemd activated) before this session exported
/// its environment has no `XDG_CURRENT_DESKTOP=Aqua` / our `WAYLAND_DISPLAY`: it then never routes
/// FileChooser & co. to Aqua (browsers' "Upload file" shows a GTK dialog or nothing).
fn restart_stale_portals(wayland_display: &str) {
    let uid = unsafe { libc::getuid() };
    let Ok(dir) = std::fs::read_dir("/proc") else { return };
    for e in dir.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<i32>().ok()) else { continue };
        let p = e.path();
        let comm = std::fs::read_to_string(p.join("comm")).unwrap_or_default();
        if comm.trim() != "xdg-desktop-por" && comm.trim() != "xdg-desktop-portal" {
            continue;
        }
        let owner = std::fs::metadata(&p).map(|m| std::os::unix::fs::MetadataExt::uid(&m)).unwrap_or(u32::MAX);
        if owner != uid {
            continue;
        }
        let Ok(env) = std::fs::read(p.join("environ")) else { continue };
        let var = |k: &str| {
            env.split(|b| *b == 0).find_map(|kv| {
                kv.strip_prefix(format!("{k}=").as_bytes()).map(|v| String::from_utf8_lossy(v).into_owned())
            })
        };
        let desk_ok =
            var("XDG_CURRENT_DESKTOP").map(|d| d.to_lowercase().split(':').any(|x| x == "aqua")).unwrap_or(false);
        let disp_ok = var("WAYLAND_DISPLAY").as_deref() == Some(wayland_display);
        if !(desk_ok && disp_ok) {
            tracing::info!("restarting xdg-desktop-portal {pid}: started without this session's environment");
            unsafe { libc::kill(pid, libc::SIGTERM) };
        }
    }
}

/// Export the variables every session needs once the Wayland socket exists.
pub fn export_session(wayland_display: &str) {
    let mut v: Vec<(String, String)> = vec![
        ("WAYLAND_DISPLAY".into(), wayland_display.into()),
        ("XDG_CURRENT_DESKTOP".into(), "Aqua".into()),
        ("XDG_SESSION_DESKTOP".into(), "aqua".into()),
        ("XDG_SESSION_TYPE".into(), "wayland".into()),
    ];
    for (k, def) in [
        ("GDK_BACKEND", "wayland,x11"),
        ("QT_QPA_PLATFORM", "wayland;xcb"),
        ("SDL_VIDEODRIVER", "wayland,x11"),
        ("CLUTTER_BACKEND", "wayland"),
        ("MOZ_ENABLE_WAYLAND", "1"),
        ("ELECTRON_OZONE_PLATFORM_HINT", "auto"),
        ("_JAVA_AWT_WM_NONREPARENTING", "1"),
        ("QT_WAYLAND_DISABLE_WINDOWDECORATION", "1"),
        ("QT_QPA_PLATFORMTHEME", "xdgdesktopportal"),
        ("GSETTINGS_BACKEND", "keyfile"),
        ("GTK_USE_PORTAL", "1"),
        ("GDK_DEBUG", "portals"),
    ] {
        let val = std::env::var(k).unwrap_or_else(|_| def.to_string());
        v.push((k.into(), val));
    }
    for k in
        ["XDG_SESSION_ID", "XDG_VTNR", "GBM_BACKEND", "__GLX_VENDOR_LIBRARY_NAME", "LIBVA_DRIVER_NAME", "NVD_BACKEND"]
    {
        if let Ok(val) = std::env::var(k) {
            v.push((k.into(), val));
        }
    }
    for (k, val) in &v {
        unsafe { std::env::set_var(k, val) };
    }
    let disp = wayland_display.to_string();
    std::thread::spawn(move || {
        publish(&v);
        restart_stale_portals(&disp);
    });
}

/// Path of a sibling Aqua binary (next to the running compositor), falling back to $PATH lookup.
pub fn own_bin(name: &str) -> String {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join(name);
            if p.exists() {
                return p.to_string_lossy().into_owned();
            }
        }
    }
    name.to_string()
}
