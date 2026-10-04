//! Small system helpers: alert sound, exporting the session environment to
//! D-Bus activation and systemd --user (so portals and services see
//! WAYLAND_DISPLAY / DISPLAY / XDG_CURRENT_DESKTOP).
use std::collections::HashMap;

static NESTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Running nested (winit backend) inside another session.
pub fn set_nested(on: bool) {
    NESTED.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Does this compositor own the user's session bus — may it publish its environment to
/// D-Bus activation / systemd, restart portals and claim notification / portal names?
/// A nested session (tests, trying Aqua in a window) must not: it would point the real
/// session's portals and services at a display that soon disappears. `AQUA_NESTED_DBUS=1`
/// opts a nested session in (e.g. under its own `dbus-run-session`).
pub fn owns_session_bus() -> bool {
    !NESTED.load(std::sync::atomic::Ordering::Relaxed) || std::env::var_os("AQUA_NESTED_DBUS").is_some()
}

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
    if owns_session_bus() {
        std::thread::spawn(move || publish(&vars));
    }
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

/// `PATH` with Aqua's compatibility shims first (`share/aqua/shims` next to the compositor:
/// `zenity` / `kdialog` file dialogs open Aqua's panel), or None when they are not installed
/// or already there.
fn path_with_shims() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.parent()?.join("share/aqua/shims");
    if !dir.is_dir() {
        return None;
    }
    let cur = std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into());
    let d = dir.to_string_lossy().into_owned();
    if cur.split(':').any(|p| p == d) {
        return None;
    }
    Some(format!("{d}:{cur}"))
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
        ("GSETTINGS_BACKEND", "keyfile"),
        ("GTK_USE_PORTAL", "1"),
        ("GDK_DEBUG", "portals"),
    ] {
        let val = std::env::var(k).unwrap_or_else(|_| def.to_string());
        v.push((k.into(), val));
    }
    let style_apps = aqua_config::Config::load().style_apps;
    v.push((
        "QT_QPA_PLATFORMTHEME".into(),
        super::apptheme::qt_platform_theme(
            std::env::var("QT_QPA_PLATFORMTHEME").ok().as_deref(),
            style_apps,
            super::apptheme::qt6ct_installed(),
        ),
    ));
    for k in [
        "XDG_SESSION_ID",
        "XDG_VTNR",
        "GBM_BACKEND",
        "__GLX_VENDOR_LIBRARY_NAME",
        "LIBVA_DRIVER_NAME",
        "NVD_BACKEND",
        "WEBKIT_DISABLE_DMABUF_RENDERER",
    ] {
        if let Ok(val) = std::env::var(k) {
            v.push((k.into(), val));
        }
    }
    if let Some(path) = path_with_shims() {
        v.push(("PATH".into(), path));
    }
    if let Some(m) = gtk_modules_with_appmenu(std::env::var("GTK_MODULES").ok().as_deref(), appmenu_gtk_module_installed()) {
        v.push(("GTK_MODULES".into(), m));
    }
    for (k, val) in &v {
        unsafe { std::env::set_var(k, val) };
    }
    if !owns_session_bus() {
        return;
    }
    let disp = wayland_display.to_string();
    let nested = NESTED.load(std::sync::atomic::Ordering::Relaxed);
    std::thread::spawn(move || {
        publish(&v);
        // Portals found in /proc may belong to the host session: never touch them nested.
        if !nested {
            restart_stale_portals(&disp);
        }
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

/// appmenu-gtk-module exports GTK 3 menu bars of X11 windows to the global menu.
fn appmenu_gtk_module_installed() -> bool {
    aqua_config::Config::load().global_menu
        && ["/usr/lib/gtk-3.0/modules", "/usr/lib64/gtk-3.0/modules", "/usr/lib/x86_64-linux-gnu/gtk-3.0/modules"]
            .iter()
            .any(|d| std::path::Path::new(d).join("libappmenu-gtk-module.so").exists())
}

/// `GTK_MODULES` with `appmenu-gtk-module` appended (once), or `None` to leave it alone.
fn gtk_modules_with_appmenu(cur: Option<&str>, installed: bool) -> Option<String> {
    if !installed {
        return None;
    }
    let cur = cur.unwrap_or("");
    if cur.split(':').any(|m| m == "appmenu-gtk-module") {
        return None;
    }
    Some(if cur.is_empty() { "appmenu-gtk-module".into() } else { format!("{cur}:appmenu-gtk-module") })
}

#[cfg(test)]
mod appmenu_env_tests {
    use super::gtk_modules_with_appmenu as m;

    #[test]
    fn gtk_modules() {
        assert_eq!(m(None, false), None);
        assert_eq!(m(None, true).as_deref(), Some("appmenu-gtk-module"));
        assert_eq!(m(Some("canberra-gtk-module"), true).as_deref(), Some("canberra-gtk-module:appmenu-gtk-module"));
        assert_eq!(m(Some("a:appmenu-gtk-module"), true), None);
    }
}
