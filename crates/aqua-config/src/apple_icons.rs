//! Linux applications and their counterparts ("Firefox → Safari"), used to give
//! apps branded icons/names. Whether a replacement happens is a user choice
//! (System Settings → Appearance → App Icons): all apps, only selected apps, or off.

/// The branded icon an app is shown with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppleIcon {
    /// A built-in procedural icon (`aqua-icons` builtin key).
    Builtin(&'static str),
    /// Look the icon up on the Mac App Store under this name.
    Store(&'static str),
}

pub struct Counterpart {
    /// Name of the app ("Safari").
    pub apple: &'static str,
    pub icon: AppleIcon,
    /// Desktop ids / binaries / icon names of the Linux apps (canonical, lower case).
    pub ids: &'static [&'static str],
}

use AppleIcon::*;

pub const TABLE: &[Counterpart] = &[
    Counterpart {
        apple: "Finder",
        icon: Builtin("finder"),
        ids: &[
            "org.aqua.finder",
            "aqua-finder",
            "org.aqua.filechooser",
            "nautilus",
            "org.gnome.nautilus",
            "thunar",
            "org.xfce.thunar",
            "dolphin",
            "org.kde.dolphin",
            "pcmanfm",
            "pcmanfm-qt",
            "nemo",
            "caja",
            "files",
        ],
    },
    Counterpart {
        apple: "Safari",
        icon: Builtin("safari"),
        ids: &[
            "firefox",
            "org.mozilla.firefox",
            "firefox-esr",
            "firefox-developer-edition",
            "librewolf",
            "io.gitlab.librewolf-community",
            "floorp",
            "waterfox",
            "zen",
            "zen-browser",
            "app.zen_browser.zen",
            "chromium",
            "chromium-browser",
            "org.chromium.chromium",
            "google-chrome",
            "google-chrome-stable",
            "com.google.chrome",
            "brave",
            "brave-browser",
            "com.brave.browser",
            "vivaldi",
            "vivaldi-stable",
            "microsoft-edge",
            "microsoft-edge-stable",
            "opera",
            "epiphany",
            "org.gnome.epiphany",
            "falkon",
            "org.kde.falkon",
            "qutebrowser",
            "org.qutebrowser.qutebrowser",
        ],
    },
    Counterpart {
        apple: "Messages",
        icon: Builtin("messages"),
        ids: &[
            "org.telegram.desktop",
            "telegram-desktop",
            "telegram",
            "signal",
            "signal-desktop",
            "org.signal.signal",
            "element",
            "element-desktop",
            "im.riot.riot",
            "fractal",
            "org.gnome.fractal",
        ],
    },
    Counterpart {
        apple: "Mail",
        icon: Builtin("mail"),
        ids: &[
            "thunderbird",
            "org.mozilla.thunderbird",
            "net.thunderbird.thunderbird",
            "evolution",
            "org.gnome.evolution",
            "geary",
            "org.gnome.geary",
            "kmail",
            "org.kde.kmail2",
            "betterbird",
        ],
    },
    Counterpart {
        apple: "Calculator",
        icon: Builtin("calculator"),
        ids: &[
            "gnome-calculator",
            "org.gnome.calculator",
            "kcalc",
            "org.kde.kcalc",
            "galculator",
            "qalculate-gtk",
            "qalculate-qt",
            "calculator",
        ],
    },
    Counterpart {
        apple: "Terminal",
        icon: Builtin("terminal"),
        ids: &[
            "foot",
            "footclient",
            "kitty",
            "alacritty",
            "org.alacritty.alacritty",
            "gnome-terminal",
            "org.gnome.terminal",
            "org.gnome.console",
            "kgx",
            "org.gnome.ptyxis",
            "ptyxis",
            "konsole",
            "org.kde.konsole",
            "xterm",
            "uxterm",
            "weston-terminal",
            "wayland-terminal",
            "org.freedesktop.weston.wayland-terminal",
            "terminator",
            "tilix",
            "com.gexperts.tilix",
            "wezterm",
            "org.wezfurlong.wezterm",
            "terminal",
            "xfce4-terminal",
            "lxterminal",
            "qterminal",
            "ghostty",
            "com.mitchellh.ghostty",
        ],
    },
    Counterpart {
        apple: "System Settings",
        icon: Builtin("settings"),
        ids: &[
            "gnome-control-center",
            "org.gnome.settings",
            "systemsettings",
            "org.kde.systemsettings",
            "xfce4-settings-manager",
        ],
    },
    Counterpart {
        apple: "TextEdit",
        icon: Builtin("textedit"),
        ids: &[
            "gnome-text-editor",
            "org.gnome.texteditor",
            "gedit",
            "org.gnome.gedit",
            "mousepad",
            "org.xfce.mousepad",
            "kate",
            "org.kde.kate",
            "kwrite",
            "org.kde.kwrite",
            "pluma",
            "xed",
            "featherpad",
        ],
    },
    Counterpart {
        apple: "Preview",
        icon: Builtin("preview"),
        ids: &[
            "evince",
            "org.gnome.evince",
            "papers",
            "org.gnome.papers",
            "okular",
            "org.kde.okular",
            "eog",
            "org.gnome.eog",
            "loupe",
            "org.gnome.loupe",
            "gwenview",
            "org.kde.gwenview",
            "zathura",
            "org.pwmt.zathura",
            "atril",
            "ristretto",
        ],
    },
    Counterpart {
        apple: "Activity Monitor",
        icon: Builtin("activity"),
        ids: &[
            "gnome-system-monitor",
            "org.gnome.systemmonitor",
            "plasma-systemmonitor",
            "org.kde.plasma-systemmonitor",
            "htop",
            "btop",
            "io.missioncenter.missioncenter",
            "resources",
            "net.nokyan.resources",
        ],
    },
    Counterpart {
        apple: "Calendar",
        icon: Builtin("calendar"),
        ids: &["gnome-calendar", "org.gnome.calendar", "korganizer", "org.kde.korganizer"],
    },
    Counterpart {
        apple: "Maps",
        icon: Builtin("maps"),
        ids: &["gnome-maps", "org.gnome.maps", "marble", "org.kde.marble"],
    },
    Counterpart {
        apple: "Music",
        icon: Builtin("music"),
        ids: &[
            "rhythmbox",
            "org.gnome.rhythmbox3",
            "lollypop",
            "org.gnome.lollypop",
            "elisa",
            "org.kde.elisa",
            "amberol",
            "io.bassi.amberol",
            "spotify",
            "com.spotify.client",
            "strawberry",
            "audacious",
        ],
    },
    Counterpart {
        apple: "Photos",
        icon: Builtin("photos"),
        ids: &["shotwell", "org.gnome.shotwell", "gnome-photos", "org.gnome.photos", "digikam", "org.kde.digikam"],
    },
    Counterpart {
        apple: "App Store",
        icon: Builtin("appstore"),
        ids: &[
            "gnome-software",
            "org.gnome.software",
            "plasma-discover",
            "org.kde.discover",
            "pamac-manager",
            "org.manjaro.pamac.manager",
            "bauh",
        ],
    },
    Counterpart {
        apple: "Notes",
        icon: Builtin("notes"),
        ids: &["org.gnome.notes", "bijiben", "gnote", "org.gnome.gnote", "xournalpp", "com.github.xournalpp.xournalpp"],
    },
    Counterpart {
        apple: "FaceTime",
        icon: Builtin("facetime"),
        ids: &["cheese", "org.gnome.cheese", "snapshot", "org.gnome.snapshot", "kamoso", "org.kde.kamoso", "guvcview"],
    },
    Counterpart {
        apple: "Pages",
        icon: Store("Pages"),
        ids: &["libreoffice-writer", "writer", "org.libreoffice.libreoffice.writer", "abiword"],
    },
    Counterpart {
        apple: "Numbers",
        icon: Store("Numbers"),
        ids: &["libreoffice-calc", "calc", "org.libreoffice.libreoffice.calc", "gnumeric"],
    },
    Counterpart {
        apple: "Keynote",
        icon: Store("Keynote"),
        ids: &["libreoffice-impress", "impress", "org.libreoffice.libreoffice.impress"],
    },
    Counterpart { apple: "Xcode", icon: Store("Xcode"), ids: &["gnome-builder", "org.gnome.builder"] },
];

/// Canonical form of an id: lower case, no `.desktop`, version numbers dropped
/// (`libreoffice26.8-calc` → `libreoffice-calc`), path stripped.
pub fn canon(s: &str) -> String {
    let s = s.rsplit('/').next().unwrap_or(s).to_lowercase();
    let s = s.trim_end_matches(".desktop");
    let mut out = String::new();
    let mut prev_digit = false;
    for c in s.chars() {
        if c.is_ascii_digit() || (c == '.' && prev_digit) {
            prev_digit = true;
            continue;
        }
        prev_digit = false;
        out.push(c);
    }
    out
}

/// Counterpart of an app given its desktop id / binary and `Icon=` value.
pub fn lookup(id: &str, icon: &str) -> Option<&'static Counterpart> {
    lookup_index(id, icon).map(|i| &TABLE[i])
}

/// Index into [`TABLE`] of the app's counterpart.
pub fn lookup_index(id: &str, icon: &str) -> Option<usize> {
    let cands = [canon(id), canon(icon)];
    TABLE.iter().position(|c| cands.iter().any(|x| !x.is_empty() && c.ids.contains(&x.as_str())))
}

/// The user's choice: replace icons of all apps, only of `apps`, or none.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Policy {
    /// "all" (default), "selected" or "off".
    pub mode: String,
    pub apps: Vec<String>,
    /// In "all" mode only one app per counterpart becomes the branded app (the one pinned in the
    /// Dock, else the first installed): (TABLE index, canonical ids of that app).
    pub owners: Vec<(usize, Vec<String>)>,
}

impl Policy {
    pub fn from_config(c: &crate::Config) -> Self {
        Self { mode: c.apple_icons.clone(), apps: c.apple_icon_apps.clone(), owners: vec![] }
    }

    /// May the app `id` (desktop id or binary; `icon` = its `Icon=`) be shown as its
    /// counterpart?
    pub fn allows(&self, id: &str, icon: &str) -> bool {
        match self.mode.as_str() {
            "off" => false,
            "selected" => {
                let cands = [canon(id), canon(icon)];
                self.apps.iter().map(|a| canon(a)).any(|a| !a.is_empty() && cands.contains(&a))
            }
            _ => {
                let Some(g) = lookup_index(id, icon) else { return true };
                match self.owners.iter().find(|(i, _)| *i == g) {
                    Some((_, ids)) => {
                        let cands = [canon(id), canon(icon)];
                        ids.iter()
                            .any(|o| cands.iter().any(|c| !c.is_empty() && (c == o || c.ends_with(&format!(".{o}")))))
                    }
                    None => true,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lookups() {
        assert_eq!(lookup("firefox", "firefox").map(|c| c.apple), Some("Safari"));
        assert_eq!(lookup("libreoffice26.8-calc", "").map(|c| c.apple), Some("Numbers"));
        assert!(lookup("gimp", "gimp").is_none());
        let p = Policy { mode: "selected".into(), apps: vec!["org.gnome.Nautilus".into()], owners: vec![] };
        assert!(p.allows("org.gnome.Nautilus", ""));
        assert!(!p.allows("firefox", "firefox"));
        assert!(!Policy { mode: "off".into(), apps: vec![], owners: vec![] }.allows("firefox", ""));
        assert!(Policy { mode: "all".into(), apps: vec![], owners: vec![] }.allows("firefox", ""));
        let safari = lookup_index("firefox", "").unwrap();
        let p = Policy { mode: "all".into(), apps: vec![], owners: vec![(safari, vec!["firefox".into()])] };
        assert!(p.allows("firefox", "firefox"));
        assert!(p.allows("org.mozilla.firefox", ""));
        assert!(!p.allows("zen", "zen-browser"));
    }
}
