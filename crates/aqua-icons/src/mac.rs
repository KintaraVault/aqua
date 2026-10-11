//! Genuine macOS icons extracted from a Mac (`linuxtosh-assets/assets/mac`), installed
//! by `dist/install.sh` into `share/aqua/mac-icons`:
//!
//! * `modes/<App>_a<appearance>_v<variant>_<tint>.png` — IconServices renders of every
//!   app icon (512 px, icon grid). `a0_v0` (Default, light) is the source our icon look is
//!   applied to, like App Store artwork; `a1_v0` is the Dark style as macOS draws it.
//! * `system/*.png` — CoreTypes icons (Trash, folders), shown unstyled like the Dock does.
//!
//! The development layout (`icons-modes/`, `icons/`) is accepted too.

use aqua_gfx::Pixmap;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

struct Index {
    modes: PathBuf,
    system: PathBuf,
    /// Normalised app name → file prefix ("systemsettings" → "System Settings").
    apps: HashMap<String, String>,
    /// Apps from `/Applications` (third-party, matched by name without the counterpart policy).
    third_party: HashSet<String>,
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

fn roots() -> Vec<(PathBuf, PathBuf)> {
    let mut v = Vec::new();
    let mut add = |base: PathBuf| {
        v.push((base.join("modes"), base.join("system")));
        v.push((base.join("icons-modes"), base.join("icons")));
    };
    if let Some(d) = std::env::var_os("AQUA_MAC_ICONS") {
        add(PathBuf::from(d));
    }
    if let Some(p) = std::env::current_exe().ok().and_then(|e| e.parent()?.parent().map(Path::to_path_buf)) {
        add(p.join("share/aqua/mac-icons"));
    }
    add(PathBuf::from("/usr/local/share/aqua/mac-icons"));
    add(PathBuf::from("/usr/share/aqua/mac-icons"));
    if let Some(h) = std::env::var_os("HOME") {
        add(PathBuf::from(h).join("linuxtosh-assets/assets/mac"));
    }
    v
}

fn index() -> Option<&'static Index> {
    static IX: OnceLock<Option<Index>> = OnceLock::new();
    IX.get_or_init(|| {
        let (modes, system) = roots().into_iter().find(|(m, _)| m.is_dir())?;
        let mut apps = HashMap::new();
        for e in std::fs::read_dir(&modes).ok()?.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if let Some(app) = n.strip_suffix("_a0_v0_none.png") {
                apps.insert(norm(app), app.to_string());
            }
        }
        let third_party = std::fs::read_to_string(modes.join("apps.txt"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.strip_prefix("/Applications/")?.strip_suffix(".app").map(norm))
            .collect();
        Some(Index { modes, system, apps, third_party })
    })
    .as_ref()
}

/// Are the Mac icons installed?
pub fn available() -> bool {
    index().is_some()
}

/// The Mac app shown for one of our built-in icon keys (None: keep drawing it ourselves —
/// Calendar shows today's date).
pub fn builtin_app(key: &str) -> Option<&'static str> {
    Some(match key {
        "finder" => "Finder",
        "launchpad" | "apps" => "Apps",
        "safari" => "Safari",
        "messages" => "Messages",
        "mail" => "Mail",
        "maps" => "Maps",
        "photos" => "Photos",
        "facetime" => "FaceTime",
        "notes" => "Notes",
        "reminders" => "Reminders",
        "music" => "Music",
        "terminal" => "Terminal",
        "calculator" => "Calculator",
        "textedit" => "TextEdit",
        "preview" => "Preview",
        "activity" => "Activity Monitor",
        "appstore" => "App Store",
        "settings" => "System Settings",
        _ => return None,
    })
}

/// CoreTypes icon for a built-in key (trash, folders).
pub fn system_icon(key: &str) -> Option<Pixmap> {
    let f = match key {
        "trash" => "TrashIcon",
        "trash-full" => "FullTrashIcon",
        "downloads" => "DownloadsFolder",
        "folder" => "GenericFolderIcon",
        "documents" => "DocumentsFolderIcon",
        "desktop" => "DesktopFolderIcon",
        "applications" => "ApplicationsFolderIcon",
        "home" => "HomeFolderIcon",
        _ => return None,
    };
    aqua_gfx::load_image(&index()?.system.join(format!("{f}.png")))
}

/// Mac app whose icon a Linux app (desktop id, `Name=`, `Icon=`) genuinely is: a
/// third-party app installed on the Mac too ("org.telegram.desktop" → Telegram), or —
/// when `counterparts` is allowed — an app with exactly the Mac app's name ("Chess").
pub fn app_for(id: &str, name: &str, icon: &str, counterparts: bool) -> Option<&'static str> {
    let ix = index()?;
    let get = |k: &str| -> Option<&'static str> { ix.apps.get(k).map(|s| s.as_str()) };
    let full = norm(name);
    if !full.is_empty() && (counterparts || ix.third_party.contains(&full)) {
        if let Some(a) = get(&full) {
            return Some(a);
        }
    }
    const GENERIC: &[&str] =
        &["org", "com", "io", "net", "dev", "app", "desktop", "github", "gitlab", "gnome", "kde", "ai", "me", "www"];
    let first = name.split_whitespace().next().map(norm).unwrap_or_default();
    let toks = std::iter::once(first)
        .chain(id.split(['.', '-', '_']).map(norm))
        .chain(icon.rsplit('/').next().unwrap_or("").split(['.', '-', '_']).map(norm));
    for t in toks {
        if t.len() < 3 || GENERIC.contains(&t.as_str()) || !ix.third_party.contains(&t) {
            continue;
        }
        if let Some(a) = get(&t) {
            return Some(a);
        }
    }
    None
}

/// The icon of Mac app `app`: appearance 0/1, variant (0 Default/Dark, 2 Tinted, 3 Clear).
pub fn app_icon(app: &str, dark: bool, variant: u8, tint: &str) -> Option<Pixmap> {
    let ix = index()?;
    let a = dark as u8;
    let mut cands = vec![format!("{app}_a{a}_v{variant}_{tint}.png")];
    if dark {
        cands.push(format!("{app}_a0_v{variant}_{tint}.png"));
    }
    cands.push(format!("{app}_a0_v0_none.png"));
    cands.iter().find_map(|f| aqua_gfx::load_image(&ix.modes.join(f)))
}
