use crate::manager::{rank, Manager};
use crate::model::{Details, Origin, Package, Permission, Scope, Update};
use crate::runner::{Cmd, SharedRunner};
use crate::units::parse_size;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const FLATHUB: &str = "flathub";
pub const FLATHUB_REPO: &str = "https://dl.flathub.org/repo/flathub.flatpakrepo";

pub struct Flatpak {
    pub run: SharedRunner,
    pub scope: Mutex<Scope>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    pub scope: Scope,
    pub url: String,
}

pub fn parse_list(out: &str) -> Vec<Package> {
    out.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').map(str::trim).collect();
            if f.len() < 6 || f[0].is_empty() || !f[0].contains('.') {
                return None;
            }
            let mut p = Package::new(Origin::Flatpak, f[0]);
            p.appstream_id = f[0].into();
            p.title = if f[1].is_empty() { f[0].rsplit('.').next().unwrap_or(f[0]).into() } else { f[1].into() };
            p.version = f[2].into();
            p.branch = f[3].into();
            p.repo = f[4].into();
            p.scope = Some(Scope::parse(f[5]));
            p.size = f.get(6).and_then(|s| parse_size(s));
            p.installed = true;
            p.installed_version = if f[2].is_empty() { f[3].into() } else { f[2].into() };
            p.is_app = true;
            Some(p)
        })
        .collect()
}

pub fn parse_updates(out: &str, scope: Scope) -> Vec<Update> {
    out.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').map(str::trim).collect();
            if f.is_empty() || !f[0].contains('.') {
                return None;
            }
            Some(Update {
                origin: Some(Origin::Flatpak),
                name: f[0].into(),
                to: f.get(1).copied().filter(|s| !s.is_empty()).or(f.get(2).copied()).unwrap_or("").into(),
                repo: f.get(3).copied().unwrap_or("").into(),
                download_size: f.get(4).and_then(|s| parse_size(s)),
                scope: Some(scope),
                ..Default::default()
            })
        })
        .collect()
}

pub fn parse_remotes(out: &str) -> Vec<Remote> {
    out.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').map(str::trim).collect();
            if f.first().is_none_or(|s| s.is_empty()) {
                return None;
            }
            let opts = f.get(2).copied().unwrap_or("");
            if opts.contains("disabled") {
                return None;
            }
            Some(Remote {
                name: f[0].into(),
                url: f.get(1).copied().unwrap_or("").into(),
                scope: if opts.contains("user") { Scope::User } else { Scope::System },
            })
        })
        .collect()
}

pub fn parse_search(out: &str) -> Vec<Package> {
    out.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').map(str::trim).collect();
            if f.len() < 3 || !f[0].contains('.') {
                return None;
            }
            let mut p = Package::new(Origin::Flatpak, f[0]);
            p.appstream_id = f[0].into();
            p.title = f[1].into();
            p.summary = f[2].into();
            p.version = f.get(3).copied().unwrap_or("").into();
            p.repo = f.get(4).and_then(|r| r.split(',').next()).unwrap_or(FLATHUB).into();
            p.is_app = true;
            Some(p)
        })
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Context {
    pub shared: Vec<String>,
    pub sockets: Vec<String>,
    pub devices: Vec<String>,
    pub filesystems: Vec<String>,
    pub session_talk: Vec<String>,
    pub system_talk: Vec<String>,
}

impl Context {
    pub fn has(list: &[String], v: &str) -> Option<bool> {
        let mut r = None;
        for x in list {
            let x = x.split(':').next().unwrap_or(x);
            if x == v {
                r = Some(true);
            } else if x.strip_prefix('!') == Some(v) {
                r = Some(false);
            }
        }
        r
    }

    pub fn overlay(&self, o: &Context) -> Context {
        let merge = |a: &[String], b: &[String]| {
            let mut v: Vec<String> = a.to_vec();
            for x in b {
                let base = x.trim_start_matches('!').split(':').next().unwrap_or("").to_string();
                v.retain(|y| y.trim_start_matches('!').split(':').next().unwrap_or("") != base);
                v.push(x.clone());
            }
            v
        };
        Context {
            shared: merge(&self.shared, &o.shared),
            sockets: merge(&self.sockets, &o.sockets),
            devices: merge(&self.devices, &o.devices),
            filesystems: merge(&self.filesystems, &o.filesystems),
            session_talk: merge(&self.session_talk, &o.session_talk),
            system_talk: merge(&self.system_talk, &o.system_talk),
        }
    }

    pub fn on(&self, toggle: &str) -> bool {
        let (list, val) = toggle_target(toggle);
        let l = match list {
            "shared" => &self.shared,
            "sockets" => &self.sockets,
            "devices" => &self.devices,
            _ => &self.filesystems,
        };
        Context::has(l, val).unwrap_or(false)
    }
}

pub fn parse_keyfile(text: &str) -> Context {
    let mut c = Context::default();
    let mut section = String::new();
    let split = |v: &str| v.split(';').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect::<Vec<_>>();
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') && l.ends_with(']') {
            section = l[1..l.len() - 1].to_string();
            continue;
        }
        let Some((k, v)) = l.split_once('=') else { continue };
        match (section.as_str(), k.trim()) {
            ("Context", "shared") => c.shared = split(v),
            ("Context", "sockets") => c.sockets = split(v),
            ("Context", "devices") => c.devices = split(v),
            ("Context", "filesystems") => c.filesystems = split(v),
            ("Session Bus Policy", name) if v.trim() == "talk" || v.trim() == "own" => c.session_talk.push(name.into()),
            ("System Bus Policy", name) if v.trim() == "talk" || v.trim() == "own" => c.system_talk.push(name.into()),
            _ => {}
        }
    }
    c
}

pub fn context_from_json(v: &serde_json::Value) -> Context {
    let list = |k: &str| -> Vec<String> {
        v.get(k)
            .and_then(|a| a.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    };
    let talk = |k: &str| -> Vec<String> {
        v.get(k)
            .and_then(|b| b.get("talk"))
            .and_then(|a| a.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    };
    Context {
        shared: list("shared"),
        sockets: list("sockets"),
        devices: list("devices"),
        filesystems: list("filesystems"),
        session_talk: talk("session-bus"),
        system_talk: talk("system-bus"),
    }
}

pub fn describe(c: &Context) -> Vec<Permission> {
    let mut v = vec![];
    let mut push = |id: &str, label: &str, detail: &str, risky: bool| {
        v.push(Permission { id: id.into(), label: label.into(), detail: detail.into(), risky });
    };
    if Context::has(&c.shared, "network") == Some(true) {
        push("network", "Network Access", "Can access the internet", false);
    }
    let fs: Vec<&String> = c.filesystems.iter().filter(|f| !f.starts_with('!')).collect();
    let ro = |f: &str| f.ends_with(":ro");
    if let Some(f) = fs.iter().find(|f| f.starts_with("host")) {
        if f.starts_with("host-os") {
            push("host-os", "System Libraries", "Can read system libraries and executables", true);
        } else if f.starts_with("host-etc") {
            push("host-etc", "System Configuration", "Can read system configuration", true);
        } else {
            push(
                "host",
                "Full File System Access",
                if ro(f) { "Can read all your files" } else { "Can read and write all your files" },
                true,
            );
        }
    } else if let Some(f) = fs.iter().find(|f| f.starts_with("home") || f.starts_with('~')) {
        push(
            "home",
            "Home Folder Access",
            if ro(f) { "Can read files in your home folder" } else { "Can read and write files in your home folder" },
            true,
        );
    }
    let named: Vec<&str> = fs
        .iter()
        .filter_map(|f| {
            let base = f.split(':').next().unwrap_or("");
            match base {
                "xdg-download" => Some("Downloads"),
                "xdg-documents" => Some("Documents"),
                "xdg-pictures" => Some("Pictures"),
                "xdg-music" => Some("Music"),
                "xdg-videos" => Some("Videos"),
                "xdg-desktop" => Some("Desktop"),
                _ => None,
            }
        })
        .collect();
    if !named.is_empty() {
        push("folders", "Folder Access", &format!("Can access {}", named.join(", ")), false);
    }
    if Context::has(&c.devices, "all") == Some(true) {
        push("devices", "Device Access", "Can access cameras, controllers and other devices", true);
    }
    if Context::has(&c.sockets, "x11") == Some(true) && Context::has(&c.sockets, "wayland") != Some(true) {
        push("x11", "Legacy Display System", "Uses X11, which lets it see other windows", true);
    }
    if Context::has(&c.sockets, "pulseaudio") == Some(true) {
        push("sound", "Sound", "Can play sounds and use the microphone", false);
    }
    if c.session_talk.iter().any(|s| s == "org.freedesktop.Flatpak") {
        push("sandbox-escape", "Unrestricted System Access", "Can run commands outside its sandbox", true);
    }
    if !c.system_talk.is_empty() || Context::has(&c.sockets, "system-bus") == Some(true) {
        push("system-bus", "System Services", "Can talk to system services", true);
    }
    let empty = v.is_empty();
    if empty {
        v.push(Permission {
            id: "none".into(),
            label: "No Permissions".into(),
            detail: "Runs fully sandboxed".into(),
            risky: false,
        });
    }
    v
}

pub const TOGGLES: [(&str, &str); 6] = [
    ("network", "Network"),
    ("home", "Home Folder"),
    ("host", "All Files"),
    ("devices", "Devices"),
    ("sound", "Sound"),
    ("x11", "X11 Display"),
];

fn toggle_target(t: &str) -> (&'static str, &'static str) {
    match t {
        "network" => ("shared", "network"),
        "home" => ("filesystems", "home"),
        "host" => ("filesystems", "host"),
        "devices" => ("devices", "all"),
        "sound" => ("sockets", "pulseaudio"),
        "x11" => ("sockets", "x11"),
        _ => ("filesystems", "none"),
    }
}

pub fn override_cmd(app: &str, toggle: &str, on: bool) -> Cmd {
    let (list, val) = toggle_target(toggle);
    let flag = match (list, on) {
        ("shared", true) => format!("--share={val}"),
        ("shared", false) => format!("--unshare={val}"),
        ("sockets", true) => format!("--socket={val}"),
        ("sockets", false) => format!("--nosocket={val}"),
        ("devices", true) => format!("--device={val}"),
        ("devices", false) => format!("--nodevice={val}"),
        (_, true) => format!("--filesystem={val}"),
        (_, false) => format!("--nofilesystem={val}"),
    };
    Cmd::new(["flatpak", "override", "--user", &flag, app])
}

pub fn data_dir(app: &str) -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".var/app").join(app)
}

pub fn exports_dirs() -> Vec<PathBuf> {
    let mut v = vec![];
    if let Some(d) = dirs::data_dir() {
        v.push(d.join("flatpak/exports/share/applications"));
    }
    v.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
    v
}

pub fn app_of_export(p: &Path) -> Option<String> {
    if !p.to_string_lossy().contains("/flatpak/exports/") {
        return None;
    }
    Some(p.file_stem()?.to_string_lossy().into_owned())
}

impl Flatpak {
    pub fn new(run: SharedRunner, scope: Scope) -> Flatpak {
        Flatpak { run, scope: Mutex::new(scope) }
    }

    pub fn scope(&self) -> Scope {
        *self.scope.lock().unwrap()
    }

    pub fn remotes(&self) -> Vec<Remote> {
        let o = self.run.run(&Cmd::new(["flatpak", "remotes", "--columns=name,url,options"]));
        parse_remotes(&o.stdout)
    }

    pub fn has_flathub(&self, scope: Scope) -> bool {
        self.remotes().iter().any(|r| r.scope == scope && (r.name == FLATHUB || r.url.contains("flathub.org")))
    }

    pub fn add_flathub(scope: Scope) -> Cmd {
        Cmd::new(["flatpak", "remote-add", "--if-not-exists", scope.flag(), FLATHUB, FLATHUB_REPO])
            .label("Adding Flathub")
    }

    pub fn permissions(&self, app: &str, scope: Option<Scope>) -> (Context, Context) {
        let mut c = Cmd::new(["flatpak", "info", "--show-permissions"]);
        if let Some(s) = scope {
            c = c.arg(s.flag());
        }
        let base = parse_keyfile(&self.run.run(&c.arg(app)).stdout);
        let ov = parse_keyfile(&self.run.run(&Cmd::new(["flatpak", "override", "--user", "--show", app])).stdout);
        (base, ov)
    }

    pub fn reset_overrides(app: &str) -> Cmd {
        Cmd::new(["flatpak", "override", "--user", "--reset", app])
    }

    pub fn remove_unused() -> Vec<Cmd> {
        vec![Cmd::new(["flatpak", "uninstall", "--unused", "-y", "--noninteractive"]).label("Removing unused runtimes")]
    }

    pub fn repair(scope: Scope) -> Cmd {
        Cmd::new(["flatpak", "repair", scope.flag()]).label("Repairing Flatpak installation")
    }
}

impl Manager for Flatpak {
    fn origin(&self) -> Origin {
        Origin::Flatpak
    }

    fn available(&self) -> bool {
        self.run.have("flatpak")
    }

    fn installed(&self) -> Vec<Package> {
        let o = self.run.run(&Cmd::new([
            "flatpak",
            "list",
            "--app",
            "--columns=application,name,version,branch,origin,installation,size",
        ]));
        parse_list(&o.stdout)
    }

    fn search(&self, query: &str) -> Vec<Package> {
        if query.trim().is_empty() {
            return vec![];
        }
        let o = self.run.run(
            &Cmd::new(["flatpak", "search", "--columns=application,name,description,version,remotes"])
                .args(query.split_whitespace()),
        );
        let mut v = parse_search(&o.stdout);
        rank(&mut v, query);
        v
    }

    fn info(&self, name: &str) -> Option<Details> {
        let p = self.installed().into_iter().find(|p| p.name == name)?;
        let (base, ov) = self.permissions(name, p.scope);
        let perms = describe(&base.overlay(&ov));
        Some(Details { pkg: p, permissions: perms, ..Default::default() })
    }

    fn updates(&self) -> Vec<Update> {
        let mut v = vec![];
        for scope in [Scope::User, Scope::System] {
            let o = self.run.run(&Cmd::new([
                "flatpak",
                "remote-ls",
                "--updates",
                scope.flag(),
                "--columns=application,version,branch,origin,download-size",
            ]));
            v.extend(parse_updates(&o.stdout, scope));
        }
        let apps: HashMap<String, String> =
            self.installed().into_iter().map(|p| (p.name.clone(), p.installed_version.clone())).collect();
        for u in &mut v {
            if let Some(ver) = apps.get(&u.name) {
                u.is_app = true;
                u.from = ver.clone();
            }
        }
        v
    }

    fn owners(&self, files: &[PathBuf]) -> HashMap<PathBuf, String> {
        files.iter().filter_map(|f| Some((f.clone(), app_of_export(f)?))).collect()
    }

    fn install(&self, pkg: &Package) -> Vec<Cmd> {
        let scope = pkg.scope.unwrap_or(self.scope());
        let remote = if pkg.repo.is_empty() { FLATHUB } else { &pkg.repo };
        vec![Cmd::new(["flatpak", "install", "-y", "--noninteractive", scope.flag(), remote, &pkg.name])
            .label(format!("Installing {}", pkg.display_name()))]
    }

    fn remove(&self, pkg: &Package, purge: bool) -> Vec<Cmd> {
        let mut c = Cmd::new(["flatpak", "uninstall", "-y", "--noninteractive"]);
        if let Some(s) = pkg.scope {
            c = c.arg(s.flag());
        }
        if purge {
            c = c.arg("--delete-data");
        }
        vec![c.arg(&pkg.name).label(format!("Removing {}", pkg.display_name()))]
    }

    fn update(&self, names: &[String]) -> Vec<Cmd> {
        if names.is_empty() {
            return [Scope::User, Scope::System]
                .iter()
                .map(|s| {
                    Cmd::new(["flatpak", "update", "-y", "--noninteractive", s.flag()]).label("Updating Flatpak apps")
                })
                .collect();
        }
        vec![Cmd::new(["flatpak", "update", "-y", "--noninteractive"]).args(names.iter().cloned()).label("Updating")]
    }

    fn refresh(&self) -> Vec<Cmd> {
        vec![Cmd::new(["flatpak", "update", "--appstream"]).label("Refreshing Flathub catalog")]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list() {
        let v = parse_list(
            "org.gimp.GIMP\tGNU Image Manipulation Program\t3.0.4\tstable\tflathub\tsystem\t312.4 MB\ncom.x.Y\t\t\tstable\tflathub\tuser\t1 kB\n",
        );
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].scope, Some(Scope::System));
        assert_eq!(v[0].size, Some(312_400_000));
        assert_eq!(v[1].title, "Y");
        assert_eq!(v[1].installed_version, "stable");
    }

    #[test]
    fn updates_and_remotes() {
        let u = parse_updates(
            "org.gimp.GIMP\t3.0.6\tstable\tflathub\t21.0 MB\norg.gnome.Platform\t\t48\tflathub\t100 MB\n",
            Scope::User,
        );
        assert_eq!(u.len(), 2);
        assert_eq!(u[1].to, "48");
        assert_eq!(u[0].download_size, Some(21_000_000));
        let r = parse_remotes("flathub\thttps://dl.flathub.org/repo/\tsystem\nflathub\thttps://dl.flathub.org/repo/\tuser\nold\thttp://x\tsystem,disabled\n");
        assert_eq!(r.len(), 2);
        assert_eq!(r[1].scope, Scope::User);
    }

    #[test]
    fn permissions() {
        let base = parse_keyfile("[Application]\nname=x\n\n[Context]\nshared=network;ipc;\nsockets=x11;pulseaudio;\ndevices=all;\nfilesystems=home;xdg-download:ro;\n\n[Session Bus Policy]\norg.freedesktop.Flatpak=talk\n");
        assert!(base.on("network") && base.on("home") && base.on("x11"));
        let d = describe(&base);
        let ids: Vec<&str> = d.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["network", "home", "folders", "devices", "x11", "sound", "sandbox-escape"]);
        let ov = parse_keyfile("[Context]\nshared=!network;\nfilesystems=!home;\n");
        let eff = base.overlay(&ov);
        assert!(!eff.on("network"));
        assert!(!eff.on("home"));
        assert!(eff.on("devices"));
        assert_eq!(describe(&Context::default())[0].id, "none");
    }

    #[test]
    fn json_context() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"filesystems":["host","/tmp"],"shared":["network"],"sockets":["wayland","fallback-x11"],"session-bus":{"talk":["org.x"]}}"#,
        )
        .unwrap();
        let c = context_from_json(&v);
        assert_eq!(describe(&c)[1].id, "host");
        assert_eq!(c.session_talk, vec!["org.x"]);
    }

    #[test]
    fn override_flags() {
        assert_eq!(
            override_cmd("a.b", "network", false).argv,
            vec!["flatpak", "override", "--user", "--unshare=network", "a.b"]
        );
        assert_eq!(override_cmd("a.b", "home", true).argv[3], "--filesystem=home");
        assert_eq!(override_cmd("a.b", "sound", false).argv[3], "--nosocket=pulseaudio");
    }

    #[test]
    fn install_uses_scope() {
        let f = Flatpak::new(std::sync::Arc::new(crate::runner::fake::FakeRunner::default()), Scope::User);
        let mut p = Package::new(Origin::Flatpak, "org.gimp.GIMP");
        assert_eq!(
            f.install(&p)[0].argv,
            vec!["flatpak", "install", "-y", "--noninteractive", "--user", "flathub", "org.gimp.GIMP"]
        );
        p.scope = Some(Scope::System);
        assert!(f.remove(&p, true)[0].argv.contains(&"--delete-data".to_string()));
        assert!(!f.install(&p)[0].root);
        assert_eq!(
            app_of_export(Path::new("/var/lib/flatpak/exports/share/applications/org.gimp.GIMP.desktop")).unwrap(),
            "org.gimp.GIMP"
        );
    }
}
