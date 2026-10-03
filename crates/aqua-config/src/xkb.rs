//! XKB layout catalogue from xkeyboard-config's rules (`evdev.lst`), with a small
//! built-in fallback so the Settings dropdown works even before the package exists.
use std::collections::BTreeMap;

pub const RULES: &[&str] = &[
    "/usr/share/X11/xkb/rules/evdev.lst",
    "/usr/share/X11/xkb/rules/base.lst",
    "/usr/local/share/X11/xkb/rules/evdev.lst",
];

#[derive(Debug, Clone, Default)]
pub struct Catalogue {
    /// (code, description), sorted by description.
    pub layouts: Vec<(String, String)>,
    /// layout code -> [(variant, description)]
    pub variants: BTreeMap<String, Vec<(String, String)>>,
    /// rules file found (false = built-in fallback list).
    pub from_system: bool,
}

const FALLBACK: &[(&str, &str)] = &[
    ("us", "English (US)"),
    ("gb", "English (UK)"),
    ("ru", "Russian"),
    ("ua", "Ukrainian"),
    ("by", "Belarusian"),
    ("kz", "Kazakh"),
    ("de", "German"),
    ("fr", "French"),
    ("es", "Spanish"),
    ("it", "Italian"),
    ("pt", "Portuguese"),
    ("br", "Portuguese (Brazil)"),
    ("pl", "Polish"),
    ("cz", "Czech"),
    ("sk", "Slovak"),
    ("hu", "Hungarian"),
    ("ro", "Romanian"),
    ("bg", "Bulgarian"),
    ("rs", "Serbian"),
    ("hr", "Croatian"),
    ("si", "Slovenian"),
    ("gr", "Greek"),
    ("tr", "Turkish"),
    ("se", "Swedish"),
    ("no", "Norwegian"),
    ("dk", "Danish"),
    ("fi", "Finnish"),
    ("ee", "Estonian"),
    ("lv", "Latvian"),
    ("lt", "Lithuanian"),
    ("nl", "Dutch"),
    ("be", "Belgian"),
    ("ch", "German (Switzerland)"),
    ("il", "Hebrew"),
    ("ara", "Arabic"),
    ("ir", "Persian"),
    ("in", "Indian"),
    ("th", "Thai"),
    ("vn", "Vietnamese"),
    ("jp", "Japanese"),
    ("kr", "Korean"),
    ("cn", "Chinese"),
    ("tw", "Taiwanese"),
    ("ge", "Georgian"),
    ("am", "Armenian"),
    ("az", "Azerbaijani"),
    ("uz", "Uzbek"),
    ("kg", "Kyrgyz"),
    ("tj", "Tajik"),
    ("mn", "Mongolian"),
    ("latam", "Spanish (Latin American)"),
    ("ca", "French (Canada)"),
    ("is", "Icelandic"),
    ("ie", "Irish"),
    ("mk", "Macedonian"),
    ("al", "Albanian"),
    ("md", "Moldavian"),
];

pub fn rules_path() -> Option<&'static str> {
    RULES.iter().copied().find(|p| std::path::Path::new(p).exists())
}

pub fn catalogue() -> Catalogue {
    let Some(text) = rules_path().and_then(|p| std::fs::read_to_string(p).ok()) else {
        let mut layouts: Vec<(String, String)> = FALLBACK.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        layouts.sort_by(|a, b| a.1.cmp(&b.1));
        return Catalogue { layouts, variants: BTreeMap::new(), from_system: false };
    };
    parse(&text)
}

pub fn parse(text: &str) -> Catalogue {
    let mut c = Catalogue { from_system: true, ..Default::default() };
    let mut section = "";
    for line in text.lines() {
        if let Some(s) = line.strip_prefix("! ") {
            section = match s.trim() {
                "layout" => "layout",
                "variant" => "variant",
                _ => "",
            };
            continue;
        }
        let t = line.trim_start();
        if t.is_empty() {
            continue;
        }
        let (code, rest) = match t.split_once(char::is_whitespace) {
            Some((a, b)) => (a.trim(), b.trim()),
            None => continue,
        };
        match section {
            "layout" => c.layouts.push((code.to_string(), rest.to_string())),
            "variant" => {
                if let Some((layout, desc)) = rest.split_once(':') {
                    c.variants
                        .entry(layout.trim().to_string())
                        .or_default()
                        .push((code.to_string(), desc.trim().to_string()));
                }
            }
            _ => {}
        }
    }
    c.layouts.sort_by_key(|a| a.1.to_lowercase());
    for v in c.variants.values_mut() {
        v.sort_by_key(|a| a.1.to_lowercase());
    }
    c
}

impl Catalogue {
    /// Human name of "ru", "us(dvorak)" or layout + variant.
    pub fn describe(&self, layout: &str, variant: &str) -> String {
        let (l, v) = match layout.split_once('(') {
            Some((l, v)) => (l, v.trim_end_matches(')')),
            None => (layout, variant),
        };
        if !v.is_empty() {
            if let Some(d) = self.variants.get(l).and_then(|vs| vs.iter().find(|(c, _)| c == v)) {
                return d.1.clone();
            }
        }
        self.layouts.iter().find(|(c, _)| c == l).map(|x| x.1.clone()).unwrap_or_else(|| l.to_uppercase())
    }
}

/// Packages (per package manager) that make a layout fully usable: the XKB data
/// itself, fonts for its script and an input method for CJK.
pub fn dependencies(layout: &str, pm: &str) -> Vec<&'static str> {
    let arch = pm == "pacman";
    let deb = pm == "apt";
    let mut v: Vec<&'static str> = vec![if deb { "xkb-data" } else { "xkeyboard-config" }];
    match layout {
        "jp" => v.extend(if arch {
            ["fcitx5", "fcitx5-mozc", "fcitx5-gtk", "fcitx5-qt", "noto-fonts-cjk"].as_slice()
        } else if deb {
            ["fcitx5", "fcitx5-mozc", "fonts-noto-cjk"].as_slice()
        } else {
            ["fcitx5", "fcitx5-mozc", "google-noto-sans-cjk-fonts"].as_slice()
        }),
        "kr" => v.extend(if arch {
            ["fcitx5", "fcitx5-hangul", "fcitx5-gtk", "fcitx5-qt", "noto-fonts-cjk"].as_slice()
        } else if deb {
            ["fcitx5", "fcitx5-hangul", "fonts-noto-cjk"].as_slice()
        } else {
            ["fcitx5", "fcitx5-hangul", "google-noto-sans-cjk-fonts"].as_slice()
        }),
        "cn" | "tw" => v.extend(if arch {
            ["fcitx5", "fcitx5-chinese-addons", "fcitx5-gtk", "fcitx5-qt", "noto-fonts-cjk"].as_slice()
        } else if deb {
            ["fcitx5", "fcitx5-chinese-addons", "fonts-noto-cjk"].as_slice()
        } else {
            ["fcitx5", "fcitx5-chinese-addons", "google-noto-sans-cjk-fonts"].as_slice()
        }),
        "ara" | "ir" | "af" | "pk" | "iq" | "sy" | "il" | "th" | "in" | "bd" | "np" | "lk" | "et" | "ge" | "am"
        | "kh" | "la" | "mm" => v.push(if arch {
            "noto-fonts"
        } else if deb {
            "fonts-noto-core"
        } else {
            "google-noto-sans-fonts"
        }),
        _ => {}
    }
    v
}

/// Layouts that need an input method framework (typing goes through fcitx5).
pub fn needs_ime(layout: &str) -> bool {
    matches!(layout, "jp" | "kr" | "cn" | "tw")
}

/// The system package manager: "pacman", "apt", "dnf", "zypper" or "".
pub fn package_manager() -> &'static str {
    for (bin, name) in [
        ("/usr/bin/pacman", "pacman"),
        ("/usr/bin/apt-get", "apt"),
        ("/usr/bin/dnf", "dnf"),
        ("/usr/bin/zypper", "zypper"),
    ] {
        if std::path::Path::new(bin).exists() {
            return name;
        }
    }
    ""
}

/// Is a package installed?
pub fn installed(pkg: &str, pm: &str) -> bool {
    let st = match pm {
        "pacman" => std::process::Command::new("pacman").args(["-Qq", pkg]).output(),
        "apt" => std::process::Command::new("dpkg").args(["-s", pkg]).output(),
        "dnf" | "zypper" => std::process::Command::new("rpm").args(["-q", pkg]).output(),
        _ => return true,
    };
    st.map(|o| o.status.success()).unwrap_or(true)
}

/// Command line that installs `pkgs` as root through polkit (pkexec).
pub fn install_command(pkgs: &[&str], pm: &str) -> Option<Vec<String>> {
    if pkgs.is_empty() {
        return None;
    }
    let mut v: Vec<String> = vec!["pkexec".into()];
    match pm {
        "pacman" => v.extend(["pacman", "-S", "--needed", "--noconfirm"].map(String::from)),
        "apt" => v.extend(["apt-get", "install", "-y"].map(String::from)),
        "dnf" => v.extend(["dnf", "install", "-y"].map(String::from)),
        "zypper" => v.extend(["zypper", "--non-interactive", "install"].map(String::from)),
        _ => return None,
    }
    v.extend(pkgs.iter().map(|s| s.to_string()));
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_rules() {
        let t = "! model\n  pc105  Generic\n\n! layout\n  us              English (US)\n  ru              Russian\n\n! variant\n  dvorak          us: English (Dvorak)\n  phonetic        ru: Russian (phonetic)\n\n! option\n  grp  Switching\n";
        let c = parse(t);
        assert_eq!(c.layouts.len(), 2);
        assert_eq!(c.layouts[0].0, "us");
        assert_eq!(c.variants["ru"][0].0, "phonetic");
        assert_eq!(c.describe("ru", ""), "Russian");
        assert_eq!(c.describe("us(dvorak)", ""), "English (Dvorak)");
        assert_eq!(c.describe("us", "dvorak"), "English (Dvorak)");
    }
    #[test]
    fn deps() {
        assert!(dependencies("jp", "pacman").contains(&"fcitx5-mozc"));
        assert_eq!(dependencies("ru", "pacman"), vec!["xkeyboard-config"]);
        assert_eq!(install_command(&["a"], "pacman").unwrap()[0], "pkexec");
    }
}
