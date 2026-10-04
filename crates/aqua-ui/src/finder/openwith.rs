//! Applications able to open a file type (desktop entries with a matching MimeType=).
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct DesktopApp {
    pub id: String,
    pub name: String,
    pub exec: String,
    pub path: PathBuf,
    pub mimes: Vec<String>,
    pub terminal: bool,
}

/// Parse the [Desktop Entry] group of a .desktop file.
pub fn parse(id: &str, path: &Path, text: &str) -> Option<DesktopApp> {
    let mut in_entry = false;
    let (mut name, mut exec, mut mimes) = (None, None, vec![]);
    let (mut hidden, mut terminal, mut app) = (false, false, false);
    for l in text.lines() {
        let l = l.trim();
        if l.starts_with('[') {
            in_entry = l == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((k, v)) = l.split_once('=') else { continue };
        match k.trim() {
            "Name" => name = Some(v.trim().to_string()),
            "Exec" => exec = Some(v.trim().to_string()),
            "MimeType" => mimes = v.split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            "NoDisplay" | "Hidden" => hidden |= v.trim() == "true",
            "Terminal" => terminal = v.trim() == "true",
            "Type" => app = v.trim() == "Application",
            _ => {}
        }
    }
    if !app || hidden {
        return None;
    }
    Some(DesktopApp { id: id.into(), name: name?, exec: exec?, path: path.to_path_buf(), mimes, terminal })
}

fn app_dirs() -> Vec<PathBuf> {
    let mut v = vec![dirs::data_dir().unwrap_or_default().join("applications")];
    let sys = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    v.extend(sys.split(':').filter(|s| !s.is_empty()).map(|d| Path::new(d).join("applications")));
    v.push("/var/lib/flatpak/exports/share/applications".into());
    v
}

pub fn all() -> Vec<DesktopApp> {
    let mut seen = std::collections::HashSet::new();
    let mut out = vec![];
    for d in app_dirs() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        files.sort();
        for p in files {
            if p.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            let id = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
            if !seen.insert(id.clone()) {
                continue;
            }
            if let Some(a) = std::fs::read_to_string(&p).ok().and_then(|t| parse(&id, &p, &t)) {
                out.push(a);
            }
        }
    }
    out
}

pub fn by_id(id: &str) -> Option<DesktopApp> {
    all().into_iter().find(|a| a.id == id)
}

fn default_for(mime: &str) -> Option<String> {
    let out = std::process::Command::new("xdg-mime").args(["query", "default", mime]).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Whether an app's MimeType list covers `mime` (exact, or a `type/*` wildcard).
pub fn handles(app: &DesktopApp, mime: &str) -> bool {
    let major = mime.split('/').next().unwrap_or("");
    app.mimes.iter().any(|m| m == mime || m.strip_suffix("/*") == Some(major))
        || (mime.starts_with("text/") && app.mimes.iter().any(|m| m == "text/plain"))
}

/// Apps for `mime`, sorted by name with the default first; returns the default's index.
pub fn for_mime(mime: &str) -> (Vec<DesktopApp>, Option<usize>) {
    let mut v: Vec<DesktopApp> = all().into_iter().filter(|a| handles(a, mime)).collect();
    v.sort_by_key(|a| a.name.to_lowercase());
    let def = default_for(mime);
    if let Some(i) = def.as_ref().and_then(|d| v.iter().position(|a| &a.id == d)) {
        let a = v.remove(i);
        v.insert(0, a);
        return (v, Some(0));
    }
    (v, None)
}

pub fn set_default(app: &DesktopApp, mime: &str) -> bool {
    std::process::Command::new("xdg-mime")
        .args(["default", &app.id, mime])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn launch(app: &DesktopApp, files: &[PathBuf]) {
    let files: Vec<String> = files.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    let refs: Vec<&str> = files.iter().map(|s| s.as_str()).collect();
    let argv = aqua_apps::expand_exec(&app.exec, &refs, &app.name, "", &app.path.to_string_lossy());
    let cmd = aqua_apps::shell_join(&argv);
    aqua_apps::launch(&if app.terminal { aqua_apps::terminal_wrap(&cmd) } else { cmd });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_entries() {
        let t = "[Desktop Entry]\nType=Application\nName=Viewer\nExec=viewer %F\nMimeType=image/png;image/jpeg;\n\n[Desktop Action new]\nName=Other\n";
        let a = parse("viewer.desktop", Path::new("/x/viewer.desktop"), t).unwrap();
        assert_eq!(a.name, "Viewer");
        assert_eq!(a.mimes, ["image/png", "image/jpeg"]);
        assert!(handles(&a, "image/png"));
        assert!(!handles(&a, "image/gif"));
        assert!(parse(
            "h.desktop",
            Path::new("/h"),
            "[Desktop Entry]\nType=Application\nName=H\nExec=h\nNoDisplay=true\n"
        )
        .is_none());
        assert!(parse("l.desktop", Path::new("/l"), "[Desktop Entry]\nType=Link\nName=L\nExec=l\n").is_none());
        let any = DesktopApp { mimes: vec!["image/*".into()], ..a.clone() };
        assert!(handles(&any, "image/gif"));
        let ed = DesktopApp { mimes: vec!["text/plain".into()], ..a };
        assert!(handles(&ed, "text/markdown"));
    }
}
