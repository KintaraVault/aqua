//! System keyboard shortcuts: ids, default chords and user remapping.
//!
//! Chords are written like `super+shift+3`, `ctrl+up`, `F3`. The compositor and
//! System Settings share this table, so remapped shortcuts show up everywhere.
use crate::Config;

pub struct SystemShortcut {
    pub id: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub defaults: &'static [&'static str],
}

const fn sc(
    id: &'static str,
    group: &'static str,
    label: &'static str,
    defaults: &'static [&'static str],
) -> SystemShortcut {
    SystemShortcut { id, label, group, defaults }
}

/// Every remappable system shortcut, grouped by section.
pub const SYSTEM: &[SystemShortcut] = &[
    sc("spotlight", "Spotlight", "Show Spotlight search", &["super+space", "alt+space"]),
    sc("switch-apps", "App Switcher", "Switch applications (⇧ goes back)", &["super+tab", "alt+tab"]),
    sc("mission", "Mission Control", "Mission Control", &["F3", "ctrl+up"]),
    sc("show-desktop", "Mission Control", "Show Desktop", &[]),
    sc("space-left", "Mission Control", "Move left a space", &["ctrl+left"]),
    sc("space-right", "Mission Control", "Move right a space", &["ctrl+right"]),
    sc("move-left", "Mission Control", "Move window one space left", &["ctrl+shift+left"]),
    sc("move-right", "Mission Control", "Move window one space right", &["ctrl+shift+right"]),
    sc("launchpad", "Launchpad & Dock", "Show Launchpad", &["F4"]),
    sc("control-center", "Launchpad & Dock", "Show Control Centre", &["F5"]),
    sc("notifications", "Launchpad & Dock", "Show Notification Centre", &[]),
    sc("close-window", "Windows", "Close window", &["super+w"]),
    sc("minimize", "Windows", "Minimise window", &["super+m"]),
    sc("zoom", "Windows", "Zoom window", &[]),
    sc("fullscreen", "Windows", "Enter / exit full screen", &["super+ctrl+f"]),
    sc("quit-app", "Windows", "Quit application", &["super+q"]),
    sc("hide-others", "Windows", "Hide others", &["super+alt+h"]),
    sc("force-quit", "Windows", "Force Quit Applications…", &["super+alt+Escape"]),
    sc("screenshot", "Screenshots", "Save picture of screen as a file", &["super+shift+3", "Print"]),
    sc("screenshot-area", "Screenshots", "Save picture of selected area as a file", &["super+shift+4", "shift+Print"]),
    sc("screenshot-window", "Screenshots", "Save picture of a window as a file", &["alt+Print"]),
    sc("screenshot-ui", "Screenshots", "Screenshot and recording options", &["super+shift+5"]),
    sc("record-stop", "Screenshots", "Stop screen recording", &["super+ctrl+Escape"]),
    sc("terminal", "Apps", "Open Terminal", &["super+Return"]),
    sc("settings", "Apps", "Open System Settings", &[]),
    sc("clipboard", "Apps", "Clipboard history", &["super+shift+v"]),
    sc("chars", "Apps", "Emoji & Symbols", &["super+ctrl+space"]),
    sc("lock", "Session", "Lock Screen", &["super+ctrl+q"]),
    sc("sleep", "Session", "Sleep", &[]),
    sc("quit-aqua", "Session", "Log out of Aqua", &["ctrl+alt+BackSpace"]),
];

/// Named actions a custom shortcut (or hot corner) can run; anything else is a shell command.
pub const ACTIONS: &[(&str, &str)] = &[
    ("spotlight", "Spotlight"),
    ("mission", "Mission Control"),
    ("show-desktop", "Show Desktop"),
    ("launchpad", "Launchpad"),
    ("notifications", "Notification Centre"),
    ("control-center", "Control Centre"),
    ("clipboard", "Clipboard history"),
    ("chars", "Emoji & Symbols"),
    ("screenshot", "Screenshot"),
    ("screenshot-area", "Screenshot of selected area"),
    ("screenshot-window", "Screenshot of a window"),
    ("screenshot-ui", "Screenshot toolbar"),
    ("record", "Record screen"),
    ("record-stop", "Stop screen recording"),
    ("fullscreen", "Toggle full screen"),
    ("zoom", "Zoom window"),
    ("minimize", "Minimise window"),
    ("close-window", "Close window"),
    ("terminal", "Open Terminal"),
    ("settings", "Open System Settings"),
    ("next-layout", "Next input source"),
    ("lock", "Lock Screen"),
    ("sleep", "Sleep"),
    ("volume-up", "Volume up"),
    ("volume-down", "Volume down"),
    ("mute", "Mute"),
    ("brightness-up", "Brightness up"),
    ("brightness-down", "Brightness down"),
];

pub fn find(id: &str) -> Option<&'static SystemShortcut> {
    SYSTEM.iter().find(|s| s.id == id)
}

/// Effective chords of a system shortcut (user override or the defaults).
pub fn chords(cfg: &Config, id: &str) -> Vec<String> {
    match cfg.shortcuts.get(id) {
        Some(v) => split(v),
        None => find(id).map(|s| s.defaults.iter().map(|d| d.to_string()).collect()).unwrap_or_default(),
    }
}

pub fn split(v: &str) -> Vec<String> {
    v.split(',').map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect()
}

pub fn is_default(cfg: &Config, id: &str) -> bool {
    !cfg.shortcuts.contains_key(id)
}

/// Normalised form used for comparisons ("Super+Shift+3" == "shift+super+3").
pub fn normalize(chord: &str) -> String {
    let mut mods = [false; 4];
    let mut key = String::new();
    for p in chord.split('+').map(|p| p.trim()) {
        match p.to_lowercase().as_str() {
            "ctrl" | "control" | "ctl" => mods[0] = true,
            "alt" | "opt" | "option" | "mod1" => mods[1] = true,
            "shift" => mods[2] = true,
            "super" | "logo" | "cmd" | "command" | "win" | "mod4" => mods[3] = true,
            "" => {}
            k => key = k.to_string(),
        }
    }
    let mut out = vec![];
    for (i, n) in ["ctrl", "alt", "shift", "super"].iter().enumerate() {
        if mods[i] {
            out.push(n.to_string());
        }
    }
    if !key.is_empty() {
        out.push(key);
    }
    out.join("+")
}

/// Which shortcut (system id or custom binding index) already uses a chord.
pub fn conflict(
    cfg: &Config,
    chord: &str,
    except_system: Option<&str>,
    except_custom: Option<usize>,
) -> Option<String> {
    let n = normalize(chord);
    for s in SYSTEM {
        if Some(s.id) == except_system {
            continue;
        }
        if chords(cfg, s.id).iter().any(|c| normalize(c) == n) {
            return Some(s.label.to_string());
        }
    }
    for (i, b) in cfg.bindings.iter().enumerate() {
        if Some(i) == except_custom {
            continue;
        }
        if normalize(&b.keys) == n {
            return Some(action_label(&b.action));
        }
    }
    None
}

pub fn action_label(action: &str) -> String {
    ACTIONS.iter().find(|(a, _)| *a == action).map(|(_, l)| l.to_string()).unwrap_or_else(|| action.to_string())
}

/// Symbol rendering: `super+shift+3` → "⇧⌘3".
pub fn pretty(chord: &str) -> String {
    let n = normalize(chord);
    let mut s = String::new();
    let mut key = "";
    let parts: Vec<&str> = n.split('+').collect();
    for p in &parts {
        match *p {
            "ctrl" => s.push('⌃'),
            "alt" => s.push('⌥'),
            "shift" => s.push('⇧'),
            "super" => s.push('⌘'),
            k => key = k,
        }
    }
    let k = match key {
        "space" => "Space".to_string(),
        "return" | "enter" => "↩".to_string(),
        "tab" => "⇥".to_string(),
        "escape" | "esc" => "⎋".to_string(),
        "backspace" => "⌫".to_string(),
        "delete" => "⌦".to_string(),
        "left" => "←".to_string(),
        "right" => "→".to_string(),
        "up" => "↑".to_string(),
        "down" => "↓".to_string(),
        "print" => "PrtSc".to_string(),
        "minus" => "-".to_string(),
        "equal" | "plus" => "=".to_string(),
        "bracketleft" => "[".to_string(),
        "bracketright" => "]".to_string(),
        "backslash" => "\\".to_string(),
        "semicolon" => ";".to_string(),
        "apostrophe" => "'".to_string(),
        "comma" => ",".to_string(),
        "period" => ".".to_string(),
        "slash" => "/".to_string(),
        "grave" => "`".to_string(),
        "prior" => "PgUp".to_string(),
        "next" => "PgDn".to_string(),
        "home" => "↖".to_string(),
        "end" => "↘".to_string(),
        "" => String::new(),
        k if k.len() == 1 => k.to_uppercase(),
        k => {
            let mut c = k.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        }
    };
    s + &k
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalize_and_pretty() {
        assert_eq!(normalize("Shift+Super+3"), "shift+super+3");
        assert_eq!(pretty("super+shift+3"), "⇧⌘3");
        assert_eq!(pretty("ctrl+up"), "⌃↑");
        assert_eq!(pretty("F3"), "F3");
        let mut cfg = Config::default();
        assert_eq!(chords(&cfg, "spotlight"), vec!["super+space", "alt+space"]);
        cfg.shortcuts.insert("spotlight".into(), "super+k".into());
        assert_eq!(chords(&cfg, "spotlight"), vec!["super+k"]);
        assert_eq!(conflict(&cfg, "SUPER+K", None, None).as_deref(), Some("Show Spotlight search"));
        assert!(conflict(&cfg, "super+k", Some("spotlight"), None).is_none());
    }
}
