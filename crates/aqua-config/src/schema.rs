//! Config file schema: versioned migrations, validation and a JSON Schema for editors.
//!
//! Loading goes `TOML text → table → migrate → drop invalid values → deserialize → clamp`;
//! every step reports what it changed as an [`Issue`], so one typo costs one setting
//! instead of the whole file.
use crate::Config;
use toml::{Table, Value};

/// Current config format. History:
/// 1. unversioned files; Dark Mode was only the `dark` flag.
/// 2. `version` key, `appearance` decides Dark Mode, per-display Spaces.
pub const CONFIG_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    /// Dotted key path (`pointer.speed`, `dock[2].name`); empty for the whole file.
    pub path: String,
    pub message: String,
}

impl std::fmt::Display for Issue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.path.is_empty() {
            f.write_str(&self.message)
        } else {
            write!(f, "{}: {}", self.path, self.message)
        }
    }
}

fn issue(path: impl Into<String>, message: impl Into<String>) -> Issue {
    Issue { path: path.into(), message: message.into() }
}

/// Version a file was written with (files without the key are version 1).
pub fn file_version(t: &Table) -> u32 {
    t.get("version").and_then(Value::as_integer).map(|v| v.clamp(0, u32::MAX as i64) as u32).unwrap_or(1)
}

/// Upgrade a parsed file to [`CONFIG_VERSION`] in place; returns what was changed.
pub fn migrate(t: &mut Table) -> Vec<String> {
    let from = file_version(t);
    let mut notes = vec![];
    if from < 2 && !t.contains_key("appearance") && t.get("dark").and_then(Value::as_bool) == Some(true) {
        t.insert("appearance".into(), Value::String("dark".into()));
        notes.push("appearance = \"dark\" (from dark = true)".into());
    }
    if let Some(dock) = t.get_mut("dock").and_then(Value::as_array_mut) {
        for it in dock.iter_mut().filter_map(Value::as_table_mut) {
            let finder = it.get("icon").and_then(Value::as_str) == Some("builtin:finder")
                && it.get("exec").and_then(Value::as_str) == Some("nautilus|thunar|dolphin|pcmanfm");
            if !finder {
                continue;
            }
            it.insert("exec".into(), Value::String("aqua-finder|nautilus|thunar|dolphin|pcmanfm".into()));
            let ids = it.entry("ids").or_insert_with(|| Value::Array(vec![]));
            if let Some(ids) = ids.as_array_mut() {
                for id in ["aqua-finder", "org.aqua.finder"] {
                    if !ids.iter().any(|v| v.as_str() == Some(id)) {
                        ids.insert(0, Value::String(id.into()));
                    }
                }
            }
            notes.push("Dock Finder opens Aqua's Finder".into());
        }
    }
    if from < CONFIG_VERSION {
        t.insert("version".into(), Value::Integer(CONFIG_VERSION as i64));
    }
    notes
}

fn try_config(v: Value) -> Result<Config, String> {
    v.try_into::<Config>().map_err(|e| e.message().trim().to_string())
}

fn single(key: &str, v: Value) -> Result<Config, String> {
    let mut t = Table::new();
    t.insert(key.into(), v);
    try_config(Value::Table(t))
}

/// Remove values that fail to deserialize, descending one level into tables and arrays so
/// that only the offending entry goes.
fn prune_invalid(t: &mut Table, issues: &mut Vec<Issue>) {
    if try_config(Value::Table(t.clone())).is_ok() {
        return;
    }
    let keys: Vec<String> = t.keys().cloned().collect();
    for k in keys {
        let v = t[&k].clone();
        let Err(err) = single(&k, v.clone()) else { continue };
        match v {
            Value::Table(sub) => {
                let mut kept = Table::new();
                for (sk, sv) in sub {
                    let mut one = Table::new();
                    one.insert(sk.clone(), sv.clone());
                    match single(&k, Value::Table(one)) {
                        Ok(_) => {
                            kept.insert(sk, sv);
                        }
                        Err(e) => issues.push(issue(format!("{k}.{sk}"), format!("{e}; using the default"))),
                    }
                }
                t.insert(k, Value::Table(kept));
            }
            Value::Array(items) if items.iter().all(Value::is_table) => {
                let mut kept = vec![];
                for (i, it) in items.into_iter().enumerate() {
                    match single(&k, Value::Array(vec![it.clone()])) {
                        Ok(_) => kept.push(it),
                        Err(e) => issues.push(issue(format!("{k}[{i}]"), format!("{e}; entry ignored"))),
                    }
                }
                t.insert(k, Value::Array(kept));
            }
            _ => {
                t.remove(&k);
                issues.push(issue(k, format!("{err}; using the default")));
            }
        }
    }
}

/// Parse, migrate and validate a config file.
pub fn check(src: &str) -> Result<(Config, Vec<Issue>), toml::de::Error> {
    let mut t: Table = toml::from_str(src)?;
    let mut issues = vec![];
    let v = file_version(&t);
    if v > CONFIG_VERSION {
        issues.push(issue("version", format!("written by a newer Aqua (format {v}, this one reads {CONFIG_VERSION})")));
    }
    migrate(&mut t);
    prune_invalid(&mut t, &mut issues);
    let mut unknown = vec![];
    let res = serde_ignored::deserialize(Value::Table(t), |p| unknown.push(p.to_string()));
    let mut cfg: Config = match res {
        Ok(c) => c,
        Err(e) => {
            issues.push(issue("", format!("{}; defaults used", e.message().trim())));
            Config::default()
        }
    };
    for p in unknown {
        issues.push(issue(p, "unknown key (ignored)"));
    }
    validate(&mut cfg, &mut issues);
    Ok((cfg, issues))
}

fn range<T: PartialOrd + Copy + std::fmt::Display>(issues: &mut Vec<Issue>, path: &str, v: &mut T, lo: T, hi: T) {
    #[allow(clippy::eq_op)]
    let nan = *v != *v;
    if nan || *v < lo || *v > hi {
        let nv = if nan || *v < lo { lo } else { hi };
        issues.push(issue(path, format!("{v} is outside {lo}…{hi}; using {nv}")));
        *v = nv;
    }
}

fn one_of(issues: &mut Vec<Issue>, path: &str, v: &mut String, allowed: &[&str], default: &str) {
    if !allowed.contains(&v.as_str()) {
        issues.push(issue(path, format!("\"{v}\" is not one of {}; using \"{default}\"", allowed.join(", "))));
        *v = default.into();
    }
}

pub const ACCENTS: &[&str] = &["multicolor", "blue", "purple", "pink", "red", "orange", "yellow", "green", "graphite"];

/// Clamp numbers to their supported ranges and reset unknown choices to the default.
pub fn validate(c: &mut Config, issues: &mut Vec<Issue>) {
    let d = Config::default();
    range(issues, "menubar_height", &mut c.menubar_height, 18.0, 48.0);
    range(issues, "dock_icon_size", &mut c.dock_icon_size, 16.0, 128.0);
    range(issues, "dock_magnification", &mut c.dock_magnification, 1.0, 3.0);
    range(issues, "window_radius", &mut c.window_radius, 0.0, 40.0);
    range(issues, "cursor_size", &mut c.cursor_size, 1.0, 4.0);
    range(issues, "screenshot_timer", &mut c.screenshot_timer, 0, 60);
    range(issues, "blur_max_fps", &mut c.blur_max_fps, 0, 240);
    range(issues, "glass.blur", &mut c.glass.blur, 0.0, 100.0);
    range(issues, "glass.saturation", &mut c.glass.saturation, 0.0, 4.0);
    range(issues, "glass.max_luma", &mut c.glass.max_luma, 0.0, 1.0);
    range(issues, "keyboard.repeat_delay", &mut c.keyboard.repeat_delay, 100, 2000);
    range(issues, "keyboard.repeat_rate", &mut c.keyboard.repeat_rate, 1, 100);
    range(issues, "pointer.speed", &mut c.pointer.speed, -1.0, 1.0);
    range(issues, "pointer.mouse_speed", &mut c.pointer.mouse_speed, -1.0, 1.0);
    range(issues, "pointer.scroll_factor", &mut c.pointer.scroll_factor, 0.1, 5.0);
    for (i, o) in c.outputs.iter_mut().enumerate() {
        if o.scale != 0.0 {
            range(issues, &format!("outputs[{i}].scale"), &mut o.scale, 0.5, 4.0);
        }
    }
    one_of(issues, "apple_icons", &mut c.apple_icons, &["all", "selected", "off"], &d.apple_icons);
    one_of(issues, "icon_style", &mut c.icon_style, &["default", "auto", "dark", "clear", "tinted"], &d.icon_style);
    one_of(issues, "minimize_effect", &mut c.minimize_effect, &["genie", "scale"], &d.minimize_effect);
    one_of(issues, "accent", &mut c.accent, ACCENTS, &d.accent);
    one_of(issues, "appearance", &mut c.appearance, &["light", "dark", "auto"], &d.appearance);
    one_of(
        issues,
        "menubar_autohide",
        &mut c.menubar_autohide,
        &["fullscreen", "always", "never"],
        &d.menubar_autohide,
    );
    one_of(issues, "dock_click", &mut c.dock_click, &["focus", "minimize", "cycle", "expose", "new"], &d.dock_click);
    one_of(
        issues,
        "screenshot_save",
        &mut c.screenshot_save,
        &["pictures", "desktop", "clipboard"],
        &d.screenshot_save,
    );
    one_of(issues, "record_save", &mut c.record_save, &["movies", "desktop"], &d.record_save);
    one_of(
        issues,
        "titlebar_double_click",
        &mut c.titlebar_double_click,
        &["zoom", "minimize", "none"],
        &d.titlebar_double_click,
    );
    one_of(issues, "pointer.accel_profile", &mut c.pointer.accel_profile, &["adaptive", "flat"], "adaptive");
    one_of(issues, "pointer.secondary_click", &mut c.pointer.secondary_click, &["fingers", "corner"], "fingers");
    for (i, b) in c.bindings.iter().enumerate() {
        if b.keys.trim().is_empty() || b.action.trim().is_empty() {
            issues.push(issue(format!("bindings[{i}]"), "needs both `keys` and `action`"));
        }
    }
    for (i, it) in c.dock.iter().enumerate() {
        if it.name.trim().is_empty() {
            issues.push(issue(format!("dock[{i}].name"), "empty name"));
        }
    }
}

/// JSON Schema of `config.toml` (for editors: taplo / Even Better TOML).
pub fn json_schema() -> String {
    let mut s = schemars::schema_for!(Config);
    if let Some(m) = s.schema.metadata.as_mut() {
        m.title = Some("Aqua configuration (config.toml)".into());
    }
    serde_json::to_string_pretty(&s).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(i: &[Issue]) -> Vec<&str> {
        i.iter().map(|x| x.path.as_str()).collect()
    }

    #[test]
    fn clean_file_has_no_issues() {
        let src = toml::to_string(&Config::default()).unwrap();
        let (c, issues) = check(&src).unwrap();
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(c.version, CONFIG_VERSION);
        assert!(check("").unwrap().1.is_empty());
    }

    #[test]
    fn one_bad_value_keeps_the_rest() {
        let (c, issues) =
            check("dock_icon_size = \"big\"\nclock_24h = false\n[pointer]\nspeed = \"fast\"\nnatural_scroll = false\n")
                .unwrap();
        assert_eq!(paths(&issues), vec!["dock_icon_size", "pointer.speed"]);
        assert_eq!(c.dock_icon_size, Config::default().dock_icon_size);
        assert!(!c.clock_24h);
        assert!(!c.pointer.natural_scroll);
        assert_eq!(c.pointer.speed, 0.0);
    }

    #[test]
    fn bad_array_entries_are_dropped() {
        let src = "[[dock]]\nname = \"A\"\n[[dock]]\napp = \"no-name\"\n[[dock]]\nname = \"B\"\n";
        let (c, issues) = check(src).unwrap();
        assert_eq!(c.dock.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), vec!["A", "B"]);
        assert_eq!(paths(&issues), vec!["dock[1]"]);
    }

    #[test]
    fn unknown_keys_are_reported() {
        let (_, issues) = check("dock_icon_sise = 40\n[keyboard]\nlayouts = [\"us\"]\nrepeat = 3\n").unwrap();
        assert_eq!(paths(&issues), vec!["dock_icon_sise", "keyboard.repeat"]);
        assert!(issues[0].message.contains("unknown"));
    }

    #[test]
    fn ranges_and_choices_are_enforced() {
        let src = "dock_icon_size = 500.0\ncursor_size = 0.2\naccent = \"teal\"\n[pointer]\nspeed = 3.0\n[[outputs]]\nname = \"X\"\nscale = 9.0\n";
        let (c, issues) = check(src).unwrap();
        assert_eq!(c.dock_icon_size, 128.0);
        assert_eq!(c.cursor_size, 1.0);
        assert_eq!(c.accent, "multicolor");
        assert_eq!(c.pointer.speed, 1.0);
        assert_eq!(c.outputs[0].scale, 4.0);
        assert_eq!(
            paths(&issues),
            vec!["dock_icon_size", "cursor_size", "pointer.speed", "outputs[0].scale", "accent"]
        );
    }

    #[test]
    fn version_one_files_are_migrated() {
        let mut t: Table = toml::from_str("dark = true\n").unwrap();
        let notes = migrate(&mut t);
        assert_eq!(t.get("appearance").and_then(Value::as_str), Some("dark"));
        assert_eq!(file_version(&t), CONFIG_VERSION);
        assert_eq!(notes.len(), 1);
        assert!(migrate(&mut t).is_empty(), "migrations are idempotent");
        let c = Config::from_toml("dark = true\n").unwrap();
        assert!(c.dark);
        let mut t2: Table = toml::from_str("version = 2\ndark = true\n").unwrap();
        migrate(&mut t2);
        assert!(!t2.contains_key("appearance"), "current files are left alone");
    }

    #[test]
    fn newer_files_are_flagged() {
        let (_, issues) = check(&format!("version = {}\n", CONFIG_VERSION + 1)).unwrap();
        assert_eq!(paths(&issues), vec!["version"]);
    }

    #[test]
    fn syntax_errors_fail() {
        assert!(check("dock_icon_size = = 3").is_err());
    }

    #[test]
    fn schema_lists_every_top_level_key() {
        let schema: serde_json::Value = serde_json::from_str(&json_schema()).unwrap();
        let props = schema["properties"].as_object().unwrap();
        let defaults = toml::Value::try_from(Config::default()).unwrap();
        for k in defaults.as_table().unwrap().keys() {
            assert!(props.contains_key(k), "schema misses {k}");
        }
        assert!(props.contains_key("wallpaper"));
        assert_eq!(schema["definitions"]["PointerCfg"]["properties"]["speed"]["type"], "number");
    }
}
