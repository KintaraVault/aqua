//! `zenity` / `kdialog` file-dialog compatibility: scripts and apps that shell out to
//! `zenity --file-selection` or `kdialog --getopenfilename` get Aqua's open/save panel.
//! `aqua-filechooser` behaves like this when started under one of those names (Aqua puts
//! such links first in `PATH`); every other dialog kind goes to the real program.

/// What the chooser should do for a compat invocation: aqua-filechooser arguments and the
/// separator between chosen paths on stdout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compat {
    pub args: Vec<String>,
    pub sep: String,
}

/// `--opt=value` or `--opt value`.
fn take_val(a: &str, name: &str, it: &mut std::iter::Peekable<std::slice::Iter<'_, String>>) -> Option<String> {
    if let Some(v) = a.strip_prefix(&format!("{name}=")) {
        return Some(v.to_string());
    }
    if a == name {
        return it.next().cloned();
    }
    None
}

fn push_start(out: &mut Vec<String>, path: &str, save: bool) {
    let p = std::path::Path::new(path);
    if path.is_empty() {
        return;
    }
    if p.is_dir() || path.ends_with('/') {
        let dir = p.canonicalize().map(|d| d.to_string_lossy().into_owned()).unwrap_or_else(|_| {
            let t = path.trim_end_matches('/');
            if t.is_empty() { "/".into() } else { t.to_string() }
        });
        out.extend(["--folder".into(), dir]);
        return;
    }
    if let Some(parent) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
        out.extend(["--folder".into(), parent.to_string_lossy().into_owned()]);
    }
    if save {
        if let Some(n) = p.file_name() {
            out.extend(["--name".into(), n.to_string_lossy().into_owned()]);
        }
    }
}

/// zenity: `--file-selection [--title=T] [--save] [--multiple] [--directory] [--filename=P]
/// [--file-filter='Name | *.a *.b']... [--separator=S]`.
pub fn zenity(args: &[String]) -> Option<Compat> {
    if !args.iter().any(|a| a == "--file-selection") {
        return None;
    }
    let (mut out, mut sep, mut save, mut start) = (vec![], "|".to_string(), false, String::new());
    let mut it = args.iter().peekable();
    while let Some(a) = it.next() {
        if a == "--save" {
            save = true;
            out.push("--save".into());
        } else if a == "--multiple" {
            out.push("--multiple".into());
        } else if a == "--directory" {
            out.push("--directory".into());
        } else if let Some(v) = take_val(a, "--title", &mut it) {
            out.extend(["--title".into(), v]);
        } else if let Some(v) = take_val(a, "--filename", &mut it) {
            start = v;
        } else if let Some(v) = take_val(a, "--separator", &mut it) {
            sep = v;
        } else if let Some(v) = take_val(a, "--file-filter", &mut it) {
            let (label, pats) = match v.split_once('|') {
                Some((l, p)) => (l.trim().to_string(), p.to_string()),
                None => (v.trim().to_string(), v.clone()),
            };
            let pats: Vec<&str> = pats.split_whitespace().collect();
            if !pats.is_empty() {
                out.extend(["--filter".into(), format!("{label}:{}", pats.join(";"))]);
            }
        }
    }
    push_start(&mut out, &start, save);
    Some(Compat { args: out, sep })
}

/// One kdialog filter: `"*.png *.jpg|Images"`, `"Images (*.png *.jpg)"`, `"image/png text/plain"`
/// or plain `"*.txt"` → (label, patterns).
fn kdialog_filter(f: &str) -> Vec<(String, Vec<String>)> {
    let mut out = vec![];
    for line in f.split('\n').map(str::trim).filter(|l| !l.is_empty()) {
        let (label, pats) = if let Some((p, l)) = line.split_once('|') {
            (l.trim().to_string(), p.to_string())
        } else if let (Some(o), true) = (line.rfind('('), line.ends_with(')')) {
            (line[..o].trim().to_string(), line[o + 1..line.len() - 1].to_string())
        } else {
            (line.to_string(), line.to_string())
        };
        let pats: Vec<String> = pats
            .split_whitespace()
            .map(|p| {
                if p.contains('/') && !p.contains('*') {
                    // MIME type: approximate with its subtype as the extension.
                    let sub = p.rsplit('/').next().unwrap_or(p);
                    format!("*.{}", sub.trim_start_matches("x-"))
                } else {
                    p.to_string()
                }
            })
            .collect();
        if !pats.is_empty() {
            out.push((label, pats));
        }
    }
    out
}

/// kdialog: `--getopenfilename [START] [FILTER]`, `--getsavefilename`, `--getexistingdirectory`,
/// `--getopenurl` / `--getsaveurl`, with `--multiple`, `--separate-output`, `--title T`.
pub fn kdialog(args: &[String]) -> Option<Compat> {
    let modes = [
        "--getopenfilename",
        "--getsavefilename",
        "--getexistingdirectory",
        "--getopenurl",
        "--getsaveurl",
    ];
    let mode = args.iter().find(|a| modes.contains(&a.as_str()))?.clone();
    let save = mode.contains("save");
    let (mut out, mut sep, mut pos) = (vec![], " ".to_string(), vec![]);
    if save {
        out.push("--save".into());
    }
    if mode == "--getexistingdirectory" {
        out.push("--directory".into());
    }
    let mut it = args.iter().peekable();
    let mut after_mode = false;
    while let Some(a) = it.next() {
        if a == &mode {
            after_mode = true;
        } else if a == "--multiple" {
            out.push("--multiple".into());
        } else if a == "--separate-output" {
            sep = "\n".into();
        } else if let Some(v) = take_val(a, "--title", &mut it) {
            out.extend(["--title".into(), v]);
        } else if a.starts_with("--") {
            // --attach WINID, --icon NAME …: skip their value.
            if matches!(a.as_str(), "--attach" | "--icon" | "--name" | "--caption") {
                it.next();
            }
        } else if after_mode {
            pos.push(a.clone());
        }
    }
    if let Some(f) = pos.get(1) {
        for (label, pats) in kdialog_filter(f) {
            out.extend(["--filter".into(), format!("{label}:{}", pats.join(";"))]);
        }
    }
    if let Some(start) = pos.first() {
        let start = start.strip_prefix("file://").unwrap_or(start);
        push_start(&mut out, start, save);
    }
    Some(Compat { args: out, sep })
}

/// The real `prog` later in `$PATH` than this executable (and its links).
pub fn real_program(prog: &str) -> Option<std::path::PathBuf> {
    let me = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok());
    std::env::var_os("PATH").iter().flat_map(std::env::split_paths).map(|d| d.join(prog)).find(|c| {
        c.is_file()
            && c.canonicalize().ok() != me
            && std::fs::metadata(c).map(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o111 != 0).unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn zenity_open_and_save() {
        assert_eq!(zenity(&s(&["--question", "--text=Hi"])), None);
        let c = zenity(&s(&["--file-selection", "--title=Pick", "--multiple", "--file-filter=Images | *.png *.jpg"]))
            .unwrap();
        assert_eq!(c.sep, "|");
        assert_eq!(c.args, s(&["--title", "Pick", "--multiple", "--filter", "Images:*.png;*.jpg"]));
        let c = zenity(&s(&["--file-selection", "--save", "--filename", "/tmp/out.txt", "--separator=\n"])).unwrap();
        assert_eq!(c.sep, "\n");
        assert_eq!(c.args, s(&["--save", "--folder", "/tmp", "--name", "out.txt"]));
        let c = zenity(&s(&["--file-selection", "--directory", "--filename=/tmp/"])).unwrap();
        assert_eq!(c.args, s(&["--directory", "--folder", "/tmp"]));
    }

    #[test]
    fn kdialog_modes() {
        assert_eq!(kdialog(&s(&["--msgbox", "hi"])), None);
        let c = kdialog(&s(&["--title", "Open", "--getopenfilename", "/tmp", "*.png *.jpg|Images"])).unwrap();
        assert_eq!(c.args, s(&["--title", "Open", "--filter", "Images:*.png;*.jpg", "--folder", "/tmp"]));
        assert_eq!(c.sep, " ");
        let c = kdialog(&s(&["--getopenfilename", ".", "Text (*.txt *.md)", "--multiple", "--separate-output"]))
            .unwrap();
        assert_eq!(c.sep, "\n");
        assert!(c.args.contains(&"--multiple".to_string()));
        assert!(c.args.contains(&"Text:*.txt;*.md".to_string()));
        let c = kdialog(&s(&["--getsavefilename", "/tmp/a.pdf", "application/pdf"])).unwrap();
        assert_eq!(c.args, s(&["--save", "--filter", "application/pdf:*.pdf", "--folder", "/tmp", "--name", "a.pdf"]));
        let c = kdialog(&s(&["--getexistingdirectory", "/tmp"])).unwrap();
        assert_eq!(c.args, s(&["--directory", "--folder", "/tmp"]));
    }
}
