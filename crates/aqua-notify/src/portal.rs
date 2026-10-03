//! Minimal `org.freedesktop.portal.Settings` provider so toolkits (libadwaita, GTK4,
//! Qt 6, Firefox) follow Aqua's appearance: colour scheme (Dark Mode toggle in
//! Control Center) and the blue accent. Changes are broadcast live through the
//! `SettingChanged` signal.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use zbus::zvariant::{OwnedValue, Value};

const NS: &str = "org.freedesktop.appearance";
pub(crate) const WM_NS: &str = "org.gnome.desktop.wm.preferences";
pub(crate) const IF_NS: &str = "org.gnome.desktop.interface";
const PATH: &str = "/org/freedesktop/portal/desktop";
const IFACE: &str = "org.freedesktop.portal.Settings";

pub(crate) struct Settings {
    pub(crate) dark: Arc<AtomicBool>,
}

/// Accent colour (System Settings → Appearance → Colour), sRGB 0..1.
static ACCENT: std::sync::Mutex<(f64, f64, f64)> = std::sync::Mutex::new((0.0, 0.478, 1.0));

fn scheme(dark: bool) -> u32 {
    if dark {
        1
    } else {
        2
    }
}

impl Settings {
    pub(crate) fn values(&self) -> HashMap<String, OwnedValue> {
        let mut m = HashMap::new();
        m.insert("color-scheme".into(), OwnedValue::from(scheme(self.dark.load(Ordering::Relaxed))));
        let a = *ACCENT.lock().unwrap();
        m.insert("accent-color".into(), Value::from(a).try_to_owned().unwrap());
        m.insert("contrast".into(), OwnedValue::from(0u32));
        m
    }
    /// `org.gnome.desktop.wm.preferences`: GTK 3/4 (and Firefox / Zen through GTK) read the
    /// title-bar button layout from the portal; without it they fall back to the GNOME
    /// default (a lone close button on the right) instead of left-side traffic lights.
    pub(crate) fn wm_values(&self) -> HashMap<String, OwnedValue> {
        let mut m = HashMap::new();
        m.insert("button-layout".into(), Value::from("close,minimize,maximize:").try_to_owned().unwrap());
        m
    }
    /// `org.gnome.desktop.interface`: with `GDK_DEBUG=portals` (and on GTK ≥ 3.24 under Wayland
    /// whenever a portal is present) GTK takes its *whole* settings from the portal.
    pub(crate) fn interface_values(&self) -> HashMap<String, OwnedValue> {
        let dark = self.dark.load(Ordering::Relaxed);
        let mut m = HashMap::new();
        let s = |v: &str| Value::from(v.to_string()).try_to_owned().unwrap();
        m.insert("color-scheme".into(), s(if dark { "prefer-dark" } else { "default" }));
        m.insert("gtk-theme".into(), s(&gtk_theme_for(dark)));
        let ini = user_gtk_settings();
        if let Some(v) = ini_get(&ini, "gtk-icon-theme-name") {
            m.insert("icon-theme".into(), s(&v));
        }
        if let Some(v) = ini_get(&ini, "gtk-cursor-theme-name").or_else(|| std::env::var("XCURSOR_THEME").ok()) {
            m.insert("cursor-theme".into(), s(&v));
        }
        m.insert("font-name".into(), s(&ini_get(&ini, "gtk-font-name").unwrap_or_else(|| "SF Pro Display 11".into())));
        m.insert("enable-animations".into(), OwnedValue::from(true));
        m
    }
    pub(crate) fn lookup(&self, namespace: &str, key: &str) -> zbus::fdo::Result<OwnedValue> {
        if namespace == NS {
            if let Some(v) = self.values().remove(key) {
                return Ok(v);
            }
        }
        if namespace == IF_NS {
            if let Some(v) = self.interface_values().remove(key) {
                return Ok(v);
            }
        }
        if namespace == WM_NS {
            if let Some(v) = self.wm_values().remove(key) {
                return Ok(v);
            }
        }
        Err(zbus::fdo::Error::Failed(format!("Requested setting {namespace}.{key} not found")))
    }
}

#[zbus::interface(name = "org.freedesktop.portal.Settings")]
impl Settings {
    fn read_all(&self, namespaces: Vec<String>) -> HashMap<String, HashMap<String, OwnedValue>> {
        let mut out = HashMap::new();
        let want = |ns: &str| {
            namespaces.is_empty()
                || namespaces
                    .iter()
                    .any(|p| p == ns || p.is_empty() || (p.ends_with('*') && ns.starts_with(p.trim_end_matches('*'))))
        };
        if want(NS) {
            out.insert(NS.to_string(), self.values());
        }
        if want(WM_NS) {
            out.insert(WM_NS.to_string(), self.wm_values());
        }
        if want(IF_NS) {
            out.insert(IF_NS.to_string(), self.interface_values());
        }
        out
    }

    /// Deprecated API: the value is wrapped in an extra variant.
    fn read(&self, namespace: &str, key: &str) -> zbus::fdo::Result<OwnedValue> {
        let v = self.lookup(namespace, key)?;
        Ok(Value::Value(Box::new(Value::from(v))).try_to_owned().unwrap())
    }

    fn read_one(&self, namespace: &str, key: &str) -> zbus::fdo::Result<OwnedValue> {
        self.lookup(namespace, key)
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }
}

/// Handle used by the compositor to change the appearance at runtime.
pub struct Appearance {
    dark: Arc<AtomicBool>,
    conn: Option<zbus::blocking::Connection>,
    backend: Option<zbus::blocking::Connection>,
}

impl Appearance {
    /// The compositor ended screen cast `session`: tell xdg-desktop-portal (Session.Closed).
    pub fn cast_closed(&self, session: &str) {
        if let Some(b) = &self.backend {
            let (c, s) = (b.inner().clone(), session.to_string());
            std::thread::spawn(move || zbus::block_on(crate::portal_impl::emit_cast_closed(&c, &s)));
        }
    }

    /// Broadcast a new accent colour (libadwaita 1.6+, Qt 6.6+, Firefox follow it live).
    pub fn set_accent(&self, rgb: (f64, f64, f64)) {
        {
            let mut a = ACCENT.lock().unwrap();
            if *a == rgb {
                return;
            }
            *a = rgb;
        }
        let v = Value::from(rgb);
        if let Some(c) = &self.conn {
            let _ = c.emit_signal(None::<&str>, PATH, IFACE, "SettingChanged", &(NS, "accent-color", &v));
        }
        if let Some(c) = &self.backend {
            let _ = c.emit_signal(
                None::<&str>,
                PATH,
                "org.freedesktop.impl.portal.Settings",
                "SettingChanged",
                &(NS, "accent-color", &v),
            );
        }
    }

    pub fn set_dark(&self, dark: bool) {
        if self.dark.swap(dark, Ordering::Relaxed) == dark {
            return;
        }
        let theme = gtk_theme_for(dark);
        let sig: [(&str, &str, Value); 3] = [
            (NS, "color-scheme", Value::from(scheme(dark))),
            (IF_NS, "color-scheme", Value::from(if dark { "prefer-dark" } else { "default" })),
            (IF_NS, "gtk-theme", Value::from(theme.as_str())),
        ];
        for (ns, key, v) in &sig {
            if let Some(c) = &self.conn {
                if let Err(e) = c.emit_signal(None::<&str>, PATH, IFACE, "SettingChanged", &(*ns, *key, v)) {
                    tracing::warn!("SettingChanged failed: {e}");
                }
            }
            if let Some(c) = &self.backend {
                let _ = c.emit_signal(
                    None::<&str>,
                    PATH,
                    "org.freedesktop.impl.portal.Settings",
                    "SettingChanged",
                    &(*ns, *key, v),
                );
            }
        }
    }
}

fn config_home() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(Into::into)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
}

fn user_gtk_settings() -> String {
    config_home().and_then(|c| std::fs::read_to_string(c.join("gtk-3.0/settings.ini")).ok()).unwrap_or_default()
}

fn ini_get(ini: &str, key: &str) -> Option<String> {
    ini.lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| v.trim().trim_matches('"').to_string())
        .filter(|v| !v.is_empty())
}

fn theme_exists(name: &str) -> bool {
    let mut dirs: Vec<std::path::PathBuf> = vec![];
    if let Some(h) = std::env::var_os("HOME") {
        let h = std::path::PathBuf::from(h);
        dirs.push(h.join(".themes"));
        dirs.push(h.join(".local/share/themes"));
    }
    let data = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    dirs.extend(data.split(':').filter(|d| !d.is_empty()).map(|d| std::path::PathBuf::from(d).join("themes")));
    dirs.iter().any(|d| d.join(name).join("gtk-3.0").is_dir())
}

/// GTK theme matching the appearance: the user's theme (gtk-3.0/settings.ini) with its dark/light
/// variant swapped in when one is installed.
pub fn gtk_theme_for(dark: bool) -> String {
    let user = ini_get(&user_gtk_settings(), "gtk-theme-name").unwrap_or_else(|| "Adwaita".into());
    let mut base = user.clone();
    for suf in ["-dark", "-Dark", "_dark", "-DARK", " Dark"] {
        if let Some(b) = user.strip_suffix(suf) {
            base = b.to_string();
            break;
        }
    }
    if base == "Breeze-Dark" || base == "BreezeDark" {
        base = "Breeze".into();
    }
    if !dark {
        if base == user || base == "Adwaita" || theme_exists(&base) {
            return base;
        }
        return "Adwaita".into();
    }
    if base == "Adwaita" {
        return "Adwaita-dark".into();
    }
    for cand in [format!("{base}-dark"), format!("{base}-Dark"), format!("{base}_dark")] {
        if theme_exists(&cand) {
            return cand;
        }
    }
    if user != base {
        return user;
    }
    base
}

/// Is a real xdg-desktop-portal available (running or D-Bus activatable)? Then Aqua must not
/// squat on `org.freedesktop.portal.Desktop` and only provides the `impl` backend.
fn frontend_available(c: &zbus::blocking::Connection) -> bool {
    let Ok(p) = zbus::blocking::fdo::DBusProxy::new(c) else { return false };
    let name = "org.freedesktop.portal.Desktop";
    let running = p.name_has_owner(name.try_into().unwrap()).unwrap_or(false);
    let activatable = p.list_activatable_names().map(|v| v.iter().any(|n| n.as_str() == name)).unwrap_or(false);
    running || activatable
}

/// Export Aqua's portals on the session bus:
pub fn spawn(dark: bool) -> Appearance {
    let flag = Arc::new(AtomicBool::new(dark));
    let backend = zbus::blocking::connection::Builder::session()
        .and_then(|b| b.name("org.freedesktop.impl.portal.desktop.aqua"))
        .and_then(|b| b.serve_at(PATH, crate::portal_impl::ImplSettings { dark: flag.clone() }))
        .and_then(|b| b.serve_at(PATH, crate::portal_impl::FileChooser))
        .and_then(|b| b.serve_at(PATH, crate::portal_impl::Screenshot))
        .and_then(|b| b.serve_at(PATH, crate::portal_impl::Inhibit))
        .and_then(|b| b.serve_at(PATH, crate::portal_impl::ScreenCast))
        .and_then(|b| b.build());
    let backend = match backend {
        Ok(c) => {
            tracing::info!("portal backend org.freedesktop.impl.portal.desktop.aqua ready");
            Some(c)
        }
        Err(e) => {
            tracing::warn!("portal backend unavailable: {e}");
            None
        }
    };
    let have_frontend = backend.as_ref().map(frontend_available).unwrap_or(false);
    let conn = if have_frontend {
        tracing::info!("xdg-desktop-portal present: Aqua acts as its backend");
        None
    } else {
        let conn = zbus::blocking::connection::Builder::session()
            .and_then(|b| b.name("org.freedesktop.portal.Desktop"))
            .and_then(|b| b.serve_at(PATH, Settings { dark: flag.clone() }))
            .and_then(|b| b.build());
        match conn {
            Ok(c) => {
                tracing::info!("settings portal running on the session bus");
                Some(c)
            }
            Err(e) => {
                tracing::warn!("settings portal unavailable: {e}");
                None
            }
        }
    };
    Appearance { dark: flag, conn, backend }
}
