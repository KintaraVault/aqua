//! aqua-apps: XDG desktop-entry discovery, categorisation
//! and process launching.

use std::path::{Path, PathBuf};

/// Category of the Applications panel (freedesktop main categories, names).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Section {
    Utilities,
    System,
    Games,
    Internet,
    Office,
    Graphics,
    Multimedia,
    Development,
    Education,
    Other,
}

impl Section {
    pub const ALL: [Section; 10] = [
        Section::Utilities,
        Section::System,
        Section::Games,
        Section::Internet,
        Section::Office,
        Section::Graphics,
        Section::Multimedia,
        Section::Development,
        Section::Education,
        Section::Other,
    ];
    pub fn title(&self) -> &'static str {
        match self {
            Section::Utilities => "Utilities",
            Section::System => "System",
            Section::Games => "Games",
            Section::Internet => "Internet",
            Section::Office => "Productivity",
            Section::Graphics => "Graphics & Design",
            Section::Multimedia => "Entertainment",
            Section::Development => "Developer Tools",
            Section::Education => "Education",
            Section::Other => "Other",
        }
    }
    /// Short label for the segmented control.
    pub fn short(&self) -> &'static str {
        match self {
            Section::Graphics => "Graphics",
            Section::Development => "Developer",
            s => s.title(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct App {
    pub id: String,
    pub name: String,
    pub exec: String,
    pub icon: String,
    pub categories: Vec<String>,
    pub wm_class: Option<String>,
    pub path: PathBuf,
    /// `Terminal=true`: the program runs inside a terminal emulator.
    pub terminal: bool,
    /// Extra search terms (`Keywords=`, `GenericName=`).
    pub keywords: Vec<String>,
    /// `Path=`: working directory to start the program in.
    pub workdir: Option<String>,
}

impl App {
    pub fn section(&self) -> Section {
        let has = |k: &[&str]| self.categories.iter().any(|c| k.contains(&c.as_str()));
        if has(&["Game", "Emulator"]) {
            Section::Games
        } else if has(&["Development", "IDE", "Debugger", "RevisionControl", "WebDevelopment", "GUIDesigner"]) {
            Section::Development
        } else if has(&[
            "Network",
            "WebBrowser",
            "Email",
            "Chat",
            "InstantMessaging",
            "IRCClient",
            "FileTransfer",
            "P2P",
            "VideoConference",
            "Feed",
            "News",
        ]) {
            Section::Internet
        } else if has(&[
            "Office",
            "Finance",
            "WordProcessor",
            "Spreadsheet",
            "Presentation",
            "Calendar",
            "ContactManagement",
            "ProjectManagement",
        ]) {
            Section::Office
        } else if has(&[
            "Graphics",
            "Photography",
            "2DGraphics",
            "3DGraphics",
            "VectorGraphics",
            "RasterGraphics",
            "Scanning",
        ]) {
            Section::Graphics
        } else if has(&["AudioVideo", "Audio", "Video", "Music", "Player", "Recorder", "TV"]) {
            Section::Multimedia
        } else if has(&["Education", "Science", "Math", "Astronomy", "Languages"]) {
            Section::Education
        } else if has(&[
            "System",
            "Settings",
            "Monitor",
            "PackageManager",
            "TerminalEmulator",
            "Security",
            "HardwareSettings",
            "DesktopSettings",
        ]) {
            Section::System
        } else if has(&[
            "Utility",
            "Accessories",
            "FileManager",
            "TextEditor",
            "Archiving",
            "Compression",
            "Calculator",
            "Clock",
            "FileTools",
        ]) {
            Section::Utilities
        } else {
            Section::Other
        }
    }

    /// Shell command line for the Exec key with no files: field codes expanded per the
    /// desktop entry spec (`%%` → `%`, `%i`/`%c`/`%k` filled in, file/URL codes dropped).
    pub fn command(&self) -> String {
        self.command_with(&[])
    }

    /// Shell command line opening `files` (paths or URLs) with this app.
    pub fn command_with(&self, files: &[&str]) -> String {
        shell_join(&self.argv(files))
    }

    /// Argument vector for the Exec key opening `files`.
    pub fn argv(&self, files: &[&str]) -> Vec<String> {
        let path = self.path.to_string_lossy();
        expand_exec(&self.exec, files, &self.name, &self.icon, &path)
    }

    /// Command to start the app: [`command`](Self::command), wrapped in a terminal
    /// emulator for `Terminal=true` entries (htop, vim, …) — run bare they start
    /// invisibly and "nothing happens".
    pub fn launch_command(&self) -> String {
        let cmd = self.command();
        let cmd = if self.terminal { terminal_wrap(&cmd) } else { cmd };
        match self.workdir.as_deref().filter(|d| Path::new(d).is_dir()) {
            Some(d) => format!("cd {} && exec {cmd}", shell_quote(d)),
            None => cmd,
        }
    }
}

/// Wrap `cmd` so it runs in the first installed terminal emulator.
pub fn terminal_wrap(cmd: &str) -> String {
    const TERMS: [(&str, &str); 11] = [
        ("foot", ""),
        ("kitty", ""),
        ("alacritty", "-e"),
        ("ghostty", "-e"),
        ("wezterm", "start --"),
        ("gnome-terminal", "--"),
        ("kgx", "--"),
        ("konsole", "-e"),
        ("xfce4-terminal", "-x"),
        ("tilix", "-e"),
        ("xterm", "-e"),
    ];
    for (t, flag) in TERMS {
        if find_in_path(t).is_some() {
            return if flag.is_empty() { format!("{t} {cmd}") } else { format!("{t} {flag} {cmd}") };
        }
    }
    cmd.to_string()
}

/// Locate an executable: absolute/relative paths are checked directly, bare names in $PATH.
pub fn find_in_path(bin: &str) -> Option<PathBuf> {
    let bin = bin.trim_matches(|c| c == '"' || c == '\'');
    if bin.is_empty() {
        return None;
    }
    let is_exe = |p: &Path| {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    };
    if bin.contains('/') {
        let p = PathBuf::from(bin);
        return is_exe(&p).then_some(p);
    }
    let path = std::env::var_os("PATH").unwrap_or_else(|| "/usr/local/bin:/usr/bin:/bin".into());
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    for extra in [
        "/usr/local/bin",
        "/usr/bin",
        "/bin",
        "/usr/local/sbin",
        "/usr/sbin",
        "/var/lib/flatpak/exports/bin",
        "/snap/bin",
    ] {
        let e = PathBuf::from(extra);
        if !dirs.contains(&e) {
            dirs.push(e);
        }
    }
    if let Some(h) = dirs::home_dir() {
        for extra in [".local/bin", ".cargo/bin", ".local/share/flatpak/exports/bin", ".nix-profile/bin"] {
            dirs.push(h.join(extra));
        }
    }
    if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        dirs.push(d);
    }
    dirs.into_iter().map(|d| d.join(bin)).find(|p| is_exe(p))
}

/// First real program of an Exec line (skips `env`, `VAR=value`, quoting).
pub fn exec_program(exec: &str) -> Option<String> {
    let toks = split_exec(exec);
    let mut it = toks.iter().peekable();
    while let Some(t) = it.next() {
        if t == "env" || t == "/usr/bin/env" {
            while let Some(n) = it.peek() {
                if n.starts_with('-') {
                    let takes_arg = n.as_str() == "-u" || n.as_str() == "-C";
                    it.next();
                    if takes_arg {
                        it.next();
                    }
                } else {
                    break;
                }
            }
            continue;
        }
        if t.contains('=')
            && !t.starts_with('/')
            && t.split('=')
                .next()
                .map(|k| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(false)
        {
            continue;
        }
        return Some(t.clone());
    }
    None
}

/// Expand the field codes of an Exec value into an argument vector.
///
/// `%f`/`%u` take the first file, `%F`/`%U` all of them (each as its own argument when the
/// code stands alone); `%i` becomes `--icon ICON`, `%c` the name, `%k` the desktop file;
/// `%%` is a literal percent sign; deprecated and unknown codes are removed.
pub fn expand_exec(exec: &str, files: &[&str], name: &str, icon: &str, desktop: &str) -> Vec<String> {
    let mut out = vec![];
    for arg in split_exec(exec) {
        match arg.as_str() {
            "%F" | "%U" => {
                out.extend(files.iter().map(|f| f.to_string()));
                continue;
            }
            "%f" | "%u" => {
                out.extend(files.first().map(|f| f.to_string()));
                continue;
            }
            "%i" => {
                if !icon.is_empty() {
                    out.push("--icon".into());
                    out.push(icon.into());
                }
                continue;
            }
            _ => {}
        }
        let mut v = String::with_capacity(arg.len());
        let mut had_code = false;
        let mut chars = arg.chars();
        while let Some(c) = chars.next() {
            if c != '%' {
                v.push(c);
                continue;
            }
            match chars.next() {
                Some('%') => v.push('%'),
                Some(code) => {
                    had_code = true;
                    match code {
                        'f' | 'u' | 'F' | 'U' => v.push_str(files.first().copied().unwrap_or("")),
                        'c' => v.push_str(name),
                        'k' => v.push_str(desktop),
                        'i' => v.push_str(icon),
                        _ => {}
                    }
                }
                None => v.push('%'),
            }
        }
        if !(had_code && v.is_empty()) {
            out.push(v);
        }
    }
    out
}

/// Quote one argument for `sh -c`.
pub fn shell_quote(a: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c);
    if !a.is_empty() && a.chars().all(safe) {
        a.to_string()
    } else {
        format!("'{}'", a.replace('\'', "'\\''"))
    }
}

/// Join an argument vector into a `sh -c` command line.
pub fn shell_join(argv: &[String]) -> String {
    argv.iter().map(|a| shell_quote(a)).collect::<Vec<_>>().join(" ")
}

/// Split an Exec value into arguments following the desktop-entry quoting rules.
fn split_exec(exec: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            '\\' if quoted => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            c if c.is_whitespace() && !quoted => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn app_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = vec![];
    let mut push = |p: PathBuf| {
        if !v.contains(&p) {
            v.push(p);
        }
    };
    if let Some(d) = dirs::data_dir() {
        push(d.join("applications"));
        push(d.join("flatpak/exports/share/applications"));
    }
    let xdg = std::env::var("XDG_DATA_DIRS").unwrap_or_default();
    for d in xdg.split(':').filter(|s| !s.is_empty()) {
        push(Path::new(d).join("applications"));
    }
    for d in [
        "/usr/local/share",
        "/usr/share",
        "/var/lib/flatpak/exports/share",
        "/var/lib/snapd/desktop",
        "/run/current-system/sw/share",
    ] {
        push(Path::new(d).join("applications"));
    }
    if let Some(h) = dirs::home_dir() {
        push(h.join(".nix-profile/share/applications"));
    }
    v
}

/// Names of the running desktop (`XDG_CURRENT_DESKTOP`, always including "Aqua").
fn current_desktops() -> Vec<String> {
    let mut v: Vec<String> = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect();
    if !v.iter().any(|d| d == "aqua") {
        v.push("aqua".into());
    }
    v
}

/// Result of parsing one desktop entry.
enum Parsed {
    App(Box<App>),
    /// A valid entry that must not be shown (NoDisplay/Hidden, other desktop, missing
    /// program): it still masks entries with the same id in lower-priority directories.
    Masked,
    /// Not an application entry at all.
    Skip,
}

pub fn parse_desktop(path: &Path) -> Option<App> {
    let id = path.file_stem()?.to_string_lossy().to_string();
    match parse_entry(path, id) {
        Parsed::App(a) => Some(*a),
        _ => None,
    }
}

fn parse_entry(path: &Path, id: String) -> Parsed {
    let Ok(text) = std::fs::read_to_string(path) else { return Parsed::Skip };
    let mut in_entry = false;
    let (mut name, mut exec, mut icon, mut cats, mut wm, mut hidden, mut ty) =
        (None, None, String::new(), vec![], None, false, String::new());
    let (mut try_exec, mut only, mut not, mut terminal, mut keywords, mut generic) =
        (None::<String>, None::<Vec<String>>, vec![], false, vec![], None::<String>);
    let mut workdir = None::<String>;
    let lang = std::env::var("LC_MESSAGES")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("LANG").ok())
        .unwrap_or_default();
    let full = lang.split('.').next().unwrap_or("").to_string();
    let short = full.split('_').next().unwrap_or("").to_string();
    let (mut lname_full, mut lname_short) = (None, None);
    let list = |v: &str| v.split(';').map(|s| s.trim()).filter(|s| !s.is_empty()).map(String::from).collect::<Vec<_>>();
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_entry = l == "[Desktop Entry]";
            continue;
        }
        if !in_entry || l.starts_with('#') {
            continue;
        }
        let Some((k, v)) = l.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        match k {
            "Name" => name = Some(clean_name(v)),
            "Exec" => exec = Some(v.to_string()),
            "TryExec" => try_exec = Some(v.to_string()),
            "Icon" => icon = v.to_string(),
            "Categories" => cats = list(v),
            "StartupWMClass" => wm = Some(v.to_string()),
            "NoDisplay" | "Hidden" => hidden |= v.eq_ignore_ascii_case("true"),
            "Type" => ty = v.to_string(),
            "OnlyShowIn" => only = Some(list(v)),
            "NotShowIn" => not = list(v),
            "Terminal" => terminal = v.eq_ignore_ascii_case("true"),
            "Keywords" => keywords.extend(list(v)),
            "GenericName" => generic = Some(v.to_string()),
            "Path" if !v.is_empty() => workdir = Some(v.to_string()),
            _ if !full.is_empty() && full != short && k == format!("Name[{full}]") => lname_full = Some(clean_name(v)),
            _ if !short.is_empty() && k == format!("Name[{short}]") => lname_short = Some(clean_name(v)),
            _ if !short.is_empty() && (k == format!("Keywords[{short}]") || k == format!("Keywords[{full}]")) => {
                keywords.extend(list(v))
            }
            _ => {}
        }
    }
    if ty != "Application" {
        return if ty.is_empty() { Parsed::Skip } else { Parsed::Masked };
    }
    if hidden {
        return Parsed::Masked;
    }
    let desks = current_desktops();
    if let Some(only) = &only {
        if !only.iter().any(|d| desks.contains(&d.to_lowercase())) {
            return Parsed::Masked;
        }
    }
    if not.iter().any(|d| desks.contains(&d.to_lowercase())) {
        return Parsed::Masked;
    }
    let Some(exec) = exec.filter(|e| !e.trim().is_empty()) else { return Parsed::Masked };
    if let Some(t) = try_exec.as_deref().filter(|t| !t.is_empty()) {
        if find_in_path(t).is_none() {
            return Parsed::Masked;
        }
    }
    match exec_program(&exec) {
        Some(prog) if find_in_path(&prog).is_some() => {}
        _ => return Parsed::Masked,
    }
    let Some(name) = lname_full.or(lname_short).or(name) else { return Parsed::Masked };
    if let Some(g) = generic {
        keywords.push(g);
    }
    Parsed::App(Box::new(App {
        id,
        name,
        exec,
        icon,
        categories: cats,
        wm_class: wm,
        path: path.to_path_buf(),
        terminal,
        keywords,
        workdir,
    }))
}

/// Desktop files below `dir` with their desktop-file ids (`sub/foo.desktop` → `sub-foo`).
fn desktop_files(dir: &Path) -> Vec<(String, PathBuf)> {
    fn walk(root: &Path, d: &Path, depth: u32, out: &mut Vec<(String, PathBuf)>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                if depth < 4 {
                    walk(root, &p, depth + 1, out);
                }
            } else if p.extension().map(|e| e == "desktop").unwrap_or(false) {
                let rel = p.strip_prefix(root).unwrap_or(&p).with_extension("");
                let id = rel.to_string_lossy().replace('/', "-");
                out.push((id, p));
            }
        }
    }
    let mut out = vec![];
    walk(dir, dir, 0, &mut out);
    out
}

/// Scan all application directories (first match per id wins; a hidden entry in a
/// higher-priority directory hides the app, as the XDG spec requires).
pub fn scan() -> Vec<App> {
    let mut seen = std::collections::HashSet::new();
    let mut out = vec![];
    for d in app_dirs() {
        for (id, f) in desktop_files(&d) {
            if seen.contains(&id) {
                continue;
            }
            match parse_entry(&f, id.clone()) {
                Parsed::App(app) => {
                    seen.insert(id);
                    out.push(*app);
                }
                Parsed::Masked => {
                    seen.insert(id);
                }
                Parsed::Skip => {}
            }
        }
    }
    out.extend(builtin_apps(&out));
    let mut names = std::collections::HashSet::new();
    out.retain(|a| {
        names.insert((
            a.name.to_lowercase(),
            exec_program(&a.exec).map(|p| p.rsplit('/').next().unwrap_or("").to_string()).unwrap_or_default(),
        ))
    });
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

/// Aqua's own programs: (desktop id, name, binary + args, icon, categories, keywords).
const BUILTIN_APPS: &[(&str, &str, &str, &str, &[&str], &[&str])] = &[
    (
        "org.aqua.settings",
        "System Settings",
        "aqua-settings",
        "builtin:settings",
        &["Settings", "System"],
        &[
            "preferences",
            "system preferences",
            "settings",
            "control panel",
            "configuration",
            "wifi",
            "bluetooth",
            "display",
            "keyboard",
            "wallpaper",
            "настройки",
            "параметры",
            "системные",
        ],
    ),
    (
        "org.aqua.finder",
        "Finder",
        "aqua-finder",
        "builtin:finder",
        &["System", "FileTools", "FileManager"],
        &["files", "file manager", "folders", "explorer", "finder", "проводник", "файлы", "папки"],
    ),
    (
        "aqua-screenshot",
        "Screenshot",
        "aqua-screenshot ui",
        "applets-screenshooter",
        &["Utility", "Graphics"],
        &["screenshot", "screen", "capture", "record", "recording", "снимок", "скриншот", "запись"],
    ),
];

/// Locate an Aqua binary: $PATH, else next to the running compositor (development
/// builds run from target/release).
fn aqua_bin(bin: &str) -> Option<PathBuf> {
    find_in_path(bin).or_else(|| {
        let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
        let p = dir.join(bin);
        p.is_file().then_some(p)
    })
}

fn builtin_apps(found: &[App]) -> Vec<App> {
    let mut v = vec![];
    for (id, name, cmd, icon, cats, kw) in BUILTIN_APPS {
        let bin = cmd.split_whitespace().next().unwrap_or("");
        let have = found
            .iter()
            .any(|a| a.id == *id || exec_program(&a.exec).map(|p| p.rsplit('/').next() == Some(bin)).unwrap_or(false));
        if have {
            continue;
        }
        let Some(path) = aqua_bin(bin) else { continue };
        let exec = cmd.replacen(bin, &path.to_string_lossy(), 1);
        v.push(App {
            id: id.to_string(),
            name: name.to_string(),
            exec,
            icon: icon.to_string(),
            categories: cats.iter().map(|s| s.to_string()).collect(),
            wm_class: None,
            path: PathBuf::new(),
            terminal: false,
            keywords: kw.iter().map(|s| s.to_string()).collect(),
            workdir: None,
        });
    }
    v
}

/// Cheap fingerprint of every application directory (file names, sizes and
/// modification times) — changes whenever an app is installed, removed or updated.
pub fn fingerprint() -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for d in app_dirs() {
        d.hash(&mut h);
        for (id, f) in desktop_files(&d) {
            id.hash(&mut h);
            if let Ok(m) = std::fs::metadata(&f) {
                m.len().hash(&mut h);
                m.modified().ok().hash(&mut h);
            }
        }
    }
    std::env::var_os("PATH").hash(&mut h);
    h.finish()
}

/// Launches requested while the X server is still starting.
static HELD: std::sync::Mutex<Option<Vec<String>>> = std::sync::Mutex::new(None);

/// Queue launches until [`release_launches`] (XWayland startup).
pub fn hold_launches() {
    let mut h = HELD.lock().unwrap_or_else(|e| e.into_inner());
    if h.is_none() {
        *h = Some(Vec::new());
    }
}

/// Run every launch queued since [`hold_launches`].
pub fn release_launches() {
    let queued = HELD.lock().unwrap_or_else(|e| e.into_inner()).take();
    for c in queued.unwrap_or_default() {
        spawn_now(&c);
    }
}

/// Spawn a shell command detached from the compositor.
pub fn launch(cmd: &str) -> bool {
    if cmd.trim().is_empty() {
        return false;
    }
    if let Some(q) = HELD.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        q.push(cmd.to_string());
        return true;
    }
    spawn_now(cmd)
}

fn spawn_now(cmd: &str) -> bool {
    let compound = ["||", "&&", ";", "|", "&"].iter().any(|op| cmd.contains(op));
    let script = if compound { cmd.to_string() } else { format!("exec {cmd}") };
    let log = exec_program(cmd).map(|p| p.rsplit('/').next().unwrap_or("app").to_string());
    let res = std::thread::Builder::new().name("aqua-launch".into()).spawn(move || {
        let mut c = launch_process(&script, log.as_deref());
        let started = std::time::Instant::now();
        match c.spawn() {
            Ok(mut child) => {
                if let Ok(st) = child.wait() {
                    report_exit(log.as_deref().unwrap_or("app"), st, started.elapsed());
                }
            }
            Err(e) => tracing::warn!("cannot start `{script}`: {e}"),
        }
    });
    res.is_ok()
}

/// Note abnormal app exits in the session log, pointing at the app's own output.
fn report_exit(prog: &str, st: std::process::ExitStatus, ran: std::time::Duration) {
    use std::os::unix::process::ExitStatusExt;
    let log = dirs::cache_dir().map(|d| d.join("aqua/logs").join(format!("{prog}.log")).display().to_string()).unwrap_or_default();
    if let Some(sig) = st.signal().or_else(|| st.code().filter(|c| *c > 128 && *c < 160).map(|c| c - 128)) {
        // SIGTERM/SIGINT/SIGKILL/SIGHUP are normal ways to be closed.
        if ![1, 2, 9, 15].contains(&sig) {
            tracing::warn!("{prog} crashed: signal {sig} after {:.0?} (output: {log})", ran);
        }
    } else if let Some(c) = st.code().filter(|c| *c != 0) {
        tracing::info!("{prog} exited with status {c} after {:.0?} (output: {log})", ran);
    }
}

/// The process for a launch: started the way a terminal would start it, so apps that work
/// from a terminal also work from Spotlight, Launchpad and the Dock — in the home folder,
/// with the user's login-shell environment (PATH additions, toolkit variables from
/// `~/.profile`, `~/.bashrc`, `~/.zshrc` …), in its own session, with output going to a
/// log file instead of the compositor's stdout (a closed pipe there kills apps that print
/// with SIGPIPE).
fn launch_process(script: &str, log: Option<&str>) -> std::process::Command {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    let mut c = std::process::Command::new("sh");
    c.arg("-c").arg(script).stdin(Stdio::null());
    c.envs(shell_env().iter().map(|(k, v)| (k.as_str(), v.as_str())));
    if let Some(home) = dirs::home_dir().filter(|h| h.is_dir()) {
        c.current_dir(home);
    }
    match log.and_then(log_file) {
        Some(f) => {
            if let Ok(f2) = f.try_clone() {
                c.stdout(f2);
            }
            c.stderr(f);
        }
        None => {
            c.stdout(Stdio::null()).stderr(Stdio::null());
        }
    }
    unsafe {
        c.pre_exec(|| {
            libc_setsid();
            Ok(())
        });
    }
    c
}

fn libc_setsid() {
    unsafe extern "C" {
        fn setsid() -> i32;
    }
    unsafe {
        setsid();
    }
}

/// `~/.cache/aqua/logs/<program>.log`, truncated on every launch.
fn log_file(prog: &str) -> Option<std::fs::File> {
    let name: String =
        prog.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '_' }).collect();
    let dir = dirs::cache_dir()?.join("aqua").join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    std::fs::File::create(dir.join(format!("{name}.log"))).ok()
}

/// Variables owned by the session (the shell must not override them) or meaningful only
/// inside an interactive shell.
fn session_owned(k: &str) -> bool {
    matches!(
        k,
        "WAYLAND_DISPLAY"
            | "WAYLAND_SOCKET"
            | "DISPLAY"
            | "XAUTHORITY"
            | "DBUS_SESSION_BUS_ADDRESS"
            | "XDG_RUNTIME_DIR"
            | "XDG_SESSION_ID"
            | "XDG_SESSION_TYPE"
            | "XDG_SESSION_CLASS"
            | "XDG_SESSION_DESKTOP"
            | "XDG_CURRENT_DESKTOP"
            | "XDG_SEAT"
            | "XDG_VTNR"
            | "XDG_ACTIVATION_TOKEN"
            | "DESKTOP_STARTUP_ID"
            | "SHLVL"
            | "PWD"
            | "OLDPWD"
            | "_"
            | "PS1"
            | "PS2"
            | "PROMPT_COMMAND"
            | "TERM"
            | "COLORTERM"
            | "LINES"
            | "COLUMNS"
            | "SHELL_SESSION_ID"
            | "TERM_PROGRAM"
            | "TERM_PROGRAM_VERSION"
    ) || k.starts_with("AQUA_")
        || k.starts_with("BASH_FUNC_")
}

/// Parse `env -0` output between two markers into the variables to apply.
fn parse_shell_env(out: &[u8], marker: &str) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(out);
    let Some(start) = text.find(marker) else { return vec![] };
    let rest = &text[start + marker.len()..];
    let Some(end) = rest.find(marker) else { return vec![] };
    rest[..end]
        .split('\0')
        .filter_map(|kv| kv.split_once('='))
        .filter(|(k, _)| !k.is_empty() && !k.contains(char::is_whitespace) && !session_owned(k))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// The user's login-shell environment (resolved once, like VS Code does): `$SHELL -i -l`
/// reads the same startup files as a terminal. Empty when there is no usable shell, it
/// takes longer than 4 s, or `AQUA_NO_SHELL_ENV` is set.
pub fn shell_env() -> &'static [(String, String)] {
    static ENV: std::sync::OnceLock<Vec<(String, String)>> = std::sync::OnceLock::new();
    ENV.get_or_init(|| {
        if std::env::var_os("AQUA_NO_SHELL_ENV").is_some() {
            return vec![];
        }
        let shell = std::env::var("SHELL").ok().filter(|s| Path::new(s).is_file()).unwrap_or_else(|| "/bin/sh".into());
        let marker = format!("_AQUA_ENV_{}_", std::process::id());
        let script = format!("printf '%s' '{marker}'; env -0; printf '%s' '{marker}'");
        let base = Path::new(&shell).file_name().and_then(|n| n.to_str()).unwrap_or("");
        let mut c = std::process::Command::new(&shell);
        if matches!(base, "sh" | "dash") {
            c.args(["-l", "-c", &script]);
        } else {
            c.args(["-i", "-l", "-c", &script]);
        }
        if let Some(home) = dirs::home_dir() {
            c.current_dir(home);
        }
        c.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
        // Only what the startup files add or change matters; the rest is the session's own
        // environment, which may legitimately change later (DISPLAY once XWayland is up …).
        let base: std::collections::HashMap<String, String> = std::env::vars().collect();
        let Ok(mut child) = c.spawn() else { return vec![] };
        let mut out = child.stdout.take();
        let reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(o) = out.as_mut() {
                let _ = std::io::Read::read_to_end(o, &mut buf);
            }
            buf
        });
        let t0 = std::time::Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if t0.elapsed() < std::time::Duration::from_secs(4) => {
                    std::thread::sleep(std::time::Duration::from_millis(20))
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    eprintln!("aqua: reading the login-shell environment from {shell} timed out");
                    return vec![];
                }
            }
        }
        let env = reader.join().map(|b| parse_shell_env(&b, &marker)).unwrap_or_default();
        env.into_iter().filter(|(k, v)| base.get(k) != Some(v)).collect()
    })
}

/// Resolve [`shell_env`] in the background (call at session start, so the first launch
/// doesn't wait for the shell).
pub fn preload_shell_env() {
    std::thread::spawn(|| {
        let _ = shell_env();
    });
}

/// Find the app matching a Wayland app_id.
pub fn match_app_id<'a>(apps: &'a [App], app_id: &str) -> Option<&'a App> {
    let a = app_id.to_lowercase();
    if a.is_empty() {
        return None;
    }
    let exe = |x: &App| -> String {
        exec_program(&x.exec).unwrap_or_default().rsplit('/').next().unwrap_or("").to_lowercase()
    };
    apps.iter()
        .find(|x| x.id.to_lowercase() == a || x.wm_class.as_deref().map(|w| w.to_lowercase() == a).unwrap_or(false))
        .or_else(|| apps.iter().find(|x| x.id.to_lowercase().ends_with(&format!(".{a}"))))
        .or_else(|| apps.iter().find(|x| exe(x) == a))
        .or_else(|| apps.iter().find(|x| x.name.to_lowercase() == a.rsplit('.').next().unwrap_or(&a)))
}

/// Drop version tokens from display names ("LibreOffice 26.8 Calc" -> "LibreOffice Calc").
pub fn clean_name(s: &str) -> String {
    let words: Vec<&str> = s
        .split_whitespace()
        .filter(|w| {
            !(w.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
                && w.chars().all(|c| c.is_ascii_digit() || c == '.'))
        })
        .collect();
    if words.is_empty() {
        s.to_string()
    } else {
        words.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, name: &str, exec: &str, cats: &[&str]) -> App {
        App {
            id: id.into(),
            name: name.into(),
            exec: exec.into(),
            icon: String::new(),
            categories: cats.iter().map(|c| c.to_string()).collect(),
            wm_class: None,
            path: PathBuf::new(),
            terminal: false,
            keywords: vec![],
            workdir: None,
        }
    }

    fn tmp_desktop(name: &str, body: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aqua-apps-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn shell_env_keeps_user_vars_but_not_session_ones() {
        let out = b"motd\n_M_PATH=/home/u/.local/bin:/usr/bin\0WAYLAND_DISPLAY=wayland-9\0SHLVL=2\0EDITOR=vim\0A=b=c\0_M_tail";
        let env = parse_shell_env(out, "_M_");
        assert_eq!(
            env,
            vec![
                ("PATH".into(), "/home/u/.local/bin:/usr/bin".into()),
                ("EDITOR".into(), "vim".into()),
                ("A".into(), "b=c".into())
            ]
        );
        assert!(parse_shell_env(b"no markers", "_M_").is_empty());
    }

    #[test]
    fn launch_command_honours_the_working_directory() {
        let mut a = app("x", "X", "xprog --flag", &[]);
        assert_eq!(a.launch_command(), "xprog --flag");
        a.workdir = Some("/tmp".into());
        assert_eq!(a.launch_command(), "cd /tmp && exec xprog --flag");
        a.workdir = Some("/nonexistent-aqua-dir".into());
        assert_eq!(a.launch_command(), "xprog --flag");
    }

    #[test]
    fn launched_processes_start_at_home_with_the_shell_env() {
        let c = launch_process("true", None);
        assert_eq!(c.get_current_dir(), dirs::home_dir().filter(|h| h.is_dir()).as_deref());
        assert_eq!(c.get_program(), "sh");
    }

    #[test]
    fn split_exec_handles_quotes_and_escapes() {
        assert_eq!(split_exec(r#"foo "a b" c"#), vec!["foo", "a b", "c"]);
        assert_eq!(split_exec(r#""/opt/My App/run" --x"#), vec!["/opt/My App/run", "--x"]);
        assert_eq!(split_exec(r#"echo "say \"hi\"""#), vec!["echo", r#"say "hi""#]);
        assert_eq!(split_exec(r#"a "" b"#), vec!["a", "", "b"]);
        assert!(split_exec("   ").is_empty());
    }

    #[test]
    fn exec_program_skips_env_and_assignments() {
        assert_eq!(exec_program("firefox %u").as_deref(), Some("firefox"));
        assert_eq!(exec_program("env GDK_BACKEND=x11 FOO=1 app --flag").as_deref(), Some("app"));
        assert_eq!(exec_program("/usr/bin/env -u VAR -i prog").as_deref(), Some("prog"));
        assert_eq!(exec_program("LANG=C prog").as_deref(), Some("prog"));
        assert_eq!(exec_program("/opt/a=b/prog").as_deref(), Some("/opt/a=b/prog"));
        assert_eq!(exec_program("env"), None);
        assert_eq!(exec_program(""), None);
    }

    #[test]
    fn command_strips_field_codes() {
        let a = app("x", "X", "gimp %U --new %f", &[]);
        assert_eq!(a.command(), "gimp --new");
        assert_eq!(app("x", "X", "plain", &[]).launch_command(), "plain");
    }

    #[test]
    fn exec_field_codes_follow_the_spec() {
        let x = |exec: &str, files: &[&str]| expand_exec(exec, files, "My App", "my-icon", "/a/my.desktop");
        assert_eq!(x("app %F", &["a b", "c"]), vec!["app", "a b", "c"]);
        assert_eq!(x("app %f", &["a", "b"]), vec!["app", "a"]);
        assert_eq!(x("app %u", &[]), vec!["app"]);
        assert_eq!(x("app --file=%f", &["/x"]), vec!["app", "--file=/x"]);
        assert_eq!(
            x("app %i --name %c %k", &[]),
            vec!["app", "--icon", "my-icon", "--name", "My App", "/a/my.desktop"]
        );
        assert_eq!(x("printf 100%%", &[]), vec!["printf", "100%"]);
        assert_eq!(x(r#"sh -c "echo 50%% done" %d %v"#, &[]), vec!["sh", "-c", "echo 50% done"]);
        assert_eq!(x("app %i", &[]).len(), 3);
        assert_eq!(expand_exec("app %i", &[], "n", "", "d"), vec!["app"]);
        assert_eq!(x("trailing %", &[]), vec!["trailing", "%"]);
    }

    #[test]
    fn shell_quoting_round_trips() {
        assert_eq!(shell_quote("plain-arg_1.txt"), "plain-arg_1.txt");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("$HOME;rm"), "'$HOME;rm'");
        let a = app("x", "X", r#""/opt/My App/run" %U"#, &[]);
        assert_eq!(a.command_with(&["/tmp/a file"]), "'/opt/My App/run' '/tmp/a file'");
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("printf '%s|' {}", shell_join(&["it's".into(), "a b".into(), "$X".into()])))
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "it's|a b|$X|");
    }

    #[test]
    fn clean_name_drops_versions() {
        assert_eq!(clean_name("LibreOffice 26.8 Calc"), "LibreOffice Calc");
        assert_eq!(clean_name("Blender 4.2"), "Blender");
        assert_eq!(clean_name("2048"), "2048");
        assert_eq!(clean_name("Qt5 Designer"), "Qt5 Designer");
    }

    #[test]
    fn sections_follow_categories() {
        assert_eq!(app("a", "A", "a", &["Game", "Network"]).section(), Section::Games);
        assert_eq!(app("a", "A", "a", &["Development", "Utility"]).section(), Section::Development);
        assert_eq!(app("a", "A", "a", &["Network", "WebBrowser"]).section(), Section::Internet);
        assert_eq!(app("a", "A", "a", &["AudioVideo"]).section(), Section::Multimedia);
        assert_eq!(app("a", "A", "a", &["TerminalEmulator"]).section(), Section::System);
        assert_eq!(app("a", "A", "a", &["Utility"]).section(), Section::Utilities);
        assert_eq!(app("a", "A", "a", &[]).section(), Section::Other);
        assert_eq!(Section::ALL.len(), 10);
        assert_eq!(Section::Graphics.short(), "Graphics");
        assert_eq!(Section::Office.short(), "Productivity");
    }

    #[test]
    fn match_app_id_strategies() {
        let mut ff = app("org.mozilla.firefox", "Firefox", "firefox %u", &[]);
        ff.wm_class = Some("Navigator".into());
        let apps = vec![
            ff,
            app("org.gnome.Nautilus", "Files", "nautilus --new-window", &[]),
            app("code", "Visual Studio Code", "env ELECTRON=1 /usr/share/code/code", &[]),
            app("calc", "Calculator", "gnome-calculator", &[]),
        ];
        let id = |q: &str| match_app_id(&apps, q).map(|a| a.id.clone());
        assert_eq!(id("org.mozilla.firefox").as_deref(), Some("org.mozilla.firefox"));
        assert_eq!(id("navigator").as_deref(), Some("org.mozilla.firefox"));
        assert_eq!(id("nautilus").as_deref(), Some("org.gnome.Nautilus"));
        assert_eq!(id("firefox").as_deref(), Some("org.mozilla.firefox"));
        assert_eq!(id("code").as_deref(), Some("code"));
        assert_eq!(id("gnome-calculator").as_deref(), Some("calc"));
        assert_eq!(id("org.example.Calculator").as_deref(), Some("calc"));
        assert_eq!(id(""), None);
        assert_eq!(id("unknown"), None);
    }

    #[test]
    fn parse_desktop_reads_entry_section_only() {
        let p = tmp_desktop(
            "aqua-test-ok.desktop",
            "[Desktop Entry]\nType=Application\nName=Test 1.0 App\nExec=sh -c true %F\nIcon=test\n\
             Categories=Utility;TextEditor;\nKeywords=foo;bar;\nGenericName=Editor\nTerminal=true\n\
             StartupWMClass=TestWM\n[Desktop Action new]\nName=Other\nExec=other\n",
        );
        let a = parse_desktop(&p).expect("valid entry");
        assert_eq!(a.id, "aqua-test-ok");
        assert_eq!(a.name, "Test App");
        assert_eq!(a.exec, "sh -c true %F");
        assert_eq!(a.categories, vec!["Utility", "TextEditor"]);
        assert_eq!(a.keywords, vec!["foo", "bar", "Editor"]);
        assert_eq!(a.wm_class.as_deref(), Some("TestWM"));
        assert!(a.terminal);
        assert_eq!(a.section(), Section::Utilities);
    }

    #[test]
    fn parse_desktop_masks_hidden_and_missing_programs() {
        let hidden = tmp_desktop(
            "aqua-test-hidden.desktop",
            "[Desktop Entry]\nType=Application\nName=H\nExec=sh\nNoDisplay=true\n",
        );
        assert!(parse_desktop(&hidden).is_none());
        let missing = tmp_desktop(
            "aqua-test-missing.desktop",
            "[Desktop Entry]\nType=Application\nName=M\nExec=/nonexistent/aqua-xyz\n",
        );
        assert!(parse_desktop(&missing).is_none());
        let link =
            tmp_desktop("aqua-test-link.desktop", "[Desktop Entry]\nType=Link\nName=L\nURL=https://example.org\n");
        assert!(parse_desktop(&link).is_none());
        assert!(matches!(parse_entry(&link, "l".into()), Parsed::Masked));
        let none = tmp_desktop("aqua-test-none.desktop", "garbage");
        assert!(matches!(parse_entry(&none, "n".into()), Parsed::Skip));
    }

    #[test]
    fn find_in_path_checks_executables() {
        assert!(find_in_path("sh").is_some());
        assert!(find_in_path("\"sh\"").is_some());
        assert!(find_in_path("").is_none());
        assert!(find_in_path("/etc/hostname-aqua-missing").is_none());
    }
}
