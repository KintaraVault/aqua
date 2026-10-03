//! XDG autostart (`~/.config/autostart`, `/etc/xdg/autostart`) and installed-app lookup.
//! Used by the compositor (to start login items) and System Settings (Login Items).
use std::path::{Path, PathBuf};

const DESKTOP: &str = "Aqua";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Entry {
    /// desktop file id, e.g. `org.gnome.Software.desktop`
    pub id: String,
    pub name: String,
    pub exec: String,
    pub enabled: bool,
    /// the user's own file (not shadowing a system one): removing deletes it
    pub user_only: bool,
}

fn user_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".config"))
        .join("autostart")
}

fn system_dirs() -> Vec<PathBuf> {
    let dirs = std::env::var("XDG_CONFIG_DIRS").unwrap_or_else(|_| "/etc/xdg".into());
    dirs.split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join("autostart")).collect()
}

/// `[Desktop Entry]` key/values of a .desktop file (localised keys are ignored).
pub fn parse_desktop(text: &str) -> Vec<(String, String)> {
    let mut out = vec![];
    let mut in_main = false;
    for l in text.lines() {
        let l = l.trim();
        if l.starts_with('[') {
            in_main = l == "[Desktop Entry]";
            continue;
        }
        if !in_main || l.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = l.split_once('=') {
            let k = k.trim();
            if !k.contains('[') {
                out.push((k.to_string(), v.trim().to_string()));
            }
        }
    }
    out
}

fn get<'a>(kv: &'a [(String, String)], k: &str) -> Option<&'a str> {
    kv.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str())
}

fn truthy(v: Option<&str>) -> bool {
    matches!(v.map(|s| s.trim().to_ascii_lowercase()).as_deref(), Some("true") | Some("1"))
}

/// Exec line without field codes (%f %U …), ready for `sh -c`.
pub fn clean_exec(exec: &str) -> String {
    let mut out: Vec<&str> = vec![];
    for w in exec.split_whitespace() {
        if w.len() == 2 && w.starts_with('%') {
            continue;
        }
        out.push(w);
    }
    out.join(" ").replace("%%", "%")
}

fn in_path(bin: &str) -> bool {
    if bin.contains('/') {
        return Path::new(bin).exists();
    }
    std::env::var("PATH").unwrap_or_default().split(':').any(|d| Path::new(d).join(bin).is_file())
}

/// Should this entry run in an Aqua session?
fn shown_here(kv: &[(String, String)]) -> bool {
    let has = |k: &str| get(kv, k).map(|v| v.split(';').any(|d| d.eq_ignore_ascii_case(DESKTOP))).unwrap_or(false);
    if get(kv, "OnlyShowIn").is_some() && !has("OnlyShowIn") {
        return false;
    }
    if has("NotShowIn") {
        return false;
    }
    if let Some(t) = get(kv, "TryExec") {
        if !in_path(t) {
            return false;
        }
    }
    true
}

fn entry_from(id: &str, kv: &[(String, String)], user_only: bool) -> Option<Entry> {
    if get(kv, "Type").map(|t| t != "Application").unwrap_or(false) {
        return None;
    }
    let exec = clean_exec(get(kv, "Exec")?);
    let enabled = !truthy(get(kv, "Hidden"))
        && get(kv, "X-GNOME-Autostart-enabled").map(|v| !v.eq_ignore_ascii_case("false")).unwrap_or(true);
    let name = get(kv, "Name").map(str::to_string).unwrap_or_else(|| id.trim_end_matches(".desktop").to_string());
    Some(Entry { id: id.to_string(), name, exec, enabled, user_only })
}

/// All autostart entries relevant to Aqua (user files override system ones with the same id).
pub fn entries() -> Vec<Entry> {
    let mut system: Vec<(String, Vec<(String, String)>)> = vec![];
    for d in system_dirs() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        let mut files: Vec<PathBuf> =
            rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e == "desktop").unwrap_or(false)).collect();
        files.sort();
        for p in files {
            let id = p.file_name().unwrap().to_string_lossy().to_string();
            if system.iter().any(|(i, _)| *i == id) {
                continue;
            }
            if let Ok(t) = std::fs::read_to_string(&p) {
                system.push((id, parse_desktop(&t)));
            }
        }
    }
    let mut user: Vec<(String, Vec<(String, String)>)> = vec![];
    if let Ok(rd) = std::fs::read_dir(user_dir()) {
        let mut files: Vec<PathBuf> =
            rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e == "desktop").unwrap_or(false)).collect();
        files.sort();
        for p in files {
            if let Ok(t) = std::fs::read_to_string(&p) {
                user.push((p.file_name().unwrap().to_string_lossy().to_string(), parse_desktop(&t)));
            }
        }
    }
    let mut out = vec![];
    for (id, kv) in &user {
        let shadows = system.iter().any(|(i, _)| i == id);
        let merged: Vec<(String, String)> = if shadows && get(kv, "Exec").is_none() {
            let mut base = system.iter().find(|(i, _)| i == id).unwrap().1.clone();
            base.extend(kv.iter().cloned());
            base.reverse();
            base
        } else {
            kv.clone()
        };
        if shown_here(&merged) {
            if let Some(e) = entry_from(id, &merged, !shadows) {
                out.push(e);
            }
        }
    }
    for (id, kv) in &system {
        if user.iter().any(|(i, _)| i == id) || !shown_here(kv) {
            continue;
        }
        if get(kv, "Exec").map(|e| e.contains("polkit")).unwrap_or(false) {
            continue;
        }
        if let Some(e) = entry_from(id, kv, false) {
            out.push(e);
        }
    }
    out
}

/// Enable/disable an entry by writing a user override (the XDG way: `Hidden=true`).
pub fn set_enabled(id: &str, on: bool) -> std::io::Result<()> {
    let dir = user_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(id);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => system_dirs()
            .iter()
            .find_map(|d| std::fs::read_to_string(d.join(id)).ok())
            .unwrap_or_else(|| "[Desktop Entry]\nType=Application\n".into()),
    };
    let mut lines: Vec<String> = text
        .lines()
        .filter(|l| !l.starts_with("Hidden=") && !l.starts_with("X-GNOME-Autostart-enabled="))
        .map(str::to_string)
        .collect();
    let at = lines.iter().position(|l| l.trim() == "[Desktop Entry]").map(|i| i + 1).unwrap_or(lines.len());
    lines.insert(at, format!("Hidden={}", if on { "false" } else { "true" }));
    std::fs::write(path, lines.join("\n") + "\n")
}

/// Add an installed app (its .desktop file) as a login item.
pub fn add(app_desktop: &Path) -> std::io::Result<()> {
    let dir = user_dir();
    std::fs::create_dir_all(&dir)?;
    let text = std::fs::read_to_string(app_desktop)?;
    let id = app_desktop.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "app.desktop".into());
    let mut lines: Vec<String> = text.lines().filter(|l| !l.starts_with("Hidden=")).map(str::to_string).collect();
    let at = lines.iter().position(|l| l.trim() == "[Desktop Entry]").map(|i| i + 1).unwrap_or(0);
    lines.insert(at, "Hidden=false".into());
    std::fs::write(dir.join(id), lines.join("\n") + "\n")
}

/// Remove a login item: delete the user's file, or hide a system entry.
pub fn remove(id: &str) -> std::io::Result<()> {
    let system = system_dirs().iter().any(|d| d.join(id).exists());
    if system {
        return set_enabled(id, false);
    }
    std::fs::remove_file(user_dir().join(id))
}

/// Installed, visible applications: (name, path of the .desktop file), sorted by name.
pub fn apps() -> Vec<(String, PathBuf)> {
    let mut dirs: Vec<PathBuf> = vec![dirs::data_dir().unwrap_or_default().join("applications")];
    let data = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    dirs.extend(data.split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join("applications")));
    dirs.push("/var/lib/flatpak/exports/share/applications".into());
    let mut out: Vec<(String, PathBuf)> = vec![];
    let mut seen: Vec<String> = vec![];
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let id = p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
            if !id.ends_with(".desktop") || seen.contains(&id) {
                continue;
            }
            seen.push(id);
            let Ok(t) = std::fs::read_to_string(&p) else { continue };
            let kv = parse_desktop(&t);
            if truthy(get(&kv, "NoDisplay"))
                || truthy(get(&kv, "Hidden"))
                || get(&kv, "Exec").is_none()
                || !shown_here(&kv)
            {
                continue;
            }
            if let Some(n) = get(&kv, "Name") {
                out.push((n.to_string(), p));
            }
        }
    }
    out.sort_by_key(|(n, _)| n.to_lowercase());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_and_filters() {
        let kv = parse_desktop("[Desktop Entry]\nName=Foo\nName[de]=Fu\nExec=foo --x %U\nOnlyShowIn=GNOME;\n[Desktop Action new]\nExec=bar\n");
        assert_eq!(get(&kv, "Name"), Some("Foo"));
        assert_eq!(get(&kv, "Exec"), Some("foo --x %U"));
        assert!(!shown_here(&kv));
        assert_eq!(clean_exec("foo --x %U"), "foo --x");
        let kv = parse_desktop("[Desktop Entry]\nExec=a\nNotShowIn=KDE;\nHidden=true\n");
        assert!(shown_here(&kv));
        assert!(!entry_from("a.desktop", &kv, true).unwrap().enabled);
    }
}
